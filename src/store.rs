//! The shared on-disk store: an append-only JSONL log of muckdb invocations.
//!
//! Both the CLI (facade role) and the daemon read this file; only the CLI
//! appends. Appends use `O_APPEND` so concurrent writers never corrupt each
//! other, and the daemon watches the file for changes to push live updates.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
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

/// Current wall-clock time in milliseconds since the Unix epoch.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
    let mut history: Vec<Invocation> = Vec::new();
    for rec in records {
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

    // Databases, most-recently-touched first, de-duplicated.
    let mut databases: Vec<String> = Vec::new();
    for inv in history.iter().rev() {
        if let Some(db) = &inv.db_path
            && !databases.contains(db)
        {
            databases.push(db.clone());
        }
    }

    State { history, databases }
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
        }
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
