//! The facade role: muckdb's default behaviour.
//!
//! Ensures the background daemon is running, records the invocation in the
//! shared store, then transparently runs the real `duckdb` binary so muckdb is
//! a drop-in replacement for the duckdb CLI.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::store::{self, Phase, Record};

/// TCP port the daemon's HTTP/WS server binds.
pub const PORT: u16 = 11000;

/// duckdb flags that consume the following argument as their value. Used when
/// scanning passthrough args to find the positional database filename.
const VALUE_FLAGS: &[&str] = &[
    "-c",
    "-cmd",
    "-s",
    "-init",
    "-f",
    "-separator",
    "-nullvalue",
    "-newline",
    "-maxrows",
    "-maxwidth",
    "-log",
    "-csv", // (some take values across duckdb versions)
];

/// Best-effort extraction of the database file from duckdb-style args.
///
/// Heuristic (good enough for the MVP): skip flags and any value they consume;
/// the first remaining positional token that isn't `:memory:` is the database.
/// Returns an absolute path so the daemon can open it regardless of its cwd.
pub fn detect_db_path(args: &[String], cwd: &Path) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with('-') {
            if VALUE_FLAGS.contains(&arg.as_str()) {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if arg == ":memory:" {
            return None;
        }
        let p = PathBuf::from(arg);
        let abs = if p.is_absolute() { p } else { cwd.join(p) };
        return Some(abs.to_string_lossy().into_owned());
    }
    None
}

/// Returns true if something is already listening on the daemon port.
fn daemon_listening() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// Start the daemon if it isn't already running, returning once it accepts
/// connections (or the timeout elapses).
pub fn ensure_daemon() -> Result<()> {
    if daemon_listening() {
        return Ok(());
    }

    let exe = std::env::current_exe().context("locating muckdb executable")?;
    // The spawned process daemonizes itself (fork + setsid), so its direct
    // parent exits almost immediately; we reap it below to avoid a zombie.
    let mut child = Command::new(exe)
        .arg("--__serve")
        .spawn()
        .context("spawning muckdb daemon")?;

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if daemon_listening() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.wait();
    Ok(())
}

/// Run muckdb in facade mode: ensure the daemon, log the invocation, run
/// `duckdb` with the given args, log completion, and return duckdb's exit code.
pub fn passthrough(args: &[String]) -> Result<i32> {
    ensure_daemon()?;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let db_path = detect_db_path(args, &cwd);
    let id = store::now_millis()
        .wrapping_mul(1000)
        .wrapping_add(std::process::id() as u64);

    store::append(&Record {
        id,
        ts: store::now_millis(),
        cwd: cwd.to_string_lossy().into_owned(),
        args: args.to_vec(),
        db_path: db_path.clone(),
        phase: Phase::Start,
        exit_code: None,
    })?;

    // Inherit stdio so muckdb behaves exactly like duckdb (interactive shell,
    // pipes, colours, exit code).
    let status = Command::new("duckdb")
        .args(args)
        .status()
        .context("failed to run `duckdb` — is it installed and on PATH?")?;
    let code = status.code().unwrap_or(1);

    store::append(&Record {
        id,
        ts: store::now_millis(),
        cwd: cwd.to_string_lossy().into_owned(),
        args: args.to_vec(),
        db_path,
        phase: Phase::End,
        exit_code: Some(code),
    })?;

    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(args: &[&str]) -> Option<String> {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        detect_db_path(&owned, Path::new("/work"))
    }

    #[test]
    fn no_db_for_flags_only() {
        assert_eq!(detect(&["--version"]), None);
        assert_eq!(detect(&[]), None);
    }

    #[test]
    fn in_memory_is_none() {
        assert_eq!(detect(&[":memory:"]), None);
    }

    #[test]
    fn absolute_db_path_is_kept() {
        assert_eq!(detect(&["/tmp/x.db"]), Some("/tmp/x.db".to_string()));
    }

    #[test]
    fn relative_db_path_is_resolved_against_cwd() {
        assert_eq!(detect(&["rel.db"]), Some("/work/rel.db".to_string()));
    }

    #[test]
    fn value_flag_consumes_its_argument() {
        // -c takes the SQL; the db is the trailing positional.
        assert_eq!(
            detect(&["-c", "SELECT 1", "/tmp/x.db"]),
            Some("/tmp/x.db".to_string())
        );
    }

    #[test]
    fn boolean_flag_does_not_consume_db() {
        assert_eq!(
            detect(&["-readonly", "/tmp/x.db"]),
            Some("/tmp/x.db".to_string())
        );
    }
}
