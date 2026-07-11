//! Sessions: named dashboards of tiles (panels) that an agent posts to from the
//! CLI. A tile is either markdown or a data view (a duckdb view or inline SQL)
//! rendered as a chart and explorable as a faceted search. Stored as one JSON
//! file per session under the data dir, shared with the daemon.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

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
    /// Map tiles: an optional per-point label column (`--label`) surfaced in the
    /// hover tooltip so each marker names its points.
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
    /// The Claude Code session UUID this dashboard was built for, if linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session: Option<String>,
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
    fs::write(&path, serde_json::to_string_pretty(&all)?)
        .with_context(|| format!("writing {path:?}"))?;
    Ok(())
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
    fs::write(&path, json).with_context(|| format!("writing {path:?}"))?;
    Ok(())
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
        claude_session: None,
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
const BOOL_FLAGS: &[&str] = &["no-validate", "up", "down"];

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
    if chart.kind == "map" {
        // A map needs latitude and longitude: explicit --lat/--lon, else a
        // column named lat/latitude and lon/lng/long/longitude (case-insensitive).
        let auto = |explicit: &Option<String>, cands: &[&str]| -> bool {
            explicit.is_some()
                || cols
                    .iter()
                    .any(|c| cands.iter().any(|n| c.eq_ignore_ascii_case(n)))
        };
        if !auto(&chart.lat, &["lat", "latitude"])
            || !auto(&chart.lon, &["lon", "lng", "long", "longitude"])
        {
            bail!(
                "--chart map needs latitude and longitude columns: pass --lat COL --lon COL, or name them lat/latitude and lon/lng/longitude\ncolumns: {}",
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
    Ok(())
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
            let mut s = load_or_new(&id, p.get("title").map(str::to_string).or(Some(name)))?;
            // Link this dashboard to a Claude Code session by UUID, when given.
            // Agents pass their own id via $CLAUDE_CODE_SESSION_ID.
            if let Some(uuid) = p.get("claude").filter(|u| !u.is_empty()) {
                s.claude_session = Some(uuid.to_string());
            }
            save(&s)?;
            crate::facade::ensure_daemon()?;
            println!(
                "session {id} ready at http://localhost:{}/session/{id}",
                crate::facade::resolved_port()
            );
            if let Some(uuid) = &s.claude_session {
                println!("linked to Claude session {uuid}");
            }
            Ok(0)
        }
        "post" => {
            let name = session_arg.context("usage: muckdb session post <name> --md <text|->")?;
            let id = slug(&name);
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
            let tile_name = p.get("name").context("--name <tile> required")?.to_string();
            let db = p.get("db").context("--db <path> required")?.to_string();
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
                "usage: muckdb session <create|list|post|section|tile|move|screenshot|export|import|rm> ...\n\
                 \n  create <name> [--title T] [--claude UUID]\n  list\n  \
                 post <name> --md <text|-> [--name TILE] [--title T]\n  \
                 section <name> --name TILE --title HEADING   (a heading that groups the panels after it)\n  \
                 move <name> --tile T (--up | --down | --to N | --before TILE | --after TILE)\n  \
                 tile <name> --name TILE --db DB (--view V | --sql SQL) [--chart bar|stacked|line|area|scatter|pie|table|heatmap|box|map] [--x COL] [--y C1,C2] [--title T] [--caption C]\n                       \
                 [--value COL]  (heatmap: the cell value; --x and --y name the two axes, one row per pair)\n                       \
                 [--no-values]  (heatmap: colour cells only — hover still shows the figure)\n                       \
                 --chart map: --lat COL --lon COL (else auto-detected lat/latitude & lon/lng/longitude); markers shade by point count, or --value COL by magnitude; --label COL names points in the hover tooltip\n                       \
                 --chart box: --x the box label, --y min,q1,median,q3,max (five columns, aggregated in the view)\n                       \
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
    fn read_md_unescapes_shell_literal_newlines() {
        assert_eq!(read_md("# T\\n\\nBody").unwrap(), "# T\n\nBody");
        assert_eq!(read_md("a\\tb").unwrap(), "a\tb");
        assert_eq!(read_md("keep \\\\n literal").unwrap(), "keep \\n literal");
        assert_eq!(read_md("real\nnewline").unwrap(), "real\nnewline"); // untouched
        assert_eq!(read_md("trailing\\").unwrap(), "trailing\\");
        assert_eq!(read_md("\\x unknown").unwrap(), "\\x unknown");
    }
}
