/// Long-term output archive backed by SQLite.
///
/// Lines are written via an unbounded channel to a background thread that
/// batches inserts (every 500 ms or 100 lines) inside a single transaction.
/// Search and scrollback-load open short-lived read-only connections so they
/// never block the writer.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use rusqlite::{Connection, params};

pub struct ScrollbackLine {
    pub ts_ms: i64,
    pub world: String,
    pub text: String,
}

/// (world_name, ts_ms_unix, ansi_text)
pub type ArchiveEntry = (String, i64, String);

pub struct ScrollbackDb {
    tx: mpsc::SyncSender<ArchiveEntry>,
    path: PathBuf,
}

impl ScrollbackDb {
    /// Return a cloneable sender that worlds can use to stream lines to the archive.
    pub fn sender(&self) -> mpsc::SyncSender<ArchiveEntry> {
        self.tx.clone()
    }
}

impl ScrollbackDb {
    /// Open (or create) the archive database and start the background writer thread.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS output_log (
                id        INTEGER PRIMARY KEY,
                ts_ms     INTEGER NOT NULL,
                world     TEXT NOT NULL,
                line_raw  TEXT NOT NULL,
                line_text TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_world_ts ON output_log(world, ts_ms);
            CREATE VIRTUAL TABLE IF NOT EXISTS output_fts USING fts5(
                line_text,
                content='output_log',
                content_rowid='id'
            );
        ")?;
        drop(conn);

        let db_path = path.to_path_buf();
        // Bound the channel to avoid unbounded memory growth under heavy load
        let (tx, rx) = mpsc::sync_channel::<ArchiveEntry>(4096);
        let writer_path = db_path.clone();

        std::thread::Builder::new()
            .name("scrollback-writer".to_string())
            .spawn(move || {
                let conn = match Connection::open(&writer_path) {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut batch: Vec<(String, i64, String, String)> = Vec::new();
                let mut last_flush = Instant::now();

                loop {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok((world, ts_ms, text)) => {
                            let line_text = crate::util::strip_ansi_codes(&text);
                            batch.push((world, ts_ms, text, line_text));
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            flush_batch(&conn, &mut batch);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }

                    let should_flush = batch.len() >= 100
                        || (!batch.is_empty() && last_flush.elapsed() >= Duration::from_millis(500));

                    if should_flush {
                        flush_batch(&conn, &mut batch);
                        last_flush = Instant::now();
                    }
                }
            })
            .ok();

        Ok(Self { tx, path: db_path })
    }

    /// Queue a line for archiving. Non-blocking (drops if channel is full).
    pub fn append(&self, world: &str, ts_ms: i64, text: &str) {
        let _ = self.tx.try_send((world.to_string(), ts_ms, text.to_string()));
    }

    /// Load up to `count` lines that precede `before_ts_ms` for the given world.
    /// Returns lines in ascending timestamp order (oldest first).
    pub fn load_before(&self, world: &str, before_ts_ms: i64, count: usize) -> Vec<ScrollbackLine> {
        load_before_path(&self.path, world, before_ts_ms, count)
    }

    /// Full-text / glob / regex search across the archive.
    pub fn search(
        path: &Path,
        world: Option<&str>,
        pattern: &str,
        since_ms: Option<i64>,
        until_ms: Option<i64>,
        limit: usize,
        use_regex: bool,
    ) -> Vec<ScrollbackLine> {
        let conn = match Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        // Build WHERE clause
        let mut conditions: Vec<String> = Vec::new();
        if let Some(w) = world {
            conditions.push(format!("world = '{}'", w.replace('\'', "''")));
        }
        if let Some(since) = since_ms {
            conditions.push(format!("ts_ms >= {}", since));
        }
        if let Some(until) = until_ms {
            conditions.push(format!("ts_ms <= {}", until));
        }

        // For FTS we search on line_text; for regex we do it in Rust after fetching
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT ts_ms, world, line_raw, line_text FROM output_log {} ORDER BY ts_ms ASC",
            where_clause
        );

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        // Compile pattern
        let regex_opt = if use_regex {
            regex::Regex::new(pattern).ok()
        } else {
            // Treat as case-insensitive glob
            let re_pat = crate::actions::wildcard_to_regex(pattern);
            regex::RegexBuilder::new(&re_pat)
                .case_insensitive(true)
                .build()
                .ok()
        };

        let rows = stmt.query_map([], |row| {
            Ok(ScrollbackLine {
                ts_ms: row.get(0)?,
                world: row.get(1)?,
                text: row.get(2)?,
            })
        });

        let mut results: Vec<ScrollbackLine> = Vec::new();
        if let Ok(iter) = rows {
            for item in iter.flatten() {
                let matches = match &regex_opt {
                    Some(re) => {
                        // Match against stripped text (col 3) — but we already have only text
                        // Re-strip from text for matching
                        let plain = crate::util::strip_ansi_codes(&item.text);
                        re.is_match(&plain)
                    }
                    None => true,
                };
                if matches {
                    results.push(item);
                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }
        results
    }
}

pub fn load_before_path(path: &Path, world: &str, before_ts_ms: i64, count: usize) -> Vec<ScrollbackLine> {
    let conn = match Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Fetch the N rows just before the timestamp, then reverse to get oldest-first
    let sql = "SELECT ts_ms, world, line_raw FROM output_log \
               WHERE world = ?1 AND ts_ms < ?2 \
               ORDER BY ts_ms DESC LIMIT ?3";

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map(params![world, before_ts_ms, count as i64], |row| {
        Ok(ScrollbackLine {
            ts_ms: row.get(0)?,
            world: row.get(1)?,
            text: row.get(2)?,
        })
    });

    let mut lines: Vec<ScrollbackLine> = rows
        .map(|iter| iter.flatten().collect())
        .unwrap_or_default();

    // Re-order oldest-first
    lines.reverse();
    lines
}

fn flush_batch(conn: &Connection, batch: &mut Vec<(String, i64, String, String)>) {
    if batch.is_empty() {
        return;
    }
    let tx = match conn.unchecked_transaction() {
        Ok(t) => t,
        Err(_) => { batch.clear(); return; }
    };
    for (world, ts_ms, line_raw, line_text) in batch.drain(..) {
        let _ = tx.execute(
            "INSERT INTO output_log (ts_ms, world, line_raw, line_text) VALUES (?1, ?2, ?3, ?4)",
            params![ts_ms, world, line_raw, line_text],
        );
    }
    let _ = tx.commit();
}
