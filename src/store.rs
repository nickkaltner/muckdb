//! The shared on-disk store: an append-only JSONL log of muckdb invocations.
//!
//! Both the CLI (facade role) and the daemon read this file; only the CLI
//! appends. Appends use `O_APPEND` so concurrent writers never corrupt each
//! other, and the daemon watches the file for changes to push live updates.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

/// Whether a record marks the beginning or completion of an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Start,
    End,
}

/// One line in the JSONL store. A single invocation emits a `Start` then an
/// `End` record sharing the same `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
    pub cwd: String,
    pub args: Vec<String>,
    /// The database file the invocation targets, if any (`None` ⇒ in-memory).
    #[serde(default)]
    pub db_path: Option<String>,
    pub phase: Phase,
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// The session this invocation belongs to (from `MUCKDB_SESSION`), if any.
    #[serde(default)]
    pub session: Option<String>,
    /// A "forget this database" tombstone rather than an invocation: hides
    /// `db_path` from the databases list until something touches it again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub forget: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A folded view of one invocation, derived from its `Start`/`End` records.
#[derive(Debug, Clone, Serialize)]
pub struct Invocation {
    pub id: u64,
    pub ts: u64,
    pub cwd: String,
    pub args: Vec<String>,
    pub db_path: Option<String>,
    pub exit_code: Option<i32>,
    pub running: bool,
    pub session: Option<String>,
}

/// The derived state the daemon serves: invocation history (newest last) plus
/// the set of databases seen, most-recent first.
#[derive(Debug, Clone, Serialize, Default)]
pub struct State {
    pub history: Vec<Invocation>,
    pub databases: Vec<String>,
}

/// A short, stable id for a database path (for clean URLs and CLI references).
pub fn db_id(path: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("{:016x}", h.finish())[..8].to_string()
}

/// Current wall-clock time in milliseconds since the Unix epoch.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Write `data` to `path` atomically: stream it into a uniquely-named temp file
/// in the same directory, fsync, then `rename` it over `path`. rename(2) is
/// atomic on one filesystem, so a concurrent reader (the daemon watcher, an
/// export) always sees either the whole old file or the whole new one — never a
/// truncated half-write. Callers that also need to avoid *lost updates* under
/// concurrent read-modify-write must serialize around their own load+write.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("muckdb");
    let tmp = dir.join(format!(
        ".{stem}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {path:?}"))
}

/// Append one record to the store as a single JSON line.
pub fn append(record: &Record) -> Result<()> {
    let path = paths::history_file()?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening history store {path:?}"))?;
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    file.write_all(line.as_bytes())
        .with_context(|| format!("appending to history store {path:?}"))?;
    Ok(())
}

/// Read every record from the store, skipping malformed lines.
pub fn read_all() -> Result<Vec<Record>> {
    let path = paths::history_file()?;
    let file = match OpenOptions::new().read(true).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading history store {path:?}")),
    };
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<Record>(&line) {
            records.push(rec);
        }
    }
    Ok(records)
}

/// Fold raw records into the served `State`.
pub fn derive_state(records: &[Record]) -> State {
    // The most recent "forget" tombstone per database path.
    let mut forgets: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for rec in records.iter().filter(|r| r.forget) {
        if let Some(db) = &rec.db_path {
            let e = forgets.entry(db).or_default();
            *e = (*e).max(rec.ts);
        }
    }
    let mut history: Vec<Invocation> = Vec::new();
    for rec in records.iter().filter(|r| !r.forget) {
        match rec.phase {
            Phase::Start => history.push(Invocation {
                id: rec.id,
                ts: rec.ts,
                cwd: rec.cwd.clone(),
                args: rec.args.clone(),
                db_path: rec.db_path.clone(),
                exit_code: None,
                running: true,
                session: rec.session.clone(),
            }),
            Phase::End => {
                if let Some(inv) = history.iter_mut().rev().find(|i| i.id == rec.id) {
                    inv.exit_code = rec.exit_code;
                    inv.running = false;
                    if inv.db_path.is_none() {
                        inv.db_path = rec.db_path.clone();
                    }
                }
            }
        }
    }

    // Databases, most-recently-touched first, de-duplicated. A db whose most
    // recent touch predates its last forget tombstone stays hidden (using it
    // again resurfaces it).
    let mut databases: Vec<String> = Vec::new();
    for inv in history.iter().rev() {
        if let Some(db) = &inv.db_path
            && !databases.contains(db)
            && forgets.get(db.as_str()).is_none_or(|&f| inv.ts > f)
        {
            databases.push(db.clone());
        }
    }

    State { history, databases }
}

/// Append a "forget this database" tombstone (the web UI's "remove this db").
pub fn forget_db(path: &str) -> Result<()> {
    append(&Record {
        id: now_millis(),
        ts: now_millis(),
        cwd: String::new(),
        args: vec!["db".into(), "forget".into()],
        db_path: Some(path.to_string()),
        phase: Phase::End,
        exit_code: Some(0),
        session: None,
        forget: true,
    })
}

/// Read the store and return the derived state in one step.
pub fn load_state() -> Result<State> {
    Ok(derive_state(&read_all()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: u64, phase: Phase, db: Option<&str>, exit: Option<i32>) -> Record {
        Record {
            id,
            ts: id,
            cwd: "/work".into(),
            args: vec!["x.db".into()],
            db_path: db.map(str::to_string),
            phase,
            exit_code: exit,
            session: None,
            forget: false,
        }
    }

    #[test]
    fn forget_hides_a_db_until_it_is_used_again() {
        let mut forget = rec(3, Phase::End, Some("/a.db"), Some(0));
        forget.forget = true;
        // Used (ts 1-2), forgotten (ts 3) → hidden; not an invocation either.
        let records = vec![
            rec(1, Phase::Start, Some("/a.db"), None),
            rec(2, Phase::End, Some("/a.db"), Some(0)),
            forget.clone(),
        ];
        let state = derive_state(&records);
        assert!(state.databases.is_empty());
        assert_eq!(state.history.len(), 1); // the tombstone is not history

        // Touched again after the tombstone → resurfaces.
        let mut records = records;
        records.push(rec(9, Phase::Start, Some("/a.db"), None));
        records.push(rec(9, Phase::End, Some("/a.db"), Some(0)));
        let state = derive_state(&records);
        assert_eq!(state.databases, vec!["/a.db".to_string()]);
    }

    #[test]
    fn start_then_end_folds_into_one_completed_invocation() {
        let records = [
            rec(1, Phase::Start, Some("/a.db"), None),
            rec(1, Phase::End, Some("/a.db"), Some(0)),
        ];
        let state = derive_state(&records);
        assert_eq!(state.history.len(), 1);
        assert!(!state.history[0].running);
        assert_eq!(state.history[0].exit_code, Some(0));
        assert_eq!(state.databases, vec!["/a.db".to_string()]);
    }

    #[test]
    fn pending_start_is_marked_running() {
        let records = [rec(7, Phase::Start, None, None)];
        let state = derive_state(&records);
        assert!(state.history[0].running);
        assert!(state.databases.is_empty());
    }

    #[test]
    fn db_id_is_stable_and_distinct_per_path() {
        // The id must be deterministic across processes/restarts — the web UI
        // persists `/db/<id>/` links and resolves them after the daemon restarts.
        assert_eq!(db_id("/data/ponds.duckdb"), db_id("/data/ponds.duckdb"));
        assert_ne!(db_id("/data/ponds.duckdb"), db_id("/data/other.duckdb"));
        // 8 lowercase hex chars.
        let id = db_id("/data/ponds.duckdb");
        assert_eq!(id.len(), 8);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn databases_are_deduped_most_recent_first() {
        let records = [
            rec(1, Phase::Start, Some("/a.db"), None),
            rec(1, Phase::End, Some("/a.db"), Some(0)),
            rec(2, Phase::Start, Some("/b.db"), None),
            rec(2, Phase::End, Some("/b.db"), Some(0)),
            rec(3, Phase::Start, Some("/a.db"), None),
            rec(3, Phase::End, Some("/a.db"), Some(0)),
        ];
        let state = derive_state(&records);
        assert_eq!(
            state.databases,
            vec!["/a.db".to_string(), "/b.db".to_string()]
        );
    }
}
