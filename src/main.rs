//! muckdb — a facade over the duckdb CLI that runs a background server with a
//! live web view of your muckdb history and databases.

mod daemon;
mod facade;
mod introspect;
mod paths;
mod server;
mod session;
mod store;

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("muckdb: {e:#}");
            1
        }
    };
    exit(code);
}

fn run(args: &[String]) -> anyhow::Result<i32> {
    match args.first().map(String::as_str) {
        // Hidden flag used by ensure_daemon to launch the detached server.
        Some("--__serve") => {
            daemon::serve()?;
            Ok(0)
        }
        Some("--status") => daemon::status(),
        Some("--stop") => daemon::stop(),
        // Session dashboards: `muckdb session <create|list|post|tile|rm> ...`
        Some("session") => session::cli(&args[1..]),
        // Agent-facing introspection as JSON: `muckdb ls <what>`.
        Some("ls") => ls(&args[1..]),
        Some("--display") => {
            facade::ensure_daemon()?;
            let url = format!("http://localhost:{}", facade::PORT);
            println!("muckdb daemon serving at {url}");
            open_browser(&url);
            Ok(0)
        }
        // Everything else is passed straight through to duckdb.
        _ => facade::passthrough(args),
    }
}

/// `muckdb ls <what>` — print state as JSON for an agent to read. Read-only;
/// never starts the daemon.
fn ls(args: &[String]) -> anyhow::Result<i32> {
    use anyhow::Context;
    let what = args.first().map(String::as_str).unwrap_or("");
    match what {
        "databases" | "dbs" => {
            let st = store::load_state()?;
            let list: Vec<_> = st
                .databases
                .iter()
                .map(|p| {
                    serde_json::json!({ "id": store::db_id(p), "path": p, "exists": std::path::Path::new(p).exists() })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        "tables" => {
            let db = args.get(1).context("usage: muckdb ls tables <db>")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&introspect::list_tables(db)?)?
            );
        }
        "sessions" => println!("{}", serde_json::to_string_pretty(&session::list()?)?),
        "session" => {
            let id = args.get(1).context("usage: muckdb ls session <id>")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&session::load(&session::slug(id))?)?
            );
        }
        "history" => {
            let st = store::load_state()?;
            let limit = args
                .iter()
                .position(|a| a == "--limit")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse::<usize>().ok());
            let h = &st.history;
            let slice = match limit {
                Some(n) => &h[h.len().saturating_sub(n)..],
                None => &h[..],
            };
            println!("{}", serde_json::to_string_pretty(slice)?);
        }
        _ => {
            eprintln!(
                "usage: muckdb ls <what>\n  databases            all databases muckdb has seen\n  \
                 tables <db>          tables and views in a database\n  sessions             session dashboards\n  \
                 session <id>         one session with its tiles\n  history [--limit N]  the command ledger"
            );
            return Ok(2);
        }
    }
    Ok(0)
}

/// Best-effort: open the web view in the default browser. Failures are ignored.
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
