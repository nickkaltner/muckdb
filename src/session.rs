//! Sessions: named dashboards of tiles (panels) that an agent posts to from the
//! CLI. A tile is either markdown or a data view (a duckdb view or inline SQL)
//! rendered as a chart and explorable as a faceted search. Stored as one JSON
//! file per session under the data dir, shared with the daemon.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{paths, store};

/// How a data tile should be charted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chart {
    /// bar | stacked | line | area | scatter | pie | table
    pub kind: String,
    #[serde(default)]
    pub x: Option<String>,
    #[serde(default)]
    pub y: Vec<String>,
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
        chart: Chart,
        #[serde(default)]
        caption: Option<String>,
    },
}

impl Tile {
    pub fn name(&self) -> &str {
        match self {
            Tile::Markdown { name, .. } | Tile::View { name, .. } => name,
        }
    }
}

/// A session dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub tiles: Vec<Tile>,
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

fn save(session: &Session) -> Result<()> {
    let path = path_for(&session.id)?;
    let json = serde_json::to_string_pretty(session)?;
    fs::write(&path, json).with_context(|| format!("writing {path:?}"))?;
    Ok(())
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
        created: now,
        updated: now,
        tiles: Vec::new(),
    })
}

/// Insert or replace a tile (matched by name), preserving order.
fn upsert_tile(session: &mut Session, tile: Tile) {
    if let Some(slot) = session.tiles.iter_mut().find(|t| t.name() == tile.name()) {
        *slot = tile;
    } else {
        session.tiles.push(tile);
    }
    session.updated = store::now_millis();
}

// ---- CLI -----------------------------------------------------------------

/// A tiny `--key value` + positional argument parser for the session CLI.
struct Args {
    flags: Vec<(String, String)>,
    positionals: Vec<String>,
}

impl Args {
    fn parse(args: &[String]) -> Self {
        let mut flags = Vec::new();
        let mut positionals = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if let Some(key) = a.strip_prefix("--") {
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
}

fn read_md(value: &str) -> Result<String> {
    if value == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        Ok(value.to_string())
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
            let s = load_or_new(&id, p.get("title").map(str::to_string).or(Some(name)));
            save(&s?)?;
            crate::facade::ensure_daemon()?;
            println!(
                "session {id} ready at http://localhost:{}/session/{id}",
                crate::facade::PORT
            );
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
                chart: Chart {
                    kind: p.get("chart").unwrap_or("table").to_string(),
                    x: p.get("x").map(str::to_string),
                    y,
                },
                caption: p.get("caption").map(str::to_string),
            };
            let mut s = load_or_new(&id, None)?;
            upsert_tile(&mut s, tile);
            save(&s)?;
            crate::facade::ensure_daemon()?;
            println!("set tile '{tile_name}' in session {id}");
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
                let path = path_for(&id)?;
                let _ = fs::remove_file(path);
                println!("removed session {id}");
            }
            Ok(0)
        }
        _ => {
            eprintln!(
                "usage: muckdb session <create|list|post|tile|rm> ...\n\
                 \n  create <name> [--title T]\n  list\n  \
                 post <name> --md <text|-> [--name TILE] [--title T]\n  \
                 tile <name> --name TILE --db DB (--view V | --sql SQL) [--chart bar|stacked|line|area|scatter|pie|table] [--x COL] [--y C1,C2] [--title T] [--caption C]\n  \
                 rm <name> [--tile TILE]"
            );
            Ok(2)
        }
    }
}
