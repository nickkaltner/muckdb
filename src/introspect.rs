//! Reading database contents by shelling out to `duckdb -json`, staying true to
//! the "facade over the duckdb CLI" design rather than linking libduckdb.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A table (or view) discovered in a database.
#[derive(Debug, Clone, Serialize)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub column_count: i64,
    pub estimated_size: Option<i64>,
    #[serde(default)]
    pub is_view: bool,
}

/// A page of a table's rows, with columns in table order.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// Total rows matching the current filters (across all pages).
    pub total: i64,
    pub offset: u32,
    pub limit: u32,
}

/// Run a read-only query against `db` and parse `duckdb -json` output into rows.
pub(crate) fn query_json(db: &str, sql: &str) -> Result<Vec<Value>> {
    let output = Command::new("duckdb")
        .arg("-readonly")
        .arg("-json")
        .arg(db)
        .arg("-c")
        .arg(sql)
        .output()
        .context("failed to run `duckdb` — is it installed and on PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("duckdb error: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).context("parsing duckdb JSON output")
}

/// Escape an identifier for safe interpolation inside double quotes.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Escape a string for safe interpolation inside single quotes (SQL literal).
fn escape_literal(value: &str) -> String {
    value.replace('\'', "''")
}

/// True if a duckdb type name denotes a numeric column.
pub fn is_numeric(col_type: &str) -> bool {
    let t = col_type.to_ascii_uppercase();
    if t.contains("INTERVAL") {
        return false; // contains "INT" but isn't a number
    }
    const NEEDLES: &[&str] = &[
        "INT", "DEC", "DOUBLE", "FLOAT", "REAL", "NUMERIC", "HUGEINT",
    ];
    NEEDLES.iter().any(|n| t.contains(n))
}

/// One facet filter on a column: an exact value (string-compared), a numeric
/// range (`min`/`max`), or a temporal range (`tmin`/`tmax`, ISO strings).
#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    pub column: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub tmin: Option<String>,
    #[serde(default)]
    pub tmax: Option<String>,
}

impl Filter {
    fn is_range(&self) -> bool {
        self.value.is_none() && (self.min.is_some() || self.max.is_some())
    }
    fn is_trange(&self) -> bool {
        self.tmin.is_some() || self.tmax.is_some()
    }
}

/// True if a duckdb type name denotes a date/time column.
pub fn is_temporal(col_type: &str) -> bool {
    let t = col_type.to_ascii_uppercase();
    t.contains("DATE") || t.contains("TIME")
}

/// Build a SQL `WHERE` body (without the `WHERE` keyword) from a free-text
/// search and a set of facet filters, or `None` when neither is present.
///
/// `q` matches any column cast to text (case-insensitive); each filter pins a
/// column to a value compared as text. Pure and side-effect free so it can be
/// unit-tested without duckdb.
pub fn build_where(columns: &[String], q: Option<&str>, filters: &[Filter]) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();

    // Value filters: group by column — values within a column combine with OR
    // (the value is any of the chosen ones), and columns combine with AND.
    let mut by_col: Vec<(String, Vec<String>)> = Vec::new();
    for f in filters {
        if let Some(v) = &f.value {
            match by_col.iter_mut().find(|(c, _)| *c == f.column) {
                Some((_, vals)) => vals.push(v.clone()),
                None => by_col.push((f.column.clone(), vec![v.clone()])),
            }
        }
    }
    for (col, vals) in &by_col {
        let ors: Vec<String> = vals
            .iter()
            .map(|v| {
                format!(
                    "CAST({} AS VARCHAR) = '{}'",
                    quote_ident(col),
                    escape_literal(v)
                )
            })
            .collect();
        clauses.push(if ors.len() == 1 {
            ors.into_iter().next().unwrap()
        } else {
            format!("({})", ors.join(" OR "))
        });
    }

    // Range filters: numeric column bounded by min and/or max.
    for f in filters.iter().filter(|f| f.is_range()) {
        let col = quote_ident(&f.column);
        let mut parts: Vec<String> = Vec::new();
        if let Some(mn) = f.min {
            parts.push(format!("{col} >= {mn}"));
        }
        if let Some(mx) = f.max {
            parts.push(format!("{col} <= {mx}"));
        }
        if !parts.is_empty() {
            clauses.push(format!("({})", parts.join(" AND ")));
        }
    }

    // Temporal range filters: compare against ISO date/time string literals,
    // which duckdb casts to the column's type.
    for f in filters.iter().filter(|f| f.is_trange()) {
        let col = quote_ident(&f.column);
        let mut parts: Vec<String> = Vec::new();
        if let Some(lo) = &f.tmin {
            parts.push(format!("{col} >= '{}'", escape_literal(lo)));
        }
        if let Some(hi) = &f.tmax {
            parts.push(format!("{col} <= '{}'", escape_literal(hi)));
        }
        if !parts.is_empty() {
            clauses.push(format!("({})", parts.join(" AND ")));
        }
    }

    if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
        let needle = escape_literal(q);
        let ors: Vec<String> = columns
            .iter()
            .map(|c| format!("CAST({} AS VARCHAR) ILIKE '%{}%'", quote_ident(c), needle))
            .collect();
        if !ors.is_empty() {
            clauses.push(format!("({})", ors.join(" OR ")));
        }
    }

    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(" AND "))
    }
}

/// One histogram bucket over a numeric column's range.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistBin {
    pub lo: f64,
    pub hi: f64,
    pub count: i64,
}

/// Distribute `(bucket_index, count)` pairs from duckdb's `width_bucket` into
/// `nbins` even bins spanning `[lo, hi]`. Under/overflow buckets (0 and
/// `nbins+1`) fold into the first/last bin. Pure, so it is unit-tested directly.
pub fn bucketize(lo: f64, hi: f64, nbins: usize, raw: &[(i64, i64)]) -> Vec<HistBin> {
    let nbins = nbins.max(1);
    // Degenerate range (hi <= lo, or a NaN bound): a single bin holding all.
    if !matches!(lo.partial_cmp(&hi), Some(std::cmp::Ordering::Less)) {
        let total: i64 = raw.iter().map(|(_, c)| c).sum();
        return vec![HistBin {
            lo,
            hi,
            count: total,
        }];
    }
    let width = (hi - lo) / nbins as f64;
    let mut bins: Vec<HistBin> = (0..nbins)
        .map(|i| HistBin {
            lo: lo + width * i as f64,
            hi: lo + width * (i + 1) as f64,
            count: 0,
        })
        .collect();
    for &(bucket, count) in raw {
        // width_bucket returns 1..=nbins in range, 0 below, nbins+1 at/above hi.
        let idx = (bucket.clamp(1, nbins as i64) - 1) as usize;
        bins[idx].count += count;
    }
    bins
}

/// A top value of a categorical column, used to build facets.
#[derive(Debug, Clone, Serialize)]
pub struct TopValue {
    pub value: String,
    pub count: i64,
}

/// Per-column statistics for the table-stats view.
#[derive(Debug, Clone, Serialize)]
pub struct ColumnStat {
    pub name: String,
    pub col_type: String,
    pub numeric: bool,
    #[serde(default)]
    pub temporal: bool,
    pub nulls: i64,
    pub distinct: i64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub avg: Option<f64>,
    /// Quartiles for numeric columns (box-plot): 25th, 50th (median), 75th.
    #[serde(default)]
    pub q1: Option<f64>,
    #[serde(default)]
    pub median: Option<f64>,
    #[serde(default)]
    pub q3: Option<f64>,
    pub histogram: Vec<HistBin>,
    pub top: Vec<TopValue>,
    /// Date-bucketed counts (time order) for temporal columns.
    #[serde(default)]
    pub timeline: Vec<TimeBin>,
}

/// Statistics for a whole table.
#[derive(Debug, Clone, Serialize)]
pub struct TableStats {
    pub row_count: i64,
    pub columns: Vec<ColumnStat>,
}

/// The `(name, type)` of each column, in table order, via `DESCRIBE`.
fn describe(db: &str, table: &str) -> Result<Vec<(String, String)>> {
    let rows = query_json(db, &format!("DESCRIBE {}", quote_ident(table)))?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let name = r.get("column_name").and_then(Value::as_str)?.to_string();
            let ty = r
                .get("column_type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some((name, ty))
        })
        .collect())
}

/// List the tables and views in a database.
pub fn list_tables(db: &str) -> Result<Vec<TableInfo>> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    let rows = query_json(
        db,
        "SELECT schema_name, table_name AS name, column_count, estimated_size, false AS is_view \
           FROM duckdb_tables() \
         UNION ALL \
         SELECT schema_name, view_name AS name, column_count, NULL AS estimated_size, true AS is_view \
           FROM duckdb_views() WHERE NOT internal \
         ORDER BY name",
    )?;
    let tables = rows
        .into_iter()
        .map(|r| TableInfo {
            schema: r
                .get("schema_name")
                .and_then(Value::as_str)
                .unwrap_or("main")
                .to_string(),
            name: r
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            column_count: r.get("column_count").and_then(Value::as_i64).unwrap_or(0),
            estimated_size: r.get("estimated_size").and_then(Value::as_i64),
            is_view: r.get("is_view").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect();
    Ok(tables)
}

/// Preview one page of a table's rows, optionally filtered by a free-text
/// search and facet filters. Also returns the total matching row count so the
/// caller can paginate.
#[allow(clippy::too_many_arguments)]
pub fn preview(
    db: &str,
    table: &str,
    limit: u32,
    offset: u32,
    q: Option<&str>,
    filters: &[Filter],
    sort: Option<&str>,
    dir: Option<&str>,
) -> Result<Preview> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    // Column order is authoritative from DESCRIBE (also drives text search and
    // the empty-result case).
    let columns: Vec<String> = describe(db, table)?.into_iter().map(|(n, _)| n).collect();
    let tbl = quote_ident(table);

    let where_sql = build_where(&columns, q, filters)
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();

    // ORDER BY only on a real column (avoids injection); default ASC.
    let order_sql = match sort {
        Some(s) if columns.iter().any(|c| c == s) => {
            let d = if dir == Some("desc") { "DESC" } else { "ASC" };
            format!(" ORDER BY {} {}", quote_ident(s), d)
        }
        _ => String::new(),
    };

    let total = query_json(db, &format!("SELECT count(*) AS n FROM {tbl}{where_sql}"))?
        .first()
        .and_then(|r| r.get("n").and_then(Value::as_i64))
        .unwrap_or(0);

    let sql = format!("SELECT * FROM {tbl}{where_sql}{order_sql} LIMIT {limit} OFFSET {offset}");
    let rows = query_json(db, &sql)?;

    let data = rows
        .into_iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| row.get(c).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();

    Ok(Preview {
        columns,
        rows: data,
        total,
        offset,
        limit,
    })
}

/// Export the full filtered result set as CSV or JSON text (no row limit),
/// using duckdb's own `-csv` / `-json` output.
pub fn export(
    db: &str,
    table: &str,
    format: &str,
    q: Option<&str>,
    filters: &[Filter],
) -> Result<String> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    let columns: Vec<String> = describe(db, table)?.into_iter().map(|(n, _)| n).collect();
    let where_sql = build_where(&columns, q, filters)
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();
    let sql = format!("SELECT * FROM {}{}", quote_ident(table), where_sql);

    let out_flag = if format.eq_ignore_ascii_case("json") {
        "-json"
    } else {
        "-csv"
    };
    let output = Command::new("duckdb")
        .arg("-readonly")
        .arg(out_flag)
        .arg(db)
        .arg("-c")
        .arg(&sql)
        .output()
        .context("failed to run `duckdb` — is it installed and on PATH?")?;
    if !output.status.success() {
        bail!(
            "duckdb error: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Result of an ad-hoc read-only query from the query editor.
#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub row_count: usize,
}

/// Run an arbitrary read-only SQL statement and return its rows.
pub fn query(db: &str, sql: &str) -> Result<QueryResult> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    let rows = query_json(db, sql)?;
    let columns: Vec<String> = match rows.first() {
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        _ => Vec::new(),
    };
    let data: Vec<Vec<Value>> = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| row.get(c).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    let row_count = data.len();
    Ok(QueryResult {
        columns,
        rows: data,
        row_count,
    })
}

/// One column's schema, from `DESCRIBE`.
#[derive(Debug, Clone, Serialize)]
pub struct ColumnSchema {
    pub name: String,
    pub col_type: String,
    pub nullable: bool,
    pub key: Option<String>,
    pub default: Option<String>,
}

/// The schema (column definitions) of a table, via `DESCRIBE`.
pub fn schema(db: &str, table: &str) -> Result<Vec<ColumnSchema>> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    let rows = query_json(db, &format!("DESCRIBE {}", quote_ident(table)))?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let name = r.get("column_name").and_then(Value::as_str)?.to_string();
            let nonempty = |k: &str| {
                r.get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            Some(ColumnSchema {
                col_type: r
                    .get("column_type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                nullable: r.get("null").and_then(Value::as_str) != Some("NO"),
                key: nonempty("key"),
                default: nonempty("default"),
                name,
            })
        })
        .collect())
}

/// The `CREATE VIEW … AS …` definition of a view, or `None` for a base table.
pub fn view_sql(db: &str, name: &str) -> Result<Option<String>> {
    if !Path::new(db).exists() {
        return Ok(None);
    }
    let rows = query_json(
        db,
        &format!(
            "SELECT sql FROM duckdb_views() WHERE view_name = '{}' AND NOT internal",
            escape_literal(name)
        ),
    )?;
    Ok(rows
        .first()
        .and_then(|r| r.get("sql").and_then(Value::as_str))
        .map(str::to_string))
}

/// A facet for the left panel: either a categorical value list or a numeric
/// range. Serialized with a `kind` tag for the frontend to switch on.
/// One bucket of a temporal histogram (e.g. a day or month), with its count.
#[derive(Debug, Clone, Serialize)]
pub struct TimeBin {
    pub label: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ColumnFacet {
    Values {
        name: String,
        values: Vec<TopValue>,
    },
    Range {
        name: String,
        min: Option<f64>,
        max: Option<f64>,
    },
    Time {
        name: String,
        min: Option<String>,
        max: Option<String>,
        bins: Vec<TimeBin>,
    },
}

/// True if a column name looks like an identifier (so it gets no range facet).
fn looks_like_id(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "id" || n == "rowid" || n.ends_with("_id")
}

/// Compute facets for each column. Categorical columns get a value list;
/// numeric (non-id) columns get a min/max range. Each column's facet is computed
/// with all active filters EXCEPT those on that column itself — the standard
/// faceted-search behaviour that keeps a column's other options visible after you
/// constrain it. The free-text search applies throughout.
pub fn facets(
    db: &str,
    table: &str,
    q: Option<&str>,
    filters: &[Filter],
) -> Result<Vec<ColumnFacet>> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    let cols = describe(db, table)?;
    let all_names: Vec<String> = cols.iter().map(|(n, _)| n.clone()).collect();
    let tbl = quote_ident(table);

    let mut out = Vec::new();
    for (name, ty) in &cols {
        if is_numeric(ty) && looks_like_id(name) {
            continue; // id-like numeric columns get no facet
        }
        let others: Vec<Filter> = filters
            .iter()
            .filter(|f| &f.column != name)
            .cloned()
            .collect();
        let qc = quote_ident(name);
        let not_null = format!("{qc} IS NOT NULL");
        let where_sql = match build_where(&all_names, q, &others) {
            Some(w) => format!(" WHERE {w} AND {not_null}"),
            None => format!(" WHERE {not_null}"),
        };

        if is_temporal(ty) {
            let row = query_json(
                db,
                &format!(
                    "SELECT min({qc})::VARCHAR AS lo, max({qc})::VARCHAR AS hi, \
                     datediff('day', CAST(min({qc}) AS TIMESTAMP), CAST(max({qc}) AS TIMESTAMP)) AS days \
                     FROM {tbl}{where_sql}"
                ),
            )?;
            let first = row.first();
            let min = first
                .and_then(|r| r.get("lo").and_then(Value::as_str))
                .map(str::to_string);
            let max = first
                .and_then(|r| r.get("hi").and_then(Value::as_str))
                .map(str::to_string);
            let days = first
                .and_then(|r| r.get("days").and_then(Value::as_i64))
                .unwrap_or(0);
            let gran = match days {
                d if d <= 2 => "hour",
                d if d <= 90 => "day",
                d if d <= 1500 => "month",
                _ => "year",
            };
            let brows = query_json(
                db,
                &format!(
                    "SELECT date_trunc('{gran}', CAST({qc} AS TIMESTAMP))::VARCHAR AS b, \
                     count(*) AS c FROM {tbl}{where_sql} GROUP BY b ORDER BY b"
                ),
            )?;
            let bins = brows
                .into_iter()
                .filter_map(|r| {
                    Some(TimeBin {
                        label: r.get("b").and_then(Value::as_str)?.to_string(),
                        count: r.get("c").and_then(Value::as_i64).unwrap_or(0),
                    })
                })
                .collect();
            out.push(ColumnFacet::Time {
                name: name.clone(),
                min,
                max,
                bins,
            });
        } else if is_numeric(ty) {
            let row = query_json(
                db,
                &format!("SELECT min({qc}) AS lo, max({qc}) AS hi FROM {tbl}{where_sql}"),
            )?;
            let (min, max) = row
                .first()
                .map(|r| {
                    (
                        r.get("lo").and_then(Value::as_f64),
                        r.get("hi").and_then(Value::as_f64),
                    )
                })
                .unwrap_or((None, None));
            out.push(ColumnFacet::Range {
                name: name.clone(),
                min,
                max,
            });
        } else {
            let rows = query_json(
                db,
                &format!(
                    "SELECT CAST({qc} AS VARCHAR) AS v, count(*) AS c FROM {tbl}{where_sql} \
                     GROUP BY v ORDER BY c DESC, v LIMIT {TOP_VALUES}"
                ),
            )?;
            let values = rows
                .into_iter()
                .filter_map(|r| {
                    Some(TopValue {
                        value: r.get("v").and_then(Value::as_str)?.to_string(),
                        count: r.get("c").and_then(Value::as_i64).unwrap_or(0),
                    })
                })
                .collect();
            out.push(ColumnFacet::Values {
                name: name.clone(),
                values,
            });
        }
    }
    Ok(out)
}

/// Number of histogram buckets for numeric columns.
const HIST_BINS: usize = 12;
/// Number of top values kept per categorical column (facets).
const TOP_VALUES: usize = 12;

/// Compute per-column statistics for a table: summaries for every column,
/// histograms for numeric columns, and top values (facets) for the rest.
pub fn stats(db: &str, table: &str) -> Result<TableStats> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    let cols = describe(db, table)?;
    let tbl = quote_ident(table);

    // One pass for row count + per-column null/distinct (+ numeric min/max/avg).
    let mut sel = String::from("SELECT count(*) AS n");
    for (i, (name, ty)) in cols.iter().enumerate() {
        let q = quote_ident(name);
        sel.push_str(&format!(
            ", count({q}) AS nn{i}, approx_count_distinct({q}) AS nd{i}"
        ));
        if is_numeric(ty) {
            sel.push_str(&format!(
                ", min({q}) AS mn{i}, max({q}) AS mx{i}, avg({q}) AS av{i}, \
                 quantile_cont({q}, 0.25) AS q1{i}, median({q}) AS md{i}, quantile_cont({q}, 0.75) AS q3{i}"
            ));
        }
    }
    sel.push_str(&format!(" FROM {tbl}"));
    let agg = query_json(db, &sel)?;
    let agg = agg.first().cloned().unwrap_or(Value::Null);

    let row_count = agg.get("n").and_then(Value::as_i64).unwrap_or(0);
    let getf = |k: &str| agg.get(k).and_then(Value::as_f64);
    let geti = |k: &str| agg.get(k).and_then(Value::as_i64).unwrap_or(0);

    let mut columns = Vec::with_capacity(cols.len());
    for (i, (name, ty)) in cols.iter().enumerate() {
        let numeric = is_numeric(ty);
        let nulls = row_count - geti(&format!("nn{i}"));
        let distinct = geti(&format!("nd{i}"));
        let (min, max, avg, q1, median, q3) = if numeric {
            (
                getf(&format!("mn{i}")),
                getf(&format!("mx{i}")),
                getf(&format!("av{i}")),
                getf(&format!("q1{i}")),
                getf(&format!("md{i}")),
                getf(&format!("q3{i}")),
            )
        } else {
            (None, None, None, None, None, None)
        };

        let temporal = is_temporal(ty);
        let mut histogram = Vec::new();
        let mut top = Vec::new();
        let mut timeline = Vec::new();
        let q = quote_ident(name);

        if numeric {
            if let (Some(lo), Some(hi)) = (min, max) {
                let raw = histogram_buckets(db, &tbl, &q, lo, hi)?;
                histogram = bucketize(lo, hi, HIST_BINS, &raw);
            }
        } else if temporal {
            timeline = temporal_buckets(db, &tbl, &q)?;
        } else {
            let rows = query_json(
                db,
                &format!(
                    "SELECT CAST({q} AS VARCHAR) AS v, count(*) AS c FROM {tbl} \
                     WHERE {q} IS NOT NULL GROUP BY v ORDER BY c DESC, v LIMIT {TOP_VALUES}"
                ),
            )?;
            top = rows
                .into_iter()
                .filter_map(|r| {
                    Some(TopValue {
                        value: r.get("v").and_then(Value::as_str)?.to_string(),
                        count: r.get("c").and_then(Value::as_i64).unwrap_or(0),
                    })
                })
                .collect();
        }

        columns.push(ColumnStat {
            name: name.clone(),
            col_type: ty.clone(),
            numeric,
            temporal,
            nulls,
            distinct,
            min,
            max,
            avg,
            q1,
            median,
            q3,
            histogram,
            top,
            timeline,
        });
    }

    Ok(TableStats { row_count, columns })
}

/// Date-bucketed counts (time order) for a temporal column, auto-granularity.
fn temporal_buckets(db: &str, tbl: &str, qc: &str) -> Result<Vec<TimeBin>> {
    let row = query_json(
        db,
        &format!(
            "SELECT datediff('day', CAST(min({qc}) AS TIMESTAMP), CAST(max({qc}) AS TIMESTAMP)) AS days \
             FROM {tbl} WHERE {qc} IS NOT NULL"
        ),
    )?;
    let days = row
        .first()
        .and_then(|r| r.get("days").and_then(Value::as_i64))
        .unwrap_or(0);
    let gran = match days {
        d if d <= 2 => "hour",
        d if d <= 90 => "day",
        d if d <= 1500 => "month",
        _ => "year",
    };
    let brows = query_json(
        db,
        &format!(
            "SELECT date_trunc('{gran}', CAST({qc} AS TIMESTAMP))::VARCHAR AS b, count(*) AS c \
             FROM {tbl} WHERE {qc} IS NOT NULL GROUP BY b ORDER BY b"
        ),
    )?;
    Ok(brows
        .into_iter()
        .filter_map(|r| {
            Some(TimeBin {
                label: r.get("b").and_then(Value::as_str)?.to_string(),
                count: r.get("c").and_then(Value::as_i64).unwrap_or(0),
            })
        })
        .collect())
}

/// Run duckdb's `width_bucket` for a numeric column, returning
/// `(bucket_index, count)` pairs for `bucketize` to lay out.
fn histogram_buckets(db: &str, tbl: &str, q: &str, lo: f64, hi: f64) -> Result<Vec<(i64, i64)>> {
    if !matches!(lo.partial_cmp(&hi), Some(std::cmp::Ordering::Less)) {
        return Ok(Vec::new());
    }
    // DuckDB has no width_bucket, so compute a 1-based bucket index arithmetically:
    // floor((v - lo) / width) + 1. The maximum value lands in HIST_BINS+1
    // (overflow), which bucketize folds into the last bin.
    let width = (hi - lo) / HIST_BINS as f64;
    let rows = query_json(
        db,
        &format!(
            "SELECT CAST(floor(({q} - {lo}) / {width}) AS BIGINT) + 1 AS b, \
             count(*) AS c FROM {tbl} WHERE {q} IS NOT NULL GROUP BY b ORDER BY b"
        ),
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some((
                r.get("b").and_then(Value::as_i64)?,
                r.get("c").and_then(Value::as_i64).unwrap_or(0),
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value (exact-match) filter.
    fn vf(column: &str, value: &str) -> Filter {
        Filter {
            column: column.into(),
            value: Some(value.into()),
            min: None,
            max: None,
            tmin: None,
            tmax: None,
        }
    }
    /// A numeric range filter.
    fn rf(column: &str, min: Option<f64>, max: Option<f64>) -> Filter {
        Filter {
            column: column.into(),
            value: None,
            min,
            max,
            tmin: None,
            tmax: None,
        }
    }
    /// A temporal range filter.
    fn tf(column: &str, tmin: Option<&str>, tmax: Option<&str>) -> Filter {
        Filter {
            column: column.into(),
            value: None,
            min: None,
            max: None,
            tmin: tmin.map(str::to_string),
            tmax: tmax.map(str::to_string),
        }
    }

    #[test]
    fn classifies_numeric_types() {
        for t in [
            "INTEGER",
            "BIGINT",
            "DOUBLE",
            "DECIMAL(10,2)",
            "FLOAT",
            "HUGEINT",
            "UTINYINT",
        ] {
            assert!(is_numeric(t), "{t} should be numeric");
        }
        for t in [
            "VARCHAR",
            "DATE",
            "BOOLEAN",
            "INTERVAL",
            "TIMESTAMP",
            "BLOB",
        ] {
            assert!(!is_numeric(t), "{t} should not be numeric");
        }
    }

    #[test]
    fn escape_literal_doubles_single_quotes() {
        assert_eq!(escape_literal("O'Brien"), "O''Brien");
        assert_eq!(escape_literal("plain"), "plain");
    }

    #[test]
    fn build_where_is_none_without_inputs() {
        let cols = vec!["a".to_string(), "b".to_string()];
        assert_eq!(build_where(&cols, None, &[]), None);
        assert_eq!(build_where(&cols, Some("   "), &[]), None);
    }

    #[test]
    fn build_where_filters_compare_as_text_and_escape() {
        let cols = vec!["name".to_string()];
        let filters = vec![vf("name", "O'Hare")];
        assert_eq!(
            build_where(&cols, None, &filters).unwrap(),
            "CAST(\"name\" AS VARCHAR) = 'O''Hare'"
        );
    }

    #[test]
    fn build_where_groups_same_column_values_with_or() {
        let cols = vec!["species".to_string()];
        let filters = vec![vf("species", "coot"), vf("species", "teal")];
        assert_eq!(
            build_where(&cols, None, &filters).unwrap(),
            "(CAST(\"species\" AS VARCHAR) = 'coot' OR CAST(\"species\" AS VARCHAR) = 'teal')"
        );
    }

    #[test]
    fn build_where_joins_different_columns_with_and() {
        let cols = vec!["a".to_string(), "b".to_string()];
        let filters = vec![vf("a", "1"), vf("b", "2")];
        assert_eq!(
            build_where(&cols, None, &filters).unwrap(),
            "CAST(\"a\" AS VARCHAR) = '1' AND CAST(\"b\" AS VARCHAR) = '2'"
        );
    }

    #[test]
    fn build_where_search_spans_all_columns() {
        let cols = vec!["a".to_string(), "b".to_string()];
        let w = build_where(&cols, Some("x"), &[]).unwrap();
        assert_eq!(
            w,
            "(CAST(\"a\" AS VARCHAR) ILIKE '%x%' OR CAST(\"b\" AS VARCHAR) ILIKE '%x%')"
        );
    }

    #[test]
    fn build_where_combines_filters_and_search_with_and() {
        let cols = vec!["a".to_string()];
        let filters = vec![vf("a", "1")];
        let w = build_where(&cols, Some("z"), &filters).unwrap();
        assert_eq!(
            w,
            "CAST(\"a\" AS VARCHAR) = '1' AND (CAST(\"a\" AS VARCHAR) ILIKE '%z%')"
        );
    }

    #[test]
    fn build_where_range_filter_bounds_numeric_column() {
        let cols = vec!["temp".to_string()];
        // both bounds
        assert_eq!(
            build_where(&cols, None, &[rf("temp", Some(0.0), Some(10.0))]).unwrap(),
            "(\"temp\" >= 0 AND \"temp\" <= 10)"
        );
        // min only
        assert_eq!(
            build_where(&cols, None, &[rf("temp", Some(5.0), None)]).unwrap(),
            "(\"temp\" >= 5)"
        );
    }

    #[test]
    fn build_where_temporal_range_uses_string_literals() {
        let cols = vec!["seen".to_string()];
        assert_eq!(
            build_where(
                &cols,
                None,
                &[tf("seen", Some("2026-01-01"), Some("2026-02-01"))]
            )
            .unwrap(),
            "(\"seen\" >= '2026-01-01' AND \"seen\" <= '2026-02-01')"
        );
    }

    #[test]
    fn classifies_temporal_types() {
        for t in ["DATE", "TIMESTAMP", "TIME", "TIMESTAMP WITH TIME ZONE"] {
            assert!(is_temporal(t), "{t} should be temporal");
        }
        for t in ["INTEGER", "VARCHAR", "DOUBLE"] {
            assert!(!is_temporal(t), "{t} should not be temporal");
        }
    }

    #[test]
    fn build_where_mixes_value_and_range_filters() {
        let cols = vec!["species".to_string(), "temp".to_string()];
        let filters = vec![vf("species", "coot"), rf("temp", Some(1.0), Some(2.0))];
        assert_eq!(
            build_where(&cols, None, &filters).unwrap(),
            "CAST(\"species\" AS VARCHAR) = 'coot' AND (\"temp\" >= 1 AND \"temp\" <= 2)"
        );
    }

    #[test]
    fn bucketize_lays_out_even_bins_and_folds_overflow() {
        // range [0,10], 5 bins of width 2.
        let raw = [(1, 3), (3, 5), (6, 2)]; // bucket 6 = overflow -> last bin
        let bins = bucketize(0.0, 10.0, 5, &raw);
        assert_eq!(bins.len(), 5);
        assert_eq!(
            bins[0],
            HistBin {
                lo: 0.0,
                hi: 2.0,
                count: 3
            }
        );
        assert_eq!(
            bins[2],
            HistBin {
                lo: 4.0,
                hi: 6.0,
                count: 5
            }
        );
        assert_eq!(bins[4].count, 2); // overflow folded into final bin
    }

    #[test]
    fn bucketize_underflow_folds_into_first_bin() {
        let bins = bucketize(0.0, 10.0, 5, &[(0, 7)]);
        assert_eq!(bins[0].count, 7);
    }

    #[test]
    fn bucketize_degenerate_range_is_single_bin() {
        let bins = bucketize(5.0, 5.0, 12, &[(1, 4)]);
        assert_eq!(bins.len(), 1);
        assert_eq!(
            bins[0],
            HistBin {
                lo: 5.0,
                hi: 5.0,
                count: 4
            }
        );
    }
}
