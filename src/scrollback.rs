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

/// Current archive schema. 0 means "any pre-v2 shape" (with or without the
/// `gagged` column); those are distinguished with `PRAGMA table_info`. 1 is
/// deliberately skipped so a stamped legacy file can never be mistaken for an
/// unstamped one.
const SCHEMA_VERSION: i32 = 2;

/// Rows copied per migration transaction. Small enough to keep the WAL bounded
/// on a multi-GB archive, large enough that the per-transaction overhead is noise.
const MIGRATE_CHUNK: usize = 50_000;

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
    /// Durable archive sequence, when the row has one (v2 schema).
    pub wseq: Option<i64>,
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

/// A world's stable identity as the archive knows it. Passed into `open` so the
/// migration can attribute legacy rows (which recorded only a name) to the world
/// that currently claims that name.
#[derive(Clone, Debug)]
pub struct WorldRef {
    pub world_uuid: String,
    pub name: String,
}

/// What `open` found on disk.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ArchiveState {
    /// Schema is current and usable.
    Ready,
    /// A pre-v2 file that still needs converting; reads use the legacy SQL.
    NeedsMigration,
    /// `user_version` is ahead of this build. Refuse to migrate or write: a newer
    /// Clay owns the file and guessing at its shape risks destroying it. Readers
    /// report this rather than returning an empty result, which would look
    /// exactly like an empty archive.
    TooNew,
}

/// The v2 schema. Written verbatim for a fresh database and created alongside
/// the legacy table during a migration.
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

/// `output_log` in its v2 shape. Parameterised by name so the migration can build
/// it under a temporary name alongside the legacy table.
fn output_log_v2_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (
             id       INTEGER PRIMARY KEY,
             wid      INTEGER NOT NULL,
             wseq     INTEGER NOT NULL,
             ts_ms    INTEGER NOT NULL,
             line_raw TEXT NOT NULL,
             gagged   INTEGER NOT NULL DEFAULT 0
         );"
    )
}

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

/// Does `table` have a column called `column`? Generalises the `PRAGMA table_info`
/// check `export_csv` already used to cope with pre-`gagged` databases.
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
        _ => {
            if table_exists(conn, "output_log") && has_column(conn, "output_log", "wid") {
                ArchiveState::Ready
            } else {
                ArchiveState::NeedsMigration
            }
        }
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
/// filled — and a long schema migration running on the writer thread would fill
/// it within seconds.
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

/// Convert a pre-v2 archive in place. Idempotent and resumable: killing the
/// process at any point and re-running reaches the same final state.
///
/// The ordering rule that matters: nothing in the legacy table is touched until
/// M6, and M6 is a single transaction. A reader therefore never observes a torn
/// pair — it sees either the whole legacy shape or the whole v2 shape.
fn migrate_to_v2(conn: &Connection, worlds: &[WorldRef]) -> Result<(), String> {
    // ---- M2: build the new objects alongside the old ones -----------------
    conn.execute_batch(SCHEMA_V2).map_err(|e| format!("create v2 objects: {e}"))?;
    conn.execute_batch(&output_log_v2_ddl("output_log_v2"))
        .map_err(|e| format!("create output_log_v2: {e}"))?;
    meta_set(conn, "migration_state", "copying");

    let legacy_has_gagged = has_column(conn, "output_log", "gagged");

    // ---- M3: attribute every legacy world name ----------------------------
    let names: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT world FROM output_log")
            .map_err(|e| format!("list worlds: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| format!("list worlds: {e}"))?;
        rows.flatten().collect()
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    for name in &names {
        // Case-insensitive, because in-memory lookup is case-insensitive while
        // SQLite `world = ?` is not: "Zmc" and "zmc" are one world to the user.
        let claimed = worlds
            .iter()
            .find(|w| w.name.eq_ignore_ascii_case(name) && !w.world_uuid.is_empty());
        let (uuid, orphan) = match claimed {
            Some(w) => (w.world_uuid.clone(), 0),
            // No live world answers to this name — it was renamed or deleted
            // before this migration existed, and the legacy schema recorded only
            // the name, so there is nothing to match on. Keep the rows under a
            // minted id rather than dropping them.
            None => (uuid::Uuid::new_v4().simple().to_string(), 1),
        };
        conn.execute(
            "INSERT INTO worlds (world_uuid, name, next_wseq, orphan, created_ms)
             VALUES (?1, ?2, 0, ?3, ?4)
             ON CONFLICT(world_uuid) DO UPDATE SET name = excluded.name",
            params![uuid, name, orphan, now_ms],
        )
        .map_err(|e| format!("record world {name}: {e}"))?;
    }

    // ---- M4: copy in resumable chunks -------------------------------------
    for name in &names {
        let wid: i64 = conn
            .query_row(
                "SELECT wid FROM worlds WHERE name = ?1 ORDER BY wid LIMIT 1",
                params![name],
                |r| r.get(0),
            )
            .map_err(|e| format!("resolve wid for {name}: {e}"))?;

        loop {
            let last: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(id), 0) FROM output_log_v2 WHERE wid = ?1",
                    params![wid],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let base: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(wseq), -1) + 1 FROM output_log_v2 WHERE wid = ?1",
                    params![wid],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            // `id` is the true insertion order and is unique; ts_ms is neither
            // (timestamps collide, which is what made the old boundary unsound).
            let gagged_expr = if legacy_has_gagged { "COALESCE(o.gagged, 0)" } else { "0" };
            let sql = format!(
                "INSERT INTO output_log_v2 (id, wid, wseq, ts_ms, line_raw, gagged)
                 SELECT o.id, ?1, ?2 + row_number() OVER (ORDER BY o.id) - 1,
                        o.ts_ms, o.line_raw, {gagged_expr}
                 FROM output_log o
                 WHERE o.world = ?3 AND o.id > ?4
                 ORDER BY o.id LIMIT ?5"
            );
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin copy chunk: {e}"))?;
            let n = tx
                .execute(&sql, params![wid, base, name, last, MIGRATE_CHUNK as i64])
                .map_err(|e| format!("copy chunk for {name}: {e}"))?;
            tx.commit().map_err(|e| format!("commit copy chunk: {e}"))?;
            if n == 0 {
                break;
            }
        }

        // Seed the durable allocator past everything just copied.
        conn.execute(
            "UPDATE worlds SET next_wseq =
               (SELECT COALESCE(MAX(wseq), -1) + 1 FROM output_log_v2 WHERE wid = ?1)
             WHERE wid = ?1",
            params![wid],
        )
        .map_err(|e| format!("seed next_wseq: {e}"))?;
    }

    // ---- M5: the no-loss gate ---------------------------------------------
    let old_n: i64 = conn
        .query_row("SELECT COUNT(*) FROM output_log", [], |r| r.get(0))
        .map_err(|e| format!("count legacy rows: {e}"))?;
    let new_n: i64 = conn
        .query_row("SELECT COUNT(*) FROM output_log_v2", [], |r| r.get(0))
        .map_err(|e| format!("count copied rows: {e}"))?;
    if old_n != new_n {
        return Err(format!(
            "row count mismatch after copy ({old_n} legacy vs {new_n} copied); \
             leaving the archive on the legacy schema"
        ));
    }

    // ---- M6: atomic swap ---------------------------------------------------
    // Drop the old FTS table FIRST. It is external-content over `output_log` and
    // stores that linkage in its own config table, which ALTER TABLE RENAME does
    // not rewrite — renaming first would leave it silently pointing at the legacy
    // table. It has never been populated, so nothing is lost.
    let tx = conn.unchecked_transaction().map_err(|e| format!("begin swap: {e}"))?;
    tx.execute_batch(
        // The legacy external-content FTS table must go BEFORE the rename: it
        // records its `content='output_log'` link in its own config table, which
        // ALTER TABLE RENAME does not rewrite. It was never populated, so nothing
        // is lost. The contentless replacement is created after the swap.
        "DROP TABLE IF EXISTS output_fts;
         DROP INDEX IF EXISTS idx_world_ts;
         ALTER TABLE output_log    RENAME TO output_log_legacy;
         ALTER TABLE output_log_v2 RENAME TO output_log;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_log_wid_wseq ON output_log(wid, wseq);
         CREATE INDEX IF NOT EXISTS idx_log_wid_ts ON output_log(wid, ts_ms);",
    )
    .map_err(|e| format!("swap tables: {e}"))?;
    // Written inside the swap so readers have a single committed signal; the
    // PRAGMA below cannot participate in a transaction on every build.
    tx.execute(
        "INSERT INTO archive_meta (k, v) VALUES ('migration_state', 'swapped')
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        [],
    )
    .map_err(|e| format!("mark swapped: {e}"))?;
    tx.commit().map_err(|e| format!("commit swap: {e}"))?;

    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
        .map_err(|e| format!("stamp user_version: {e}"))?;
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS output_fts USING fts5(
             txt, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');",
    )
    .map_err(|e| format!("create fts table: {e}"))?;

    // ---- M7: build the FTS index, resumably --------------------------------
    // `INSERT INTO output_fts(output_fts) VALUES('rebuild')` does NOT work on a
    // contentless table — there is no content to read back — so this is a
    // row-by-row pass, chunked so a kill mid-build resumes rather than restarts.
    meta_set(conn, "migration_state", "fts");
    loop {
        let last: i64 = meta_get(conn, "fts_last_id")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let rows: Vec<(i64, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, line_raw FROM output_log WHERE id > ?1 ORDER BY id LIMIT ?2")
                .map_err(|e| format!("prepare fts scan: {e}"))?;
            let it = stmt
                .query_map(params![last, MIGRATE_CHUNK as i64], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| format!("fts scan: {e}"))?;
            it.flatten().collect()
        };
        if rows.is_empty() {
            break;
        }
        let tx = conn.unchecked_transaction().map_err(|e| format!("begin fts chunk: {e}"))?;
        let mut high = last;
        for (id, raw) in &rows {
            tx.execute(
                "INSERT INTO output_fts (rowid, txt) VALUES (?1, ?2)",
                params![id, crate::util::strip_ansi_codes(raw)],
            )
            .map_err(|e| format!("index row {id}: {e}"))?;
            high = high.max(*id);
        }
        tx.execute(
            "INSERT INTO archive_meta (k, v) VALUES ('fts_last_id', ?1)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![high.to_string()],
        )
        .map_err(|e| format!("record fts progress: {e}"))?;
        tx.commit().map_err(|e| format!("commit fts chunk: {e}"))?;
    }
    let _ = conn.execute("INSERT INTO output_fts(output_fts) VALUES('optimize')", []);

    // ---- M8: reclaim (best effort) ----------------------------------------
    meta_set(conn, "migration_state", "done");
    let _ = conn.execute_batch("DROP TABLE IF EXISTS output_log_legacy;");
    // VACUUM needs roughly the database size in free space and rewrites the whole
    // file; a failure leaves a correct, merely larger, archive.
    if let Err(e) = conn.execute_batch("VACUUM;") {
        crate::debug_log(true, &format!("SCROLLBACK: VACUUM after migration failed: {e}"));
    }
    Ok(())
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
    pub fn open(path: &Path, worlds: &[WorldRef]) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        tune_connection(&conn);
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let state = archive_state(&conn);
        if state == ArchiveState::TooNew {
            crate::debug_log(true, &format!(
                "SCROLLBACK: {} was written by a newer Clay (user_version {}); \
                 not migrating and not writing to it",
                path.display(), user_version(&conn)
            ));
        } else if !table_exists(&conn, "output_log") {
            // Fresh database: create v2 directly, no migration to run.
            conn.execute_batch(SCHEMA_V2)?;
            conn.execute_batch(&output_log_v2_ddl("output_log"))?;
            conn.execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_log_wid_wseq ON output_log(wid, wseq);
                 CREATE INDEX IF NOT EXISTS idx_log_wid_ts ON output_log(wid, ts_ms);",
            )?;
            meta_set(&conn, "migration_state", "done");
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        } else if state == ArchiveState::Ready {
            // Already v2; make sure the auxiliary objects exist (a file half-built
            // by an older interrupted run).
            conn.execute_batch(SCHEMA_V2)?;
        }
        drop(conn);

        let db_path = path.to_path_buf();
        let (tx, rx) = mpsc::channel::<ArchiveEntry>();
        let writer_path = db_path.clone();
        let stats = Arc::new(ArchiveStats::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let writer_stats = Arc::clone(&stats);
        let writer_shutdown = Arc::clone(&shutdown);
        let writer_worlds: Vec<WorldRef> = worlds.to_vec();

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

                // Convert before serving the queue. Entries pile up in the
                // (unbounded) channel meanwhile — which is exactly why the bounded
                // try_send channel had to go first: it would have dropped every
                // line after the first few thousand.
                if archive_state(&conn) == ArchiveState::NeedsMigration {
                    let t0 = Instant::now();
                    crate::debug_log(true, "SCROLLBACK: migrating archive to schema v2...");
                    match migrate_to_v2(&conn, &writer_worlds) {
                        Ok(()) => crate::debug_log(true, &format!(
                            "SCROLLBACK: migration finished in {:?}", t0.elapsed()
                        )),
                        Err(e) => crate::debug_log(true, &format!(
                            "SCROLLBACK: migration aborted ({e}); continuing on the legacy schema"
                        )),
                    }
                }
                let schema_ready = archive_state(&conn) == ArchiveState::Ready;
                let too_new = archive_state(&conn) == ArchiveState::TooNew;
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
                                flush_batch(&conn, &mut batch, &writer_stats, schema_ready && !too_new, &mut wid_cache);
                                if batch.len() == before {
                                    std::thread::sleep(Duration::from_millis(20));
                                    flush_batch(&conn, &mut batch, &writer_stats, schema_ready && !too_new, &mut wid_cache);
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
                            flush_batch(&conn, &mut batch, &writer_stats, schema_ready && !too_new, &mut wid_cache);
                            if batch.len() == before {
                                // Could not write; retrying immediately would spin.
                                // Give the lock a moment, then try once more.
                                std::thread::sleep(Duration::from_millis(20));
                                flush_batch(&conn, &mut batch, &writer_stats, schema_ready && !too_new, &mut wid_cache);
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
                        flush_batch(&conn, &mut batch, &writer_stats, schema_ready && !too_new, &mut wid_cache);
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
        let alloc = Connection::open(path).ok().map(|c| {
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

        // The v2 table has no `world` column (worlds are a dimension table) and no
        // `line_text` (it is recomputed). Both shapes must be readable: the
        // migration runs on the writer thread, so a read can land before it.
        let sql = if archive_state(&conn) == ArchiveState::Ready {
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
            let where_v2 = if conds.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", conds.join(" AND "))
            };
            format!(
                "SELECT o.ts_ms, w.name, o.line_raw FROM output_log o
                 JOIN worlds w ON w.wid = o.wid {} ORDER BY o.ts_ms ASC",
                where_v2
            )
        } else {
            format!(
                "SELECT ts_ms, world, line_raw FROM output_log {} ORDER BY ts_ms ASC",
                where_clause
            )
        };

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
        // Pre-migration file: fall back to the legacy path so the first run after
        // an upgrade still answers, just without the sequence boundary.
        return ScrollbackDb::search(
            path, q.world, q.pattern, q.since_ms, q.until_ms, q.limit,
            q.kind == PatternKind::Regex,
        );
    }

    let regex = build_pattern_regex(q.pattern, q.kind);
    // FTS is a CANDIDATE FILTER only: every surviving row is still matched by the
    // regex above. Over-matching from tokenisation is harmless; under-matching is
    // a silent wrong answer, which is why `fts_match_expr` refuses anything it
    // cannot represent exactly.
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
            out.push(ScrollbackLine { ts_ms, world, text, wseq: Some(wseq) });
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
        "SELECT ts_ms, line_raw, wseq FROM output_log
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

    // Fetch the N rows just before the timestamp, then reverse to get oldest-first
    let sql = if archive_state(&conn) == ArchiveState::Ready {
        "SELECT o.ts_ms, w.name, o.line_raw FROM output_log o \
         JOIN worlds w ON w.wid = o.wid \
         WHERE w.name = ?1 COLLATE NOCASE AND o.ts_ms < ?2 \
         ORDER BY o.ts_ms DESC LIMIT ?3"
    } else {
        "SELECT ts_ms, world, line_raw FROM output_log \
         WHERE world = ?1 AND ts_ms < ?2 \
         ORDER BY ts_ms DESC LIMIT ?3"
    };

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
    // Match on name for a world the migration never saw. Phase 4 replaces this
    // with a uuid-keyed lookup driven by the producer.
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
fn flush_batch(
    conn: &Connection,
    batch: &mut Vec<ArchiveEntry>,
    stats: &Arc<ArchiveStats>,
    v2: bool,
    wid_cache: &mut std::collections::HashMap<String, (i64, i64)>,
) {
    if batch.is_empty() {
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
        let res = if v2 {
            // Prefer the producer's number; fall back to writer-side assignment
            // so a line is never dropped just because the allocator was busy.
            let slot = match (row.wid, row.wseq) {
                (Some(wid), Some(wseq)) => Some((wid, wseq)),
                _ => resolve_wid(&tx, &row.world, wid_cache),
            };
            match slot {
                Some((wid, wseq)) => {
                    let r = tx.execute(
                        "INSERT INTO output_log (wid, wseq, ts_ms, line_raw, gagged)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![wid, wseq, row.ts_ms, row.text, row.gagged as i32],
                    );
                    if r.is_ok() {
                        // Index the ANSI-stripped form, in the same transaction, so
                        // the index can never disagree with the table. There are no
                        // triggers because a trigger cannot strip ANSI.
                        let rowid = tx.last_insert_rowid();
                        let _ = tx.execute(
                            "INSERT INTO output_fts (rowid, txt) VALUES (?1, ?2)",
                            params![rowid, crate::util::strip_ansi_codes(&row.text)],
                        );
                    }
                    r
                }
                None => {
                    failed.push(row);
                    continue;
                }
            }
        } else {
            let line_text = crate::util::strip_ansi_codes(&row.text);
            tx.execute(
                "INSERT INTO output_log (ts_ms, world, line_raw, line_text, gagged) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![row.ts_ms, row.world, row.text, line_text, row.gagged as i32],
            )
        };
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
/// Columns: id, world, datetime_local, ts_epoch_ms, gagged, text, raw
/// Rows sorted by world ascending then timestamp ascending.
/// Returns `Err(String)` with a human-readable message on failure.
pub fn export_csv(db_path: &Path, out_path: &Path) -> Result<usize, String> {
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("cannot open database: {}", e))?;
    tune_connection(&conn);

    // Check whether the gagged column exists (absent in DBs before this migration).
    let has_gagged: bool = {
        let mut stmt = conn.prepare("PRAGMA table_info(output_log)")
            .map_err(|e| format!("PRAGMA table_info failed: {}", e))?;
        let cols: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("query_map failed: {}", e))?
            .flatten()
            .collect();
        cols.iter().any(|c| c == "gagged")
    };

    if archive_state(&conn) == ArchiveState::Ready {
        return export_csv_v2(&conn, out_path);
    }
    let sql = if has_gagged {
        "SELECT id, ts_ms, world, line_raw, line_text, gagged \
         FROM output_log ORDER BY world ASC, ts_ms ASC"
    } else {
        "SELECT id, ts_ms, world, line_raw, line_text, 0 \
         FROM output_log ORDER BY world ASC, ts_ms ASC"
    };

    let mut stmt = conn.prepare(sql)
        .map_err(|e| format!("prepare failed: {}", e))?;

    use std::io::Write as _;
    let mut file = std::fs::File::create(out_path)
        .map_err(|e| format!("cannot create {}: {}", out_path.display(), e))?;

    // Header row
    writeln!(file, "id,world,datetime_local,ts_epoch_ms,gagged,text,raw")
        .map_err(|e| format!("write error: {}", e))?;

    let mut count = 0usize;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,    // id
            row.get::<_, i64>(1)?,    // ts_ms
            row.get::<_, String>(2)?, // world
            row.get::<_, String>(3)?, // line_raw
            row.get::<_, String>(4)?, // line_text
            row.get::<_, i32>(5)?,    // gagged
        ))
    }).map_err(|e| format!("query failed: {}", e))?;

    for row in rows.flatten() {
        let (id, ts_ms, world, line_raw, line_text, gagged_int) = row;
        let ts_secs = ts_ms / 1000;
        let lt = crate::util::local_time_from_epoch(ts_secs);
        let datetime_str = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            lt.year, lt.month, lt.day, lt.hour, lt.minute, lt.second
        );
        let gagged_str = if gagged_int != 0 { "true" } else { "false" };
        writeln!(
            file,
            "{},{},{},{},{},{},{}",
            id,
            csv_escape(&world),
            csv_escape(&datetime_str),
            ts_ms,
            gagged_str,
            csv_escape(&line_text),
            csv_escape(&line_raw),
        ).map_err(|e| format!("write error: {}", e))?;
        count += 1;
    }

    Ok(count)
}

/// v2 export. `line_text` is no longer stored, so the stripped column is derived
/// the same way `search` already derives it for matching. The six legacy columns
/// keep their exact names, order and formatting; `wseq` is appended.
fn export_csv_v2(conn: &Connection, out_path: &Path) -> Result<usize, String> {
    use std::io::Write;
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
// This module had none before the schema work, and nothing anywhere else in the
// crate opens a rusqlite `Connection` in a test. Every function here takes an
// explicit `&Path`, so a test can never reach the developer's real
// `~/.clay/scrollback.db` — but that only holds as long as new code keeps taking
// the path as a parameter rather than calling `clay_config_path` internally.
//
// The point of this module is to pin CURRENT behaviour before the v2 schema
// lands, so the migration can be shown to preserve it.
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

    /// Build a database in the pre-v2 schema. `with_gagged` reproduces the two
    /// legacy shapes in the wild: databases created before the `gagged` column
    /// existed, and those created after.
    ///
    /// Rows are `(ts_ms, world, line_raw, gagged)`; `line_text` is derived the
    /// same way the writer thread derives it.
    fn write_legacy_db(path: &Path, with_gagged: bool, rows: &[(i64, &str, &str, bool)]) {
        let conn = Connection::open(path).expect("open legacy db");
        if with_gagged {
            conn.execute_batch(
                "CREATE TABLE output_log (
                     id        INTEGER PRIMARY KEY,
                     ts_ms     INTEGER NOT NULL,
                     world     TEXT NOT NULL,
                     line_raw  TEXT NOT NULL,
                     line_text TEXT NOT NULL,
                     gagged    INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX idx_world_ts ON output_log(world, ts_ms);",
            )
            .expect("create legacy schema");
        } else {
            conn.execute_batch(
                "CREATE TABLE output_log (
                     id        INTEGER PRIMARY KEY,
                     ts_ms     INTEGER NOT NULL,
                     world     TEXT NOT NULL,
                     line_raw  TEXT NOT NULL,
                     line_text TEXT NOT NULL
                 );
                 CREATE INDEX idx_world_ts ON output_log(world, ts_ms);",
            )
            .expect("create pre-gagged legacy schema");
        }
        for (ts_ms, world, line_raw, gagged) in rows {
            let line_text = crate::util::strip_ansi_codes(line_raw);
            if with_gagged {
                conn.execute(
                    "INSERT INTO output_log (ts_ms, world, line_raw, line_text, gagged)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![ts_ms, world, line_raw, line_text, *gagged as i32],
                )
                .expect("insert legacy row");
            } else {
                conn.execute(
                    "INSERT INTO output_log (ts_ms, world, line_raw, line_text)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![ts_ms, world, line_raw, line_text],
                )
                .expect("insert pre-gagged legacy row");
            }
        }
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
        write_legacy_db(db.path(), true, &fixture());

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
        write_legacy_db(db.path(), true, &fixture());

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
        write_legacy_db(db.path(), true, &fixture());

        let hits = ScrollbackDb::search(db.path(), Some("O'Brien"), "*", None, None, 100, false);
        assert_eq!(hits.len(), 1, "an apostrophe in a world name must not break the query");
        assert_eq!(hits[0].world, "O'Brien");
    }

    #[test]
    fn test_search_all_worlds_when_world_is_none() {
        let db = TempDb::new("search_all");
        write_legacy_db(db.path(), true, &fixture());

        let hits = ScrollbackDb::search(db.path(), None, "*dragon*", None, None, 100, false);
        assert_eq!(hits.len(), 5, "no world filter searches every world");
    }

    #[test]
    fn test_search_regex_mode() {
        let db = TempDb::new("search_regex");
        write_legacy_db(db.path(), true, &fixture());

        let hits = ScrollbackDb::search(
            db.path(), Some("Zmc"), r"^a \w+ (roars|appears)", None, None, 100, true,
        );
        assert_eq!(hits.len(), 2, "regex mode compiles the pattern verbatim");
    }

    #[test]
    fn test_search_time_bounds() {
        let db = TempDb::new("search_time");
        write_legacy_db(db.path(), true, &fixture());

        let hits = ScrollbackDb::search(
            db.path(), Some("Zmc"), "*", Some(2_000), Some(3_000), 100, false,
        );
        let ts: Vec<i64> = hits.iter().map(|l| l.ts_ms).collect();
        assert_eq!(ts, vec![2_000, 3_000], "since/until are inclusive on both ends");
    }

    #[test]
    fn test_search_returns_ascending_and_limit_keeps_the_oldest() {
        let db = TempDb::new("search_limit");
        write_legacy_db(db.path(), true, &fixture());

        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*", None, None, 2, false);
        let ts: Vec<i64> = hits.iter().map(|l| l.ts_ms).collect();
        // Pinning today's behaviour deliberately: results come back oldest-first
        // and the limit therefore keeps the OLDEST matches, hiding recent history
        // on a large archive. The v2 query reverses this on purpose.
        assert_eq!(ts, vec![1_000, 2_000]);
    }

    #[test]
    fn test_search_missing_database_returns_empty() {
        let db = TempDb::new("search_missing");
        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*", None, None, 100, false);
        assert!(hits.is_empty(), "a missing file must not panic");
    }

    #[test]
    fn test_search_reads_a_pre_gagged_database() {
        let db = TempDb::new("search_nogagged");
        write_legacy_db(db.path(), false, &fixture());

        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*dragon*", None, None, 100, false);
        assert_eq!(
            hits.len(), 3,
            "search never selects `gagged`, so it works on a database predating that column"
        );
    }

    // -- load_before_path --------------------------------------------------

    #[test]
    fn test_load_before_path_returns_oldest_first_and_excludes_the_boundary() {
        let db = TempDb::new("loadbefore");
        write_legacy_db(db.path(), true, &fixture());

        let lines = load_before_path(db.path(), "Zmc", 4_000, 10);
        let ts: Vec<i64> = lines.iter().map(|l| l.ts_ms).collect();
        assert_eq!(ts, vec![1_000, 2_000, 3_000], "strictly `< before_ts_ms`, oldest first");
    }

    #[test]
    fn test_load_before_path_count_keeps_the_newest_and_still_sorts_oldest_first() {
        let db = TempDb::new("loadbefore_count");
        write_legacy_db(db.path(), true, &fixture());

        let lines = load_before_path(db.path(), "Zmc", 5_000, 2);
        let ts: Vec<i64> = lines.iter().map(|l| l.ts_ms).collect();
        assert_eq!(ts, vec![3_000, 4_000], "takes the N nearest the boundary, then reverses");
    }

    #[test]
    fn test_load_before_path_does_not_filter_gagged() {
        let db = TempDb::new("loadbefore_gagged");
        write_legacy_db(db.path(), true, &fixture());

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
        write_legacy_db(db.path(), true, &fixture());
        let out = std::env::temp_dir().join(format!("clay_test_csv_{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&out);

        let n = export_csv(db.path(), &out).expect("export");
        assert_eq!(n, 6);

        let body = std::fs::read_to_string(&out).expect("read csv");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            lines[0], "id,world,datetime_local,ts_epoch_ms,gagged,text,raw",
            "the header is a user-facing contract"
        );
        // Sorted by world ASC then ts ASC: O'Brien, Other, then Zmc's four.
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
    fn test_export_csv_on_a_pre_gagged_database() {
        let db = TempDb::new("csv_nogagged");
        write_legacy_db(db.path(), false, &fixture());
        let out = std::env::temp_dir()
            .join(format!("clay_test_csv_ng_{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&out);

        let n = export_csv(db.path(), &out).expect("export from pre-gagged db");
        assert_eq!(n, 6, "the PRAGMA table_info fallback substitutes 0 for the missing column");
        let body = std::fs::read_to_string(&out).expect("read csv");
        assert!(body.lines().skip(1).all(|l| l.contains(",false,")));

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
            let sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
        let mut sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
        write_legacy_db(db.path(), true, &fixture());

        let writer = Connection::open(db.path()).expect("writer conn");
        tune_connection(&writer);
        writer.execute_batch("BEGIN IMMEDIATE;").expect("take the write lock");
        writer
            .execute(
                "INSERT INTO output_log (ts_ms, world, line_raw, line_text, gagged) \
                 VALUES (9000, 'Zmc', 'held', 'held', 0)",
                [],
            )
            .expect("write inside the held transaction");

        // A WAL reader must still see the committed snapshot while that lock is held.
        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*dragon*", None, None, 100, false);
        assert_eq!(hits.len(), 3, "reads must succeed while a write transaction is open");

        writer.execute_batch("COMMIT;").expect("release");
    }

    // -- migration ---------------------------------------------------------

    fn wseqs_for(conn: &Connection, world: &str) -> Vec<i64> {
        let mut stmt = conn
            .prepare(
                "SELECT o.wseq FROM output_log o JOIN worlds w ON w.wid = o.wid
                 WHERE w.name = ?1 ORDER BY o.id",
            )
            .expect("prepare");
        stmt.query_map(params![world], |r| r.get::<_, i64>(0))
            .expect("query")
            .flatten()
            .collect()
    }

    #[test]
    fn test_migration_preserves_every_row_and_rebuilds_wseq_in_id_order() {
        let db = TempDb::new("mig_basic");
        write_legacy_db(db.path(), true, &fixture());

        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        let worlds = vec![WorldRef { world_uuid: "uuid-zmc".into(), name: "Zmc".into() }];
        migrate_to_v2(&conn, &worlds).expect("migration");

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert_eq!(
            meta_get(&conn, "migration_state").as_deref(), Some("done"),
            "state must reach 'done' so readers can enable the fast path"
        );

        let n: i64 = conn.query_row("SELECT COUNT(*) FROM output_log", [], |r| r.get(0)).unwrap();
        assert_eq!(n, fixture().len() as i64, "no row may be lost");

        // wseq is dense from 0 per world, in id (insertion) order — never ts order.
        assert_eq!(wseqs_for(&conn, "Zmc"), vec![0, 1, 2, 3]);
        assert_eq!(wseqs_for(&conn, "Other"), vec![0]);
        assert_eq!(wseqs_for(&conn, "O'Brien"), vec![0]);

        // The dropped column is really gone, and the legacy table with it.
        assert!(!has_column(&conn, "output_log", "line_text"));
        assert!(!has_column(&conn, "output_log", "world"));
        assert!(!table_exists(&conn, "output_log_legacy"));

        // The allocator is seeded past everything copied.
        let next: i64 = conn
            .query_row(
                "SELECT next_wseq FROM worlds w WHERE w.name = 'Zmc'", [], |r| r.get(0))
            .unwrap();
        assert!(next >= 4, "next_wseq must clear the migrated rows, got {next}");
    }

    #[test]
    fn test_migration_attributes_claimed_worlds_and_orphans_the_rest() {
        let db = TempDb::new("mig_attr");
        write_legacy_db(db.path(), true, &fixture());

        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        // Only Zmc is claimed, and deliberately with different casing: memory
        // matches names case-insensitively while SQL does not.
        let worlds = vec![WorldRef { world_uuid: "uuid-zmc".into(), name: "zMC".into() }];
        migrate_to_v2(&conn, &worlds).expect("migration");

        let claimed: String = conn
            .query_row("SELECT world_uuid FROM worlds WHERE name = 'Zmc'", [], |r| r.get(0))
            .expect("Zmc row");
        assert_eq!(claimed, "uuid-zmc", "case-insensitive match must claim the world");
        let orphan_flag: i64 = conn
            .query_row("SELECT orphan FROM worlds WHERE name = 'Zmc'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphan_flag, 0);

        let orphans: i64 = conn
            .query_row("SELECT COUNT(*) FROM worlds WHERE orphan = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 2, "Other and O'Brien have no live world; they become orphans");

        // Orphaned history is preserved, never dropped.
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM output_log o JOIN worlds w ON w.wid = o.wid
                 WHERE w.orphan = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 2, "orphan rows must survive the migration");
    }

    #[test]
    fn test_migration_resumes_from_a_partial_copy_without_duplicating() {
        // Simulates a process killed mid-copy: output_log_v2 already holds some
        // rows. The re-run must continue, not restart and not duplicate.
        let db = TempDb::new("mig_resume");
        write_legacy_db(db.path(), true, &fixture());

        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        conn.execute_batch(SCHEMA_V2).unwrap();
        conn.execute_batch(&output_log_v2_ddl("output_log_v2")).unwrap();
        conn.execute(
            "INSERT INTO worlds (world_uuid, name, next_wseq, orphan, created_ms)
             VALUES ('uuid-zmc', 'Zmc', 0, 0, 0)", []).unwrap();
        let wid: i64 = conn
            .query_row("SELECT wid FROM worlds WHERE name='Zmc'", [], |r| r.get(0)).unwrap();
        // First two Zmc rows already copied, ids 1 and 2.
        conn.execute(
            "INSERT INTO output_log_v2 (id, wid, wseq, ts_ms, line_raw, gagged)
             SELECT id, ?1, id - 1, ts_ms, line_raw, gagged FROM output_log
             WHERE world='Zmc' AND id <= 2", params![wid]).unwrap();
        meta_set(&conn, "migration_state", "copying");

        let worlds = vec![WorldRef { world_uuid: "uuid-zmc".into(), name: "Zmc".into() }];
        migrate_to_v2(&conn, &worlds).expect("resumed migration");

        let n: i64 = conn.query_row("SELECT COUNT(*) FROM output_log", [], |r| r.get(0)).unwrap();
        assert_eq!(n, fixture().len() as i64, "resume must not duplicate or drop rows");
        assert_eq!(wseqs_for(&conn, "Zmc"), vec![0, 1, 2, 3], "sequence stays dense across a resume");
    }

    #[test]
    fn test_migration_of_a_pre_gagged_database() {
        let db = TempDb::new("mig_nogagged");
        write_legacy_db(db.path(), false, &fixture());

        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        migrate_to_v2(&conn, &[]).expect("migration from pre-gagged schema");

        let n: i64 = conn.query_row("SELECT COUNT(*) FROM output_log", [], |r| r.get(0)).unwrap();
        assert_eq!(n, fixture().len() as i64);
        let gagged: i64 = conn
            .query_row("SELECT COUNT(*) FROM output_log WHERE gagged = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(gagged, 0, "a database with no gagged column defaults every row to 0");
    }

    #[test]
    fn test_open_migrates_a_legacy_database_end_to_end() {
        let db = TempDb::new("mig_open");
        write_legacy_db(db.path(), true, &fixture());

        {
            let worlds = vec![WorldRef { world_uuid: "uuid-zmc".into(), name: "Zmc".into() }];
            let mut sdb = ScrollbackDb::open(db.path(), &worlds).expect("open");
            let tx = sdb.sender();
            // A line archived while (or just after) the migration runs must land.
            assert!(tx.send(ArchiveEntry::new("Zmc".to_string(), 9_000, "post-migration line".to_string(), false)));
            sdb.flush_and_close();
        }

        let conn = Connection::open(db.path()).expect("reopen");
        assert_eq!(archive_state(&conn), ArchiveState::Ready);
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM output_log", [], |r| r.get(0)).unwrap();
        assert_eq!(n, fixture().len() as i64 + 1, "migrated rows plus the new one");

        // And it is reachable through the normal read path.
        let hits = ScrollbackDb::search(db.path(), Some("Zmc"), "*post-migration*", None, None, 10, false);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn test_search_and_page_up_agree_before_and_after_migration() {
        // The read paths must return the same thing either side of the swap;
        // that is what makes the migration invisible to the user.
        let db = TempDb::new("mig_reads");
        write_legacy_db(db.path(), true, &fixture());

        let before_search: Vec<String> =
            ScrollbackDb::search(db.path(), Some("Zmc"), "*dragon*", None, None, 100, false)
                .into_iter().map(|l| l.text).collect();
        let before_page: Vec<String> =
            load_before_path(db.path(), "Zmc", 5_000, 10).into_iter().map(|l| l.text).collect();
        assert!(!before_search.is_empty() && !before_page.is_empty());

        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        migrate_to_v2(&conn, &[WorldRef { world_uuid: "u".into(), name: "Zmc".into() }])
            .expect("migration");
        drop(conn);

        let after_search: Vec<String> =
            ScrollbackDb::search(db.path(), Some("Zmc"), "*dragon*", None, None, 100, false)
                .into_iter().map(|l| l.text).collect();
        let after_page: Vec<String> =
            load_before_path(db.path(), "Zmc", 5_000, 10).into_iter().map(|l| l.text).collect();

        assert_eq!(before_search, after_search, "search results must be unchanged by migration");
        assert_eq!(before_page, after_page, "Page Up results must be unchanged by migration");
    }

    #[test]
    fn test_export_csv_after_migration_keeps_the_legacy_columns() {
        let db = TempDb::new("mig_csv");
        write_legacy_db(db.path(), true, &fixture());
        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        migrate_to_v2(&conn, &[]).expect("migration");
        drop(conn);

        let out = std::env::temp_dir().join(format!("clay_test_csv_v2_{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let n = export_csv(db.path(), &out).expect("export");
        assert_eq!(n, 6);
        let body = std::fs::read_to_string(&out).expect("read");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            lines[0], "id,world,datetime_local,ts_epoch_ms,gagged,text,raw,wseq",
            "the six legacy columns keep their names and order; wseq is appended"
        );
        // `text` is still the stripped form even though the column no longer exists.
        let coloured = lines.iter().find(|l| l.contains("flees west")).expect("coloured row");
        assert!(coloured.contains("the dragon flees west"));
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
        let sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
            let sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
        let a = ScrollbackDb::open(db.path(), &[]).expect("open a");
        let b = ScrollbackDb::open(db.path(), &[]).expect("open b");
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
        let sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
            let mut sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
            let mut sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
        let mut sdb = ScrollbackDb::open(path, &[]).expect("open");
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
            let mut sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
            let mut sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
    fn test_load_before_wseq_declines_on_a_pre_migration_archive() {
        // Must return empty rather than error, so the caller falls back to the
        // timestamp path instead of showing the user nothing.
        let db = TempDb::new("pageup_legacy");
        write_legacy_db(db.path(), true, &fixture());
        assert!(load_before_wseq(db.path(), "Zmc", 999, 10).is_empty());
        assert!(!load_before_path(db.path(), "Zmc", 9_999, 10).is_empty());
    }

    #[test]
    fn test_a_line_is_findable_through_fts_immediately_after_it_is_written() {
        let db = TempDb::new("fts_live");
        {
            let mut sdb = ScrollbackDb::open(db.path(), &[]).expect("open");
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
    fn test_migration_builds_the_fts_index_over_existing_rows() {
        let db = TempDb::new("fts_migrate");
        write_legacy_db(db.path(), true, &fixture());
        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        migrate_to_v2(&conn, &[WorldRef { world_uuid: "u".into(), name: "Zmc".into() }])
            .expect("migration");
        let indexed: i64 = conn
            .query_row("SELECT COUNT(*) FROM output_fts", [], |r| r.get(0))
            .expect("count fts");
        assert_eq!(indexed, fixture().len() as i64, "every migrated row must be indexed");
        assert_eq!(meta_get(&conn, "migration_state").as_deref(), Some("done"));
        drop(conn);

        // And the index actually answers.
        let hits = query(db.path(), &ArchiveQuery {
            world: Some("Zmc"), pattern: "a goblin*", kind: PatternKind::Glob,
            before_wseq: None, since_ms: None, until_ms: None,
            limit: 10, newest_first: true,
        });
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn test_flush_batch_retains_the_batch_when_the_transaction_cannot_start() {
        // Regression guard for `Err(_) => { batch.clear(); return; }`, which threw
        // away up to a full batch of MUD output on a transient lock.
        let db = TempDb::new("retain");
        let conn = Connection::open(db.path()).expect("open");
        tune_connection(&conn);
        conn.execute_batch(
            "CREATE TABLE output_log (
                 id INTEGER PRIMARY KEY, ts_ms INTEGER NOT NULL, world TEXT NOT NULL,
                 line_raw TEXT NOT NULL, line_text TEXT NOT NULL,
                 gagged INTEGER NOT NULL DEFAULT 0);",
        )
        .expect("schema");

        let stats = Arc::new(ArchiveStats::default());
        let mut batch = vec![
            ArchiveEntry::new("Zmc".to_string(), 1, "a".to_string(), false),
            ArchiveEntry::new("Zmc".to_string(), 2, "b".to_string(), false),
        ];

        // An already-open transaction on the SAME connection makes
        // unchecked_transaction fail immediately.
        conn.execute_batch("BEGIN;").expect("occupy the connection");
        flush_batch(&conn, &mut batch, &stats, false, &mut std::collections::HashMap::new());
        assert_eq!(batch.len(), 2, "the batch must survive a failed transaction start");
        assert_eq!(stats.write_failed.load(Ordering::Relaxed), 0, "retained is not failed");

        conn.execute_batch("COMMIT;").expect("release");
        flush_batch(&conn, &mut batch, &stats, false, &mut std::collections::HashMap::new());
        assert!(batch.is_empty(), "the retry must then drain it");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM output_log", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 2, "no line may be lost across the retry");
    }
}
