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
