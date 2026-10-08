//! Database work must not depend on writable muckdb home-directory state.
use std::path::PathBuf;
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("muckdb-sandbox-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_muckdb"));
    // An ordinary file cannot contain a data or state directory, even when
    // tests run as root. Each command must still reach DuckDB.
    let dir = scratch("state");
    let unavailable = dir.join("unavailable");
    std::fs::write(&unavailable, []).unwrap();
    command
        .env("XDG_DATA_HOME", &unavailable)
        .env("XDG_STATE_HOME", &unavailable)
        .env("MUCKDB_SESSION", "sandbox-test")
        .env("CODEX_THREAD_ID", "sandbox-test-thread");
    command
}

#[test]
fn database_commands_succeed_without_ui_or_history() {
    if Command::new("duckdb").arg("-version").output().is_err() {
        return;
    }
    let dir = scratch("database");
    let db = dir.join("metrics.duckdb");
    let result = command()
        .arg(&db)
        .args([
            "-c",
            "CREATE TABLE metrics(n INTEGER); INSERT INTO metrics VALUES (42);",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(dir.join("metrics.duckdb.muckdb.lock").exists());
    let result = command()
        .arg("-readonly")
        .arg(&db)
        .args(["-c", "SELECT n FROM metrics;"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("42"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn duckdb_errors_still_fail_without_ui_or_history() {
    if Command::new("duckdb").arg("-version").output().is_err() {
        return;
    }
    let result = command()
        .args([":memory:", "-c", "SELECT * FROM table_that_does_not_exist;"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("table_that_does_not_exist"));
}
