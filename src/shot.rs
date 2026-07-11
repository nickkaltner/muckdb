//! Panel screenshots: render a session dashboard (or a single tile) in a
//! headless Chromium and return the PNG. Used by `muckdb session screenshot`
//! and the daemon's `/api/shot` endpoint (the copy-image button), so agents
//! and humans get pixel-identical captures of what the web UI shows.
//!
//! The web app has a dedicated `?shot=1[&tile=NAME]` mode that strips the
//! chrome, disables animations, and stamps the rendered content height on
//! `<html data-shot-h="...">`. Capture is two passes: a `--dump-dom` measure
//! pass reads that height, then the real `--screenshot` pass uses it as the
//! window height so the PNG fits the content exactly.
//!
//! Both passes are *output-driven*, not exit-driven: we take the result the
//! moment it's ready (the `data-shot-h` marker appears in the dumped DOM, or the
//! PNG file finishes being written) and then kill the whole browser process
//! group. Headless Chrome on macOS has been seen to produce its output and then
//! never exit, which used to hang every capture until the 45s ceiling; grabbing
//! the artifact directly and reaping the group sidesteps that entirely.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::facade::PORT;

pub const DEFAULT_WIDTH: u32 = 1200;
pub const MIN_WIDTH: u32 = 320;
pub const MAX_WIDTH: u32 = 4000;
const MIN_HEIGHT: u32 = 200;
/// Chromium caps surfaces around 16k; stay comfortably below.
const MAX_HEIGHT: u32 = 12000;
/// Used when the measure pass fails (e.g. the page errored before marking).
const FALLBACK_HEIGHT: u32 = 900;
/// Virtual time given to the page for fetches + chart rendering.
const TIME_BUDGET_MS: u32 = 10_000;
/// Wall-clock ceiling for the screenshot pass.
const RUN_TIMEOUT: Duration = Duration::from_secs(45);
/// The measure pass is best-effort (a fallback height exists), so bound it more
/// tightly than the screenshot pass.
const MEASURE_TIMEOUT: Duration = Duration::from_secs(20);
/// Poll cadence while waiting for a pass's output to appear.
const POLL: Duration = Duration::from_millis(100);

/// The shot-mode URL for a session (optionally narrowed to one tile).
pub fn shot_url(session: &str, tile: Option<&str>) -> String {
    let mut url = format!(
        "http://127.0.0.1:{PORT}/session/{}/?shot=1",
        urlencode(session)
    );
    if let Some(t) = tile {
        url.push_str("&tile=");
        url.push_str(&urlencode(t));
    }
    url
}

/// Percent-encode a URL path/query component (RFC 3986 unreserved kept as-is).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Capture a session (or one tile) as a PNG. `height: None` auto-fits to the
/// rendered content via the measure pass. The daemon must already be running.
pub fn capture_png(
    session: &str,
    tile: Option<&str>,
    width: u32,
    height: Option<u32>,
) -> Result<Vec<u8>> {
    let browser = find_browser()?;
    // A private profile dir per capture: headless refuses to share a profile
    // with a running browser, and parallel captures must not share either.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "muckdb-shot-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&tmp).with_context(|| format!("creating {tmp:?}"))?;
    let result = capture_in(&browser, &tmp, session, tile, width, height);
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn capture_in(
    browser: &Path,
    tmp: &Path,
    session: &str,
    tile: Option<&str>,
    width: u32,
    height: Option<u32>,
) -> Result<Vec<u8>> {
    let url = shot_url(session, tile);
    let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
    let height = match height {
        Some(h) => h.clamp(MIN_HEIGHT, MAX_HEIGHT),
        None => measure_height(browser, tmp, &url, width).unwrap_or(FALLBACK_HEIGHT),
    };

    let png = tmp.join("shot.png");
    let _ = std::fs::remove_file(&png);
    let mut cmd = browser_cmd(browser, tmp, width, height);
    cmd.arg(format!("--screenshot={}", png.display())).arg(&url);
    let mut run = spawn_group(cmd)?;

    // Wait for the PNG to appear and stop growing (the browser has finished
    // writing it) — or for the process to exit, or the ceiling. Then reap the
    // whole group, whether or not it was going to exit on its own.
    let deadline = Instant::now() + RUN_TIMEOUT;
    let (mut last_len, mut stable) = (0u64, 0);
    loop {
        if let Ok(meta) = std::fs::metadata(&png) {
            let len = meta.len();
            stable = if len > 0 && len == last_len {
                stable + 1
            } else {
                0
            };
            last_len = len;
            if stable >= 2 {
                break;
            }
        }
        match run.child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(POLL);
    }
    let err = String::from_utf8_lossy(&run.err.lock().unwrap())
        .trim()
        .to_string();
    kill_tree(&mut run.child);

    let bytes = std::fs::read(&png).unwrap_or_default();
    if bytes.is_empty() {
        bail!("browser produced no screenshot (timed out or failed): {err}");
    }
    Ok(bytes)
}

/// Measure pass: dump the rendered DOM and read the `data-shot-h` attribute the
/// shot-mode page stamps on `<html>` once every tile has loaded. Output-driven:
/// returns as soon as the marker shows up in stdout, then reaps the browser.
fn measure_height(browser: &Path, tmp: &Path, url: &str, width: u32) -> Option<u32> {
    let mut cmd = browser_cmd(browser, tmp, width, FALLBACK_HEIGHT);
    cmd.arg("--dump-dom").arg(url);
    let mut run = spawn_group(cmd).ok()?;
    let deadline = Instant::now() + MEASURE_TIMEOUT;
    let read_h = |run: &Running| {
        parse_shot_height(&String::from_utf8_lossy(&run.out.lock().unwrap()))
            .map(|h| h.clamp(MIN_HEIGHT, MAX_HEIGHT))
    };
    let found = loop {
        if let Some(h) = read_h(&run) {
            break Some(h);
        }
        match run.child.try_wait() {
            Ok(Some(_)) | Err(_) => break read_h(&run),
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break read_h(&run);
        }
        std::thread::sleep(POLL);
    };
    kill_tree(&mut run.child);
    found
}

/// A spawned browser plus threads draining its stdout/stderr into buffers we can
/// poll while it runs.
struct Running {
    child: std::process::Child,
    out: Arc<Mutex<Vec<u8>>>,
    err: Arc<Mutex<Vec<u8>>>,
}

/// Spawn `cmd` in its own process group (so the whole browser tree can be reaped
/// together) with stdout/stderr drained on threads into shared buffers.
fn spawn_group(mut cmd: Command) -> Result<Running> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // New group led by the child, so pgid == child pid.
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().context("spawning browser")?;
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink = buf.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&chunk[..n]);
            }
        });
        buf
    };
    let out = drain(Box::new(child.stdout.take().expect("piped stdout")));
    let err = drain(Box::new(child.stderr.take().expect("piped stderr")));
    Ok(Running { child, out, err })
}

/// Kill the whole process group (reaping any Chrome helper processes) so nothing
/// is left running; on non-unix just kill the child.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // pgid == child pid (see spawn_group); SIGKILL the group. ESRCH if it's
        // already gone, which is fine.
        unsafe {
            libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Extract the height from `data-shot-h="N"` in a serialized DOM.
fn parse_shot_height(dom: &str) -> Option<u32> {
    let key = "data-shot-h=\"";
    let rest = &dom[dom.find(key)? + key.len()..];
    rest[..rest.find('"')?].parse().ok()
}

/// The flags shared by both passes.
fn browser_cmd(browser: &Path, profile: &Path, width: u32, height: u32) -> Command {
    let mut cmd = Command::new(browser);
    cmd.arg("--headless")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        .arg("--no-first-run")
        .arg("--disable-extensions")
        .arg("--mute-audio")
        // On macOS, headless Chrome otherwise blocks on a Keychain-access
        // ("Chrome Safe Storage") confirmation it can never show, so the process
        // never becomes ready and every capture times out. A mock keychain / the
        // basic password store sidestep the system keyring entirely. Both are
        // harmless no-ops on Linux (no Keychain there).
        .arg("--use-mock-keychain")
        .arg("--password-store=basic")
        // Quiet the startup so virtual time can drain and the browser exits
        // promptly instead of idling on background chatter.
        .arg("--disable-sync")
        .arg("--disable-background-networking")
        .arg("--disable-default-apps")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--window-size={width},{height}"))
        .arg(format!("--virtual-time-budget={TIME_BUDGET_MS}"));
    cmd
}

/// Find a Chromium-based browser: $MUCKDB_BROWSER, then well-known names on
/// PATH, then macOS app bundles.
fn find_browser() -> Result<PathBuf> {
    if let Ok(b) = std::env::var("MUCKDB_BROWSER")
        && !b.is_empty()
    {
        return Ok(PathBuf::from(b));
    }
    const NAMES: &[&str] = &[
        "chromium",
        "chromium-browser",
        "google-chrome-stable",
        "google-chrome",
        "chrome",
        "brave",
        "brave-browser",
        "microsoft-edge",
    ];
    for name in NAMES {
        if let Some(p) = which(name) {
            return Ok(p);
        }
    }
    #[cfg(target_os = "macos")]
    {
        const APPS: &[&str] = &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ];
        for app in APPS {
            if Path::new(app).exists() {
                return Ok(PathBuf::from(app));
            }
        }
    }
    bail!(
        "no Chromium-based browser found for screenshots — install chromium \
         (or chrome/brave/edge), or set MUCKDB_BROWSER to a browser binary"
    )
}

/// Locate an executable on PATH.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_url_includes_tile_when_given() {
        assert_eq!(
            shot_url("pond-analysis", None),
            format!("http://127.0.0.1:{PORT}/session/pond-analysis/?shot=1")
        );
        assert_eq!(
            shot_url("pond-analysis", Some("by species")),
            format!("http://127.0.0.1:{PORT}/session/pond-analysis/?shot=1&tile=by%20species")
        );
    }

    #[test]
    fn urlencode_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(urlencode("abc-XYZ_0.9~"), "abc-XYZ_0.9~");
        assert_eq!(urlencode("a b/c&d"), "a%20b%2Fc%26d");
    }

    #[test]
    fn parse_shot_height_reads_the_html_attribute() {
        let dom = r#"<html lang="en" data-shot-ready="1" data-shot-h="1234"><head>"#;
        assert_eq!(parse_shot_height(dom), Some(1234));
        assert_eq!(parse_shot_height("<html><head>"), None);
        assert_eq!(parse_shot_height(r#"data-shot-h="nope""#), None);
    }
}
