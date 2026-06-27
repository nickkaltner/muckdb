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

/// Pidfile used as the single-instance guard for the daemon.
pub fn pid_file() -> Result<PathBuf> {
    Ok(state_dir()?.join("daemon.pid"))
}

/// Log file the detached daemon redirects stdout/stderr into.
pub fn daemon_log() -> Result<PathBuf> {
    Ok(state_dir()?.join("daemon.log"))
}
