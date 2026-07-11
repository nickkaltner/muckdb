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

/// Default TCP port the daemon's HTTP/WS server binds.
pub const PORT: u16 = 11000;

/// The port the daemon should use, resolved consistently across the process.
///
/// Priority: an explicit `--port <N>` CLI flag (which `main` records into the
/// `MUCKDB_PORT` env var so every consumer — and any spawned child — agrees),
/// else a pre-existing `MUCKDB_PORT`, else the default [`PORT`]. A `0`/invalid
/// value falls back to the default.
pub fn resolved_port() -> u16 {
    std::env::var("MUCKDB_PORT")
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|&p| p != 0)
        .unwrap_or(PORT)
}

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
    let addr = SocketAddr::from(([127, 0, 0, 1], resolved_port()));
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
    // Propagate the resolved port explicitly so the child re-resolves to the
    // same value (it re-reads MUCKDB_PORT).
    let mut child = Command::new(exe)
        .arg("--__serve")
        .env("MUCKDB_PORT", resolved_port().to_string())
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
    // Surface a non-loopback bind on the starting terminal too — the daemon's
    // own warning only reaches its detached log.
    if let Some(w) = crate::server::public_bind_warning() {
        eprintln!("{w}");
    }
    Ok(())
}

/// Run muckdb in facade mode: ensure the daemon, log the invocation, run
/// `duckdb` with the given args, log completion, and return duckdb's exit code.
pub fn passthrough(args: &[String]) -> Result<i32> {
    ensure_daemon()?;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let db_path = detect_db_path(args, &cwd);
    // Group this invocation under a session if the agent set one.
    let session = std::env::var("MUCKDB_SESSION")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| crate::session::slug(&s));
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
        session: session.clone(),
        forget: false,
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
        session,
        forget: false,
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
