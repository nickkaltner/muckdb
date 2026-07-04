//! Column display formats — how a numeric column should be rendered (currency,
//! units, decimals) in facets, charts, stats and tables.
//!
//! Two sources, merged with the registry winning:
//!   1. the DuckDB column COMMENT (travels with the database) — a `muckdb:{json}`
//!      marker, or a bare `{json}` comment;
//!   2. a muckdb-side registry (`<data-dir>/formats.json`) the agent sets via
//!      `muckdb format ...`, which can target a column by name (any table) or a
//!      specific table/view column, and works even on read-only databases.

use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{paths, store};

/// How to render a column value. All fields optional; an empty Format is a no-op.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Format {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// Group the integer part with thousands separators.
    #[serde(default, skip_serializing_if = "is_false")]
    pub group: bool,
    /// Render the value as a date/time in this zone: "local" (the viewer's),
    /// "utc", or an IANA name like "Australia/Brisbane". Naive DB timestamps
    /// are read as UTC instants. Charts with this column on the x-axis draw
    /// their time axis in the same zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// The stored value is a numeric epoch in this unit: "s" | "ms" | "us".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
    /// Render the cell as a hyperlink to this URL template. Templates
    /// substitute `{value}` (this column's value) and `{column_name}` (any
    /// other column in the same row). In the URL substitutions are
    /// percent-encoded by default; `{name:raw}` injects verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The link's text, as a template with the same substitutions (verbatim by
    /// default; `{name:url}` percent-encodes). Defaults to the formatted value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_title: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Format {
    fn is_empty(&self) -> bool {
        self.prefix.is_none()
            && self.suffix.is_none()
            && self.decimals.is_none()
            && !self.group
            && self.tz.is_none()
            && self.epoch.is_none()
            && self.link.is_none()
            && self.link_title.is_none()
    }
}

/// One stored registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// The database id (stable hash of its path) this applies to.
    pub db: String,
    /// A specific table/view, or `None` to match the column in any relation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    pub column: String,
    pub format: Format,
}

fn registry_path() -> Result<std::path::PathBuf> {
    Ok(paths::data_dir()?.join("formats.json"))
}

fn load_registry() -> Result<Vec<Entry>> {
    let path = registry_path()?;
    match fs::read_to_string(&path) {
        Ok(s) if !s.trim().is_empty() => serde_json::from_str(&s).context("parsing formats.json"),
        _ => Ok(Vec::new()),
    }
}

fn save_registry(entries: &[Entry]) -> Result<()> {
    let path = registry_path()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok();
    }
    fs::write(&path, serde_json::to_string_pretty(entries)?)
        .with_context(|| format!("writing {path:?}"))?;
    Ok(())
}

/// A fingerprint of the format registry file, folded into the daemon's state
/// snapshot — so a `muckdb format` write changes the snapshot and the watcher
/// broadcasts it, refreshing every open browser. Masked to 53 bits so the
/// value survives JSON→JavaScript number round-tripping exactly.
pub fn registry_rev() -> u64 {
    use std::hash::{Hash, Hasher};
    let Ok(path) = registry_path() else { return 0 };
    match fs::read(&path) {
        Ok(bytes) => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut h);
            h.finish() & ((1 << 53) - 1)
        }
        Err(_) => 0,
    }
}

/// Registry entries keyed to any of these db ids (for bundling into an export).
pub fn entries_for_db_ids(ids: &[String]) -> Result<Vec<Entry>> {
    Ok(load_registry()?
        .into_iter()
        .filter(|e| ids.contains(&e.db))
        .collect())
}

/// Merge imported entries into the registry; an imported entry replaces an
/// existing one for the same db/table/column.
pub fn merge_entries(new: Vec<Entry>) -> Result<()> {
    let mut entries = load_registry()?;
    for e in new {
        entries.retain(|x| !(x.db == e.db && x.table == e.table && x.column == e.column));
        entries.push(e);
    }
    save_registry(&entries)
}

/// Upsert (or, if `format` is empty, remove) a registry entry for db/table/column.
fn set_entry(db_id: &str, table: Option<&str>, column: &str, format: Format) -> Result<()> {
    let mut entries = load_registry()?;
    entries.retain(|e| !(e.db == db_id && e.table.as_deref() == table && e.column == column));
    if !format.is_empty() {
        entries.push(Entry {
            db: db_id.to_string(),
            table: table.map(str::to_string),
            column: column.to_string(),
            format,
        });
    }
    save_registry(&entries)
}

/// Parse a column comment for a format spec: a `muckdb:{...}` marker anywhere, or
/// a bare `{...}` comment. Returns None if there's no parseable spec.
fn parse_comment(comment: &str) -> Option<Format> {
    let json = if let Some(idx) = comment.find("muckdb:") {
        comment[idx + "muckdb:".len()..].trim()
    } else {
        let t = comment.trim();
        if t.starts_with('{') { t } else { return None }
    };
    serde_json::from_str::<Format>(json)
        .ok()
        .filter(|f| !f.is_empty())
}

/// The merged set of formats for a database, ready for the UI to apply by column.
#[derive(Debug, Default, Serialize)]
pub struct Merged {
    /// Keyed by column name — applies to that column in any relation.
    pub columns: BTreeMap<String, Format>,
    /// Keyed by `"table.column"` — wins over the name-only entry.
    pub tables: BTreeMap<String, Format>,
}

/// Build the merged format map for a database: DuckDB column comments first,
/// then registry entries (which override).
pub fn merged_for(db: &str) -> Merged {
    let mut m = Merged::default();
    // 1. Column comments (per actual table.column).
    if let Ok(rows) = crate::introspect::query_json(
        db,
        "SELECT table_name, column_name, comment FROM duckdb_columns() \
         WHERE comment IS NOT NULL AND NOT internal",
    ) {
        for r in rows {
            let (Some(t), Some(c), Some(cm)) = (
                r.get("table_name").and_then(|v| v.as_str()),
                r.get("column_name").and_then(|v| v.as_str()),
                r.get("comment").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            if let Some(f) = parse_comment(cm) {
                m.tables.insert(format!("{t}.{c}"), f);
            }
        }
    }
    // 2. Registry overrides for this db.
    let db_id = store::db_id(db);
    if let Ok(entries) = load_registry() {
        for e in entries.into_iter().filter(|e| e.db == db_id) {
            match e.table {
                Some(t) => {
                    m.tables.insert(format!("{t}.{}", e.column), e.format);
                }
                None => {
                    m.columns.insert(e.column, e.format);
                }
            }
        }
    }
    m
}

// ---- CLI -----------------------------------------------------------------

/// `muckdb format <db> <column> [flags]` / `muckdb format list [<db>]`.
pub fn cli(args: &[String]) -> Result<i32> {
    if args.first().map(String::as_str) == Some("list") {
        return list(args.get(1).map(String::as_str));
    }
    let Some(db) = args.first() else {
        return usage();
    };
    let Some(column) = args.get(1) else {
        return usage();
    };
    if column.starts_with("--") {
        return usage();
    }
    let db_id = store::db_id(db);
    let mut table: Option<String> = None;
    let mut clear = false;
    let mut fmt = Format::default();
    let mut i = 2;
    while i < args.len() {
        let a = args[i].as_str();
        let mut next = || {
            i += 1;
            args.get(i).cloned().unwrap_or_default()
        };
        match a {
            "--table" => table = Some(next()),
            "--clear" => clear = true,
            "--prefix" => fmt.prefix = Some(next()),
            "--suffix" => fmt.suffix = Some(next()),
            "--decimals" => fmt.decimals = next().parse().ok(),
            "--thousands" | "--group" => fmt.group = true,
            "--percent" => fmt.suffix = Some("%".to_string()),
            "--tz" | "--timezone" => {
                let z = next();
                // "local"/"utc" are keywords (any case); IANA names pass through.
                fmt.tz = Some(match z.to_lowercase().as_str() {
                    "local" => "local".to_string(),
                    "utc" => "utc".to_string(),
                    _ => z,
                });
            }
            "--epoch" => {
                let u = next().to_lowercase();
                if !matches!(u.as_str(), "s" | "ms" | "us") {
                    eprintln!("--epoch must be s, ms or us (got {u:?})");
                    return Ok(2);
                }
                fmt.epoch = Some(u);
            }
            "--link" => fmt.link = Some(next()),
            "--link-title" => fmt.link_title = Some(next()),
            "--currency" => {
                let code = next().to_uppercase();
                fmt.prefix = Some(currency_symbol(&code));
                fmt.suffix = Some(format!(" {code}"));
                fmt.group = true;
                if fmt.decimals.is_none() {
                    fmt.decimals = Some(2);
                }
            }
            _ => {}
        }
        i += 1;
    }
    let final_fmt = if clear { Format::default() } else { fmt };
    set_entry(&db_id, table.as_deref(), column, final_fmt)?;
    if clear {
        println!("cleared format for {column}");
    } else {
        println!(
            "set format for {column}{}",
            table.map(|t| format!(" (in {t})")).unwrap_or_default()
        );
    }
    crate::facade::ensure_daemon().ok();
    Ok(0)
}

fn currency_symbol(code: &str) -> String {
    match code {
        "USD" | "AUD" | "CAD" | "NZD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" | "CNY" => "¥",
        "INR" => "₹",
        _ => return format!("{code} "),
    }
    .to_string()
}

fn list(db: Option<&str>) -> Result<i32> {
    let filter = db.map(store::db_id);
    let entries = load_registry()?;
    for e in entries
        .iter()
        .filter(|e| filter.as_deref().is_none_or(|d| e.db == d))
    {
        let scope = e
            .table
            .as_deref()
            .map(|t| format!("{t}."))
            .unwrap_or_default();
        println!(
            "{}  {}{}  {}",
            e.db,
            scope,
            e.column,
            serde_json::to_string(&e.format).unwrap_or_default()
        );
    }
    Ok(0)
}

fn usage() -> Result<i32> {
    eprintln!(
        "usage: muckdb format <db> <column> [--table T] [flags]\n       \
         muckdb format list [<db>]\n\nflags:\n  \
         --currency CODE     e.g. USD → $1,234.56 USD\n  \
         --prefix S          text before the number (e.g. $)\n  \
         --suffix S          text after the number (e.g. ' ms')\n  \
         --decimals N        fixed decimal places\n  \
         --thousands         group with thousands separators\n  \
         --percent           append %\n  \
         --tz Z              show a timestamp column in a zone: local, utc,\n                      \
         or an IANA name (e.g. Australia/Brisbane); naive\n                      \
         timestamps are read as UTC. Applies to charts too.\n  \
         --epoch U           the column is a numeric epoch: s, ms or us\n  \
         --link URL          render the cell as a link. The URL is a template:\n                      \
         {{value}} is this column's value, {{other_col}} any\n                      \
         other column in the same row — percent-encoded by\n                      \
         default, {{name:raw}} to inject verbatim.\n  \
         --link-title T      the link's text, same substitutions (verbatim by\n                      \
         default, {{name:url}} to percent-encode). Defaults\n                      \
         to the formatted value.\n  \
         --clear             remove the format for this column"
    );
    Ok(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comment_reads_muckdb_marker() {
        let f = parse_comment("order size muckdb:{\"suffix\":\" units\"}").unwrap();
        assert_eq!(f.suffix.as_deref(), Some(" units"));
        assert_eq!(f.prefix, None);
    }

    #[test]
    fn parse_comment_reads_bare_json_object() {
        let f = parse_comment("{\"prefix\":\"$\",\"decimals\":2,\"group\":true}").unwrap();
        assert_eq!(f.prefix.as_deref(), Some("$"));
        assert_eq!(f.decimals, Some(2));
        assert!(f.group);
    }

    #[test]
    fn parse_comment_ignores_plain_and_empty_and_invalid() {
        assert!(parse_comment("just a normal comment").is_none());
        assert!(parse_comment("muckdb:{}").is_none()); // empty format is a no-op
        assert!(parse_comment("muckdb:not json").is_none());
    }

    #[test]
    fn currency_symbol_maps_known_codes_and_falls_back() {
        assert_eq!(currency_symbol("USD"), "$");
        assert_eq!(currency_symbol("EUR"), "€");
        assert_eq!(currency_symbol("GBP"), "£");
        assert_eq!(currency_symbol("JPY"), "¥");
        assert_eq!(currency_symbol("XYZ"), "XYZ "); // unknown → code + space prefix
    }

    #[test]
    fn parse_comment_reads_tz_and_epoch() {
        let f = parse_comment("muckdb:{\"tz\":\"local\",\"epoch\":\"ms\"}").unwrap();
        assert_eq!(f.tz.as_deref(), Some("local"));
        assert_eq!(f.epoch.as_deref(), Some("ms"));
        assert!(!f.is_empty()); // tz/epoch alone make the format non-empty
    }

    #[test]
    fn parse_comment_reads_link_templates() {
        let f = parse_comment(
            "muckdb:{\"link\":\"https://x.test/u/{value}?c={company_id}\",\"link_title\":\"open {name}\"}",
        )
        .unwrap();
        assert_eq!(
            f.link.as_deref(),
            Some("https://x.test/u/{value}?c={company_id}")
        );
        assert_eq!(f.link_title.as_deref(), Some("open {name}"));
        assert!(!f.is_empty()); // a link alone makes the format non-empty
    }

    #[test]
    fn empty_format_serializes_to_empty_object() {
        let f = Format::default();
        assert!(f.is_empty());
        assert_eq!(serde_json::to_string(&f).unwrap(), "{}");
    }

    #[test]
    fn format_skips_default_fields_when_serializing() {
        let f = Format {
            prefix: Some("$".into()),
            decimals: Some(0),
            ..Format::default()
        };
        // suffix/group are defaults and must be omitted (so comments stay terse).
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            "{\"prefix\":\"$\",\"decimals\":0}"
        );
    }
}
