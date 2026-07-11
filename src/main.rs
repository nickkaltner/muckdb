//! muckdb — a facade over the duckdb CLI that runs a background server with a
//! live web view of your muckdb history and databases.

mod daemon;
mod export;
mod facade;
mod formats;
mod introspect;
mod paths;
mod predict;
mod server;
mod session;
mod shot;
mod skill;
mod store;

use std::process::exit;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Resolve `--port <N>` / `--port=<N>` up front and record it in MUCKDB_PORT
    // so every consumer (facade, server, daemon control, and any spawned child)
    // agrees on the port; then drop the flag so it never reaches duckdb.
    if let Some(port) = extract_port_flag(&mut args) {
        // SAFETY: single-threaded here — set before any daemon/thread starts.
        unsafe { std::env::set_var("MUCKDB_PORT", port.to_string()) };
    }
    let code = match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("muckdb: {e:#}");
            1
        }
    };
    exit(code);
}

/// Pull a `--port <N>` or `--port=<N>` flag out of `args` (removing it) and
/// return the parsed port. Returns `None` if the flag is absent; an unparseable
/// or zero value is ignored (the default/env resolution then applies).
fn extract_port_flag(args: &mut Vec<String>) -> Option<u16> {
    let mut port = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--port" {
            let val = args.get(i + 1).and_then(|s| s.trim().parse::<u16>().ok());
            let consume = if args.get(i + 1).is_some() { 2 } else { 1 };
            if let Some(p) = val.filter(|&p| p != 0) {
                port = Some(p);
            }
            args.drain(i..i + consume.min(args.len() - i));
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--port=") {
            if let Some(p) = rest.trim().parse::<u16>().ok().filter(|&p| p != 0) {
                port = Some(p);
            }
            args.remove(i);
            continue;
        }
        i += 1;
    }
    port
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
        // Start the background daemon without opening a browser.
        Some("start" | "--start") => {
            facade::ensure_daemon()?;
            println!(
                "muckdb daemon serving at http://localhost:{}",
                facade::resolved_port()
            );
            Ok(0)
        }
        // muckdb's own help (duckdb's help, rebranded, with muckdb commands on top).
        Some("--help" | "-help" | "-h" | "help") => help(),
        // muckdb's version, then the underlying duckdb's version line.
        Some("--version" | "-version") => version(),
        // Session dashboards: `muckdb session <create|list|post|tile|rm> ...`
        Some("session") => session::cli(&args[1..]),
        // Install the bundled Claude skill: `muckdb skill install`.
        Some("skill") => skill::cli(&args[1..]),
        // Column display formats: `muckdb format <db> <col> --currency USD`.
        Some("format") => formats::cli(&args[1..]),
        // Agent-facing introspection as JSON: `muckdb ls <what>`.
        Some("ls") => ls(&args[1..]),
        Some("--display") => {
            facade::ensure_daemon()?;
            let url = format!("http://localhost:{}", facade::resolved_port());
            println!("muckdb daemon serving at {url}");
            open_browser(&url);
            Ok(0)
        }
        // Everything else is passed straight through to duckdb.
        _ => facade::passthrough(args),
    }
}

/// `muckdb --help` — muckdb's own help. Leads with what muckdb adds over duckdb
/// and its extra commands, then appends `duckdb`'s help (rebranded) so every
/// passthrough option is still documented.
fn help() -> anyhow::Result<i32> {
    print!(
        "\
muckdb — a duckdb CLI facade with a live web view.

Runs exactly like `duckdb` (same arguments, stdout, and exit codes) and also
records every invocation and serves a live web UI at http://localhost:11000
(override the port with --port <N> or MUCKDB_PORT; a non-default port uses its
own pidfile/log so multiple daemons can run at once).

muckdb commands:
  start                  start the background daemon (without opening a browser)
  --display              open the web UI (starts the background daemon if needed)
  --status               report whether the daemon is running
  --stop                 stop the background daemon
  --port <N>             use TCP port N for the daemon (default 11000; per-port pidfile)
  --version              print muckdb's version, then duckdb's
  session <subcommand>   build dashboards: create | list | post | tile | screenshot | export | import | rm
  ls <what>              print state as JSON: databases | tables | sessions | session | history
  format <db> <col>      attach a display format to a column ($, %, units, decimals)
  skill <install|uninstall|path>   manage the muckdb Claude Code skill

Claude Code skill:
  muckdb ships a skill that teaches coding agents to use muckdb by default for
  any data work — charting, SQL analysis, and presenting verifiable dashboards.
  Install it into your user skills directory so agents pick it up automatically:

    muckdb skill install            write it to ~/.claude/skills/muckdb/SKILL.md
    muckdb skill install --force    overwrite an existing copy
    muckdb skill path               print where it would be installed
    muckdb skill uninstall          remove it

  Then restart Claude Code (or start a new session) to load it.

Anything else is passed straight through to duckdb:

"
    );
    // Append duckdb's own help, rebranded, so passthrough options are documented.
    match std::process::Command::new("duckdb").arg("-help").output() {
        Ok(out) => {
            // Rebrand every casing duckdb uses in its help (e.g. "DuckDB database",
            // "show DuckDB version", "Usage: duckdb …") so nothing reads as duckdb.
            let text = String::from_utf8_lossy(&out.stdout)
                .replace("DuckDB", "MuckDB")
                .replace("DUCKDB", "MUCKDB")
                .replace("duckdb", "muckdb");
            print!("{text}");
        }
        Err(_) => {
            println!("(duckdb not found on PATH — install it to see its options here.)");
        }
    }
    Ok(0)
}

/// `muckdb --version` — muckdb's own version first, then the duckdb CLI's
/// version line so the passthrough engine's build is visible too.
fn version() -> anyhow::Result<i32> {
    println!("muckdb v{}", env!("CARGO_PKG_VERSION"));
    match std::process::Command::new("duckdb")
        .arg("-version")
        .output()
    {
        Ok(out) => print!("duckdb {}", String::from_utf8_lossy(&out.stdout)),
        Err(_) => println!("duckdb not found on PATH"),
    }
    Ok(0)
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
        // Sessions carry an "activity" block (views, per-tile zooms/explores)
        // recorded from the web UI — what the human has actually looked at.
        "sessions" => {
            let acts = session::load_activity();
            let list: Vec<serde_json::Value> = session::list()?
                .into_iter()
                .map(|s| {
                    let mut v = serde_json::to_value(&s).unwrap_or_default();
                    if let Some(a) = acts.get(&s.id) {
                        v["activity"] =
                            serde_json::json!({ "views": a.views, "last_viewed": a.last_viewed });
                    }
                    v
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        "session" => {
            let id = args.get(1).context("usage: muckdb ls session <id>")?;
            let id = session::slug(id);
            let mut v = serde_json::to_value(session::load(&id)?)?;
            if !v.is_null()
                && let Some(a) = session::load_activity().get(&id)
            {
                v["activity"] = serde_json::to_value(a)?;
            }
            println!("{}", serde_json::to_string_pretty(&v)?);
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

#[cfg(test)]
mod tests {
    use super::extract_port_flag;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn absent_port_flag_leaves_args_untouched() {
        let mut args = v(&["db.duckdb", "-c", "SELECT 1"]);
        assert_eq!(extract_port_flag(&mut args), None);
        assert_eq!(args, v(&["db.duckdb", "-c", "SELECT 1"]));
    }

    #[test]
    fn space_separated_port_is_parsed_and_removed() {
        let mut args = v(&["--port", "11055", "start"]);
        assert_eq!(extract_port_flag(&mut args), Some(11055));
        assert_eq!(args, v(&["start"]));
    }

    #[test]
    fn equals_form_is_parsed_and_removed() {
        let mut args = v(&["--port=12000", "--status"]);
        assert_eq!(extract_port_flag(&mut args), Some(12000));
        assert_eq!(args, v(&["--status"]));
    }

    #[test]
    fn zero_and_unparseable_ports_are_ignored_but_still_stripped() {
        let mut args = v(&["--port", "0", "x"]);
        assert_eq!(extract_port_flag(&mut args), None);
        assert_eq!(args, v(&["x"]));

        let mut args = v(&["--port", "nope", "x"]);
        assert_eq!(extract_port_flag(&mut args), None);
        assert_eq!(args, v(&["x"]));
    }

    #[test]
    fn trailing_port_flag_without_value_is_dropped() {
        let mut args = v(&["--port"]);
        assert_eq!(extract_port_flag(&mut args), None);
        assert!(args.is_empty());
    }
}
