//! Long-term output archive backed by SQLite.
//!
//! Lines are written via an unbounded channel to a background thread that
//! batches inserts (every 500 ms or 100 lines) inside a single transaction.
//! Search and scrollback-load open short-lived read-only connections so they
//! never block the writer.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use rusqlite::{Connection, params};

/// Current archive schema (since v1.5.41). A file in any other shape is refused,
/// never converted: see `ArchiveState`.
const SCHEMA_VERSION: i32 = 2;

/// Sequence numbers reserved per allocation. A restart burns the unused
/// remainder, which is harmless because every consumer compares with `<`, never
/// `== previous + 1`. At 65536 per process start, 200 starts/day for 50 years
/// wastes ~2.4e11 — negligible against i64's 9.2e18.
const WSEQ_BLOCK: i64 = 65_536;

/// Upper bound for a sequence number. 2^53 keeps it exactly representable as a
/// JSON number, so `wseq` stays safe if it is ever put on the WebSocket wire.
const WSEQ_MAX: i64 = 1 << 53;

/// Candidate count at which the FTS index stops being worth using. Above this the
/// term is too common to narrow anything down and the plain indexed scan wins.
const FTS_SELECTIVITY_CAP: i64 = 20_000;

/// Seconds a connection waits on a lock before giving up. Without this, a reader
/// that collides with the writer's transaction gets SQLITE_BUSY immediately and
/// every read path here turns that into an empty result — indistinguishable from
/// "nothing archived", which is a silent wrong answer rather than an error.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// Queue depth at which we start warning. Not a limit; just a smell.
const QUEUE_SOFT_CAP: usize = 500_000;
/// Queue depth at which we start refusing new entries to bound memory. Anything
/// refused here is COUNTED (see `ArchiveStats::dropped`) and reported, never
/// silently discarded the way the old bounded-channel `try_send` did.
const QUEUE_HARD_CAP: usize = 2_000_000;

pub struct ScrollbackLine {
    pub ts_ms: i64,
    pub world: String,
    pub text: String,
    /// Durable archive sequence, when the reader selected it (`search` and
    /// `load_before_path` do not).
    pub wseq: Option<i64>,
    /// The line was gagged when it arrived. Filled only by the Page Up readers
    /// (`load_before_wseq`/`load_before_path`), which show it the way it was shown live;
    /// a search's results leave it false.
    pub gagged: bool,
}

/// A search against the archive.
///
/// `before_wseq` is the whole point of the v2 schema: it excludes exactly the
/// rows the caller's in-memory buffer already covers, using a key that cannot
/// collide. The old timestamp cut could not make that guarantee — equal `ts_ms`
/// values straddled the boundary in both directions.
/// How `pattern` should be interpreted. Must mirror `execute_recall`'s own
/// semantics (src/actions.rs): the archive pre-filter deciding differently from
/// the final filter means rows are dropped before anything can match them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PatternKind {
    /// Substring, case-insensitive (`regex::escape`).
    Simple,
    /// `*`/`?` wildcards, anchored.
    Glob,
    /// Raw regex.
    Regex,
}

pub struct ArchiveQuery<'a> {
    pub world: Option<&'a str>,
    pub pattern: &'a str,
    pub kind: PatternKind,
    pub before_wseq: Option<i64>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub limit: usize,
    /// Take the most recent matches rather than the oldest. The legacy `search`
    /// always took the oldest, which on a large archive silently hid recent
    /// history behind the limit.
    pub newest_first: bool,
}

/// What `open` found on disk.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ArchiveState {
    /// Schema is current and usable.
    Ready,
    /// No archive yet: `open` creates the v2 schema.
    Empty,
    /// An archive in the layout before v1.5.41's, which no 1.7 Clay can have left
    /// behind (each upgraded it on first open). Refused like `TooNew`.
    Old,
    /// `user_version` is ahead of this build. Refused: a newer Clay owns the file
    /// and guessing at its shape risks destroying it.
    TooNew,
}

/// The v2 schema. Written verbatim for a fresh database, and re-run on open to
/// put back any auxiliary object a file is missing.
const SCHEMA_V2: &str = "
    CREATE TABLE IF NOT EXISTS worlds (
        wid        INTEGER PRIMARY KEY,
        world_uuid TEXT NOT NULL UNIQUE,
        name       TEXT NOT NULL,
        next_wseq  INTEGER NOT NULL DEFAULT 0,
        orphan     INTEGER NOT NULL DEFAULT 0,
        created_ms INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS archive_meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
    CREATE VIRTUAL TABLE IF NOT EXISTS output_fts USING fts5(
        txt, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2'
    );
";

/// `output_log` in its v2 shape, with its two indexes.
const OUTPUT_LOG_V2: &str = "
    CREATE TABLE IF NOT EXISTS output_log (
        id       INTEGER PRIMARY KEY,
        wid      INTEGER NOT NULL,
        wseq     INTEGER NOT NULL,
        ts_ms    INTEGER NOT NULL,
        line_raw TEXT NOT NULL,
        gagged   INTEGER NOT NULL DEFAULT 0
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_log_wid_wseq ON output_log(wid, wseq);
    CREATE INDEX IF NOT EXISTS idx_log_wid_ts ON output_log(wid, ts_ms);
";

fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT v FROM archive_meta WHERE k = ?1", params![key], |r| r.get(0))
        .ok()
}

fn meta_set(conn: &Connection, key: &str, value: &str) {
    let _ = conn.execute(
        "INSERT INTO archive_meta (k, v) VALUES (?1, ?2)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![key, value],
    );
}

fn user_version(conn: &Connection) -> i32 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0)
}

/// Does `table` have a column called `column`?
fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map(|it| it.flatten().collect())
        .unwrap_or_default();
    cols.iter().any(|c| c == column)
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |r| r.get::<_, i64>(0),
    )
    .is_ok()
}

/// Read the archive's state without opening a writer. Used by the read paths,
/// which must not assume the schema they were compiled against.
fn archive_state(conn: &Connection) -> ArchiveState {
    match user_version(conn) {
        v if v == SCHEMA_VERSION => ArchiveState::Ready,
        v if v > SCHEMA_VERSION => ArchiveState::TooNew,
        _ if !table_exists(conn, "output_log") => ArchiveState::Empty,
        // A v2 file whose stamp never landed (an upgrade killed after its table swap)
        // is still a v2 file.
        _ if has_column(conn, "output_log", "wid") => ArchiveState::Ready,
        _ => ArchiveState::Old,
    }
}

/// One line on its way to the archive.
///
/// `wid`/`wseq` are filled in by the PRODUCER when it could reach the allocator,
/// so the live `OutputLine` knows its durable sequence number the instant it
/// lands in `output_lines` — which is when `/recall -D` needs it. When they are
/// `None` (allocator unreachable: read-only filesystem, disk full, a lock held
/// too long) the writer assigns one itself rather than dropping the line.
pub struct ArchiveEntry {
    pub world: String,
    pub ts_ms: i64,
    pub text: String,
    pub gagged: bool,
    pub wid: Option<i64>,
    pub wseq: Option<i64>,
}

impl ArchiveEntry {
    pub fn new(world: String, ts_ms: i64, text: String, gagged: bool) -> Self {
        Self { world, ts_ms, text, gagged, wid: None, wseq: None }
    }
}

/// Hands out durable per-world sequence numbers.
///
/// Reserves `WSEQ_BLOCK` at a time under `BEGIN IMMEDIATE`, so two Clay processes
/// sharing one archive get disjoint blocks rather than colliding. The reserved
/// remainder is burned on restart, which is fine because every consumer compares
/// with `<`, never `== previous + 1`.
struct WseqAllocator {
    conn: Connection,
    /// world_uuid -> (wid, next, limit)
    blocks: std::collections::HashMap<String, (i64, i64, i64)>,
}

impl WseqAllocator {
    fn alloc(&mut self, world_uuid: &str, name: &str) -> Option<(i64, i64)> {
        if let Some((wid, next, limit)) = self.blocks.get_mut(world_uuid) {
            if *next < *limit {
                let out = (*wid, *next);
                *next += 1;
                return Some(out);
            }
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let tx = self.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).ok()?;
        tx.execute(
            "INSERT INTO worlds (world_uuid, name, next_wseq, orphan, created_ms)
             VALUES (?1, ?2, 0, 0, ?3)
             ON CONFLICT(world_uuid) DO UPDATE SET name = excluded.name, orphan = 0",
            params![world_uuid, name, now_ms],
        ).ok()?;
        let (wid, base): (i64, i64) = tx
            .query_row(
                "SELECT wid, next_wseq FROM worlds WHERE world_uuid = ?1",
                params![world_uuid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        // Refuse to cross the JS-safe integer bound rather than wrapping. At any
        // plausible rate this is ~5000x more headroom than 50 years needs.
        let limit = base.checked_add(WSEQ_BLOCK)?;
        if limit >= WSEQ_MAX {
            crate::debug_log(true, "SCROLLBACK: wseq space exhausted; archiving without sequence numbers");
            return None;
        }
        tx.execute("UPDATE worlds SET next_wseq = ?2 WHERE wid = ?1", params![wid, limit]).ok()?;
        tx.commit().ok()?;
        self.blocks.insert(world_uuid.to_string(), (wid, base + 1, limit));
        Some((wid, base))
    }
}

/// Counters describing archive health. Exposed so the UI can say "N lines were
/// not archived" instead of quietly losing them.
#[derive(Default)]
pub struct ArchiveStats {
    /// Entries queued but not yet written.
    pub depth: AtomicUsize,
    /// Entries refused at the hard cap.
    pub dropped: AtomicU64,
    /// Rows the writer could not insert after exhausting its retries.
    pub write_failed: AtomicU64,
    /// Set once when the queue first crosses the soft cap, so the log gets one
    /// line rather than one per line archived.
    warned: AtomicBool,
}

/// Cloneable handle a `World` uses to stream lines to the archive.
///
/// Wraps an UNBOUNDED channel. The previous `SyncSender` + `try_send` pair
/// dropped lines on the floor with the `Result` discarded whenever the buffer
/// filled — and a writer held up for a few seconds (a lock, a slow disk) would
/// fill it.
#[derive(Clone)]
pub struct ArchiveSender {
    tx: mpsc::Sender<ArchiveEntry>,
    stats: Arc<ArchiveStats>,
    alloc: Arc<std::sync::Mutex<Option<WseqAllocator>>>,
}

impl ArchiveSender {
    /// Reserve the next durable sequence number for a world. `None` means the
    /// allocator is unreachable; the caller must still archive the line (the
    /// writer will assign one) — never skip it.
    pub fn alloc_wseq(&self, world_uuid: &str, name: &str) -> Option<(i64, i64)> {
        let mut guard = self.alloc.lock().ok()?;
        guard.as_mut()?.alloc(world_uuid, name)
    }
}

impl ArchiveSender {
    /// Queue a line. Never blocks. Returns false only if the line was not
    /// queued, which is always counted in `ArchiveStats::dropped`.
    pub fn send(&self, entry: ArchiveEntry) -> bool {
        let depth = self.stats.depth.load(Ordering::Relaxed);
        if depth >= QUEUE_HARD_CAP {
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if depth >= QUEUE_SOFT_CAP && !self.stats.warned.swap(true, Ordering::Relaxed) {
            crate::debug_log(true, &format!(
                "SCROLLBACK: archive queue depth {depth} exceeds soft cap {QUEUE_SOFT_CAP}; \
                 the writer is falling behind"
            ));
        }
        self.stats.depth.fetch_add(1, Ordering::Relaxed);
        if self.tx.send(entry).is_err() {
            // Writer thread is gone (it failed to open the database). Undo the
            // depth bump and count the loss.
            self.stats.depth.fetch_sub(1, Ordering::Relaxed);
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    pub fn stats(&self) -> &Arc<ArchiveStats> {
        &self.stats
    }
}

pub struct ScrollbackDb {
    tx: ArchiveSender,
    path: PathBuf,
    stats: Arc<ArchiveStats>,
    shutdown: Arc<AtomicBool>,
    writer: Option<std::thread::JoinHandle<()>>,
}

impl ScrollbackDb {
    /// Return a cloneable sender that worlds can use to stream lines to the archive.
    pub fn sender(&self) -> ArchiveSender {
        self.tx.clone()
    }

    pub fn stats(&self) -> &Arc<ArchiveStats> {
        &self.stats
    }

    /// Path of the database this handle writes to. The read helpers are free
    /// functions taking an explicit path, so callers holding a `ScrollbackDb`
    /// need this to query the same file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Flush everything queued and stop the writer thread.
    ///
    /// The writer used to flush only when the channel disconnected, which never
    /// happens on the hot-reload path (`std::process::exit` with senders still
    /// alive in every `World`), so up to a full batch was lost on every reload.
    /// Call this before quitting or re-execing.
    pub fn flush_and_close(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.writer.take() {
            // The writer wakes at least every 50ms, so this returns promptly.
            let _ = handle.join();
        }
    }
}

/// Apply the pragmas every connection in this module needs. `busy_timeout` is
/// the important one; see BUSY_TIMEOUT_MS.
fn tune_connection(conn: &Connection) {
    let _ = conn.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS as u64));
    // NORMAL is the correct durability level under WAL and much faster than FULL.
    let _ = conn.execute_batch("PRAGMA synchronous=NORMAL;");
}

/// Open a read-only connection with the shared pragmas applied.
fn open_reader(path: &Path) -> Option<Connection> {
    let conn =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    tune_connection(&conn);
    Some(conn)
}

impl ScrollbackDb {
    /// Open (or create) the archive database and start the background writer thread.
    ///
    /// A file that is not a v2 archive (`ArchiveState::Old` or `TooNew`) is refused:
    /// it is left exactly as it is (not even switched to WAL), and every line sent
    /// to it is counted in `ArchiveStats::write_failed`, so `/info` says so.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        tune_connection(&conn);

        let state = archive_state(&conn);
        match state {
            ArchiveState::TooNew => crate::debug_log(true, &format!(
                "SCROLLBACK: {} was written by a newer Clay (user_version {}); \
                 not reading or writing it",
                path.display(), user_version(&conn)
            )),
            ArchiveState::Old => crate::debug_log(true, &format!(
                "SCROLLBACK: {} is an archive from before Clay 1.5.41; not reading or \
                 writing it (move it aside to start a new one)",
                path.display()
            )),
            ArchiveState::Empty => {
                conn.execute_batch("PRAGMA journal_mode=WAL;")?;
                conn.execute_batch(SCHEMA_V2)?;
                conn.execute_batch(OUTPUT_LOG_V2)?;
                // Says the FTS index covers every row (see `query`); an archive whose
                // index is incomplete lacks it and is searched without the index.
                meta_set(&conn, "migration_state", "done");
                conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
            }
            ArchiveState::Ready => {
                // Put back any auxiliary object the file is missing.
                conn.execute_batch("PRAGMA journal_mode=WAL;")?;
                conn.execute_batch(SCHEMA_V2)?;
                conn.execute_batch(OUTPUT_LOG_V2)?;
            }
        }
        drop(conn);
        let writable = matches!(state, ArchiveState::Empty | ArchiveState::Ready);

        let db_path = path.to_path_buf();
        let (tx, rx) = mpsc::channel::<ArchiveEntry>();
        let writer_path = db_path.clone();
        let stats = Arc::new(ArchiveStats::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let writer_stats = Arc::clone(&stats);
        let writer_shutdown = Arc::clone(&shutdown);

        let writer = std::thread::Builder::new()
            .name("scrollback-writer".to_string())
            .spawn(move || {
                let conn = match Connection::open(&writer_path) {
                    Ok(c) => c,
                    Err(e) => {
                        crate::debug_log(true, &format!(
                            "SCROLLBACK: writer could not open {}: {e}", writer_path.display()
                        ));
                        return;
                    }
                };
                tune_connection(&conn);
                let mut wid_cache: std::collections::HashMap<String, (i64, i64)> =
                    std::collections::HashMap::new();

                let mut batch: Vec<ArchiveEntry> = Vec::new();
                let mut last_flush = Instant::now();
                let mut backoff_ms: u64 = 0;

                loop {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(entry) => {
                            writer_stats.depth.fetch_sub(1, Ordering::Relaxed);
                            batch.push(entry);
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            // Every sender is gone, so anything still buffered is all
                            // there will ever be — drain it rather than one batch.
                            while !batch.is_empty() {
                                let before = batch.len();
                                flush_batch(&conn, &mut batch, &writer_stats, writable, &mut wid_cache);
                                if batch.len() == before {
                                    std::thread::sleep(Duration::from_millis(20));
                                    flush_batch(&conn, &mut batch, &writer_stats, writable, &mut wid_cache);
                                    if batch.len() == before {
                                        writer_stats
                                            .write_failed
                                            .fetch_add(batch.len() as u64, Ordering::Relaxed);
                                        break;
                                    }
                                }
                            }
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }

                    // Drain anything else already queued before deciding to flush,
                    // so a burst becomes one transaction rather than many.
                    while batch.len() < 1000 {
                        match rx.try_recv() {
                            Ok(entry) => {
                                writer_stats.depth.fetch_sub(1, Ordering::Relaxed);
                                batch.push(entry);
                            }
                            Err(_) => break,
                        }
                    }

                    if writer_shutdown.load(Ordering::SeqCst) {
                        // Drain EVERYTHING still queued, not just the current batch.
                        // Stopping after one batch silently abandoned the rest of the
                        // queue — on a shutdown with 20k lines pending that lost 90%
                        // of them, which is precisely what flush_and_close exists to
                        // prevent.
                        loop {
                            while batch.len() < 1000 {
                                match rx.try_recv() {
                                    Ok(entry) => {
                                        writer_stats.depth.fetch_sub(1, Ordering::Relaxed);
                                        batch.push(entry);
                                    }
                                    Err(_) => break,
                                }
                            }
                            if batch.is_empty() {
                                break;
                            }
                            let before = batch.len();
                            flush_batch(&conn, &mut batch, &writer_stats, writable, &mut wid_cache);
                            if batch.len() == before {
                                // Could not write; retrying immediately would spin.
                                // Give the lock a moment, then try once more.
                                std::thread::sleep(Duration::from_millis(20));
                                flush_batch(&conn, &mut batch, &writer_stats, writable, &mut wid_cache);
                                if batch.len() == before {
                                    writer_stats
                                        .write_failed
                                        .fetch_add(batch.len() as u64, Ordering::Relaxed);
                                    crate::debug_log(true, &format!(
                                        "SCROLLBACK: shutdown could not write {} queued lines", batch.len()
                                    ));
                                    break;
                                }
                            }
                        }
                        break;
                    }

                    let due = last_flush.elapsed() >= Duration::from_millis(500 + backoff_ms);
                    let should_flush = (batch.len() >= 100 && backoff_ms == 0)
                        || (!batch.is_empty() && due);

                    if should_flush {
                        let before = batch.len();
                        flush_batch(&conn, &mut batch, &writer_stats, writable, &mut wid_cache);
                        last_flush = Instant::now();
                        // flush_batch RETAINS the batch when the transaction could
                        // not even be started (a transient lock). Back off instead
                        // of spinning, and never throw the lines away.
                        if batch.len() == before {
                            backoff_ms = (backoff_ms * 2).clamp(10, 500);
                        } else {
                            backoff_ms = 0;
                        }
                    }
                }
            })
            .ok();

        // Control connection for sequence allocation. Separate from the writer's
        // so a long batch commit never blocks a producer needing a number.
        // None for a refused file, so nothing reserves numbers in it either.
        let alloc = Connection::open(path).ok().filter(|_| writable).map(|c| {
            tune_connection(&c);
            WseqAllocator { conn: c, blocks: std::collections::HashMap::new() }
        });
        let sender = ArchiveSender {
            tx,
            stats: Arc::clone(&stats),
            alloc: Arc::new(std::sync::Mutex::new(alloc)),
        };
        Ok(Self { tx: sender, path: db_path, stats, shutdown, writer })
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
        let conn = match open_reader(path) {
            Some(c) => c,
            None => return Vec::new(),
        };
        if archive_state(&conn) != ArchiveState::Ready {
            return Vec::new();
        }

        // Worlds are a dimension table, and the stripped text is recomputed below.
        let mut conds: Vec<String> = Vec::new();
        if let Some(w) = world {
            conds.push(format!("w.name = '{}' COLLATE NOCASE", w.replace('\'', "''")));
        }
        if let Some(since) = since_ms {
            conds.push(format!("o.ts_ms >= {}", since));
        }
        if let Some(until) = until_ms {
            conds.push(format!("o.ts_ms <= {}", until));
        }
        let where_clause = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        let sql = format!(
            "SELECT o.ts_ms, w.name, o.line_raw FROM output_log o
             JOIN worlds w ON w.wid = o.wid {} ORDER BY o.ts_ms ASC",
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
                wseq: None,
                gagged: false,
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

/// Compile `pattern` the same way `execute_recall` will, so the archive
/// pre-filter and the final filter agree.
///
/// This previously always used glob semantics, so `/recall -D -msimple rc` asked
/// the archive for an ANCHORED match of "rc" and got nothing back — the rows were
/// discarded before the substring filter ever saw them.
fn build_pattern_regex(pattern: &str, kind: PatternKind) -> Option<regex::Regex> {
    let src = match kind {
        PatternKind::Simple => regex::escape(pattern),
        PatternKind::Glob => crate::actions::wildcard_to_regex(pattern),
        PatternKind::Regex => pattern.to_string(),
    };
    regex::RegexBuilder::new(&src).case_insensitive(true).build().ok()
}

/// Build an FTS5 MATCH expression for `pattern`, or `None` when FTS cannot
/// represent it exactly.
///
/// Conservative on purpose. In particular `Simple` NEVER routes: it is a
/// substring match while FTS tokens are whole words, so `-msimple rc` must still
/// find "orc" — an FTS phrase query would not.
fn fts_match_expr(pattern: &str, kind: PatternKind) -> Option<String> {
    if kind != PatternKind::Glob {
        return None;
    }
    let p = pattern.trim();
    if p.is_empty() || p == "*" {
        return None;
    }
    // Strip one leading and/or trailing '*': "*orc*" is a substring search, which
    // FTS can approximate as a prefix query ONLY if we keep the Rust regex as the
    // decider — which we do. Anything else (internal wildcards, '?') is refused.
    let trimmed = p.trim_start_matches('*');
    let had_leading = trimmed.len() != p.len();
    let core = trimmed.trim_end_matches('*');
    let had_trailing = core.len() != trimmed.len();
    if core.is_empty() || core.contains('*') || core.contains('?') {
        return None;
    }
    // A leading wildcard means the match may start mid-token, which no FTS5
    // tokenizer can find without a trigram index. Refuse rather than under-match.
    if had_leading {
        return None;
    }
    // Only whole alphanumeric words are safe to hand to the tokenizer.
    let words: Vec<&str> = core.split_whitespace().collect();
    if words.is_empty() || !words.iter().all(|w| w.chars().all(|c| c.is_alphanumeric())) {
        return None;
    }
    let quoted: Vec<String> = words.iter().map(|w| format!("\"{}\"", w)).collect();
    let mut expr = quoted.join(" ");
    if had_trailing {
        expr.push('*');
    }
    Some(expr)
}

/// Run an `ArchiveQuery`. This is the v2 read path; `search` remains as the
/// name-and-timestamp wrapper `--grep-archive` uses.
///
/// The bounded work is what matters here: `wid = ? AND wseq < ?` is a leading
/// equality plus a range on `idx_log_wid_wseq`, so SQLite walks an index range
/// in order rather than scanning the table and sorting.
pub fn query(path: &Path, q: &ArchiveQuery) -> Vec<ScrollbackLine> {
    let conn = match open_reader(path) {
        Some(c) => c,
        None => return Vec::new(),
    };
    if archive_state(&conn) != ArchiveState::Ready {
        return Vec::new();
    }

    let regex = build_pattern_regex(q.pattern, q.kind);
    // FTS is a CANDIDATE FILTER only: every surviving row is still matched by the
    // regex above. Over-matching from tokenisation is harmless; under-matching is
    // a silent wrong answer, which is why `fts_match_expr` refuses anything it
    // cannot represent exactly. `migration_state = done` says the index covers
    // every row (the key keeps its name from the old upgrade, which set it last).
    let mut fts_expr = if meta_get(&conn, "migration_state").as_deref() == Some("done") {
        fts_match_expr(q.pattern, q.kind)
    } else {
        None
    };
    // FTS only pays when the term is SELECTIVE. For a word that appears in most
    // lines the candidate set is enormous, and probing it per row costs far more
    // than the ordered range scan it replaces — measured at 131ms vs 17ms for a
    // term matching ~70% of rows. Probe the candidate count, bounded so the probe
    // itself is cheap, and fall back to the scan when the term is common.
    if let Some(expr) = &fts_expr {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (SELECT rowid FROM output_fts WHERE output_fts MATCH ?1 LIMIT ?2)",
                params![expr, FTS_SELECTIVITY_CAP],
                |r| r.get(0),
            )
            .unwrap_or(FTS_SELECTIVITY_CAP);
        if n >= FTS_SELECTIVITY_CAP {
            fts_expr = None;
        }
    }

    // Resolve the world to its wid(s) FIRST rather than joining. Joining makes
    // SQLite drive from `worlds`, and because it cannot prove `name` is unique it
    // adds a TEMP B-TREE to re-sort by wseq — reintroducing the sort this schema
    // exists to avoid. With a single wid the query is a pure ordered range scan.
    let wids: Vec<i64> = match q.world {
        Some(name) => {
            let mut stmt = match conn.prepare("SELECT wid, name FROM worlds WHERE name = ?1 COLLATE NOCASE") {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            let found: Vec<i64> = stmt
                .query_map(params![name], |r| r.get::<_, i64>(0))
                .map(|it| it.flatten().collect())
                .unwrap_or_default();
            if found.is_empty() {
                return Vec::new();
            }
            found
        }
        None => Vec::new(),
    };
    let world_label = q.world.unwrap_or("").to_string();

    let mut conds: Vec<String> = Vec::new();
    match wids.len() {
        0 => {}
        1 => conds.push(format!("o.wid = {}", wids[0])),
        _ => conds.push(format!(
            "o.wid IN ({})",
            wids.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(",")
        )),
    }
    if let Some(b) = q.before_wseq {
        conds.push(format!("o.wseq < {b}"));
    }
    if let Some(sm) = q.since_ms {
        conds.push(format!("o.ts_ms >= {sm}"));
    }
    if let Some(um) = q.until_ms {
        conds.push(format!("o.ts_ms <= {um}"));
    }
    if fts_expr.is_some() {
        // `IN (subquery)` rather than a JOIN so the planner keeps driving from the
        // highly selective (wid, wseq) range rather than from the FTS result.
        conds.push("o.id IN (SELECT rowid FROM output_fts WHERE output_fts MATCH ?1)".to_string());
    }
    let where_clause = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    let order = if q.newest_first { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT o.ts_ms, o.line_raw, o.wseq, o.wid FROM output_log o {where_clause} ORDER BY o.wseq {order}"
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            crate::debug_log(true, &format!("SCROLLBACK: query prepare failed: {e}"));
            return Vec::new();
        }
    };
    // Only needed when searching every world; a single-world query already knows
    // the name it was asked for.
    let mut name_cache: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    let mapper = |row: &rusqlite::Row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    };
    let rows = match &fts_expr {
        Some(expr) => stmt.query_map(params![expr], mapper),
        None => stmt.query_map([], mapper),
    };

    let mut out: Vec<ScrollbackLine> = Vec::new();
    if let Ok(iter) = rows {
        for (ts_ms, text, wseq, wid) in iter.flatten() {
            let matched = match &regex {
                Some(re) => re.is_match(&crate::util::strip_ansi_codes(&text)),
                None => true,
            };
            if !matched {
                continue;
            }
            let world = if q.world.is_some() {
                world_label.clone()
            } else {
                name_cache
                    .entry(wid)
                    .or_insert_with(|| {
                        conn.query_row(
                            "SELECT name FROM worlds WHERE wid = ?1",
                            params![wid],
                            |r| r.get::<_, String>(0),
                        )
                        .unwrap_or_default()
                    })
                    .clone()
            };
            out.push(ScrollbackLine { ts_ms, world, text, wseq: Some(wseq), gagged: false });
            if out.len() >= q.limit {
                break;
            }
        }
    }
    // Callers always want oldest-first for display; newest_first only decides
    // which end of the archive the limit keeps.
    if q.newest_first {
        out.reverse();
    }
    out
}

/// Load up to `count` archived lines immediately preceding `before_wseq` for a
/// world, oldest-first.
///
/// The `wseq` counterpart of `load_before_path`. Preferred when the buffer's
/// oldest line carries an archive sequence, because it cuts on an exact key
/// rather than a timestamp — so Page Up cannot re-show a line the buffer already
/// holds, nor skip one whose timestamp happens to tie.
pub fn load_before_wseq(path: &Path, world: &str, before_wseq: i64, count: usize) -> Vec<ScrollbackLine> {
    let conn = match open_reader(path) {
        Some(c) => c,
        None => return Vec::new(),
    };
    if archive_state(&conn) != ArchiveState::Ready {
        return Vec::new();
    }
    let wid: i64 = match conn.query_row(
        "SELECT wid FROM worlds WHERE name = ?1 COLLATE NOCASE ORDER BY wid LIMIT 1",
        params![world],
        |r| r.get(0),
    ) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT ts_ms, line_raw, wseq, gagged FROM output_log
         WHERE wid = ?1 AND wseq < ?2 ORDER BY wseq DESC LIMIT ?3",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(params![wid, before_wseq, count as i64], |row| {
        Ok(ScrollbackLine {
            ts_ms: row.get(0)?,
            world: world.to_string(),
            text: row.get(1)?,
            wseq: row.get(2).ok(),
            gagged: row.get::<_, i64>(3).unwrap_or(0) != 0,
        })
    });
    let mut lines: Vec<ScrollbackLine> =
        rows.map(|iter| iter.flatten().collect()).unwrap_or_default();
    lines.reverse();
    lines
}

pub fn load_before_path(path: &Path, world: &str, before_ts_ms: i64, count: usize) -> Vec<ScrollbackLine> {
    let conn = match open_reader(path) {
        Some(c) => c,
        None => return Vec::new(),
    };

    if archive_state(&conn) != ArchiveState::Ready {
        return Vec::new();
    }

    // Fetch the N rows just before the timestamp, then reverse to get oldest-first
    let sql = "SELECT o.ts_ms, w.name, o.line_raw, o.gagged FROM output_log o \
         JOIN worlds w ON w.wid = o.wid \
         WHERE w.name = ?1 COLLATE NOCASE AND o.ts_ms < ?2 \
         ORDER BY o.ts_ms DESC LIMIT ?3";

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map(params![world, before_ts_ms, count as i64], |row| {
        Ok(ScrollbackLine {
            ts_ms: row.get(0)?,
            world: row.get(1)?,
            text: row.get(2)?,
            wseq: None,
            gagged: row.get::<_, i64>(3).unwrap_or(0) != 0,
        })
    });

    let mut lines: Vec<ScrollbackLine> = rows
        .map(|iter| iter.flatten().collect())
        .unwrap_or_default();

    // Re-order oldest-first
    lines.reverse();
    lines
}

/// Resolve a world name to `(wid, wseq)`, minting the `worlds` row on first sight
/// and handing out the next sequence number.
///
/// This is the writer-side allocator: it is what keeps `wseq` durable across
/// restarts, because the counter is seeded from what is already in the table
/// rather than from an in-memory value that resets to zero every cold start (the
/// reason the protocol `seq` cannot be used here).
fn resolve_wid(
    conn: &Connection,
    world: &str,
    cache: &mut std::collections::HashMap<String, (i64, i64)>,
) -> Option<(i64, i64)> {
    if let Some(slot) = cache.get_mut(world) {
        let out = (slot.0, slot.1);
        slot.1 += 1;
        return Some(out);
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    // The producer had no number for this line (allocator unreachable), so all
    // there is to go on is the world's name.
    let wid: i64 = conn
        .query_row("SELECT wid FROM worlds WHERE name = ?1 ORDER BY wid LIMIT 1", params![world], |r| r.get(0))
        .or_else(|_| {
            conn.execute(
                "INSERT INTO worlds (world_uuid, name, next_wseq, orphan, created_ms)
                 VALUES (?1, ?2, 0, 0, ?3)",
                params![uuid::Uuid::new_v4().simple().to_string(), world, now_ms],
            )?;
            conn.query_row("SELECT wid FROM worlds WHERE name = ?1 ORDER BY wid LIMIT 1", params![world], |r| r.get(0))
        })
        .ok()?;
    let next: i64 = conn
        .query_row("SELECT next_wseq FROM worlds WHERE wid = ?1", params![wid], |r| r.get(0))
        .unwrap_or(0);
    // Reserve a block so a crash cannot hand the same number out twice.
    let _ = conn.execute(
        "UPDATE worlds SET next_wseq = next_wseq + ?2 WHERE wid = ?1",
        params![wid, WSEQ_BLOCK],
    );
    cache.insert(world.to_string(), (wid, next + 1));
    Some((wid, next))
}

/// Write a batch inside one transaction.
///
/// On a failure to even START the transaction the batch is RETAINED, not
/// cleared. The previous version did `batch.clear(); return;`, which threw away
/// up to a full batch of real MUD output on a transient SQLITE_BUSY — the exact
/// "no record loss" violation this archive is supposed to prevent. The caller
/// detects the retained batch and backs off.
///
/// `writable` is false for a refused file (see `ScrollbackDb::open`): the batch is
/// counted as failed and dropped, never written.
fn flush_batch(
    conn: &Connection,
    batch: &mut Vec<ArchiveEntry>,
    stats: &Arc<ArchiveStats>,
    writable: bool,
    wid_cache: &mut std::collections::HashMap<String, (i64, i64)>,
) {
    if batch.is_empty() {
        return;
    }
    if !writable {
        stats.write_failed.fetch_add(batch.len() as u64, Ordering::Relaxed);
        batch.clear();
        return;
    }
    let tx = match conn.unchecked_transaction() {
        Ok(t) => t,
        Err(e) => {
            crate::debug_log(true, &format!(
                "SCROLLBACK: could not begin transaction ({e}); retaining {} queued lines",
                batch.len()
            ));
            return; // batch retained on purpose
        }
    };

    let mut failed: Vec<ArchiveEntry> = Vec::new();
    for row in batch.drain(..) {
        // Prefer the producer's number; fall back to writer-side assignment
        // so a line is never dropped just because the allocator was busy.
        let slot = match (row.wid, row.wseq) {
            (Some(wid), Some(wseq)) => Some((wid, wseq)),
            _ => resolve_wid(&tx, &row.world, wid_cache),
        };
        let Some((wid, wseq)) = slot else {
            failed.push(row);
            continue;
        };
        let res = tx.execute(
            "INSERT INTO output_log (wid, wseq, ts_ms, line_raw, gagged)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![wid, wseq, row.ts_ms, row.text, row.gagged as i32],
        );
        if res.is_ok() {
            // Index the ANSI-stripped form, in the same transaction, so
            // the index can never disagree with the table. There are no
            // triggers because a trigger cannot strip ANSI.
            let rowid = tx.last_insert_rowid();
            let _ = tx.execute(
                "INSERT INTO output_fts (rowid, txt) VALUES (?1, ?2)",
                params![rowid, crate::util::strip_ansi_codes(&row.text)],
            );
        }
        if let Err(e) = res {
            crate::debug_log(true, &format!(
                "SCROLLBACK: insert failed ({e}) for [{}] {}", row.world, row.text
            ));
            failed.push(row);
        }
    }

    if let Err(e) = tx.commit() {
        // The whole transaction is lost; put everything back so the next tick
        // retries rather than silently dropping it.
        crate::debug_log(true, &format!("SCROLLBACK: commit failed ({e}); retrying batch"));
        batch.append(&mut failed);
        return;
    }

    if !failed.is_empty() {
        stats.write_failed.fetch_add(failed.len() as u64, Ordering::Relaxed);
    }
}

/// Export the entire archive to a CSV file.  Returns the number of data rows written.
///
/// Columns: id, world, datetime_local, ts_epoch_ms, gagged, text, raw, wseq. `text`
/// is the ANSI-stripped form, derived as `search` derives it for matching.
/// Rows sorted by world ascending then archive sequence ascending.
/// Returns `Err(String)` with a human-readable message on failure.
pub fn export_csv(db_path: &Path, out_path: &Path) -> Result<usize, String> {
    use std::io::Write;
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("cannot open database: {}", e))?;
    tune_connection(&conn);
    match archive_state(&conn) {
        ArchiveState::Ready => {}
        ArchiveState::Empty => return Err("the archive is empty".to_string()),
        ArchiveState::Old => return Err(format!(
            "{} is an archive from before Clay 1.5.41, which this Clay does not read", db_path.display())),
        ArchiveState::TooNew => return Err(format!(
            "{} was written by a newer Clay", db_path.display())),
    }

    let mut file = std::fs::File::create(out_path)
        .map_err(|e| format!("cannot create {}: {}", out_path.display(), e))?;
    writeln!(file, "id,world,datetime_local,ts_epoch_ms,gagged,text,raw,wseq")
        .map_err(|e| format!("write error: {}", e))?;

    let mut stmt = conn
        .prepare(
            "SELECT o.id, o.ts_ms, w.name, o.line_raw, o.gagged, o.wseq
             FROM output_log o JOIN worlds w ON w.wid = o.wid
             ORDER BY w.name ASC, o.wseq ASC",
        )
        .map_err(|e| format!("prepare failed: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i32>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| format!("query failed: {}", e))?;

    let mut count = 0usize;
    for row in rows.flatten() {
        let (id, ts_ms, world, line_raw, gagged_int, wseq) = row;
        let lt = crate::util::local_time_from_epoch(ts_ms / 1000);
        let datetime_str = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            lt.year, lt.month, lt.day, lt.hour, lt.minute, lt.second
        );
        writeln!(
            file,
            "{},{},{},{},{},{},{},{}",
            id,
            csv_escape(&world),
            csv_escape(&datetime_str),
            ts_ms,
            if gagged_int != 0 { "true" } else { "false" },
            csv_escape(&crate::util::strip_ansi_codes(&line_raw)),
            csv_escape(&line_raw),
            wseq,
        )
        .map_err(|e| format!("write error: {}", e))?;
        count += 1;
    }
    Ok(count)
}

/// Escape a CSV field value: wrap in double-quotes if the value contains a comma,
/// double-quote, newline, or carriage return; double any embedded double-quotes.
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
//
// Every function here takes an explicit `&Path`, so a test can never reach the
// developer's real `~/.clay/scrollback.db` — but that only holds as long as new
// code keeps taking the path as a parameter rather than calling
// `clay_config_path` internally.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch database path that cleans up its `-wal`/`-shm` sidecars too.
    /// Forgetting the sidecars leaves SQLite state behind that makes the *next*
    /// run of the same test open a file it did not create — exactly the orphaned
    /// `scrollback.db-wal` with no `scrollback.db` seen in the wild.
    struct TempDb {
        path: PathBuf,
    }

    impl TempDb {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "clay_test_scrollback_{}_{}.db",
                tag,
                std::process::id()
            ));
            let me = Self { path };
            me.remove();
            me
        }

        fn remove(&self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.path.clone().into_os_string();
                p.push(suffix);
                let _ = std::fs::remove_file(PathBuf::from(p));
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            self.remove();
        }
    }

    /// Build an archive holding `rows` (`(ts_ms, world, line_raw, gagged)`), written
    /// through the real writer, in order.
    fn write_db(path: &Path, rows: &[(i64, &str, &str, bool)]) {
        let mut sdb = ScrollbackDb::open(path).expect("open");
        let tx = sdb.sender();
        for (ts_ms, world, line_raw, gagged) in rows {
            assert!(tx.send(ArchiveEntry::new(world.to_string(), *ts_ms, line_raw.to_string(), *gagged)));
        }
        sdb.flush_and_close();
    }

    /// Representative fixture: two worlds, ANSI colour, a name needing SQL
    /// quoting, and a gagged line.
    fn fixture() -> Vec<(i64, &'static str, &'static str, bool)> {
        vec![
            (1_000, "Zmc", "a dragon roars nearby", false),
            (2_000, "Zmc", "\x1b[1;33mthe dragon\x1b[0m flees west", false),
            (3_000, "Zmc", "a goblin appears", false),
            (4_000, "Zmc", "gagged dragon whisper", true),
            (5_000, "Other", "a dragon in another world", false),
            (6_000, "O'Brien", "a dragon at O'Brien's", false),
        ]
    }

    // -- search ------------------------------------------------------------

    #[test]
    fn test_search_filters_by_world_and_glob_pattern() {
        let db = TempDb::new("search_world");
        write_db(db.path(), &fixture());

        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*dragon*", None, None, 100, false);
        let texts: Vec<&str> = hits.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "a dragon roars nearby",
                "\x1b[1;33mthe dragon\x1b[0m flees west",
                "gagged dragon whisper",
            ],
            "world filter must exclude other worlds; gagged rows ARE returned today"
        );
    }

    #[test]
    fn test_search_matches_against_ansi_stripped_text() {
        let db = TempDb::new("search_ansi");
        write_db(db.path(), &fixture());

        // "the dragon flees" only reads contiguously once the SGR codes between
        // "dragon" and "flees" are stripped.
        let hits = ScrollbackDb::search(
            db.path(), Some("Zmc"), "*the dragon flees*", None, None, 100, false,
        );
        assert_eq!(hits.len(), 1, "pattern must match the stripped form, not the raw bytes");
        assert!(
            hits[0].text.contains('\x1b'),
            "but the row returned to the caller must keep its ANSI intact"
        );
    }

    #[test]
    fn test_search_world_name_containing_a_quote() {
        let db = TempDb::new("search_quote");
        write_db(db.path(), &fixture());

        let hits = ScrollbackDb::search(db.path(), Some("O'Brien"), "*", None, None, 100, false);
        assert_eq!(hits.len(), 1, "an apostrophe in a world name must not break the query");
        assert_eq!(hits[0].world, "O'Brien");
    }

    #[test]
    fn test_search_all_worlds_when_world_is_none() {
        let db = TempDb::new("search_all");
        write_db(db.path(), &fixture());

        let hits = ScrollbackDb::search(db.path(), None, "*dragon*", None, None, 100, false);
        assert_eq!(hits.len(), 5, "no world filter searches every world");
    }

    #[test]
    fn test_search_regex_mode() {
        let db = TempDb::new("search_regex");
        write_db(db.path(), &fixture());

        let hits = ScrollbackDb::search(
            db.path(), Some("Zmc"), r"^a \w+ (roars|appears)", None, None, 100, true,
        );
        assert_eq!(hits.len(), 2, "regex mode compiles the pattern verbatim");
    }

    #[test]
    fn test_search_time_bounds() {
        let db = TempDb::new("search_time");
        write_db(db.path(), &fixture());

        let hits = ScrollbackDb::search(
            db.path(), Some("Zmc"), "*", Some(2_000), Some(3_000), 100, false,
        );
        let ts: Vec<i64> = hits.iter().map(|l| l.ts_ms).collect();
        assert_eq!(ts, vec![2_000, 3_000], "since/until are inclusive on both ends");
    }

    #[test]
    fn test_search_returns_ascending_and_limit_keeps_the_oldest() {
        let db = TempDb::new("search_limit");
        write_db(db.path(), &fixture());

        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*", None, None, 2, false);
        let ts: Vec<i64> = hits.iter().map(|l| l.ts_ms).collect();
        // Pinning today's behaviour deliberately: results come back oldest-first
        // and the limit therefore keeps the OLDEST matches, hiding recent history
        // on a large archive. `query`'s `newest_first` reverses this on purpose.
        assert_eq!(ts, vec![1_000, 2_000]);
    }

    #[test]
    fn test_search_missing_database_returns_empty() {
        let db = TempDb::new("search_missing");
        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*", None, None, 100, false);
        assert!(hits.is_empty(), "a missing file must not panic");
    }

    // -- load_before_path --------------------------------------------------

    #[test]
    fn test_load_before_path_returns_oldest_first_and_excludes_the_boundary() {
        let db = TempDb::new("loadbefore");
        write_db(db.path(), &fixture());

        let lines = load_before_path(db.path(), "Zmc", 4_000, 10);
        let ts: Vec<i64> = lines.iter().map(|l| l.ts_ms).collect();
        assert_eq!(ts, vec![1_000, 2_000, 3_000], "strictly `< before_ts_ms`, oldest first");
    }

    #[test]
    fn test_load_before_path_count_keeps_the_newest_and_still_sorts_oldest_first() {
        let db = TempDb::new("loadbefore_count");
        write_db(db.path(), &fixture());

        let lines = load_before_path(db.path(), "Zmc", 5_000, 2);
        let ts: Vec<i64> = lines.iter().map(|l| l.ts_ms).collect();
        assert_eq!(ts, vec![3_000, 4_000], "takes the N nearest the boundary, then reverses");
    }

    #[test]
    fn test_load_before_path_does_not_filter_gagged() {
        let db = TempDb::new("loadbefore_gagged");
        write_db(db.path(), &fixture());

        let lines = load_before_path(db.path(), "Zmc", 5_000, 10);
        assert!(
            lines.iter().any(|l| l.text == "gagged dragon whisper"),
            "Page Up shows archived gagged lines today; the v2 rewrite must not change that silently"
        );
    }

    #[test]
    fn test_load_before_path_missing_database_returns_empty() {
        let db = TempDb::new("loadbefore_missing");
        assert!(load_before_path(db.path(), "Zmc", 5_000, 10).is_empty());
    }

    // -- export_csv --------------------------------------------------------

    #[test]
    fn test_export_csv_shape_and_ordering() {
        let db = TempDb::new("csv");
        write_db(db.path(), &fixture());
        let out = std::env::temp_dir().join(format!("clay_test_csv_{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&out);

        let n = export_csv(db.path(), &out).expect("export");
        assert_eq!(n, 6);

        let body = std::fs::read_to_string(&out).expect("read csv");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            lines[0], "id,world,datetime_local,ts_epoch_ms,gagged,text,raw,wseq",
            "the header is a user-facing contract: the six original columns, then wseq"
        );
        // Sorted by world ASC then wseq ASC: O'Brien, Other, then Zmc's four.
        assert!(lines[1].contains("O'Brien"));
        assert!(lines[2].contains("Other"));
        assert_eq!(lines.len(), 7);

        // `text` is the stripped form and `raw` keeps the escapes.
        let coloured = lines.iter().find(|l| l.contains("flees west")).expect("coloured row");
        assert!(coloured.contains("the dragon flees west"));
        assert!(coloured.contains('\x1b'));

        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn test_csv_escape_quotes_only_when_needed() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("has,comma"), "\"has,comma\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape("two\nlines"), "\"two\nlines\"");
    }

    // -- writer round trip -------------------------------------------------

    #[test]
    fn test_open_then_send_persists_rows() {
        let db = TempDb::new("writer");
        {
            let sdb = ScrollbackDb::open(db.path()).expect("open");
            let tx = sdb.sender();
            for i in 0..5 {
                assert!(tx.send(ArchiveEntry::new("Zmc".to_string(), 1_000 + i, format!("line {i}"), false)));
            }
            // Dropping the ScrollbackDb drops the only sender, which disconnects
            // the channel and makes the writer thread do its final flush.
        }
        // The writer flushes on disconnect, but the thread is detached, so poll
        // rather than assuming it has been scheduled.
        let mut hits = Vec::new();
        for _ in 0..100 {
            hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*line*", None, None, 100, false);
            if hits.len() == 5 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(hits.len(), 5, "all sent lines must reach the database");
    }

    #[test]
    fn test_flush_and_close_drains_the_queue() {
        // The hot-reload path exits with senders still alive, so the writer never
        // sees a disconnect. flush_and_close is what makes that lossless.
        let db = TempDb::new("flush_close");
        let mut sdb = ScrollbackDb::open(db.path()).expect("open");
        assert_eq!(sdb.path(), db.path(), "the handle must report the file it writes");
        let tx = sdb.sender();
        // Deliberately far more than one batch (and more than the writer's 1000-row
        // drain step). An earlier version stopped after the current batch and
        // abandoned the rest of the queue, losing ~90% of a deep queue on shutdown.
        const N: i64 = 5_000;
        for i in 0..N {
            assert!(tx.send(ArchiveEntry::new("Zmc".to_string(), 1_000 + i, format!("line {i}"), false)));
        }
        sdb.flush_and_close();

        // No polling: flush_and_close joined the writer, so everything is durable.
        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*line*", None, None, 100_000, false);
        assert_eq!(
            hits.len() as i64, N,
            "flush_and_close must drain the WHOLE queue, not just the batch in hand"
        );
        assert_eq!(sdb.stats().dropped.load(Ordering::Relaxed), 0);
        assert_eq!(sdb.stats().write_failed.load(Ordering::Relaxed), 0);
        assert_eq!(sdb.stats().depth.load(Ordering::Relaxed), 0, "queue accounting must balance");
    }

    #[test]
    fn test_reader_is_not_starved_by_a_held_write_lock() {
        // Without busy_timeout a reader colliding with the writer got SQLITE_BUSY,
        // which every read path here turns into an empty Vec — reported to the user
        // as "No matches", indistinguishable from an empty archive.
        let db = TempDb::new("busy");
        write_db(db.path(), &fixture());

        let writer = Connection::open(db.path()).expect("writer conn");
        tune_connection(&writer);
        writer.execute_batch("BEGIN IMMEDIATE;").expect("take the write lock");
        writer
            .execute(
                "INSERT INTO output_log (wid, wseq, ts_ms, line_raw, gagged) \
                 VALUES (1, 900000, 9000, 'held', 0)",
                [],
            )
            .expect("write inside the held transaction");

        // A WAL reader must still see the committed snapshot while that lock is held.
        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*dragon*", None, None, 100, false);
        assert_eq!(hits.len(), 3, "reads must succeed while a write transaction is open");

        writer.execute_batch("COMMIT;").expect("release");
    }

    // -- refused files -----------------------------------------------------

    /// An archive in the layout before v1.5.41's (no `wid`) is refused like a newer one:
    /// left exactly as it is, never written, and read as empty by every reader.
    #[test]
    fn test_an_archive_from_before_v2_is_refused_and_left_untouched() {
        let db = TempDb::new("pre_v2");
        {
            let conn = Connection::open(db.path()).expect("open");
            conn.execute_batch(
                "CREATE TABLE output_log (
                     id INTEGER PRIMARY KEY, ts_ms INTEGER NOT NULL, world TEXT NOT NULL,
                     line_raw TEXT NOT NULL, line_text TEXT NOT NULL,
                     gagged INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO output_log (ts_ms, world, line_raw, line_text)
                     VALUES (1000, 'Zmc', 'a dragon roars', 'a dragon roars');",
            ).expect("old layout");
        }
        let before = std::fs::read(db.path()).expect("read");

        {
            let mut sdb = ScrollbackDb::open(db.path()).expect("open");
            assert!(sdb.sender().send(ArchiveEntry::new("Zmc".into(), 2_000, "new line".into(), false)));
            sdb.flush_and_close();
            assert_eq!(sdb.stats().write_failed.load(Ordering::Relaxed), 1,
                "a line for a refused archive is counted, so /info can say so");
        }
        assert_eq!(std::fs::read(db.path()).expect("reread"), before, "the file must be left exactly as it was");

        assert!(ScrollbackDb::search(db.path(), Some("Zmc"), "*", None, None, 10, false).is_empty());
        assert!(query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None, limit: 10, newest_first: true,
        }).is_empty());
        assert!(load_before_path(db.path(), "Zmc", 9_999, 10).is_empty());
        assert!(load_before_wseq(db.path(), "Zmc", 999, 10).is_empty());
        let out = std::env::temp_dir().join(format!("clay_test_csv_pre_v2_{}.csv", std::process::id()));
        assert!(export_csv(db.path(), &out).is_err(), "an export says why it has nothing");
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn test_a_newer_schema_is_refused_rather_than_read_as_empty() {
        let db = TempDb::new("too_new");
        let conn = Connection::open(db.path()).expect("open");
        conn.execute_batch("CREATE TABLE output_log (id INTEGER PRIMARY KEY);
                            PRAGMA user_version = 99;").unwrap();
        assert_eq!(archive_state(&conn), ArchiveState::TooNew);
        drop(conn);

        // open must not migrate or clobber it.
        let sdb = ScrollbackDb::open(db.path()).expect("open");
        drop(sdb);
        let conn = Connection::open(db.path()).expect("reopen");
        assert_eq!(user_version(&conn), 99, "a newer archive must be left untouched");
        assert!(!table_exists(&conn, "worlds"));
    }

    // -- durable sequence allocation ---------------------------------------

    #[test]
    fn test_wseq_is_unique_monotonic_and_survives_restarts() {
        let db = TempDb::new("wseq_alloc");
        // Three "process lifetimes" over the same file. The protocol seq would
        // restart at 0 each time; wseq must not.
        let mut all: Vec<i64> = Vec::new();
        for _ in 0..3 {
            let sdb = ScrollbackDb::open(db.path()).expect("open");
            let tx = sdb.sender();
            for _ in 0..10 {
                let (_wid, wseq) = tx.alloc_wseq("uuid-a", "Zmc").expect("alloc");
                all.push(wseq);
            }
        }
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "wseq must never repeat across restarts");
        assert!(all.windows(2).all(|w| w[0] < w[1]), "and must increase: {all:?}");
        assert_eq!(all[0], 0, "a fresh archive starts at 0");
        assert!(all[10] >= WSEQ_BLOCK, "a restart burns the unused remainder of its block");
    }

    #[test]
    fn test_wseq_blocks_are_disjoint_across_two_handles_on_one_file() {
        // Two Clay processes sharing one archive must not hand out the same number.
        let db = TempDb::new("wseq_two");
        let a = ScrollbackDb::open(db.path()).expect("open a");
        let b = ScrollbackDb::open(db.path()).expect("open b");
        let (sa, sb) = (a.sender(), b.sender());

        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            assert!(seen.insert(sa.alloc_wseq("uuid-a", "Zmc").expect("a").1), "duplicate from a");
            assert!(seen.insert(sb.alloc_wseq("uuid-a", "Zmc").expect("b").1), "duplicate from b");
        }
        assert_eq!(seen.len(), 100);
    }

    #[test]
    fn test_wseq_is_per_world_not_global() {
        let db = TempDb::new("wseq_perworld");
        let sdb = ScrollbackDb::open(db.path()).expect("open");
        let tx = sdb.sender();
        let (wid_a, seq_a) = tx.alloc_wseq("uuid-a", "Zmc").expect("a");
        let (wid_b, seq_b) = tx.alloc_wseq("uuid-b", "Other").expect("b");
        assert_ne!(wid_a, wid_b, "distinct worlds get distinct wids");
        assert_eq!((seq_a, seq_b), (0, 0), "each world has its own sequence space");
    }

    #[test]
    fn test_producer_assigned_wseq_reaches_the_database() {
        let db = TempDb::new("wseq_write");
        {
            let mut sdb = ScrollbackDb::open(db.path()).expect("open");
            let tx = sdb.sender();
            for i in 0..5 {
                let (wid, wseq) = tx.alloc_wseq("uuid-a", "Zmc").expect("alloc");
                let mut e = ArchiveEntry::new("Zmc".into(), 1_000 + i, format!("line {i}"), false);
                e.wid = Some(wid);
                e.wseq = Some(wseq);
                assert!(tx.send(e));
            }
            sdb.flush_and_close();
        }
        let conn = Connection::open(db.path()).expect("reopen");
        let got: Vec<i64> = {
            let mut stmt = conn.prepare("SELECT wseq FROM output_log ORDER BY wseq").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().flatten().collect()
        };
        assert_eq!(got, vec![0, 1, 2, 3, 4], "the producer's numbers must be the stored ones");
    }

    #[test]
    fn test_a_line_without_a_producer_wseq_is_still_archived() {
        // Allocator unreachable => the line must still be written, with a
        // writer-assigned number. Losing the number is acceptable; losing the
        // line is not.
        let db = TempDb::new("wseq_fallback");
        {
            let mut sdb = ScrollbackDb::open(db.path()).expect("open");
            let tx = sdb.sender();
            assert!(tx.send(ArchiveEntry::new("Zmc".into(), 1, "no seq supplied".into(), false)));
            sdb.flush_and_close();
        }
        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*no seq*", None, None, 10, false);
        assert_eq!(hits.len(), 1, "a line with no producer-assigned wseq must not be dropped");
    }

    // -- the memory/SQL boundary -------------------------------------------

    /// Archive `n` lines for one world and return their wseqs.
    fn seed_v2(path: &Path, world: &str, n: i64) -> Vec<i64> {
        let mut sdb = ScrollbackDb::open(path).expect("open");
        let tx = sdb.sender();
        let mut seqs = Vec::new();
        for i in 0..n {
            let (wid, wseq) = tx.alloc_wseq("uuid-a", world).expect("alloc");
            let mut e = ArchiveEntry::new(world.into(), 1_000 + i, format!("dragon line {i}"), false);
            e.wid = Some(wid);
            e.wseq = Some(wseq);
            assert!(tx.send(e));
            seqs.push(wseq);
        }
        sdb.flush_and_close();
        seqs
    }

    #[test]
    fn test_before_wseq_returns_exactly_the_rows_memory_does_not_hold() {
        let db = TempDb::new("boundary");
        let seqs = seed_v2(db.path(), "Zmc", 1000);
        assert_eq!(seqs.first(), Some(&0));

        // Pretend the live buffer holds the newest 500 (wseq 500..999).
        let floor = seqs[500];
        let got = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "*dragon*", kind: PatternKind::Glob,
            before_wseq: Some(floor), since_ms: None, until_ms: None,
            limit: 10_000, newest_first: true,
        });
        let got_seqs: Vec<i64> = got.iter().filter_map(|l| l.wseq).collect();
        assert_eq!(got_seqs.len(), 500, "exactly the half memory does not cover");
        assert_eq!(got_seqs.first(), Some(&seqs[0]));
        assert_eq!(got_seqs.last(), Some(&seqs[499]));
        assert!(got_seqs.iter().all(|s| *s < floor), "nothing at or past the floor may be returned");
        // Disjointness is what removes the need to de-duplicate at the seam.
        let mut uniq = got_seqs.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), got_seqs.len(), "no duplicates");
        assert!(got.windows(2).all(|w| w[0].wseq < w[1].wseq), "returned oldest-first");
    }

    #[test]
    fn test_no_boundary_searches_the_whole_archive() {
        let db = TempDb::new("boundary_none");
        seed_v2(db.path(), "Zmc", 50);
        let got = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "*dragon*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None,
            limit: 10_000, newest_first: true,
        });
        assert_eq!(got.len(), 50, "with nothing in memory, everything is fair game");
    }

    #[test]
    fn test_newest_first_keeps_recent_history_that_the_old_scan_hid() {
        let db = TempDb::new("boundary_limit");
        let seqs = seed_v2(db.path(), "Zmc", 100);
        let newest = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "*dragon*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None,
            limit: 10, newest_first: true,
        });
        let seqs_out: Vec<i64> = newest.iter().filter_map(|l| l.wseq).collect();
        assert_eq!(seqs_out, seqs[90..].to_vec(), "the limit must keep the most recent matches");

        let oldest = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "*dragon*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None,
            limit: 10, newest_first: false,
        });
        let seqs_old: Vec<i64> = oldest.iter().filter_map(|l| l.wseq).collect();
        assert_eq!(seqs_old, seqs[..10].to_vec(), "and the old ordering is still available");
    }

    #[test]
    fn test_boundary_query_uses_the_wseq_index_and_needs_no_sort() {
        // A full scan plus a sort is exactly what this change exists to avoid, so
        // assert the plan rather than trusting it.
        let db = TempDb::new("boundary_plan");
        seed_v2(db.path(), "Zmc", 10);
        let conn = Connection::open(db.path()).expect("open");
        let plan: Vec<String> = {
            let mut stmt = conn.prepare(
                "EXPLAIN QUERY PLAN
                 SELECT o.ts_ms, o.line_raw, o.wseq, o.wid FROM output_log o
                 WHERE o.wid = 1 AND o.wseq < 500
                 ORDER BY o.wseq DESC").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(3)).unwrap().flatten().collect()
        };
        let joined = plan.join(" | ");
        assert!(
            joined.contains("idx_log_wid_wseq"),
            "the boundary must be served by the (wid, wseq) index, got: {joined}"
        );
        assert!(
            !joined.contains("TEMP B-TREE"),
            "index order IS wseq order, so there must be no sort step: {joined}"
        );
    }

    // -- FTS routing -------------------------------------------------------

    #[test]
    fn test_fts_routing_refuses_everything_it_cannot_represent_exactly() {
        use PatternKind::*;
        // Simple is a SUBSTRING match; FTS tokens are whole words. Routing it
        // would make `-msimple rc` stop finding "orc" — a silent wrong answer.
        assert_eq!(fts_match_expr("orc", Simple), None);
        assert_eq!(fts_match_expr("*orc*", Simple), None);
        // Regex is never routed.
        assert_eq!(fts_match_expr(r"^a \w+", Regex), None);
        // Match-everything patterns have nothing to look up.
        assert_eq!(fts_match_expr("*", Glob), None);
        assert_eq!(fts_match_expr("", Glob), None);
        // A leading wildcard can start mid-token; no FTS5 tokenizer finds that
        // without a trigram index.
        assert_eq!(fts_match_expr("*orc", Glob), None);
        assert_eq!(fts_match_expr("*orc*", Glob), None);
        // Internal metacharacters are refused.
        assert_eq!(fts_match_expr("or*c", Glob), None);
        assert_eq!(fts_match_expr("or?c", Glob), None);
        // Non-word characters would be parsed as FTS operators.
        assert_eq!(fts_match_expr("orc-mage", Glob), None);
        assert_eq!(fts_match_expr("orc:mage", Glob), None);
        // What IS routed, always quoted so AND/OR/NEAR cannot be interpreted.
        assert_eq!(fts_match_expr("orc", Glob).as_deref(), Some("\"orc\""));
        assert_eq!(fts_match_expr("orc*", Glob).as_deref(), Some("\"orc\"*"));
        assert_eq!(fts_match_expr("big orc", Glob).as_deref(), Some("\"big\" \"orc\""));
        assert_eq!(fts_match_expr("big orc*", Glob).as_deref(), Some("\"big\" \"orc\"*"));
    }

    #[test]
    fn test_fts_and_scan_return_identical_results() {
        // THE load-bearing test. Every other failure here is loud; a routing bug
        // is silent — it just returns fewer rows than it should.
        let db = TempDb::new("fts_equiv");
        let corpus = [
            "a dragon roars nearby",
            "\x1b[1;33mthe dragon\x1b[0m flees west",
            "dragon scales glitter",
            "an orc appears",
            "orcs and dragons together",
            "ORC in capitals",
            "the orc-mage casts",
            "Dragonfly lands",
            "nothing interesting here",
            "café society",
            "big orc warrior",
        ];
        {
            let mut sdb = ScrollbackDb::open(db.path()).expect("open");
            let tx = sdb.sender();
            for (i, line) in corpus.iter().enumerate() {
                let (wid, wseq) = tx.alloc_wseq("uuid-a", "Zmc").expect("alloc");
                let mut e = ArchiveEntry::new("Zmc".into(), 1_000 + i as i64, (*line).into(), false);
                e.wid = Some(wid);
                e.wseq = Some(wseq);
                assert!(tx.send(e));
            }
            sdb.flush_and_close();
        }

        let patterns: Vec<(&str, PatternKind)> = vec![
            // These route to FTS *and* match, so a routing bug shows up as a
            // difference rather than as two equally empty results.
            ("a dragon*", PatternKind::Glob), ("an orc*", PatternKind::Glob),
            ("big orc*", PatternKind::Glob), ("orcs and*", PatternKind::Glob),
            ("orc", PatternKind::Glob), ("orc*", PatternKind::Glob),
            ("dragon", PatternKind::Glob), ("dragon*", PatternKind::Glob),
            ("*dragon*", PatternKind::Glob), ("*orc*", PatternKind::Glob),
            ("ORC", PatternKind::Glob), ("Dragon*", PatternKind::Glob),
            ("big orc", PatternKind::Glob), ("big orc*", PatternKind::Glob),
            ("or?c", PatternKind::Glob), ("or*c", PatternKind::Glob),
            ("orc-mage", PatternKind::Glob), ("café*", PatternKind::Glob),
            ("*", PatternKind::Glob), ("nothing*", PatternKind::Glob),
            ("zzz*", PatternKind::Glob), ("dragonfly", PatternKind::Glob),
            ("rc", PatternKind::Simple), ("orc", PatternKind::Simple),
            ("dragon", PatternKind::Simple), ("ORC", PatternKind::Simple),
            ("orc-mage", PatternKind::Simple),
            (r"^a \w+ roars", PatternKind::Regex),
            (r"dragon|orc", PatternKind::Regex),
            (r"^ORC", PatternKind::Regex),
        ];

        for (pat, kind) in patterns {
            let with_fts = query(db.path(), &ArchiveQuery {
                world: Some("Zmc"), pattern: pat, kind,
                before_wseq: None, since_ms: None, until_ms: None,
                limit: 10_000, newest_first: true,
            });
            // Force the scan path by disabling the gate, then restore it.
            let conn = Connection::open(db.path()).expect("open");
            meta_set(&conn, "migration_state", "fts");
            drop(conn);
            let scan_only = query(db.path(), &ArchiveQuery {
                world: Some("Zmc"), pattern: pat, kind,
                before_wseq: None, since_ms: None, until_ms: None,
                limit: 10_000, newest_first: true,
            });
            let conn = Connection::open(db.path()).expect("open");
            meta_set(&conn, "migration_state", "done");
            drop(conn);

            let a: Vec<&str> = with_fts.iter().map(|l| l.text.as_str()).collect();
            let b: Vec<&str> = scan_only.iter().map(|l| l.text.as_str()).collect();
            assert_eq!(
                a, b,
                "FTS and scan disagree for pattern {pat:?} ({kind:?}) — \
                 the index dropped rows the regex would have matched"
            );
        }
    }

    #[test]
    fn test_a_common_term_falls_back_to_the_scan_instead_of_using_fts() {
        // A term appearing in most lines produces an enormous candidate set, and
        // probing it per row costs far more than the ordered range scan it would
        // replace (measured 131ms vs 17ms). The guard must notice and step aside —
        // and must still return the SAME rows either way.
        let db = TempDb::new("fts_common");
        {
            let mut sdb = ScrollbackDb::open(db.path()).expect("open");
            let tx = sdb.sender();
            // Well past FTS_SELECTIVITY_CAP so the guard definitely trips.
            for i in 0..(FTS_SELECTIVITY_CAP + 500) {
                let (wid, wseq) = tx.alloc_wseq("uuid-a", "Zmc").expect("alloc");
                let mut e = ArchiveEntry::new("Zmc".into(), i, "common word here".into(), false);
                e.wid = Some(wid);
                e.wseq = Some(wseq);
                assert!(tx.send(e));
            }
            // One rare line, to prove the selective path still works on the same table.
            let (wid, wseq) = tx.alloc_wseq("uuid-a", "Zmc").expect("alloc");
            let mut e = ArchiveEntry::new("Zmc".into(), 999_999, "zebra sighting".into(), false);
            e.wid = Some(wid);
            e.wseq = Some(wseq);
            assert!(tx.send(e));
            sdb.flush_and_close();
        }

        let conn = open_reader(db.path()).expect("reader");
        // Common term: guard trips, so no MATCH expression survives.
        let common = fts_match_expr("common*", PatternKind::Glob).expect("routable");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (SELECT rowid FROM output_fts WHERE output_fts MATCH ?1 LIMIT ?2)",
                params![common, FTS_SELECTIVITY_CAP],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n >= FTS_SELECTIVITY_CAP, "the fixture must actually be non-selective (probe={n})");
        drop(conn);

        // Results must be identical regardless of which path was taken.
        let hits = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "common word*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None,
            limit: 100_000, newest_first: true,
        });
        assert_eq!(hits.len() as i64, FTS_SELECTIVITY_CAP + 500);

        let rare = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "zebra*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None,
            limit: 100, newest_first: true,
        });
        assert_eq!(rare.len(), 1, "a selective term still finds its row through the index");
    }

    #[test]
    fn test_load_before_wseq_returns_the_rows_just_before_the_boundary() {
        let db = TempDb::new("pageup_wseq");
        let seqs = seed_v2(db.path(), "Zmc", 100);
        let lines = load_before_wseq(db.path(), "Zmc", seqs[50], 10);
        let got: Vec<i64> = lines.iter().filter_map(|l| l.wseq).collect();
        assert_eq!(got, seqs[40..50].to_vec(), "the 10 immediately older rows, oldest-first");
        assert!(
            got.iter().all(|s| *s < seqs[50]),
            "nothing at or past the boundary — a Page Up must not re-show what memory holds"
        );
        // Case-insensitive, matching in-memory world lookup.
        assert_eq!(load_before_wseq(db.path(), "zMC", seqs[50], 10).len(), 10);
        // Unknown world, and a boundary at the very start, both return nothing.
        assert!(load_before_wseq(db.path(), "Nope", 50, 10).is_empty());
        assert!(load_before_wseq(db.path(), "Zmc", 0, 10).is_empty());
    }

    #[test]
    fn test_a_line_is_findable_through_fts_immediately_after_it_is_written() {
        let db = TempDb::new("fts_live");
        {
            let mut sdb = ScrollbackDb::open(db.path()).expect("open");
            let tx = sdb.sender();
            let (wid, wseq) = tx.alloc_wseq("uuid-a", "Zmc").expect("alloc");
            let mut e = ArchiveEntry::new("Zmc".into(), 1, "a wandering minstrel".into(), false);
            e.wid = Some(wid);
            e.wseq = Some(wseq);
            assert!(tx.send(e));
            sdb.flush_and_close();
        }
        let hits = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "a wandering*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None,
            limit: 10, newest_first: true,
        });
        assert_eq!(hits.len(), 1, "the write path must index as it inserts, not on a later rebuild");
    }

    #[test]
    fn test_flush_batch_retains_the_batch_when_the_transaction_cannot_start() {
        // Regression guard for `Err(_) => { batch.clear(); return; }`, which threw
        // away up to a full batch of MUD output on a transient lock.
        let db = TempDb::new("retain");
        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        conn.execute_batch(SCHEMA_V2).expect("schema");
        conn.execute_batch(OUTPUT_LOG_V2).expect("output_log");

        let stats = Arc::new(ArchiveStats::default());
        let mut batch = vec![
            ArchiveEntry::new("Zmc".to_string(), 1, "a".to_string(), false),
            ArchiveEntry::new("Zmc".to_string(), 2, "b".to_string(), false),
        ];

        // An already-open transaction on the SAME connection makes
        // unchecked_transaction fail immediately.
        conn.execute_batch("BEGIN;").expect("occupy the connection");
        flush_batch(&conn, &mut batch, &stats, true, &mut std::collections::HashMap::new());
        assert_eq!(batch.len(), 2, "the batch must survive a failed transaction start");
        assert_eq!(stats.write_failed.load(Ordering::Relaxed), 0, "retained is not failed");

        conn.execute_batch("COMMIT;").expect("release");
        flush_batch(&conn, &mut batch, &stats, true, &mut std::collections::HashMap::new());
        assert!(batch.is_empty(), "the retry must then drain it");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM output_log", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 2, "no line may be lost across the retry");
    }
}
