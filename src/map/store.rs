//! Maps on disk: `~/.clay/maps.db`, one map per world, keyed by the world's id
//! (`World::world_id`), so renaming a world keeps its map. Its own file, not the
//! scrollback archive's (off by default; a map is wanted without it). Writes go to a
//! thread that batches them; a load is answered by the same thread.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use rusqlite::{params, Connection};

use super::model::{MapMeta, Room};

/// The newest schema this build knows; a newer file is left alone.
const SCHEMA_VERSION: i64 = 1;

enum Op {
    Upsert { uuid: String, name: String, rooms: Vec<Room>, meta: MapMeta },
    Load { uuid: String, reply: Sender<Option<(Vec<Room>, MapMeta)>> },
    Forget { uuid: String },
    /// Delete the maps of worlds that no longer exist (`live` are the ids that do) and
    /// that haven't changed since `before_ms`.
    Purge { live: Vec<String>, before_ms: i64 },
    Flush(Sender<()>),
}

/// How long a deleted world's map is kept (in case the world comes back, e.g. imported
/// again with its id) before it is purged.
pub const ORPHAN_KEEP_MS: i64 = 30 * 24 * 60 * 60 * 1000;

pub struct MapStore {
    tx: Sender<Op>,
}

impl MapStore {
    /// Open (or create) the file and start its writer. `None` when it can't be used:
    /// unwritable, or from a newer Clay.
    pub fn open(path: PathBuf) -> Option<MapStore> {
        let conn = Connection::open(&path).ok()?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;");
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).ok()?;
        if version > SCHEMA_VERSION {
            crate::debug_log(true, &format!("maps.db: schema {version} is newer than this Clay's {SCHEMA_VERSION}; the map is not saved"));
            return None;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS maps (uuid TEXT PRIMARY KEY, name TEXT NOT NULL, meta TEXT NOT NULL, updated_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS rooms (uuid TEXT NOT NULL, rid TEXT NOT NULL, room TEXT NOT NULL, PRIMARY KEY (uuid, rid)) WITHOUT ROWID;
             PRAGMA user_version = 1;",
        )
        .ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name("maps-writer".into()).spawn(move || writer(conn, rx)).ok()?;
        Some(MapStore { tx })
    }

    /// Save these rooms (and the map's meta) of a world's map.
    pub fn upsert(&self, uuid: &str, name: &str, rooms: Vec<Room>, meta: MapMeta) {
        let _ = self.tx.send(Op::Upsert { uuid: uuid.to_string(), name: name.to_string(), rooms, meta });
    }

    /// A world's saved map (waits for it, a few seconds at most).
    pub fn load(&self, uuid: &str) -> Option<(Vec<Room>, MapMeta)> {
        let (reply, answer) = mpsc::channel();
        self.tx.send(Op::Load { uuid: uuid.to_string(), reply }).ok()?;
        answer.recv_timeout(Duration::from_secs(5)).ok().flatten()
    }

    /// Delete a world's map.
    pub fn forget(&self, uuid: &str) {
        let _ = self.tx.send(Op::Forget { uuid: uuid.to_string() });
    }

    /// Purge the maps of deleted worlds untouched for `ORPHAN_KEEP_MS`: every world that
    /// still exists is in `live` (an empty id is no world).
    pub fn purge_orphans(&self, live: Vec<String>) {
        let _ = self.tx.send(Op::Purge { live, before_ms: now_ms() - ORPHAN_KEEP_MS });
    }

    /// Wait until everything sent so far is written (before a reload or quit).
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        if self.tx.send(Op::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(5));
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The writer: upserts batched into one transaction until the queue goes quiet.
fn writer(mut conn: Connection, rx: Receiver<Op>) {
    let mut pending: Vec<(String, String, Vec<Room>, MapMeta)> = Vec::new();
    let write = |conn: &mut Connection, pending: &mut Vec<(String, String, Vec<Room>, MapMeta)>| {
        if pending.is_empty() {
            return;
        }
        let Ok(tx) = conn.transaction() else { return };
        for (uuid, name, rooms, meta) in pending.drain(..) {
            let meta = serde_json::to_string(&meta).unwrap_or_default();
            let _ = tx.execute(
                "INSERT INTO maps (uuid, name, meta, updated_ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(uuid) DO UPDATE SET name = excluded.name, meta = excluded.meta, updated_ms = excluded.updated_ms",
                params![uuid, name, meta, now_ms()],
            );
            for room in rooms {
                if let Ok(json) = serde_json::to_string(&room) {
                    let _ = tx.execute("INSERT OR REPLACE INTO rooms (uuid, rid, room) VALUES (?1, ?2, ?3)", params![uuid, room.id, json]);
                }
            }
        }
        let _ = tx.commit();
    };
    loop {
        let op = if pending.is_empty() { rx.recv().ok() } else { rx.recv_timeout(Duration::from_millis(250)).ok() };
        match op {
            None if pending.is_empty() => return, // the store is gone
            None => write(&mut conn, &mut pending),
            Some(Op::Upsert { uuid, name, rooms, meta }) => {
                pending.push((uuid, name, rooms, meta));
                if pending.iter().map(|p| p.2.len()).sum::<usize>() > 500 {
                    write(&mut conn, &mut pending);
                }
            }
            Some(Op::Load { uuid, reply }) => {
                write(&mut conn, &mut pending);
                let _ = reply.send(read(&conn, &uuid));
            }
            Some(Op::Forget { uuid }) => {
                write(&mut conn, &mut pending);
                let _ = conn.execute("DELETE FROM rooms WHERE uuid = ?1", params![uuid]);
                let _ = conn.execute("DELETE FROM maps WHERE uuid = ?1", params![uuid]);
            }
            Some(Op::Purge { live, before_ms }) => {
                write(&mut conn, &mut pending);
                purge(&mut conn, &live, before_ms);
            }
            Some(Op::Flush(done)) => {
                write(&mut conn, &mut pending);
                let _ = done.send(());
            }
        }
    }
}

fn purge(conn: &mut Connection, live: &[String], before_ms: i64) {
    let old: Vec<(String, String)> = match conn.prepare("SELECT uuid, name FROM maps WHERE updated_ms < ?1") {
        Ok(mut stmt) => stmt.query_map(params![before_ms], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default(),
        Err(_) => return,
    };
    let Ok(tx) = conn.transaction() else { return };
    for (uuid, name) in old.into_iter().filter(|(uuid, _)| !live.iter().any(|l| l == uuid)) {
        let _ = tx.execute("DELETE FROM rooms WHERE uuid = ?1", params![uuid]);
        let _ = tx.execute("DELETE FROM maps WHERE uuid = ?1", params![uuid]);
        crate::debug_log(true, &format!("maps.db: purged the map of deleted world {name} ({uuid})"));
    }
    let _ = tx.commit();
}

fn read(conn: &Connection, uuid: &str) -> Option<(Vec<Room>, MapMeta)> {
    let meta: String = conn.query_row("SELECT meta FROM maps WHERE uuid = ?1", params![uuid], |r| r.get(0)).ok()?;
    let meta: MapMeta = serde_json::from_str(&meta).unwrap_or_default();
    let mut stmt = conn.prepare("SELECT room FROM rooms WHERE uuid = ?1").ok()?;
    let rooms = stmt
        .query_map(params![uuid], |r| r.get::<_, String>(0))
        .ok()?
        .filter_map(|r| r.ok())
        .filter_map(|json| serde_json::from_str::<Room>(&json).ok())
        .collect();
    Some((rooms, meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_written_reads_back_and_is_forgotten() {
        let dir = std::env::temp_dir().join(format!("clay-maps-test-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("maps.db");
        let g = crate::map::path::tests_support::rome();
        let store = MapStore::open(path.clone()).expect("opens");
        let rooms: Vec<Room> = g.rooms.values().cloned().collect();
        store.upsert("w1", "Rome", rooms.clone(), MapMeta { y_sign: -1, y_doubts: 0 });
        let (back, meta) = store.load("w1").expect("there");
        assert_eq!(back.len(), rooms.len());
        assert_eq!(meta.y_sign, -1);
        assert!(back.iter().any(|r| r.id == "M" && r.exits.iter().any(|e| e.cmd == "w")));
        assert!(store.load("w2").is_none(), "another world has none");
        store.forget("w1");
        assert!(store.load("w1").is_none());
        // A deleted world's map goes once it is old; a live world's never does.
        store.upsert("gone", "Gone", rooms.clone(), MapMeta::default());
        store.upsert("kept", "Kept", rooms.clone(), MapMeta::default());
        store.flush();
        store.purge_orphans(vec!["kept".into()]);
        assert!(store.load("gone").is_some(), "a deleted world's map is kept a while");
        let _ = store.tx.send(Op::Purge { live: vec!["kept".into()], before_ms: now_ms() + 1000 });
        assert!(store.load("gone").is_none(), "then purged");
        assert!(store.load("kept").is_some(), "a live world's map stays however old");
        drop(store);
        // A file from a newer Clay is left alone.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 9;").unwrap();
        drop(conn);
        assert!(MapStore::open(path).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
