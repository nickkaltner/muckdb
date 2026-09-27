//! Identity of the executable that serves the HTTP API.

use std::fs;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Identity {
    pub version: String,
    pub commit: String,
    pub executable: String,
    pub binary_size: u64,
    pub binary_modified_ns: u128,
    pub pid: u32,
    pub port: u16,
}

pub fn current(port: u16) -> Result<Identity> {
    let executable = std::env::current_exe().context("locating muckdb executable")?;
    let metadata = fs::metadata(&executable).context("reading muckdb executable metadata")?;
    let modified = metadata
        .modified()
        .context("reading muckdb executable modification time")?
        .duration_since(UNIX_EPOCH)
        .context("muckdb executable predates the Unix epoch")?;
    Ok(Identity {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("MUCKDB_COMMIT").to_string(),
        executable: executable.to_string_lossy().into_owned(),
        binary_size: metadata.len(),
        binary_modified_ns: modified.as_nanos(),
        pid: std::process::id(),
        port,
    })
}

impl Identity {
    pub fn same_binary(&self, other: &Self) -> bool {
        self.executable == other.executable
            && self.binary_size == other.binary_size
            && self.binary_modified_ns == other.binary_modified_ns
    }
}
