//! Daemon lifecycle: detach into the background, and the `--status` / `--stop`
//! control commands.

use std::fs;

use anyhow::{Context, Result};
use daemonize::Daemonize;

use crate::facade;
use crate::{paths, server};

/// Detach from the terminal (fork + setsid), write the pidfile, redirect output
/// to the daemon log, then run the server. Forking happens *before* any tokio
/// runtime is created, which is the only safe ordering.
pub fn serve() -> Result<()> {
    let port = facade::resolved_port();
    let pid_file = paths::pid_file(port)?;
    let log_path = paths::daemon_log(port)?;
    let stdout = fs::File::create(&log_path).with_context(|| format!("creating {log_path:?}"))?;
    let stderr = stdout.try_clone()?;

    Daemonize::new()
        .pid_file(&pid_file)
        .stdout(stdout)
        .stderr(stderr)
        .start()
        .context("failed to daemonize (another daemon may already hold the pidfile)")?;

    // From here we are the detached daemon process.
    let rt = tokio::runtime::Runtime::new().context("building tokio runtime")?;
    rt.block_on(server::run())
}

/// Read the daemon pid from the port's pidfile, if present and parseable.
fn read_pid(port: u16) -> Option<i32> {
    let path = paths::pid_file(port).ok()?;
    let contents = fs::read_to_string(path).ok()?;
    contents.trim().parse::<i32>().ok()
}

/// True if a process with the given pid currently exists.
fn pid_alive(pid: i32) -> bool {
    // signal 0 performs error checking without delivering a signal.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Whether the daemon is accepting connections on `port`.
fn port_open(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// `muckdb --status`: report whether the daemon for the selected port is running.
pub fn status() -> Result<i32> {
    let port = facade::resolved_port();
    let listening = port_open(port);
    let pid = read_pid(port);
    match (listening, pid) {
        (true, Some(pid)) => {
            println!("muckdb daemon running (pid {pid}) at http://localhost:{port}");
            Ok(0)
        }
        (true, None) => {
            println!("muckdb daemon running at http://localhost:{port} (no pidfile)");
            Ok(0)
        }
        (false, Some(pid)) if pid_alive(pid) => {
            println!("muckdb daemon process {pid} is alive but not yet serving on {port}");
            Ok(0)
        }
        _ => {
            println!("muckdb daemon is not running (port {port})");
            Ok(1)
        }
    }
}

/// `muckdb --stop`: terminate the daemon for the selected port and clean up its
/// pidfile.
pub fn stop() -> Result<i32> {
    let port = facade::resolved_port();
    let Some(pid) = read_pid(port) else {
        println!("muckdb daemon is not running (no pidfile for port {port})");
        return Ok(1);
    };
    if !pid_alive(pid) {
        let _ = fs::remove_file(paths::pid_file(port)?);
        println!("muckdb daemon was not running; cleaned up stale pidfile");
        return Ok(1);
    }
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc != 0 {
        anyhow::bail!("failed to signal muckdb daemon (pid {pid})");
    }
    // SIGTERM is asynchronous: the process keeps holding the port until it
    // actually exits. Wait for it to die before returning, otherwise a
    // following `--display`/facade call sees the dying daemon as still
    // listening, no-ops, and ends up with no daemon at all.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !pid_alive(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if pid_alive(pid) {
        let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    let _ = fs::remove_file(paths::pid_file(port)?);
    println!("stopped muckdb daemon (pid {pid}, port {port})");
    Ok(0)
}
