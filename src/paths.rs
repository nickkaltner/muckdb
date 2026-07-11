//! Per-OS filesystem locations for muckdb's shared state.
//!
//! Uses the `directories` crate so paths are correct on both Linux
//! (XDG: `~/.local/share/muckdb`, `~/.local/state/muckdb`) and macOS
//! (`~/Library/Application Support/muckdb`).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("dev", "muckdb", "muckdb")
        .context("could not determine a home directory for muckdb state")
}

/// Directory holding the append-only history store. Created if missing.
pub fn data_dir() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    let dir = dirs.data_dir().to_path_buf();
    fs::create_dir_all(&dir).with_context(|| format!("creating data dir {dir:?}"))?;
    Ok(dir)
}

/// Directory holding runtime state (pidfile, daemon log). Created if missing.
pub fn state_dir() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    // `state_dir` is only defined on Linux; fall back to the data dir elsewhere.
    let dir = dirs
        .state_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dirs.data_dir().to_path_buf());
    fs::create_dir_all(&dir).with_context(|| format!("creating state dir {dir:?}"))?;
    Ok(dir)
}

/// The append-only JSONL history store shared between the CLI and daemon.
pub fn history_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("history.jsonl"))
}

/// Pidfile used as the single-instance guard for the daemon on `port`.
///
/// The default port keeps the historic `daemon.pid` name for backward
/// compatibility; any other port gets a port-suffixed name so daemons on
/// different ports don't fight over one pidfile lock.
pub fn pid_file(port: u16) -> Result<PathBuf> {
    let name = if port == crate::facade::PORT {
        "daemon.pid".to_string()
    } else {
        format!("daemon-{port}.pid")
    };
    Ok(state_dir()?.join(name))
}

/// Log file the detached daemon on `port` redirects stdout/stderr into.
/// Suffixed by port like [`pid_file`] so concurrent daemons don't share a log.
pub fn daemon_log(port: u16) -> Result<PathBuf> {
    let name = if port == crate::facade::PORT {
        "daemon.log".to_string()
    } else {
        format!("daemon-{port}.log")
    };
    Ok(state_dir()?.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::PORT;

    #[test]
    fn default_port_keeps_legacy_filenames() {
        assert_eq!(pid_file(PORT).unwrap().file_name().unwrap(), "daemon.pid");
        assert_eq!(daemon_log(PORT).unwrap().file_name().unwrap(), "daemon.log");
    }

    #[test]
    fn non_default_port_is_suffixed() {
        assert_eq!(
            pid_file(11055).unwrap().file_name().unwrap(),
            "daemon-11055.pid"
        );
        assert_eq!(
            daemon_log(11055).unwrap().file_name().unwrap(),
            "daemon-11055.log"
        );
    }
}
