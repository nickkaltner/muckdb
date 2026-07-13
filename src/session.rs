//! Sessions: named dashboards of tiles (panels) that an agent posts to from the
//! CLI. A tile is either markdown or a data view (a duckdb view or inline SQL)
//! rendered as a chart and explorable as a faceted search. Stored as one JSON
//! file per session under the data dir, shared with the daemon.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{paths, store};

/// A reference marker drawn on a chart: a horizontal target/threshold line, or a
/// vertical event line. `value` is a y-number (target/threshold) or an x-position
/// (event: a timestamp or category); `label` is the optional caption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// How a data tile should be charted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chart {
    /// bar | stacked | line | area | scatter | pie | table | heatmap
    pub kind: String,
    #[serde(default)]
    pub x: Option<String>,
    #[serde(default)]
    pub y: Vec<String>,
    /// Map tiles: the latitude / longitude columns (`--lat`/`--lon`). When unset,
    /// a map tile auto-detects columns named lat/latitude and lon/lng/longitude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lon: Option<String>,
    /// Map tiles: connection endpoints (`--from-lat`/`--from-lon`/`--to-lat`/
    /// `--to-lon`). When all four are set, each row is drawn as a semi-transparent
    /// arc between the two points (with the endpoints plotted as markers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_lat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_lon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_lat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_lon: Option<String>,
    /// Map connections: optional per-endpoint label columns (`--from-label`/
    /// `--to-label`) naming the source/destination point in each marker's hover
    /// tooltip. Distinct from `--label`, which labels the arc itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_label: Option<String>,
    /// Map tiles: an optional per-point label column (`--label`) surfaced in the
    /// hover tooltip so each marker names its points. For a connections map it
    /// labels each arc (drawn on top, nudged to avoid overlap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Heatmap cell value column (`--value`); with `--x` and the first `--y`
    /// giving the two axes, one row per x×y pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Heatmap: colour cells only, without printing the value in each
    /// (`--no-values`); hover still reveals the exact figure.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_values: bool,
    /// Box plots: the column holding each box's descriptive note (`--desc`),
    /// shown beside the plot so boxes can be compared with context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    /// Optional axis titles. When unset the chart shows no axis label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xlabel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ylabel: Option<String>,
    /// Bar fill style: "gradient" (continuous) or "solid" (per-bar palette colours
    /// for categorical data). Unset → gradient for a single series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars: Option<String>,
    /// Horizontal reference lines at a y-value (drawn in the accent colour).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<Marker>,
    /// Horizontal limit lines at a y-value (drawn dashed, in the warning colour).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thresholds: Vec<Marker>,
    /// Vertical event lines at an x-position (timestamp or category).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Marker>,
    /// Overlay a smoothed trendline (LOESS; single-series bar/line/area/scatter).
    #[serde(default, skip_serializing_if = "is_false")]
    pub trend: bool,
    /// Timeline tiles: the lane (row / resource) each bar belongs to (`--lane`).
    /// Bar text reuses `--label`. `--start`/`--end` are numeric (relative seconds)
    /// or timestamps — auto-detected in the browser. `--duration` (numeric
    /// seconds) is an alternative to `--end` (end = start + duration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    /// Timeline: colour bars by this category column (`--color`); adds a legend.
    /// When unset each lane gets its own palette colour instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Timeline dependencies: `--id` gives each bar a unique id; `--depends-on`
    /// holds comma-separated parent id(s), drawn as right-angle connectors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
    /// Sequence diagram tiles: one row per message. `--from`/`--to` name the
    /// source and destination participant columns (`from == to` → self-message);
    /// the message text reuses `--label`. Participants and their order are
    /// inferred from the rows (first appearance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_participant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_participant: Option<String>,
    /// Sequence: per-message arrow kind (`--message-type`): sync (default) |
    /// reply | async | lost. Sequence: per-participant shape (`--from-type`/
    /// `--to-type`): participant (default) | actor | database | boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_type: Option<String>,
    /// Sequence group frames (`--group`): a `kind:label` value (loop|opt|alt|par);
    /// contiguous rows sharing the value are wrapped in one frame. `--group-branch`
    /// gives the else/and compartment label within a frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_branch: Option<String>,
    /// Sequence: number the messages 1,2,3… (`--autonumber`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub autonumber: bool,
}

/// One panel in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Tile {
    Markdown {
        name: String,
        #[serde(default)]
        title: Option<String>,
        markdown: String,
        /// Hidden in the dashboard's trash (persisted with the session, so it
        /// follows the dashboard across browsers — restore from the contents).
        #[serde(default, skip_serializing_if = "is_false")]
        trashed: bool,
    },
    /// A heading-only tile that groups the panels after it: renders as a section
    /// divider in the dashboard and as a section header in the contents.
    Section {
        name: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        trashed: bool,
    },
    View {
        name: String,
        #[serde(default)]
        title: Option<String>,
        db: String,
        #[serde(default)]
        view: Option<String>,
        #[serde(default)]
        sql: Option<String>,
        chart: Box<Chart>,
        #[serde(default)]
        caption: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        trashed: bool,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Tile {
    pub fn name(&self) -> &str {
        match self {
            Tile::Markdown { name, .. } | Tile::Section { name, .. } | Tile::View { name, .. } => {
                name
            }
        }
    }

    pub fn trashed(&self) -> bool {
        match self {
            Tile::Markdown { trashed, .. }
            | Tile::Section { trashed, .. }
            | Tile::View { trashed, .. } => *trashed,
        }
    }

    fn set_trashed(&mut self, on: bool) {
        match self {
            Tile::Markdown { trashed, .. }
            | Tile::Section { trashed, .. }
            | Tile::View { trashed, .. } => *trashed = on,
        }
    }
}

/// A session dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Markdown handoff for agents: data-source provenance and session-wide
    /// decisions that should survive across conversations and exports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_context: Option<String>,
    /// The agent conversation/thread UUID this dashboard was built for, if linked.
    ///
    /// Older session files used `claude_session`; keep reading that field so
    /// existing dashboards migrate on their next save.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "claude_session"
    )]
    pub agent_session: Option<String>,
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub tiles: Vec<Tile>,
}

// ---- viewer activity -------------------------------------------------------
//
// What the human has actually looked at: session opens, panel zooms, explore
// clicks. Kept in its own file (activity.json) rather than the session JSON so
// activity writes don't churn the watched sessions dir (which would re-render
// every open dashboard) and can't race the CLI over session files. Agents read
// it back through `muckdb ls sessions` / `ls session <id>`, which merge it in —
// e.g. a tile with zero zooms/explores that the human has had many chances to
// open is a hint to present that data differently.

/// Per-tile interaction counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TileActivity {
    #[serde(default)]
    pub zooms: u64,
    #[serde(default)]
    pub explores: u64,
    /// Last interaction (ms since epoch).
    #[serde(default)]
    pub last: u64,
}

/// Per-session view counts plus per-tile interactions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionActivity {
    /// Times a human opened this dashboard (screenshot renders don't count).
    #[serde(default)]
    pub views: u64,
    /// Last human open (ms since epoch).
    #[serde(default)]
    pub last_viewed: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tiles: BTreeMap<String, TileActivity>,
}

fn activity_path() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("activity.json"))
}

/// The whole activity registry, keyed by session id (empty when none yet).
pub fn load_activity() -> BTreeMap<String, SessionActivity> {
    let Ok(path) = activity_path() else {
        return BTreeMap::new();
    };
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Record one interaction: `tile: None` counts a session open; otherwise
/// `action` is "zoom" or "explore" on that tile.
pub fn record_activity(session: &str, tile: Option<&str>, action: &str) -> Result<()> {
    // The daemon services activity POSTs concurrently; serialize the whole
    // read-modify-write (all writers are in this one process) so no update is
    // lost, and pair it with an atomic file write so readers never tear.
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load_activity();
    let s = all.entry(session.to_string()).or_default();
    let now = store::now_millis();
    match tile {
        None => {
            s.views += 1;
            s.last_viewed = now;
        }
        Some(t) => {
            let ta = s.tiles.entry(t.to_string()).or_default();
            match action {
                "zoom" => ta.zooms += 1,
                "explore" => ta.explores += 1,
                _ => {}
            }
            ta.last = now;
        }
    }
    let path = activity_path()?;
    store::write_atomic(&path, serde_json::to_string_pretty(&all)?.as_bytes())
}

/// Slugify a name into a filesystem/URL-safe id.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "session".to_string()
    } else {
        s
    }
}

/// The directory holding session JSON files (created if missing).
pub fn sessions_dir() -> Result<PathBuf> {
    let dir = paths::data_dir()?.join("sessions");
    fs::create_dir_all(&dir).with_context(|| format!("creating {dir:?}"))?;
    Ok(dir)
}

fn path_for(id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("{id}.json")))
}

/// Load one session by id, if it exists.
pub fn load(id: &str) -> Result<Option<Session>> {
    let path = path_for(id)?;
    match fs::read_to_string(&path) {
        Ok(s) => Ok(Some(
            serde_json::from_str(&s).context("parsing session json")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {path:?}")),
    }
}

/// All sessions, newest-updated first.
pub fn list() -> Result<Vec<Session>> {
    let dir = sessions_dir()?;
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && let Ok(text) = fs::read_to_string(&path)
            && let Ok(s) = serde_json::from_str::<Session>(&text)
        {
            out.push(s);
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.updated)); // newest first
    Ok(out)
}

pub fn save(session: &Session) -> Result<()> {
    let path = path_for(&session.id)?;
    let json = serde_json::to_string_pretty(session)?;
    // Atomic (temp + rename) so the daemon/export never read a half-written file.
    store::write_atomic(&path, json.as_bytes())
}

/// A held advisory lock on a session, released when dropped.
pub struct SessionLock {
    _file: fs::File,
}

/// Take an exclusive advisory lock over a session's read-modify-write so
/// concurrent CLI writers can't lose each other's updates (last-writer-wins on
/// the whole-file rewrite). Blocks until free; released when the guard drops.
/// The lock lives in a sibling `<id>.json.lock` (ignored by `list`, which only
/// reads `.json`). Acquire it at exactly one level per operation — flock isn't
/// reentrant across fds in the same process. No-op guard off unix.
pub fn lock_session(id: &str) -> Result<SessionLock> {
    let path = sessions_dir()?.join(format!("{id}.json.lock"));
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("opening lock {path:?}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            bail!("locking {path:?}: {}", std::io::Error::last_os_error());
        }
    }
    Ok(SessionLock { _file: file })
}

/// Delete a session's JSON file (the CLI `rm` and the web UI's delete button).
/// Returns whether it existed.
pub fn remove(id: &str) -> Result<bool> {
    let path = path_for(id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {path:?}")),
    }
}

/// Load a session or create a fresh one with this id.
fn load_or_new(id: &str, title: Option<String>) -> Result<Session> {
    if let Some(mut s) = load(id)? {
        if title.is_some() {
            s.title = title;
        }
        return Ok(s);
    }
    let now = store::now_millis();
    Ok(Session {
        id: id.to_string(),
        title,
        agent_context: None,
        agent_session: None,
        created: now,
        updated: now,
        tiles: Vec::new(),
    })
}

/// Insert or replace a tile (matched by name), preserving order.
fn upsert_tile(session: &mut Session, mut tile: Tile) {
    if let Some(slot) = session.tiles.iter_mut().find(|t| t.name() == tile.name()) {
        // A re-post keeps the trashed flag: agents update tiles in loops, and
        // an update must not resurrect a panel the human threw away.
        tile.set_trashed(slot.trashed());
        *slot = tile;
    } else {
        session.tiles.push(tile);
    }
    session.updated = store::now_millis();
}

/// Set or clear a tile's trashed flag, persisting the session. Returns whether
/// the session and tile both existed.
pub fn set_tile_trashed(id: &str, tile: &str, on: bool) -> Result<bool> {
    let _lock = lock_session(id)?;
    let Some(mut s) = load(id)? else {
        return Ok(false);
    };
    let Some(t) = s.tiles.iter_mut().find(|t| t.name() == tile) else {
        return Ok(false);
    };
    t.set_trashed(on);
    s.updated = store::now_millis();
    save(&s)?;
    Ok(true)
}

/// Where to move a tile within its session's ordering.
pub enum Move<'a> {
    Up,
    Down,
    /// A 1-based target position.
    To(usize),
    Before(&'a str),
    After(&'a str),
}

/// Reorder a tile within its session. Returns false if the session, the tile,
/// or a named anchor (`--before`/`--after`) doesn't exist.
pub fn move_tile(id: &str, tile: &str, mv: Move) -> Result<bool> {
    let _lock = lock_session(id)?;
    let Some(mut s) = load(id)? else {
        return Ok(false);
    };
    let Some(from) = s.tiles.iter().position(|t| t.name() == tile) else {
        return Ok(false);
    };
    let item = s.tiles.remove(from);
    let len = s.tiles.len();
    // Target index within the list *after* the tile has been removed.
    let insert = match mv {
        Move::Up => from.saturating_sub(1),
        Move::Down => (from + 1).min(len),
        Move::To(pos) => pos.saturating_sub(1).min(len),
        Move::Before(anchor) => match s.tiles.iter().position(|t| t.name() == anchor) {
            Some(a) => a,
            None => return Ok(false),
        },
        Move::After(anchor) => match s.tiles.iter().position(|t| t.name() == anchor) {
            Some(a) => a + 1,
            None => return Ok(false),
        },
    };
    s.tiles.insert(insert.min(len), item);
    s.updated = store::now_millis();
    save(&s)?;
    Ok(true)
}

// ---- CLI -----------------------------------------------------------------

/// A tiny `--key value` + positional argument parser for the session CLI.
struct Args {
    flags: Vec<(String, String)>,
    positionals: Vec<String>,
}

/// Flags that take no value — the parser must not eat the next argument.
const BOOL_FLAGS: &[&str] = &["no-validate", "up", "down", "trend", "autonumber"];

impl Args {
    fn parse(args: &[String]) -> Self {
        let mut flags = Vec::new();
        let mut positionals = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if let Some(key) = a.strip_prefix("--") {
                if BOOL_FLAGS.contains(&key) {
                    flags.push((key.to_string(), String::new()));
                    i += 1;
                    continue;
                }
                let val = args.get(i + 1).cloned().unwrap_or_default();
                flags.push((key.to_string(), val));
                i += 2;
            } else {
                positionals.push(a.clone());
                i += 1;
            }
        }
        Self { flags, positionals }
    }
    fn get(&self, key: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    /// All values given for a repeatable flag, in order.
    fn get_all(&self, key: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// Parse a marker flag `VALUE` or `VALUE|LABEL` (split on the first `|`, so
/// timestamps with colons stay intact). Empty values are dropped.
fn parse_markers(raw: &[&str]) -> Vec<Marker> {
    raw.iter()
        .filter_map(|s| {
            let (value, label) = match s.split_once('|') {
                Some((v, l)) => (v.trim(), Some(l.trim().to_string())),
                None => (s.trim(), None),
            };
            if value.is_empty() {
                None
            } else {
                Some(Marker {
                    value: value.to_string(),
                    label: label.filter(|l| !l.is_empty()),
                })
            }
        })
        .collect()
}

/// Levenshtein distance, for "did you mean" suggestions on typo'd names.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The closest of `options` to `target` (case-insensitive, distance ≤ 2).
fn closest<'a>(target: &str, options: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    options
        .map(|o| (edit_distance(&target.to_lowercase(), &o.to_lowercase()), o))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, o)| o)
}

/// Validate a data tile against its database before saving, so a typo'd view
/// or column fails loudly at post time instead of rendering an empty panel.
/// A db that exists but can't be queried right now (e.g. lock-held) only
/// warns — a busy database shouldn't block a dashboard update.
/// True for paths that get cleaned out from under a dashboard (system temp
/// dirs, agent session scratchpads) — a tile that references one will show
/// "database does not exist" as soon as the dir is reaped.
fn is_volatile_path(db: &str) -> bool {
    let temp = std::env::temp_dir().display().to_string();
    db.starts_with(&temp) || db.starts_with("/tmp/") || db.starts_with("/var/tmp/")
}

/// Resolve a database reference before it is persisted in a session tile.
///
/// Tiles are rendered by the daemon, whose working directory is not necessarily
/// the CLI's working directory. Persisting a relative path would therefore pass
/// CLI-time validation but fail when the dashboard is viewed. Canonicalise an
/// existing path (which also resolves symlinks); retain an absolute lexical path
/// for a not-yet-created database so `--no-validate` keeps its documented use.
fn resolve_db_path_from(base: &Path, db: &str) -> PathBuf {
    let path = Path::new(db);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    fs::canonicalize(&absolute).unwrap_or(absolute)
}

fn resolve_db_path(db: &str) -> Result<String> {
    let cwd = std::env::current_dir().context("resolving the current directory for --db")?;
    Ok(resolve_db_path_from(&cwd, db)
        .to_string_lossy()
        .into_owned())
}

fn validate_tile(db: &str, view: Option<&str>, sql: Option<&str>, chart: &Chart) -> Result<()> {
    if !std::path::Path::new(db).exists() {
        bail!("database not found: {db}");
    }
    if is_volatile_path(db) {
        eprintln!(
            "warning: --db {db} lives in a temp directory — the dashboard will break when it's \
             cleaned. Keep session databases somewhere durable (e.g. the project dir or \
             ~/.local/share/muckdb/data/)."
        );
    }
    // Resolve the columns the tile will plot.
    let described = match view {
        Some(v) => {
            crate::introspect::query_json(db, &format!("DESCRIBE \"{}\"", v.replace('"', "\"\"")))
        }
        None => {
            let q = sql.unwrap_or("").trim().trim_end_matches(';');
            crate::introspect::query_json(db, &format!("DESCRIBE {q}"))
        }
    };
    let rows = match described {
        Ok(rows) => rows,
        Err(e) => {
            // Distinguish "the relation/SQL is wrong" from "the db is unreadable".
            match crate::introspect::list_tables(db) {
                Err(_) => {
                    eprintln!("warning: could not validate tile ({e:#}); posting anyway");
                    return Ok(());
                }
                Ok(rels) => {
                    if let Some(v) = view {
                        let names: Vec<String> = rels.iter().map(|r| r.name.clone()).collect();
                        let hint = closest(v, names.iter().map(String::as_str))
                            .map(|c| format!(" — did you mean '{c}'?"))
                            .unwrap_or_default();
                        bail!(
                            "view '{v}' not found in {db}{hint}\navailable: {}\n(use --no-validate to skip this check)",
                            names.join(", ")
                        );
                    }
                    bail!("--sql failed to parse: {e:#}\n(use --no-validate to skip this check)");
                }
            }
        }
    };
    let cols: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            r.get("column_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let check = |what: &str, c: &str| -> Result<()> {
        if !cols.iter().any(|x| x == c) {
            let hint = closest(c, cols.iter().map(String::as_str))
                .map(|m| format!(" — did you mean '{m}'?"))
                .unwrap_or_default();
            bail!(
                "{what} '{c}' is not a column of the tile's data{hint}\ncolumns: {}\n(use --no-validate to skip this check)",
                cols.join(", ")
            );
        }
        Ok(())
    };
    if let Some(x) = &chart.x {
        check("--x", x)?;
    }
    for y in &chart.y {
        check("--y", y)?;
    }
    if let Some(v) = &chart.value {
        check("--value", v)?;
    }
    if let Some(d) = &chart.desc {
        check("--desc", d)?;
    }
    if let Some(l) = &chart.lat {
        check("--lat", l)?;
    }
    if let Some(l) = &chart.lon {
        check("--lon", l)?;
    }
    if let Some(l) = &chart.label {
        check("--label", l)?;
    }
    for (flag, col) in [
        ("--from-lat", &chart.from_lat),
        ("--from-lon", &chart.from_lon),
        ("--to-lat", &chart.to_lat),
        ("--to-lon", &chart.to_lon),
        ("--from-label", &chart.from_label),
        ("--to-label", &chart.to_label),
    ] {
        if let Some(c) = col {
            check(flag, c)?;
        }
    }
    if chart.kind == "map" {
        // A map needs latitude and longitude: explicit --lat/--lon, else a
        // column named lat/latitude and lon/lng/long/longitude (case-insensitive).
        let auto = |explicit: &Option<String>, cands: &[&str]| -> bool {
            explicit.is_some()
                || cols
                    .iter()
                    .any(|c| cands.iter().any(|n| c.eq_ignore_ascii_case(n)))
        };
        let has_points = auto(&chart.lat, &["lat", "latitude"])
            && auto(&chart.lon, &["lon", "lng", "long", "longitude"]);
        // A connections map instead takes four endpoint columns.
        let has_conns = auto(&chart.from_lat, &["from_lat", "src_lat", "origin_lat"])
            && auto(&chart.from_lon, &["from_lon", "src_lon", "origin_lon"])
            && auto(&chart.to_lat, &["to_lat", "dst_lat", "dest_lat"])
            && auto(&chart.to_lon, &["to_lon", "dst_lon", "dest_lon"]);
        if !has_points && !has_conns {
            bail!(
                "--chart map needs point columns (--lat COL --lon COL, or lat/latitude & lon/lng/longitude) or connection columns (--from-lat --from-lon --to-lat --to-lon)\ncolumns: {}",
                cols.join(", ")
            );
        }
    }
    if chart.kind == "heatmap" && (chart.x.is_none() || chart.y.is_empty()) {
        bail!("--chart heatmap needs --x and --y (the two axes; --value for the cell value)");
    }
    if chart.kind == "box" && (chart.x.is_none() || chart.y.len() != 5) {
        bail!(
            "--chart box needs --x (the box label) and --y with exactly five columns, in order: min,q1,median,q3,max (aggregate in the view, e.g. min(v), quantile_cont(v,0.25), median(v), quantile_cont(v,0.75), max(v))"
        );
    }
    if chart.kind == "timeline" {
        // Core columns must be named and must exist.
        let lane = chart
            .lane
            .as_deref()
            .context("--chart timeline needs --lane <column> (the row / resource label)")?;
        check("--lane", lane)?;
        chart
            .label
            .as_deref()
            .context("--chart timeline needs --label <column> (the bar text)")?;
        // (--label column existence is validated generically above.)
        let start = chart
            .start
            .as_deref()
            .context("--chart timeline needs --start <column>")?;
        check("--start", start)?;
        match (&chart.end, &chart.duration) {
            (Some(_), Some(_)) => {
                bail!("--chart timeline takes either --end or --duration, not both")
            }
            (None, None) => bail!(
                "--chart timeline needs --end <column> or --duration <column> (numeric seconds)"
            ),
            (Some(e), None) => check("--end", e)?,
            (None, Some(d)) => check("--duration", d)?,
        }
        for (flag, col) in [
            ("--color", &chart.color),
            ("--id", &chart.id),
            ("--depends-on", &chart.depends_on),
        ] {
            if let Some(c) = col {
                check(flag, c)?;
            }
        }
        // Start and end must agree on type: both numeric (relative seconds) or
        // both temporal (timestamps/dates). A mismatch is almost always a bug.
        let col_type = |name: &str| -> Option<String> {
            rows.iter().find_map(|r| {
                let cn = r.get("column_name").and_then(Value::as_str)?;
                if cn == name {
                    r.get("column_type")
                        .and_then(Value::as_str)
                        .map(str::to_ascii_uppercase)
                } else {
                    None
                }
            })
        };
        let is_temporal =
            |t: &str| t.contains("TIMESTAMP") || t.contains("DATE") || t.contains("TIME");
        let is_numeric = |t: &str| {
            [
                "INT", "DEC", "DOUBLE", "FLOAT", "REAL", "HUGEINT", "NUMERIC", "BIGINT",
            ]
            .iter()
            .any(|k| t.contains(k))
        };
        if let Some(end) = chart.end.as_deref()
            && let (Some(st), Some(et)) = (col_type(start), col_type(end))
        {
            let st_temporal = is_temporal(&st) && !is_numeric(&st);
            let et_temporal = is_temporal(&et) && !is_numeric(&et);
            if st_temporal != et_temporal {
                bail!(
                    "--chart timeline: --start ({start}: {st}) and --end ({end}: {et}) must both be \
                     numeric (relative seconds) or both be timestamps"
                );
            }
        }
        // --duration is numeric seconds — a temporal column would silently render
        // every bar empty (Number(timestamp) is NaN), so reject it up front.
        if let Some(dur) = chart.duration.as_deref()
            && let Some(dt) = col_type(dur)
            && is_temporal(&dt)
            && !is_numeric(&dt)
        {
            bail!(
                "--chart timeline: --duration ({dur}: {dt}) must be numeric seconds, not a timestamp"
            );
        }
    }
    if chart.kind == "sequence" {
        // One row per message: --from, --to and --label (the message text) are
        // required; every optional column flag must exist if named.
        let from = chart
            .from_participant
            .as_deref()
            .context("--chart sequence needs --from <column> (the source participant)")?;
        check("--from", from)?;
        let to = chart
            .to_participant
            .as_deref()
            .context("--chart sequence needs --to <column> (the destination participant)")?;
        check("--to", to)?;
        chart
            .label
            .as_deref()
            .context("--chart sequence needs --label <column> (the message text)")?;
        // (--label column existence is validated generically above.)
        for (flag, col) in [
            ("--message-type", &chart.message_type),
            ("--from-type", &chart.from_type),
            ("--to-type", &chart.to_type),
            ("--group", &chart.group),
            ("--group-branch", &chart.group_branch),
        ] {
            if let Some(c) = col {
                check(flag, c)?;
            }
        }
    }
    Ok(())
}

/// Return the neutral session-link flag, falling back to the legacy alias.
/// An explicit `--agent-session` wins when both are supplied.
fn agent_session_arg(args: &Args) -> Option<&str> {
    args.get("agent-session")
        .filter(|id| !id.is_empty())
        .or_else(|| args.get("claude").filter(|id| !id.is_empty()))
}

fn read_md(value: &str) -> Result<String> {
    if value == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        // Agents routinely pass `--md "# Title\n\nBody"` inside double quotes,
        // where the shell leaves `\n` literal — honour those escapes so the
        // panel doesn't render a one-line "\n"-riddled string. A literal
        // backslash-n survives via `\\n`; multi-line content is better piped
        // through `--md -` (stdin, taken verbatim above).
        let mut out = String::with_capacity(value.len());
        let mut chars = value.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        Ok(out)
    }
}

/// Entry point for `muckdb session <action> ...`.
pub fn cli(args: &[String]) -> Result<i32> {
    let action = args.first().map(String::as_str).unwrap_or("");
    let p = Args::parse(&args[1..]);
    // The session is the first positional (or --session).
    let session_arg = p
        .positionals
        .first()
        .cloned()
        .or_else(|| p.get("session").map(str::to_string));

    match action {
        "list" => {
            for s in list()? {
                let title = s.title.as_deref().unwrap_or("");
                println!("{:<24} {:>3} tiles  {}", s.id, s.tiles.len(), title);
            }
            Ok(0)
        }
        "create" => {
            let name = session_arg.context("usage: muckdb session create <name>")?;
            let id = slug(&name);
            let _lock = lock_session(&id)?;
            let mut s = load_or_new(&id, p.get("title").map(str::to_string).or(Some(name)))?;
            // Link this dashboard to its creating agent's conversation/thread.
            // `--claude` remains a compatibility alias for existing scripts.
            if let Some(uuid) = agent_session_arg(&p) {
                s.agent_session = Some(uuid.to_string());
            }
            save(&s)?;
            crate::facade::ensure_daemon()?;
            println!(
                "session {id} ready at http://localhost:{}/session/{id}",
                crate::facade::resolved_port()
            );
            if let Some(uuid) = &s.agent_session {
                println!("linked to agent session {uuid}");
            }
            Ok(0)
        }
        "post" => {
            let name = session_arg.context("usage: muckdb session post <name> --md <text|->")?;
            let id = slug(&name);
            let _lock = lock_session(&id)?;
            let md = read_md(
                p.get("md")
                    .or(p.get("markdown"))
                    .context("--md <text|-> required")?,
            )?;
            let tile = Tile::Markdown {
                name: p.get("name").unwrap_or("note").to_string(),
                title: p.get("title").map(str::to_string),
                markdown: md,
                trashed: false,
            };
            let mut s = load_or_new(&id, None)?;
            upsert_tile(&mut s, tile);
            save(&s)?;
            crate::facade::ensure_daemon()?;
            println!(
                "posted tile '{}' to session {id}",
                p.get("name").unwrap_or("note")
            );
            Ok(0)
        }
        "section" => {
            let name = session_arg
                .context("usage: muckdb session section <name> --name TILE --title HEADING")?;
            let id = slug(&name);
            let _lock = lock_session(&id)?;
            let tile_name = p.get("name").context("--name <tile> required")?.to_string();
            let tile = Tile::Section {
                name: tile_name.clone(),
                title: p.get("title").map(str::to_string),
                trashed: false,
            };
            let mut s = load_or_new(&id, None)?;
            upsert_tile(&mut s, tile);
            save(&s)?;
            crate::facade::ensure_daemon()?;
            println!("set section '{tile_name}' in session {id}");
            Ok(0)
        }
        // Session-level Markdown handoff: unlike a markdown tile, this is not
        // rendered on the dashboard. It gives later agents provenance for every
        // source feeding the session and durable notes about its assumptions.
        "context" | "agent-metadata" => {
            let name = session_arg
                .context("usage: muckdb session context <name> <read|save> [--md <text|->]")?;
            let id = slug(&name);
            let op = p
                .positionals
                .get(1)
                .map(String::as_str)
                .context("say context read or context save")?;
            match op {
                "read" => {
                    let s = load(&id)?.with_context(|| format!("no such session '{id}'"))?;
                    print!("{}", s.agent_context.as_deref().unwrap_or(""));
                    Ok(0)
                }
                "save" => {
                    let md = read_md(
                        p.get("md")
                            .or(p.get("markdown"))
                            .context("context save needs --md <text|->")?,
                    )?;
                    let _lock = lock_session(&id)?;
                    let mut s = load_or_new(&id, None)?;
                    s.agent_context = Some(md);
                    s.updated = store::now_millis();
                    save(&s)?;
                    crate::facade::ensure_daemon()?;
                    println!("saved agent context for session {id}");
                    Ok(0)
                }
                _ => bail!("context action must be read or save"),
            }
        }
        "move" => {
            let name = session_arg.context(
                "usage: muckdb session move <name> --tile T (--up | --down | --to N | --before X | --after X)",
            )?;
            let id = slug(&name);
            let tile = p.get("tile").context("--tile <name> required")?;
            let mv = if p.get("up").is_some() {
                Move::Up
            } else if p.get("down").is_some() {
                Move::Down
            } else if let Some(to) = p.get("to") {
                Move::To(to.parse().context("--to needs a positive integer")?)
            } else if let Some(b) = p.get("before") {
                Move::Before(b)
            } else if let Some(a) = p.get("after") {
                Move::After(a)
            } else {
                bail!("say where to move it: --up, --down, --to N, --before TILE, or --after TILE");
            };
            if move_tile(&id, tile, mv)? {
                crate::facade::ensure_daemon()?;
                println!("moved tile '{tile}' in session {id}");
                Ok(0)
            } else {
                bail!("no such session '{id}', tile '{tile}', or anchor tile");
            }
        }
        "tile" => {
            let name = session_arg
                .context("usage: muckdb session tile <name> --name T --db D (--view V|--sql S)")?;
            let id = slug(&name);
            let _lock = lock_session(&id)?;
            let tile_name = p.get("name").context("--name <tile> required")?.to_string();
            let db = resolve_db_path(p.get("db").context("--db <path> required")?)?;
            let view = p.get("view").map(str::to_string);
            let sql = p.get("sql").map(str::to_string);
            if view.is_none() && sql.is_none() {
                bail!("a data tile needs --view <name> or --sql <query>");
            }
            let y = p
                .get("y")
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let tile = Tile::View {
                name: tile_name.clone(),
                title: p.get("title").map(str::to_string),
                db,
                view,
                sql,
                chart: Box::new(Chart {
                    kind: p.get("chart").unwrap_or("table").to_string(),
                    x: p.get("x").map(str::to_string),
                    lat: p.get("lat").map(str::to_string),
                    lon: p.get("lon").map(str::to_string),
                    from_lat: p.get("from-lat").map(str::to_string),
                    from_lon: p.get("from-lon").map(str::to_string),
                    to_lat: p.get("to-lat").map(str::to_string),
                    to_lon: p.get("to-lon").map(str::to_string),
                    from_label: p.get("from-label").map(str::to_string),
                    to_label: p.get("to-label").map(str::to_string),
                    label: p.get("label").map(str::to_string),
                    value: p.get("value").map(str::to_string),
                    no_values: p.get("no-values").is_some(),
                    desc: p.get("desc").map(str::to_string),
                    xlabel: p.get("xlabel").map(str::to_string),
                    ylabel: p.get("ylabel").map(str::to_string),
                    bars: p.get("bars").map(str::to_string),
                    y,
                    targets: parse_markers(&p.get_all("target")),
                    thresholds: parse_markers(&p.get_all("threshold")),
                    events: parse_markers(&p.get_all("event")),
                    trend: p.get("trend").is_some(),
                    lane: p.get("lane").map(str::to_string),
                    start: p.get("start").map(str::to_string),
                    end: p.get("end").map(str::to_string),
                    duration: p.get("duration").map(str::to_string),
                    color: p.get("color").map(str::to_string),
                    id: p.get("id").map(str::to_string),
                    depends_on: p.get("depends-on").map(str::to_string),
                    from_participant: p.get("from").map(str::to_string),
                    to_participant: p.get("to").map(str::to_string),
                    message_type: p.get("message-type").map(str::to_string),
                    from_type: p.get("from-type").map(str::to_string),
                    to_type: p.get("to-type").map(str::to_string),
                    group: p.get("group").map(str::to_string),
                    group_branch: p.get("group-branch").map(str::to_string),
                    autonumber: p.get("autonumber").is_some(),
                }),
                caption: p.get("caption").map(str::to_string),
                trashed: false,
            };
            if p.get("no-validate").is_none()
                && let Tile::View {
                    db,
                    view,
                    sql,
                    chart,
                    ..
                } = &tile
            {
                validate_tile(db, view.as_deref(), sql.as_deref(), chart)?;
            }
            let mut s = load_or_new(&id, None)?;
            upsert_tile(&mut s, tile);
            save(&s)?;
            crate::facade::ensure_daemon()?;
            println!("set tile '{tile_name}' in session {id}");
            Ok(0)
        }
        // Capture the dashboard (or one panel) as a PNG so agents can *see*
        // what they built. Renders via a local headless Chromium.
        "screenshot" | "shot" => {
            let name = session_arg
                .context("usage: muckdb session screenshot <name> [--tile T] [--out F.png]")?;
            let id = slug(&name);
            let s = load(&id)?.with_context(|| format!("no such session '{id}'"))?;
            let tile = p.get("tile").map(str::to_string);
            if let Some(t) = &tile
                && !s.tiles.iter().any(|x| x.name() == t.as_str())
            {
                let names: Vec<&str> = s.tiles.iter().map(|x| x.name()).collect();
                bail!(
                    "no tile '{t}' in session {id} (tiles: {})",
                    names.join(", ")
                );
            }
            let width = p
                .get("width")
                .and_then(|w| w.parse().ok())
                .unwrap_or(crate::shot::DEFAULT_WIDTH);
            let height = p.get("height").and_then(|h| h.parse().ok());
            let out = p.get("out").map(PathBuf::from).unwrap_or_else(|| {
                PathBuf::from(match &tile {
                    Some(t) => format!("muckdb-{id}-{}.png", slug(t)),
                    None => format!("muckdb-{id}.png"),
                })
            });
            crate::facade::ensure_daemon()?;
            let png = crate::shot::capture_png(&id, tile.as_deref(), width, height)?;
            fs::write(&out, &png).with_context(|| format!("writing {out:?}"))?;
            let abs = out.canonicalize().unwrap_or(out);
            match &tile {
                Some(t) => println!(
                    "screenshot of tile '{t}' in session {id}: {} ({} kB)",
                    abs.display(),
                    png.len() / 1024
                ),
                None => println!(
                    "screenshot of session {id}: {} ({} kB)",
                    abs.display(),
                    png.len() / 1024
                ),
            }
            Ok(0)
        }
        // Bundle a session + full snapshots of its databases into a portable
        // `<id>.muckdb` zip (import on any machine with `session import`).
        "export" => {
            let name =
                session_arg.context("usage: muckdb session export <name> [--out FILE.muckdb]")?;
            let id = slug(&name);
            let out = p.get("out").map(PathBuf::from);
            let path = crate::export::export_session(&id, out)?;
            let abs = path.canonicalize().unwrap_or(path);
            let kb = fs::metadata(&abs).map(|m| m.len() / 1024).unwrap_or(0);
            println!("exported session {id}: {} ({kb} kB)", abs.display());
            Ok(0)
        }
        "import" => {
            let file = session_arg.context("usage: muckdb session import <file.muckdb>")?;
            let bytes = fs::read(&file).with_context(|| format!("reading {file}"))?;
            let imported = crate::export::import_and_install(&bytes)?;
            crate::facade::ensure_daemon()?;
            println!(
                "imported session {} ({} tiles, {} db{}) — http://localhost:{}/session/{}/",
                imported.session.id,
                imported.session.tiles.len(),
                imported.dbs.len(),
                if imported.dbs.len() == 1 { "" } else { "s" },
                crate::facade::resolved_port(),
                imported.session.id
            );
            Ok(0)
        }
        "rm" => {
            let name = session_arg.context("usage: muckdb session rm <name> [--tile T]")?;
            let id = slug(&name);
            let _lock = lock_session(&id)?;
            if let Some(tile) = p.get("tile") {
                if let Some(mut s) = load(&id)? {
                    s.tiles.retain(|t| t.name() != tile);
                    s.updated = store::now_millis();
                    save(&s)?;
                    println!("removed tile '{tile}' from session {id}");
                }
            } else {
                let _ = remove(&id)?;
                println!("removed session {id}");
            }
            Ok(0)
        }
        _ => {
            eprintln!(
                "usage: muckdb session <create|list|post|section|context|tile|move|screenshot|export|import|rm> ...\n\
                 \n  create <name> [--title T] [--agent-session UUID]\n  list\n  \
                 post <name> --md <text|-> [--name TILE] [--title T]\n  \
                 section <name> --name TILE --title HEADING   (a heading that groups the panels after it)\n  \
                 context <name> <read|save> [--md <text|->]  (agent handoff: data sources + session-wide notes)\n  \
                 move <name> --tile T (--up | --down | --to N | --before TILE | --after TILE)\n  \
                 tile <name> --name TILE --db DB (--view V | --sql SQL) [--chart bar|stacked|line|area|scatter|pie|table|heatmap|box|map|timeline|sequence] [--x COL] [--y C1,C2] [--title T] [--caption C]\n                       \
                 [--value COL]  (heatmap: the cell value; --x and --y name the two axes, one row per pair)\n                       \
                 [--no-values]  (heatmap: colour cells only — hover still shows the figure)\n                       \
                 --chart map: --lat COL --lon COL (else auto-detected lat/latitude & lon/lng/longitude); markers shade by point count, or --value COL by magnitude; --label COL names points in the hover tooltip; connections: --from-lat/--from-lon/--to-lat/--to-lon per arc, --from-label/--to-label name each endpoint marker\n                       \
                 --chart box: --x the box label, --y min,q1,median,q3,max (five columns, aggregated in the view)\n                       \
                 --chart timeline: --lane COL --label COL --start COL (--end COL | --duration COL); optional --color CAT --id COL --depends-on COL; --event 'T|label' markers\n                       \
                 --chart sequence: --from COL --to COL --label COL (one row per message); optional --message-type sync|reply|async|lost, --from-type/--to-type participant|actor|database|boundary, --group 'kind:label', --group-branch COL, --autonumber\n                       \
                 [--desc COL]  (box: a per-box note column, shown beside each plot)\n                       \
                 [--xlabel L] [--ylabel L]  (axis titles)\n                       \
                 [--bars gradient|solid]  (bar fill: solid = per-bar palette colours for categorical data)\n                       \
                 [--target 'VAL|label'] [--threshold 'VAL|label'] [--event 'X|label']  (repeatable reference lines)\n                       \
                 [--trend]  (overlay a smoothed trendline; single-series bar/line/area/scatter)\n  \
                 screenshot <name> [--tile TILE] [--out F.png] [--width W] [--height H]  (capture as PNG via headless Chromium)\n  \
                 export <name> [--out FILE.muckdb]  (bundle session + database snapshots into a portable zip)\n  \
                 import <file.muckdb>               (load an exported session; dbs land in muckdb's data dir)\n  \
                 rm <name> [--tile TILE]"
            );
            Ok(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn volatile_paths_are_flagged() {
        assert!(is_volatile_path(
            "/tmp/claude-1000/x/scratchpad/demo.duckdb"
        ));
        assert!(is_volatile_path(&format!(
            "{}/x.duckdb",
            std::env::temp_dir().display()
        )));
        assert!(!is_volatile_path("/home/anko/muckdb-demo/demo.duckdb"));
        assert!(!is_volatile_path("relative.duckdb"));
    }

    #[test]
    fn slug_lowercases_and_dashes_runs_of_non_alnum() {
        assert_eq!(slug("Pond Analysis"), "pond-analysis");
        assert_eq!(slug("  Q2 / 2026 report!! "), "q2-2026-report");
        assert_eq!(slug("already-good"), "already-good");
    }

    #[test]
    fn heatmap_chart_serde_roundtrips_value_column() {
        let c = Chart {
            kind: "heatmap".into(),
            x: Some("port_speed".into()),
            y: vec!["country".into()],
            lat: None,
            lon: None,
            from_lat: None,
            from_lon: None,
            to_lat: None,
            to_lon: None,
            from_label: None,
            to_label: None,
            label: None,
            value: Some("sites".into()),
            no_values: false,
            desc: None,
            xlabel: None,
            ylabel: None,
            bars: None,
            targets: vec![],
            thresholds: vec![],
            events: vec![],
            trend: false,
            lane: None,
            start: None,
            end: None,
            duration: None,
            color: None,
            id: None,
            depends_on: None,
            from_participant: None,
            to_participant: None,
            message_type: None,
            from_type: None,
            to_type: None,
            group: None,
            group_branch: None,
            autonumber: false,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"value\":\"sites\""));
        let back: Chart = serde_json::from_str(&json).unwrap();
        assert_eq!(back.value.as_deref(), Some("sites"));
        // Older sessions without the field still load.
        let old: Chart = serde_json::from_str("{\"kind\":\"bar\",\"y\":[]}").unwrap();
        assert!(old.value.is_none());
    }

    #[test]
    fn map_chart_serde_roundtrips_lat_lon() {
        let c = Chart {
            kind: "map".into(),
            x: None,
            y: vec![],
            lat: Some("latitude".into()),
            lon: Some("longitude".into()),
            from_lat: None,
            from_lon: None,
            to_lat: None,
            to_lon: None,
            from_label: None,
            to_label: None,
            label: Some("city".into()),
            value: None,
            no_values: false,
            desc: None,
            xlabel: None,
            ylabel: None,
            bars: None,
            targets: vec![],
            thresholds: vec![],
            events: vec![],
            trend: false,
            lane: None,
            start: None,
            end: None,
            duration: None,
            color: None,
            id: None,
            depends_on: None,
            from_participant: None,
            to_participant: None,
            message_type: None,
            from_type: None,
            to_type: None,
            group: None,
            group_branch: None,
            autonumber: false,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Chart = serde_json::from_str(&json).unwrap();
        assert_eq!(back.lat.as_deref(), Some("latitude"));
        assert_eq!(back.lon.as_deref(), Some("longitude"));
        assert_eq!(back.label.as_deref(), Some("city"));
        // Non-map charts omit the fields entirely (skip_serializing_if).
        let bar = serde_json::to_string(&Chart {
            lat: None,
            lon: None,
            ..c
        })
        .unwrap();
        assert!(!bar.contains("\"lat\""));
    }

    #[test]
    fn timeline_chart_serde_roundtrips_fields() {
        let c = Chart {
            kind: "timeline".into(),
            x: None,
            y: vec![],
            lat: None,
            lon: None,
            from_lat: None,
            from_lon: None,
            to_lat: None,
            to_lon: None,
            from_label: None,
            to_label: None,
            label: Some("task".into()),
            value: None,
            no_values: false,
            desc: None,
            xlabel: None,
            ylabel: None,
            bars: None,
            targets: vec![],
            thresholds: vec![],
            events: vec![],
            trend: false,
            lane: Some("resource".into()),
            start: Some("started_at".into()),
            end: Some("ended_at".into()),
            duration: None,
            color: Some("status".into()),
            id: Some("span_id".into()),
            depends_on: Some("parent_ids".into()),
            from_participant: None,
            to_participant: None,
            message_type: None,
            from_type: None,
            to_type: None,
            group: None,
            group_branch: None,
            autonumber: false,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Chart = serde_json::from_str(&json).unwrap();
        assert_eq!(back.lane.as_deref(), Some("resource"));
        assert_eq!(back.start.as_deref(), Some("started_at"));
        assert_eq!(back.end.as_deref(), Some("ended_at"));
        assert_eq!(back.color.as_deref(), Some("status"));
        assert_eq!(back.depends_on.as_deref(), Some("parent_ids"));
        // Other kinds omit the timeline fields entirely (skip_serializing_if).
        let bar = serde_json::to_string(&Chart {
            kind: "bar".into(),
            lane: None,
            start: None,
            end: None,
            color: None,
            id: None,
            depends_on: None,
            ..c
        })
        .unwrap();
        assert!(!bar.contains("\"lane\""));
        assert!(!bar.contains("\"depends_on\""));
    }

    #[test]
    fn sequence_chart_serde_roundtrips_fields() {
        let c = Chart {
            kind: "sequence".into(),
            x: None,
            y: vec![],
            lat: None,
            lon: None,
            from_lat: None,
            from_lon: None,
            to_lat: None,
            to_lon: None,
            from_label: None,
            to_label: None,
            label: Some("msg".into()),
            value: None,
            no_values: false,
            desc: None,
            xlabel: None,
            ylabel: None,
            bars: None,
            targets: vec![],
            thresholds: vec![],
            events: vec![],
            trend: false,
            lane: None,
            start: None,
            end: None,
            duration: None,
            color: None,
            id: None,
            depends_on: None,
            from_participant: Some("src".into()),
            to_participant: Some("dst".into()),
            message_type: Some("kind".into()),
            from_type: Some("src_type".into()),
            to_type: Some("dst_type".into()),
            group: Some("grp".into()),
            group_branch: Some("branch".into()),
            autonumber: true,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Chart = serde_json::from_str(&json).unwrap();
        assert_eq!(back.from_participant.as_deref(), Some("src"));
        assert_eq!(back.to_participant.as_deref(), Some("dst"));
        assert_eq!(back.message_type.as_deref(), Some("kind"));
        assert_eq!(back.group_branch.as_deref(), Some("branch"));
        assert!(back.autonumber);
        // Other kinds omit the sequence fields entirely (skip_serializing_if).
        let bar = serde_json::to_string(&Chart {
            kind: "bar".into(),
            from_participant: None,
            to_participant: None,
            message_type: None,
            from_type: None,
            to_type: None,
            group: None,
            group_branch: None,
            autonumber: false,
            ..c
        })
        .unwrap();
        assert!(!bar.contains("from_participant"));
        assert!(!bar.contains("autonumber"));
    }

    #[test]
    fn parse_markers_splits_value_and_label_on_first_pipe() {
        let m = parse_markers(&["30|max"]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].value, "30");
        assert_eq!(m[0].label.as_deref(), Some("max"));
    }

    #[test]
    fn parse_markers_keeps_colons_in_timestamps() {
        // split_once('|') must not break on the timestamp's colons.
        let m = parse_markers(&["2026-05-15T00:00|deploy"]);
        assert_eq!(m[0].value, "2026-05-15T00:00");
        assert_eq!(m[0].label.as_deref(), Some("deploy"));
    }

    #[test]
    fn parse_markers_handles_no_label_and_drops_empty() {
        let m = parse_markers(&["20", "  ", "|orphan-label"]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].value, "20");
        assert_eq!(m[0].label, None);
    }

    #[test]
    fn args_parse_separates_flags_and_positionals() {
        let raw = [s("create"), s("demo"), s("--title"), s("My demo")];
        let a = Args::parse(&raw);
        assert_eq!(a.positionals, vec!["create", "demo"]);
        assert_eq!(a.get("title"), Some("My demo"));
        assert_eq!(a.get("missing"), None);
    }

    #[test]
    fn args_get_all_collects_repeated_flags_in_order() {
        let raw = [
            s("--event"),
            s("a|one"),
            s("--event"),
            s("b|two"),
            s("--x"),
            s("col"),
        ];
        let a = Args::parse(&raw);
        assert_eq!(a.get_all("event"), vec!["a|one", "b|two"]);
        assert_eq!(a.get_all("x"), vec!["col"]);
        assert!(a.get_all("nope").is_empty());
    }

    #[test]
    fn args_trailing_flag_without_value_is_empty_string() {
        let raw = [s("--flag")];
        let a = Args::parse(&raw);
        assert_eq!(a.get("flag"), Some(""));
    }

    #[test]
    fn closest_suggests_near_misses_only() {
        let opts = ["v_species", "v_daily", "readings"];
        assert_eq!(
            closest("v_speciess", opts.iter().copied()),
            Some("v_species")
        );
        assert_eq!(closest("V_DAILY", opts.iter().copied()), Some("v_daily")); // case-insensitive
        assert_eq!(closest("totally_wrong", opts.iter().copied()), None); // distance > 2
    }

    #[test]
    fn bool_flags_do_not_eat_the_next_argument() {
        let raw = [s("--no-validate"), s("--db"), s("x.duckdb")];
        let a = Args::parse(&raw);
        assert_eq!(a.get("no-validate"), Some(""));
        assert_eq!(a.get("db"), Some("x.duckdb"));
    }

    #[test]
    fn resolves_relative_tile_database_paths_to_absolute_paths() {
        let base = std::env::temp_dir().join(format!("muckdb-session-path-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let db = base.join("tile.duckdb");
        fs::write(&db, []).unwrap();

        assert_eq!(
            resolve_db_path_from(&base, "tile.duckdb"),
            db.canonicalize().unwrap()
        );
        // Keep `--no-validate` useful for a database that will be created later,
        // while still making its stored path independent of the daemon's cwd.
        assert_eq!(
            resolve_db_path_from(&base, "later.duckdb"),
            base.join("later.duckdb")
        );

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_md_unescapes_shell_literal_newlines() {
        assert_eq!(read_md("# T\\n\\nBody").unwrap(), "# T\n\nBody");
        assert_eq!(read_md("a\\tb").unwrap(), "a\tb");
        assert_eq!(read_md("keep \\\\n literal").unwrap(), "keep \\n literal");
        assert_eq!(read_md("real\nnewline").unwrap(), "real\nnewline"); // untouched
        assert_eq!(read_md("trailing\\").unwrap(), "trailing\\");
        assert_eq!(read_md("\\x unknown").unwrap(), "\\x unknown");
    }

    #[test]
    fn agent_context_is_backward_compatible_and_serializes_when_set() {
        let mut session: Session =
            serde_json::from_str(r#"{"id":"handoff","created":1,"updated":1,"tiles":[]}"#).unwrap();
        assert_eq!(session.agent_context, None);

        session.agent_context = Some("# Data sources\n\n- warehouse.duckdb".into());
        let value = serde_json::to_value(session).unwrap();
        assert_eq!(
            value["agent_context"],
            "# Data sources\n\n- warehouse.duckdb"
        );
    }

    #[test]
    fn agent_session_reads_legacy_field_and_serializes_neutral_field() {
        let session: Session = serde_json::from_str(
            r#"{"id":"handoff","claude_session":"legacy-id","created":1,"updated":1,"tiles":[]}"#,
        )
        .unwrap();
        assert_eq!(session.agent_session.as_deref(), Some("legacy-id"));

        let value = serde_json::to_value(session).unwrap();
        assert_eq!(value["agent_session"], "legacy-id");
        assert!(value.get("claude_session").is_none());
    }

    #[test]
    fn agent_session_flag_prefers_neutral_name_and_keeps_legacy_alias() {
        let legacy = Args::parse(&[s("--claude"), s("legacy-id")]);
        assert_eq!(agent_session_arg(&legacy), Some("legacy-id"));

        let both = Args::parse(&[
            s("--claude"),
            s("legacy-id"),
            s("--agent-session"),
            s("agent-id"),
        ]);
        assert_eq!(agent_session_arg(&both), Some("agent-id"));
    }

    // ---- timeline validation (needs the `duckdb` CLI) --------------------
    fn duckdb_ok() -> bool {
        std::process::Command::new("duckdb")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    fn run_sql(db: &str, sql: &str) {
        let out = std::process::Command::new("duckdb")
            .arg(db)
            .arg("-c")
            .arg(sql)
            .output()
            .expect("run duckdb");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn timeline_validation_requires_core_flags() {
        if !duckdb_ok() {
            eprintln!("skipping timeline_validation_requires_core_flags: no duckdb");
            return;
        }
        let dir = std::env::temp_dir().join(format!("muckdb-tl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("tl.duckdb");
        let dbs = db.to_str().unwrap();
        // Build a spans table with the columns a timeline uses.
        run_sql(
            dbs,
            "CREATE TABLE spans AS SELECT 'web' AS lane, 'deploy' AS task, \
             0.0 AS t0, 10.0 AS t1, 'ok' AS status, 'a' AS sid, NULL AS pids, \
             TIMESTAMP '2026-01-01' AS ts",
        );
        let mk = |chart: Chart| validate_tile(dbs, Some("spans"), None, &chart);
        let base = || Chart {
            kind: "timeline".into(),
            x: None,
            y: vec![],
            lat: None,
            lon: None,
            from_lat: None,
            from_lon: None,
            to_lat: None,
            to_lon: None,
            from_label: None,
            to_label: None,
            label: Some("task".into()),
            value: None,
            no_values: false,
            desc: None,
            xlabel: None,
            ylabel: None,
            bars: None,
            targets: vec![],
            thresholds: vec![],
            events: vec![],
            trend: false,
            lane: Some("lane".into()),
            start: Some("t0".into()),
            end: Some("t1".into()),
            duration: None,
            color: None,
            id: None,
            depends_on: None,
            from_participant: None,
            to_participant: None,
            message_type: None,
            from_type: None,
            to_type: None,
            group: None,
            group_branch: None,
            autonumber: false,
        };
        // Missing --lane fails.
        let mut c = base();
        c.lane = None;
        assert!(mk(c).is_err());
        // Both --end and --duration fails.
        let mut c = base();
        c.duration = Some("t1".into());
        assert!(mk(c).is_err());
        // Neither --end nor --duration fails.
        let mut c = base();
        c.end = None;
        assert!(mk(c).is_err());
        // A temporal --duration fails (must be numeric seconds).
        let mut c = base();
        c.end = None;
        c.duration = Some("ts".into());
        assert!(mk(c).is_err());
        // A numeric --duration is accepted.
        let mut c = base();
        c.end = None;
        c.duration = Some("t1".into());
        assert!(mk(c).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sequence_validation_requires_core_flags() {
        if !duckdb_ok() {
            eprintln!("skipping sequence_validation_requires_core_flags: no duckdb");
            return;
        }
        let dir = std::env::temp_dir().join(format!("muckdb-seq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("seq.duckdb");
        let dbs = db.to_str().unwrap();
        run_sql(
            dbs,
            "CREATE TABLE calls AS SELECT 'gateway' AS src, 'auth' AS dst, \
             'POST /login' AS msg, 'sync' AS mtype, 'actor' AS st, 'database' AS dt, \
             'loop:retry' AS grp, 'ok' AS branch",
        );
        let mk = |chart: Chart| validate_tile(dbs, Some("calls"), None, &chart);
        let base = || Chart {
            kind: "sequence".into(),
            x: None,
            y: vec![],
            lat: None,
            lon: None,
            from_lat: None,
            from_lon: None,
            to_lat: None,
            to_lon: None,
            from_label: None,
            to_label: None,
            label: Some("msg".into()),
            value: None,
            no_values: false,
            desc: None,
            xlabel: None,
            ylabel: None,
            bars: None,
            targets: vec![],
            thresholds: vec![],
            events: vec![],
            trend: false,
            lane: None,
            start: None,
            end: None,
            duration: None,
            color: None,
            id: None,
            depends_on: None,
            from_participant: Some("src".into()),
            to_participant: Some("dst".into()),
            message_type: None,
            from_type: None,
            to_type: None,
            group: None,
            group_branch: None,
            autonumber: false,
        };
        // A fully-specified valid spec passes.
        assert!(mk(base()).is_ok());
        // Missing --from / --to / --label each fail.
        let mut c = base();
        c.from_participant = None;
        assert!(mk(c).is_err());
        let mut c = base();
        c.to_participant = None;
        assert!(mk(c).is_err());
        let mut c = base();
        c.label = None;
        assert!(mk(c).is_err());
        // A bad column name fails (the generic check + "did you mean").
        let mut c = base();
        c.from_participant = Some("nope".into());
        assert!(mk(c).is_err());
        // All optional columns, valid, pass.
        let mut c = base();
        c.message_type = Some("mtype".into());
        c.from_type = Some("st".into());
        c.to_type = Some("dt".into());
        c.group = Some("grp".into());
        c.group_branch = Some("branch".into());
        assert!(mk(c).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }
}
