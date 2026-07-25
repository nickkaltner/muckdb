//! Daily, cached check for the latest muckdb GitHub release.
//!
//! The daemon performs this away from its request threads. The cache records
//! both when it last contacted GitHub and the most recently fetched version,
//! so normal launches make at most one request per 24 hours.

use std::fs;
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{paths, store};

const RELEASE_URL: &str = "https://api.github.com/repos/nickkaltner/muckdb/releases/latest";
const DAY_MILLIS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CachedCheck {
    #[serde(default)]
    last_checked_at: u64,
    #[serde(default)]
    latest_version: Option<String>,
}

/// The small, public update payload embedded in `/api/state` and websocket
/// snapshots. A failed fetch preserves the last known release version.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub checked_at: Option<u64>,
    pub latest_version: Option<String>,
    pub update_available: bool,
}

fn status(cached: CachedCheck) -> Status {
    let latest_version = cached.latest_version;
    Status {
        checked_at: (cached.last_checked_at != 0).then_some(cached.last_checked_at),
        update_available: latest_version
            .as_deref()
            .is_some_and(|latest| is_newer(latest, env!("CARGO_PKG_VERSION"))),
        latest_version,
    }
}

fn load_cache() -> Result<CachedCheck> {
    let path = paths::update_check_file()?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CachedCheck::default()),
        Err(e) => Err(e).with_context(|| format!("reading update check cache {path:?}")),
    }
}

fn save_cache(cache: &CachedCheck) -> Result<()> {
    let path = paths::update_check_file()?;
    store::write_atomic(&path, serde_json::to_string_pretty(cache)?.as_bytes())
}

pub fn cached_status() -> Result<Status> {
    Ok(status(load_cache()?))
}

fn due(last_checked_at: u64, now: u64) -> bool {
    last_checked_at == 0 || now.saturating_sub(last_checked_at) >= DAY_MILLIS
}

/// Check GitHub if the persistent cache is at least a day old. Fetch errors
/// are deliberately non-fatal: we still store the attempt time to avoid
/// retrying repeatedly while offline, and retain any previous known version.
pub fn check_if_due() -> Result<Option<Status>> {
    let mut cache = load_cache()?;
    let now = store::now_millis();
    if !due(cache.last_checked_at, now) {
        return Ok(None);
    }

    cache.last_checked_at = now;
    match fetch_latest_version() {
        Ok(version) => cache.latest_version = Some(version),
        Err(e) => eprintln!("muckdb: update check failed (will retry tomorrow): {e:#}"),
    }
    save_cache(&cache)?;
    Ok(Some(status(cache)))
}

fn fetch_latest_version() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            RELEASE_URL,
        ])
        .output()
        .context("starting curl for GitHub release check")?;
    if !output.status.success() {
        anyhow::bail!(
            "GitHub release request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let body: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("decoding GitHub latest-release response")?;
    body.get("tag_name")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .context("GitHub latest-release response had no tag_name")
}

/// Compare normal release versions without another dependency. GitHub's
/// `releases/latest` endpoint only returns stable releases, but the parser also
/// safely ignores a leading `v` and any prerelease/build suffix.
fn is_newer(remote: &str, local: &str) -> bool {
    fn parts(version: &str) -> Option<Vec<u64>> {
        let core = version
            .trim()
            .trim_start_matches('v')
            .split(['-', '+'])
            .next()?;
        let values: Option<Vec<_>> = core.split('.').map(|part| part.parse().ok()).collect();
        values.filter(|values| !values.is_empty())
    }
    let (Some(remote), Some(local)) = (parts(remote), parts(local)) else {
        return false;
    };
    let len = remote.len().max(local.len());
    for i in 0..len {
        match remote
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&local.get(i).copied().unwrap_or(0))
        {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_check_is_due_on_first_boot_and_then_daily() {
        assert!(due(0, 10));
        assert!(!due(100, 100 + DAY_MILLIS - 1));
        assert!(due(100, 100 + DAY_MILLIS));
    }

    #[test]
    fn version_comparison_handles_v_prefix_and_patch_numbers() {
        assert!(is_newer("v0.4.20", "0.4.19"));
        assert!(is_newer("1.0.0", "0.99.9"));
        assert!(!is_newer("0.4.19", "0.4.19"));
        assert!(!is_newer("0.4.18", "0.4.19"));
        assert!(!is_newer("not-a-version", "0.4.19"));
    }
}
