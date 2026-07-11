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

/// Coerce a duckdb JSON scalar to f64. DuckDB's `-json` renders DECIMAL and
/// HUGEINT as quoted strings (to avoid precision loss), so a plain `as_f64()`
/// misses them — fall back to parsing the string.
pub(crate) fn json_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
}

/// Escape an identifier for safe interpolation inside double quotes.
pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Escape a string for safe interpolation inside single quotes (SQL literal).
/// A plain `YYYY-MM-DD` (no time component) — the shape a day-level facet sends.
fn is_bare_date(s: &str) -> bool {
    s.len() == 10
        && s.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

fn escape_literal(value: &str) -> String {
    value.replace('\'', "''")
}

/// True if a duckdb type name denotes a numeric column. Nested types are
/// never numeric — `DOUBLE[]` contains "DOUBLE" but avg()/min()/histograms
/// don't apply to a list.
pub fn is_numeric(col_type: &str) -> bool {
    let t = col_type.to_ascii_uppercase();
    if t.contains("INTERVAL") || is_nested_type(&t) {
        return false; // INTERVAL contains "INT" but isn't a number
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
    /// Containment match for list columns: the row's array must contain
    /// `value` (as text) rather than equal it — so a `10.0` facet matches
    /// `[1, 10]`, `[10]` and `[10, 400]` alike.
    #[serde(default)]
    pub contains: bool,
    /// Match rows where the column IS NULL (no `value` accompanies this).
    #[serde(default)]
    pub is_null: bool,
    /// Negate the filter: exclude the matching value/null/containment instead
    /// of keeping it (a "NOT" filter — `≠`, `is not NULL`, `∌`). NULL-preserving
    /// for value filters (`IS DISTINCT FROM`).
    #[serde(default)]
    pub negate: bool,
}

impl Filter {
    fn is_range(&self) -> bool {
        self.value.is_none() && (self.min.is_some() || self.max.is_some())
    }
    fn is_trange(&self) -> bool {
        self.tmin.is_some() || self.tmax.is_some()
    }
}

/// True if a duckdb type name denotes a date/time column (a `DATE[]` list is
/// not — date functions don't apply to it).
pub fn is_temporal(col_type: &str) -> bool {
    let t = col_type.to_ascii_uppercase();
    !is_nested_type(&t) && (t.contains("DATE") || t.contains("TIME"))
}

/// True for a scalar BOOLEAN column (not a `BOOLEAN[]` list). Boolean facets
/// keep their zero-count values on screen (true/false both matter) where other
/// columns hide zero-count noise to the "show all" modal.
pub fn is_boolean(col_type: &str) -> bool {
    col_type.eq_ignore_ascii_case("BOOLEAN")
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
    // Containment filters (list columns) group the same way but test whether
    // the row's array holds the value instead of equalling it. An IS NULL
    // filter joins its column's OR group ("QLD or missing" is one choice set).
    let mut by_col: Vec<(String, bool, Vec<String>, bool)> = Vec::new();
    for f in filters {
        if f.negate || (f.value.is_none() && !f.is_null) {
            continue;
        }
        let entry = match by_col
            .iter_mut()
            .find(|(c, k, ..)| *c == f.column && *k == f.contains)
        {
            Some(e) => e,
            None => {
                by_col.push((f.column.clone(), f.contains, Vec::new(), false));
                by_col.last_mut().unwrap()
            }
        };
        match &f.value {
            Some(v) => entry.2.push(v.clone()),
            None => entry.3 = true,
        }
    }
    for (col, contains, vals, has_null) in &by_col {
        let mut ors: Vec<String> = vals
            .iter()
            .map(|v| {
                if *contains {
                    format!(
                        "list_contains(CAST({} AS VARCHAR[]), '{}')",
                        quote_ident(col),
                        escape_literal(v)
                    )
                } else {
                    format!(
                        "CAST({} AS VARCHAR) = '{}'",
                        quote_ident(col),
                        escape_literal(v)
                    )
                }
            })
            .collect();
        if *has_null {
            ors.push(format!("{} IS NULL", quote_ident(col)));
        }
        clauses.push(if ors.len() == 1 {
            ors.into_iter().next().unwrap()
        } else {
            format!("({})", ors.join(" OR "))
        });
    }

    // Negated (NOT) filters: each excludes a value/null/containment and ANDs
    // with everything else (so several negations on one column exclude all of
    // them). Value negation uses `IS DISTINCT FROM` so NULL rows are retained.
    for f in filters.iter().filter(|f| f.negate) {
        let col = quote_ident(&f.column);
        let clause = match &f.value {
            Some(v) if f.contains => format!(
                "NOT list_contains(CAST({col} AS VARCHAR[]), '{}')",
                escape_literal(v)
            ),
            Some(v) => format!(
                "CAST({col} AS VARCHAR) IS DISTINCT FROM '{}'",
                escape_literal(v)
            ),
            None if f.is_null => format!("{col} IS NOT NULL"),
            None => continue,
        };
        clauses.push(clause);
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
    // which duckdb casts to the column's type. A bare-date tmax means "that
    // whole day" — cast to midnight it would drop the rest of the day for
    // timestamp columns, so widen it to an exclusive next-day bound. A tmax
    // with a time component is already an exclusive boundary (the client sends
    // the display zone's next-midnight instant).
    for f in filters.iter().filter(|f| f.is_trange()) {
        let col = quote_ident(&f.column);
        let mut parts: Vec<String> = Vec::new();
        if let Some(lo) = &f.tmin {
            parts.push(format!("{col} >= '{}'", escape_literal(lo)));
        }
        if let Some(hi) = &f.tmax {
            let hi_lit = escape_literal(hi);
            if is_bare_date(hi) {
                parts.push(format!("{col} < DATE '{hi_lit}' + INTERVAL 1 DAY"));
            } else {
                parts.push(format!("{col} < '{hi_lit}'"));
            }
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
pub(crate) fn describe(db: &str, table: &str) -> Result<Vec<(String, String)>> {
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
        bail!("database file does not exist: {db}");
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

/// Nested duckdb types (LIST/STRUCT/MAP/UNION). `duckdb -json` emits their
/// values in duckdb literal syntax (`{red=3, blue=5}`) — not valid JSON, which
/// breaks the whole payload parse. Columns of these types are wrapped in
/// `to_json()`, which the CLI then embeds as real nested JSON.
fn is_nested_type(t: &str) -> bool {
    let t = t.to_uppercase();
    t.contains("STRUCT") || t.contains("MAP(") || t.contains("UNION") || t.contains("[]")
}

/// A list of scalars (`DOUBLE[]`, `VARCHAR[]`) — facetable by element. Lists
/// of structs or lists of lists are not.
fn is_scalar_list_type(t: &str) -> bool {
    let t = t.trim();
    t.ends_with("[]") && !is_nested_type(t[..t.len() - 2].trim_end())
}

/// An element value filter on a scalar-list column means containment. Older
/// clients and saved links may omit the `contains` flag — infer it from the
/// schema. Exception: a *whole-array* value like `[red, blue]` (from the stats
/// per-column top-values) means equality on the whole list, which the equality
/// path handles as `CAST(col AS VARCHAR) = '[red, blue]'`; forcing containment
/// there would test for an element literally named "[red, blue]" and match
/// nothing. So skip values that are themselves an array literal.
pub(crate) fn normalize_contains(cols: &[(String, String)], filters: &[Filter]) -> Vec<Filter> {
    filters
        .iter()
        .cloned()
        .map(|mut f| {
            let whole_array = f
                .value
                .as_deref()
                .map(str::trim)
                .is_some_and(|v| v.starts_with('[') && v.ends_with(']'));
            if f.value.is_some()
                && !f.contains
                && !whole_array
                && cols
                    .iter()
                    .any(|(n, t)| *n == f.column && is_scalar_list_type(t))
            {
                f.contains = true;
            }
            f
        })
        .collect()
}

/// A SELECT projection over (name, type) columns with nested types made
/// JSON-safe via `to_json(col) AS col`.
fn json_safe_projection(cols: &[(String, String)]) -> String {
    cols.iter()
        .map(|(n, t)| {
            let q = quote_ident(n);
            if is_nested_type(t) {
                format!("to_json({q}) AS {q}")
            } else {
                q
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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
        bail!("database file does not exist: {db}");
    }
    // Column order is authoritative from DESCRIBE (also drives text search and
    // the empty-result case).
    let described = describe(db, table)?;
    let columns: Vec<String> = described.iter().map(|(n, _)| n.clone()).collect();
    let tbl = quote_ident(table);

    let filters = normalize_contains(&described, filters);
    let where_sql = build_where(&columns, q, &filters)
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

    let proj = json_safe_projection(&described);
    let sql =
        format!("SELECT {proj} FROM {tbl}{where_sql}{order_sql} LIMIT {limit} OFFSET {offset}");
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
    hidden: &[String],
) -> Result<String> {
    if !Path::new(db).exists() {
        bail!("database file does not exist: {db}");
    }
    let described = describe(db, table)?;
    let columns: Vec<String> = described.iter().map(|(n, _)| n.clone()).collect();
    let filters = normalize_contains(&described, filters);
    // Search still runs across every column; only the projection drops hidden ones.
    let where_sql = build_where(&columns, q, &filters)
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();
    // Columns whose facet eye is closed are excluded from the export. If that
    // would leave nothing (everything hidden), fall back to all columns.
    let visible: Vec<&String> = columns.iter().filter(|c| !hidden.contains(c)).collect();
    let projection = if hidden.is_empty() || visible.is_empty() {
        "*".to_string()
    } else {
        visible
            .iter()
            .map(|c| quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sql = format!(
        "SELECT {} FROM {}{}",
        projection,
        quote_ident(table),
        where_sql
    );

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
        bail!("database file does not exist: {db}");
    }
    // If the result has nested-typed columns, rewrap the query so they come
    // back as real JSON (see is_nested_type). A failed DESCRIBE (multi-statement
    // input, syntax error) falls through to the original SQL untouched.
    let trimmed = sql.trim().trim_end_matches(';');
    let effective = match query_json(db, &format!("DESCRIBE {trimmed}")) {
        Ok(desc) => {
            let cols: Vec<(String, String)> = desc
                .iter()
                .filter_map(|r| {
                    Some((
                        r.get("column_name")?.as_str()?.to_string(),
                        r.get("column_type")?.as_str()?.to_string(),
                    ))
                })
                .collect();
            if cols.iter().any(|(_, t)| is_nested_type(t)) {
                format!("SELECT {} FROM ({trimmed})", json_safe_projection(&cols))
            } else {
                sql.to_string()
            }
        }
        Err(_) => sql.to_string(),
    };
    let rows = query_json(db, &effective)?;
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
        bail!("database file does not exist: {db}");
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

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ColumnFacet {
    Values {
        name: String,
        values: Vec<TopValue>,
        /// True for a list column faceted by element — selecting a value
        /// filters with containment (Filter.contains) rather than equality.
        #[serde(skip_serializing_if = "is_false")]
        list: bool,
        /// True for a BOOLEAN column — the panel keeps its zero-count values
        /// (true/false both shown); other columns hide zero-count values to
        /// the "show all" modal so the panel stays quiet.
        #[serde(skip_serializing_if = "is_false")]
        boolean: bool,
        /// Rows where the column is NULL under the active filters — shown as
        /// its own facet bucket (with a 0 count when filters exclude them).
        nulls: i64,
        /// Whether the column has NULL rows at all (unfiltered) — decides
        /// whether the NULL bucket is offered.
        has_nulls: bool,
        /// Distinct non-null values in the table (elements for a list
        /// column), unfiltered; more than `values.len()` means the list was
        /// truncated to TOP_VALUES.
        total: i64,
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
        bail!("database file does not exist: {db}");
    }
    let cols = describe(db, table)?;
    let all_names: Vec<String> = cols.iter().map(|(n, _)| n.clone()).collect();
    let tbl = quote_ident(table);
    let filters = normalize_contains(&cols, filters);

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
        let cond = build_where(&all_names, q, &others);
        let where_sql = match &cond {
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
            let list = is_scalar_list_type(ty);
            let (values, nulls, has_nulls, total) =
                value_buckets(db, &tbl, &qc, list, cond.as_deref(), TOP_VALUES)?;
            out.push(ColumnFacet::Values {
                name: name.clone(),
                values,
                list,
                boolean: is_boolean(ty),
                nulls,
                has_nulls,
                total,
            });
        }
    }
    Ok(out)
}

/// Every distinct value of one column (with row counts), the shape behind the
/// facet panel's "show all" view. Scoped like the facet panel itself: filters
/// on OTHER columns constrain the counts (values they exclude stay listed
/// with a 0 count), the column's own filters don't. `values` is
/// count-descending and capped at ALL_FACET_VALUES; `total` above
/// `values.len()` means it was truncated.
#[derive(Debug, Clone, Serialize)]
pub struct FacetValues {
    pub values: Vec<TopValue>,
    pub nulls: i64,
    pub has_nulls: bool,
    pub total: i64,
}

pub fn facet_values(
    db: &str,
    table: &str,
    column: &str,
    q: Option<&str>,
    filters: &[Filter],
) -> Result<FacetValues> {
    if !Path::new(db).exists() {
        bail!("database file does not exist: {db}");
    }
    let cols = describe(db, table)?;
    let Some((_, ty)) = cols.iter().find(|(n, _)| n == column) else {
        bail!("no such column: {column}");
    };
    let all_names: Vec<String> = cols.iter().map(|(n, _)| n.clone()).collect();
    let filters = normalize_contains(&cols, filters);
    let others: Vec<Filter> = filters
        .iter()
        .filter(|f| f.column != column)
        .cloned()
        .collect();
    let cond = build_where(&all_names, q, &others);
    let (values, nulls, has_nulls, total) = value_buckets(
        db,
        &quote_ident(table),
        &quote_ident(column),
        is_scalar_list_type(ty),
        cond.as_deref(),
        ALL_FACET_VALUES,
    )?;
    Ok(FacetValues {
        values,
        nulls,
        has_nulls,
        total,
    })
}

/// Distinct values of one column with row counts — by ELEMENT for a list
/// column: each distinct element with the number of rows whose array contains
/// it (list_distinct dedupes within a row so a row counts once per element;
/// selecting one filters by containment, so the 10.0 facet matches [1, 10],
/// [10] and [10, 400] alike).
///
/// The value universe is the whole table (so a value never disappears when a
/// filter on another column drops its count to zero); `cond` — the WHERE built
/// from the other columns' filters + search — is applied as a per-row `FILTER`
/// on the count instead, leaving excluded values listed with a 0 count.
/// Returns the filtered null-row count, whether the column has any nulls at
/// all (unfiltered — decides if the NULL bucket is offered), and the distinct
/// non-null total (unfiltered — `> values.len()` means the list was truncated).
fn value_buckets(
    db: &str,
    tbl: &str,
    qc: &str,
    list: bool,
    cond: Option<&str>,
    limit: usize,
) -> Result<(Vec<TopValue>, i64, bool, i64)> {
    // count(*) over the group, but only rows matching the active filters.
    let count_expr = match cond {
        Some(c) => format!("count(*) FILTER (WHERE ({c}))"),
        None => "count(*)".to_string(),
    };
    let rows = if list {
        // Carry the per-row match flag through the unnest so the FILTER counts
        // only kept rows' elements.
        let keep = match cond {
            Some(c) => format!(", ({c}) AS keep"),
            None => String::new(),
        };
        let cnt = if cond.is_some() {
            "count(*) FILTER (WHERE keep)"
        } else {
            "count(*)"
        };
        query_json(
            db,
            &format!(
                "SELECT CAST(u AS VARCHAR) AS v, {cnt} AS c \
                 FROM (SELECT unnest(list_distinct({qc})) AS u{keep} FROM {tbl}) \
                 WHERE u IS NOT NULL GROUP BY v ORDER BY c DESC, v LIMIT {limit}"
            ),
        )?
    } else {
        query_json(
            db,
            &format!(
                "SELECT CAST({qc} AS VARCHAR) AS v, {count_expr} AS c \
                 FROM {tbl} WHERE {qc} IS NOT NULL \
                 GROUP BY v ORDER BY c DESC, v LIMIT {limit}"
            ),
        )?
    };
    let values = rows
        .into_iter()
        .filter_map(|r| {
            Some(TopValue {
                value: r.get("v").and_then(Value::as_str)?.to_string(),
                count: r.get("c").and_then(Value::as_i64).unwrap_or(0),
            })
        })
        .collect();
    // The NULL bucket's count reflects the filter; has_nulls is unfiltered.
    let nulls_filtered = match cond {
        Some(c) => format!("count(*) FILTER (WHERE {qc} IS NULL AND ({c}))"),
        None => format!("count(*) FILTER (WHERE {qc} IS NULL)"),
    };
    let total_expr = if list {
        format!(
            "(SELECT count(DISTINCT u) \
                FROM (SELECT unnest(list_distinct({qc})) AS u FROM {tbl}) WHERE u IS NOT NULL)"
        )
    } else {
        format!("count(DISTINCT {qc})")
    };
    let meta = query_json(
        db,
        &format!(
            "SELECT {nulls_filtered} AS nulls, \
             count(*) FILTER (WHERE {qc} IS NULL) AS anynull, \
             {total_expr} AS total FROM {tbl}"
        ),
    )?;
    let first = meta.first();
    let nulls = first
        .and_then(|r| r.get("nulls").and_then(Value::as_i64))
        .unwrap_or(0);
    let has_nulls = first
        .and_then(|r| r.get("anynull").and_then(Value::as_i64))
        .unwrap_or(0)
        > 0;
    let total = first
        .and_then(|r| r.get("total").and_then(Value::as_i64))
        .unwrap_or(0);
    Ok((values, nulls, has_nulls, total))
}

/// Number of histogram buckets for numeric columns.
const HIST_BINS: usize = 12;
/// Number of top values kept per categorical column (facets).
const TOP_VALUES: usize = 12;
/// Cap on the "show all values" facet listing (`facet_values`).
const ALL_FACET_VALUES: usize = 1000;
/// Columns whose approximate distinct count is at or below this get an exact
/// `count(DISTINCT …)` instead — cheap at low cardinality, and avoids the
/// HyperLogLog approximation being visibly wrong for small sets.
const EXACT_DISTINCT_MAX: i64 = 10_000;

/// Compute per-column statistics for a table: summaries for every column,
/// histograms for numeric columns, and top values (facets) for the rest.
pub fn stats(db: &str, table: &str, q: Option<&str>, filters: &[Filter]) -> Result<TableStats> {
    if !Path::new(db).exists() {
        bail!("database file does not exist: {db}");
    }
    let cols = describe(db, table)?;
    let tbl = quote_ident(table);

    // Active search + facet filters scope every figure on this tab. `where_sql`
    // is the leading clause for `FROM tbl`; `and_sql` appends to helper queries
    // that already carry their own `WHERE col IS NOT NULL`.
    let col_names: Vec<String> = cols.iter().map(|(n, _)| n.clone()).collect();
    let filters = normalize_contains(&cols, filters);
    let where_cond = build_where(&col_names, q, &filters);
    let where_sql = where_cond
        .as_ref()
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();
    let and_sql = where_cond
        .as_ref()
        .map(|w| format!(" AND ({w})"))
        .unwrap_or_default();

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
    sel.push_str(&format!(" FROM {tbl}{where_sql}"));
    let agg = query_json(db, &sel)?;
    let agg = agg.first().cloned().unwrap_or(Value::Null);

    let row_count = agg.get("n").and_then(Value::as_i64).unwrap_or(0);
    let getf = |k: &str| agg.get(k).and_then(json_f64);
    let geti = |k: &str| {
        agg.get(k)
            .and_then(|v| v.as_i64().or_else(|| json_f64(v).map(|f| f as i64)))
            .unwrap_or(0)
    };

    // Refine the approximate distinct counts: any column the HLL estimate puts at
    // low cardinality is recounted exactly in a single extra pass (cheap there),
    // so small sets show a precise distinct count rather than an HLL guess.
    let small: Vec<usize> = (0..cols.len())
        .filter(|i| geti(&format!("nd{i}")) <= EXACT_DISTINCT_MAX)
        .collect();
    let mut exact_distinct = std::collections::HashMap::new();
    if !small.is_empty() {
        let proj = small
            .iter()
            .map(|&i| format!("count(DISTINCT {}) AS d{i}", quote_ident(&cols[i].0)))
            .collect::<Vec<_>>()
            .join(", ");
        let row = query_json(db, &format!("SELECT {proj} FROM {tbl}{where_sql}"))?
            .first()
            .cloned()
            .unwrap_or(Value::Null);
        for &i in &small {
            if let Some(v) = row
                .get(format!("d{i}"))
                .and_then(|v| v.as_i64().or_else(|| json_f64(v).map(|f| f as i64)))
            {
                exact_distinct.insert(i, v);
            }
        }
    }

    let mut columns = Vec::with_capacity(cols.len());
    for (i, (name, ty)) in cols.iter().enumerate() {
        let numeric = is_numeric(ty);
        let nulls = row_count - geti(&format!("nn{i}"));
        let distinct = exact_distinct
            .get(&i)
            .copied()
            .unwrap_or_else(|| geti(&format!("nd{i}")));
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
                let raw = histogram_buckets(db, &tbl, &q, lo, hi, &and_sql)?;
                histogram = bucketize(lo, hi, HIST_BINS, &raw);
            }
        } else if temporal {
            timeline = temporal_buckets(db, &tbl, &q, &and_sql)?;
        } else {
            let rows = query_json(
                db,
                &format!(
                    "SELECT CAST({q} AS VARCHAR) AS v, count(*) AS c FROM {tbl} \
                     WHERE {q} IS NOT NULL{and_sql} GROUP BY v ORDER BY c DESC, v LIMIT {TOP_VALUES}"
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
fn temporal_buckets(db: &str, tbl: &str, qc: &str, and_sql: &str) -> Result<Vec<TimeBin>> {
    let row = query_json(
        db,
        &format!(
            "SELECT datediff('day', CAST(min({qc}) AS TIMESTAMP), CAST(max({qc}) AS TIMESTAMP)) AS days \
             FROM {tbl} WHERE {qc} IS NOT NULL{and_sql}"
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
             FROM {tbl} WHERE {qc} IS NOT NULL{and_sql} GROUP BY b ORDER BY b"
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
fn histogram_buckets(
    db: &str,
    tbl: &str,
    q: &str,
    lo: f64,
    hi: f64,
    and_sql: &str,
) -> Result<Vec<(i64, i64)>> {
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
             count(*) AS c FROM {tbl} WHERE {q} IS NOT NULL{and_sql} GROUP BY b ORDER BY b"
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
            contains: false,
            is_null: false,
            negate: false,
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
            contains: false,
            is_null: false,
            negate: false,
        }
    }
    /// An IS NULL filter.
    fn nf(column: &str) -> Filter {
        Filter {
            column: column.into(),
            value: None,
            min: None,
            max: None,
            tmin: None,
            tmax: None,
            contains: false,
            is_null: true,
            negate: false,
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
            contains: false,
            is_null: false,
            negate: false,
        }
    }

    /// A negated (NOT) value filter.
    fn nvf(column: &str, value: &str) -> Filter {
        Filter {
            negate: true,
            ..vf(column, value)
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
    fn nested_types_are_not_numeric_or_temporal() {
        // DOUBLE[] contains "DOUBLE" and DATE[] contains "DATE", but list /
        // struct / map columns can't take avg()/histograms/date_trunc — they
        // must classify as plain "other" (stats then falls back to top values).
        for t in [
            "DOUBLE[]",
            "INTEGER[]",
            "DECIMAL(10,2)[]",
            "STRUCT(a DOUBLE, b INTEGER)",
            "MAP(VARCHAR, BIGINT)",
            "STRUCT(speed INTEGER, \"zone\" VARCHAR)[]",
            "UNION(n INTEGER, s VARCHAR)",
        ] {
            assert!(!is_numeric(t), "{t} should not be numeric");
            assert!(is_nested_type(t), "{t} should be nested");
        }
        for t in [
            "DATE[]",
            "TIMESTAMP[]",
            "STRUCT(d DATE)",
            "MAP(DATE, VARCHAR)",
        ] {
            assert!(!is_temporal(t), "{t} should not be temporal");
        }
        // The plain forms keep their classification.
        assert!(is_numeric("DOUBLE") && !is_nested_type("DOUBLE"));
        assert!(is_temporal("DATE") && is_temporal("TIMESTAMP WITH TIME ZONE"));
    }

    #[test]
    fn build_where_contains_filters_use_list_containment() {
        let cols = vec!["port_gbps".to_string(), "zone".to_string()];
        let mut f = vf("port_gbps", "10.0");
        f.contains = true;
        assert_eq!(
            build_where(&cols, None, &[f.clone()]).unwrap(),
            "list_contains(CAST(\"port_gbps\" AS VARCHAR[]), '10.0')"
        );
        // Two elements on one column OR together; an exact filter on another
        // column still ANDs alongside.
        let mut f2 = vf("port_gbps", "400.0");
        f2.contains = true;
        let w = build_where(&cols, None, &[f, f2, vf("zone", "red")]).unwrap();
        assert_eq!(
            w,
            "(list_contains(CAST(\"port_gbps\" AS VARCHAR[]), '10.0') OR \
             list_contains(CAST(\"port_gbps\" AS VARCHAR[]), '400.0')) AND \
             CAST(\"zone\" AS VARCHAR) = 'red'"
        );
    }

    #[test]
    fn normalize_contains_infers_containment_from_schema() {
        // The shape an old link carries: a plain value filter on a list column.
        let cols = vec![
            ("port_gbps".to_string(), "DOUBLE[]".to_string()),
            ("country".to_string(), "VARCHAR".to_string()),
        ];
        let out = normalize_contains(&cols, &[vf("port_gbps", "10.0"), vf("country", "Japan")]);
        assert!(
            out[0].contains,
            "list-column filter must become containment"
        );
        assert!(!out[1].contains, "plain column filter stays equality");

        // Regression: a *whole-array* value (from the stats per-column
        // top-values, e.g. clicking "[red, blue]") means equality on the whole
        // list — it must NOT be forced to containment (which matched nothing and
        // collapsed the stats page). The equality path compares CAST(col AS
        // VARCHAR) to the literal, which does match.
        let whole = normalize_contains(&cols, &[vf("port_gbps", "[1.0, 10.0]")]);
        assert!(
            !whole[0].contains,
            "whole-array value stays equality, not containment"
        );
        let w = build_where(
            &["port_gbps".to_string()],
            None,
            &normalize_contains(&cols, &[vf("port_gbps", "[1.0, 10.0]")]),
        )
        .unwrap();
        assert_eq!(w, "CAST(\"port_gbps\" AS VARCHAR) = '[1.0, 10.0]'");
    }

    #[test]
    fn scalar_list_types_are_detected() {
        assert!(is_scalar_list_type("DOUBLE[]"));
        assert!(is_scalar_list_type("VARCHAR[]"));
        assert!(!is_scalar_list_type("STRUCT(a INTEGER)[]"));
        assert!(!is_scalar_list_type("INTEGER[][]"));
        assert!(!is_scalar_list_type("DOUBLE"));
        assert!(!is_scalar_list_type("MAP(VARCHAR, INTEGER)"));
    }

    #[test]
    fn json_safe_projection_wraps_only_nested_columns() {
        let cols = vec![
            ("id".to_string(), "BIGINT".to_string()),
            ("nums".to_string(), "DOUBLE[]".to_string()),
            ("st".to_string(), "STRUCT(k VARCHAR)".to_string()),
        ];
        assert_eq!(
            json_safe_projection(&cols),
            "\"id\", to_json(\"nums\") AS \"nums\", to_json(\"st\") AS \"st\""
        );
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
    fn build_where_is_null_filter() {
        let cols = vec!["state".to_string()];
        assert_eq!(
            build_where(&cols, None, &[nf("state")]).unwrap(),
            "\"state\" IS NULL"
        );
    }

    #[test]
    fn build_where_is_null_ors_with_same_column_values() {
        // Facet semantics: choices within one column OR together, so
        // "QLD or missing" reads as one grouped clause.
        let cols = vec!["state".to_string()];
        assert_eq!(
            build_where(&cols, None, &[vf("state", "QLD"), nf("state")]).unwrap(),
            "(CAST(\"state\" AS VARCHAR) = 'QLD' OR \"state\" IS NULL)"
        );
    }

    #[test]
    fn build_where_is_null_ands_across_columns() {
        let cols = vec!["state".to_string(), "market".to_string()];
        assert_eq!(
            build_where(&cols, None, &[nf("state"), vf("market", "US")]).unwrap(),
            "\"state\" IS NULL AND CAST(\"market\" AS VARCHAR) = 'US'"
        );
    }

    #[test]
    fn build_where_negated_value_uses_is_distinct_from() {
        // NOT filter excludes the value while keeping NULLs and other values.
        let cols = vec!["species".to_string()];
        assert_eq!(
            build_where(&cols, None, &[nvf("species", "coot")]).unwrap(),
            "CAST(\"species\" AS VARCHAR) IS DISTINCT FROM 'coot'"
        );
    }

    #[test]
    fn build_where_negated_is_null_is_not_null() {
        let cols = vec!["state".to_string()];
        let f = Filter {
            negate: true,
            ..nf("state")
        };
        assert_eq!(
            build_where(&cols, None, &[f]).unwrap(),
            "\"state\" IS NOT NULL"
        );
    }

    #[test]
    fn build_where_negated_contains_wraps_in_not() {
        let cols = vec!["port_gbps".to_string()];
        let mut f = nvf("port_gbps", "10.0");
        f.contains = true;
        assert_eq!(
            build_where(&cols, None, &[f]).unwrap(),
            "NOT list_contains(CAST(\"port_gbps\" AS VARCHAR[]), '10.0')"
        );
    }

    #[test]
    fn build_where_mixed_positive_and_negated_same_column_ands() {
        // A `=` and a `≠` on the same column are distinct filters: the positive
        // OR-group ANDs with the negated clause.
        let cols = vec!["species".to_string()];
        assert_eq!(
            build_where(
                &cols,
                None,
                &[vf("species", "coot"), nvf("species", "teal")]
            )
            .unwrap(),
            "CAST(\"species\" AS VARCHAR) = 'coot' AND \
             CAST(\"species\" AS VARCHAR) IS DISTINCT FROM 'teal'"
        );
        // Two negations on one column exclude both (AND).
        assert_eq!(
            build_where(
                &cols,
                None,
                &[nvf("species", "coot"), nvf("species", "teal")]
            )
            .unwrap(),
            "CAST(\"species\" AS VARCHAR) IS DISTINCT FROM 'coot' AND \
             CAST(\"species\" AS VARCHAR) IS DISTINCT FROM 'teal'"
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
        // A bare-date tmax covers that whole day (exclusive next-day bound),
        // so timestamp columns keep their last day's 00:01–23:59 rows.
        assert_eq!(
            build_where(
                &cols,
                None,
                &[tf("seen", Some("2026-01-01"), Some("2026-02-01"))]
            )
            .unwrap(),
            "(\"seen\" >= '2026-01-01' AND \"seen\" < DATE '2026-02-01' + INTERVAL 1 DAY)"
        );
        // A timestamp tmax is already an exclusive boundary.
        assert_eq!(
            build_where(
                &cols,
                None,
                &[tf("seen", None, Some("2026-01-31 14:00:00"))]
            )
            .unwrap(),
            "(\"seen\" < '2026-01-31 14:00:00')"
        );
    }

    #[test]
    fn bare_date_detection() {
        assert!(is_bare_date("2026-06-30"));
        assert!(!is_bare_date("2026-06-30 14:00:00"));
        assert!(!is_bare_date("2026-6-30"));
        assert!(!is_bare_date("30/06/2026"));
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

    #[test]
    fn json_f64_reads_numbers_and_quoted_decimals() {
        // DuckDB renders DOUBLE/BIGINT as JSON numbers but DECIMAL/HUGEINT as
        // quoted strings — both must coerce to f64.
        assert_eq!(json_f64(&Value::from(185.0)), Some(185.0));
        assert_eq!(json_f64(&Value::from(9)), Some(9.0));
        assert_eq!(json_f64(&Value::from("185.0")), Some(185.0));
        assert_eq!(json_f64(&Value::from("-3.5")), Some(-3.5));
        assert_eq!(json_f64(&Value::Null), None);
        assert_eq!(json_f64(&Value::from("not a number")), None);
    }

    // ---- stats() integration (needs the `duckdb` CLI) --------------------
    fn duckdb_ok() -> bool {
        Command::new("duckdb")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    /// A unique temp db path; the file is created when duckdb first writes it.
    fn temp_db(tag: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("muckdb_test_{}_{tag}.duckdb", std::process::id()));
        let path = p.to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        path
    }
    fn run_sql(db: &str, sql: &str) {
        let out = Command::new("duckdb")
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
    fn col<'a>(s: &'a TableStats, name: &str) -> &'a ColumnStat {
        s.columns.iter().find(|c| c.name == name).expect("column")
    }

    /// A table with list / struct / map columns beside plain ones — the shape
    /// that used to break stats (avg on DOUBLE[]) and the -json row parse.
    fn make_nested_db(tag: &str) -> String {
        let db = temp_db(tag);
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT i AS id, \
             [i * 1.0, i + 1.0] AS nums, \
             {'k': 'v' || i::VARCHAR, 'n': i} AS st, \
             MAP {'red': i, 'blue': i + 1} AS mp, \
             [DATE '2026-01-01'] AS dts, \
             ('g' || (i % 3)::VARCHAR) AS grp \
             FROM range(10) g(i);",
        );
        db
    }

    #[test]
    fn stats_handles_nested_columns() {
        if !duckdb_ok() {
            eprintln!("skipping stats_handles_nested_columns: no duckdb");
            return;
        }
        let db = make_nested_db("nested_stats");
        let s = stats(&db, "t", None, &[]).expect("stats must not fail on nested columns");
        assert_eq!(s.row_count, 10);
        for name in ["nums", "st", "mp", "dts"] {
            let c = col(&s, name);
            assert!(!c.numeric, "{name} must not be numeric");
            assert!(!c.temporal, "{name} must not be temporal");
            assert!(c.avg.is_none(), "{name} must not carry numeric stats");
            assert!(!c.top.is_empty(), "{name} should still report top values");
        }
        // Plain columns keep full treatment alongside.
        assert!(col(&s, "id").numeric);
        assert!(col(&s, "id").avg.is_some());
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn preview_serializes_nested_values_as_json() {
        if !duckdb_ok() {
            eprintln!("skipping preview_serializes_nested_values_as_json: no duckdb");
            return;
        }
        let db = make_nested_db("nested_preview");
        let p = preview(&db, "t", 5, 0, None, &[], Some("id"), Some("asc"))
            .expect("preview must not fail on nested columns");
        assert_eq!(p.total, 10);
        let idx = |n: &str| p.columns.iter().position(|c| c == n).expect("column");
        let row = &p.rows[0];
        // Without the to_json() wrap these come back in duckdb literal syntax
        // ({red=0, blue=1}) and the whole payload fails to parse.
        assert!(row[idx("nums")].is_array(), "nums must be a JSON array");
        assert!(row[idx("st")].is_object(), "st must be a JSON object");
        assert!(row[idx("mp")].is_object(), "mp must be a JSON object");
        assert_eq!(row[idx("mp")].get("red").and_then(Value::as_i64), Some(0));
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn preview_filters_null_rows_via_is_null() {
        if !duckdb_ok() {
            eprintln!("skipping preview_filters_null_rows_via_is_null: no duckdb");
            return;
        }
        let db = temp_db("is_null");
        // state is NULL for i = 0, 3, 6 — three of nine rows.
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT i AS id, \
             CASE WHEN i % 3 = 0 THEN NULL ELSE 'v' || i::VARCHAR END AS state \
             FROM range(9) g(i);",
        );
        // Deserialize from JSON — the exact shape the web UI sends in `filter=`.
        let filters: Vec<Filter> =
            serde_json::from_str(r#"[{"column":"state","is_null":true}]"#).unwrap();
        let p = preview(&db, "t", 100, 0, None, &filters, None, None).unwrap();
        assert_eq!(p.total, 3);
        let state_idx = p.columns.iter().position(|c| c == "state").unwrap();
        assert!(p.rows.iter().all(|r| r[state_idx].is_null()));

        // A value filter on the same column ORs with the null filter.
        let filters: Vec<Filter> = serde_json::from_str(
            r#"[{"column":"state","value":"v1"},{"column":"state","is_null":true}]"#,
        )
        .unwrap();
        let p = preview(&db, "t", 100, 0, None, &filters, None, None).unwrap();
        assert_eq!(p.total, 4);
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn list_columns_facet_by_element_and_filter_by_containment() {
        if !duckdb_ok() {
            eprintln!(
                "skipping list_columns_facet_by_element_and_filter_by_containment: no duckdb"
            );
            return;
        }
        let db = make_nested_db("list_facet");
        // nums for row i is [i, i+1] — the element "1.0" appears in rows 0 and 1.
        let fs = facets(&db, "t", None, &[]).expect("facets must not fail on nested columns");
        let nums = fs
            .iter()
            .find_map(|f| match f {
                ColumnFacet::Values {
                    name, values, list, ..
                } if name == "nums" => Some((values.clone(), *list)),
                _ => None,
            })
            .expect("nums must get a values facet");
        assert!(nums.1, "nums facet must be flagged as a list facet");
        let one = nums
            .0
            .iter()
            .find(|v| v.value == "1.0")
            .expect("element 1.0");
        assert_eq!(one.count, 2, "rows [0,1] and [1,2] both contain 1.0");

        // Selecting the element filters rows by containment, not equality.
        let mut f = vf("nums", "1.0");
        f.contains = true;
        let p = preview(&db, "t", 10, 0, None, &[f], Some("id"), Some("asc")).unwrap();
        assert_eq!(p.total, 2);
        let idx = |n: &str| p.columns.iter().position(|c| c == n).unwrap();
        assert_eq!(p.rows[0][idx("id")].as_i64(), Some(0));
        assert_eq!(p.rows[1][idx("id")].as_i64(), Some(1));
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn query_rewraps_nested_result_columns() {
        if !duckdb_ok() {
            eprintln!("skipping query_rewraps_nested_result_columns: no duckdb");
            return;
        }
        let db = make_nested_db("nested_query");
        // Trailing semicolon exercises the DESCRIBE pre-pass trimming.
        let r = query(&db, "SELECT id, st, mp FROM t WHERE id = 3;")
            .expect("query must not fail on nested columns");
        assert_eq!(r.row_count, 1);
        let idx = |n: &str| r.columns.iter().position(|c| c == n).expect("column");
        assert!(r.rows[0][idx("st")].is_object());
        assert_eq!(
            r.rows[0][idx("mp")].get("blue").and_then(Value::as_i64),
            Some(4)
        );
        // A query with no nested columns runs untouched.
        let plain = query(&db, "SELECT count(*) AS n FROM t").unwrap();
        assert_eq!(plain.rows[0][0].as_i64(), Some(10));
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn stats_reports_exact_distinct_for_small_cardinality() {
        if !duckdb_ok() {
            eprintln!("skipping stats_reports_exact_distinct_for_small_cardinality: no duckdb");
            return;
        }
        let db = temp_db("distinct");
        // 37 groups is small enough for the exact recount but large enough that
        // the HyperLogLog approximation would not reliably land on 37.
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT i AS id, ('g' || (i % 37)::VARCHAR) AS grp, \
             (i * 1.0) AS amt FROM range(200) g(i);",
        );
        let s = stats(&db, "t", None, &[]).unwrap();
        assert_eq!(s.row_count, 200);
        assert_eq!(col(&s, "grp").distinct, 37, "grp distinct must be exact");
        assert_eq!(col(&s, "id").distinct, 200, "id distinct must be exact");
        std::fs::remove_file(&db).ok();
    }

    /// grp is NULL when i % 5 == 0 (i%20 ∈ {0,5,10,15}) → 20 null rows and
    /// 16 distinct non-null values (g1..g19 minus g5/g10/g15), 5 rows each.
    /// mkt splits rows evenly between 'A' (even i) and 'B'.
    fn make_nullable_db(tag: &str) -> String {
        let db = temp_db(tag);
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT i AS id, \
             CASE WHEN i % 5 = 0 THEN NULL ELSE 'g' || (i % 20)::VARCHAR END AS grp, \
             CASE WHEN i % 2 = 0 THEN 'A' ELSE 'B' END AS mkt \
             FROM range(100) g(i);",
        );
        db
    }

    #[test]
    fn values_facets_carry_null_and_total_counts() {
        if !duckdb_ok() {
            eprintln!("skipping values_facets_carry_null_and_total_counts: no duckdb");
            return;
        }
        let db = make_nullable_db("facet_nulls");
        let fs = facets(&db, "t", None, &[]).unwrap();
        let grp = fs
            .iter()
            .find_map(|f| match f {
                ColumnFacet::Values {
                    name,
                    values,
                    nulls,
                    has_nulls,
                    total,
                    ..
                } if name == "grp" => Some((values, *nulls, *has_nulls, *total)),
                _ => None,
            })
            .expect("grp values facet");
        assert_eq!(grp.1, 20, "20 rows have NULL grp");
        assert!(grp.2, "grp has nulls");
        assert_eq!(grp.3, 16, "16 distinct non-null values");
        assert_eq!(grp.0.len(), 12, "value list stays capped at TOP_VALUES");
        // A column without NULLs reports zero.
        let mkt = fs
            .iter()
            .find_map(|f| match f {
                ColumnFacet::Values {
                    name,
                    nulls,
                    has_nulls,
                    ..
                } if name == "mkt" => Some((*nulls, *has_nulls)),
                _ => None,
            })
            .expect("mkt values facet");
        assert_eq!(mkt, (0, false));
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn facets_keep_zero_count_values_visible() {
        if !duckdb_ok() {
            eprintln!("skipping facets_keep_zero_count_values_visible: no duckdb");
            return;
        }
        let db = make_nullable_db("facet_zero");
        // grp = 'g1' pins odd rows only, so every matching row has mkt = 'B'.
        // 'A' must still be offered — with a 0 count — not vanish.
        let fs = facets(&db, "t", None, &[vf("grp", "g1")]).unwrap();
        let mkt = fs
            .iter()
            .find_map(|f| match f {
                ColumnFacet::Values { name, values, .. } if name == "mkt" => Some(values),
                _ => None,
            })
            .expect("mkt values facet");
        assert_eq!(mkt.len(), 2, "both values stay visible");
        assert!(mkt.iter().any(|v| v.value == "B" && v.count == 5));
        assert!(mkt.iter().any(|v| v.value == "A" && v.count == 0));
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn boolean_columns_are_flagged_for_facets() {
        if !duckdb_ok() {
            eprintln!("skipping boolean_columns_are_flagged_for_facets: no duckdb");
            return;
        }
        let db = temp_db("facet_bool");
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT i AS id, (i % 3 = 0) AS flag, \
             ('g' || (i % 4)::VARCHAR) AS grp FROM range(40) g(i);",
        );
        let fs = facets(&db, "t", None, &[]).unwrap();
        let flag = fs.iter().find_map(|f| match f {
            ColumnFacet::Values { name, boolean, .. } if name == "flag" => Some(*boolean),
            _ => None,
        });
        let grp = fs.iter().find_map(|f| match f {
            ColumnFacet::Values { name, boolean, .. } if name == "grp" => Some(*boolean),
            _ => None,
        });
        assert_eq!(flag, Some(true), "boolean column is flagged");
        assert_eq!(grp, Some(false), "varchar column is not");
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn facet_values_lists_every_value_scoped_to_other_columns() {
        if !duckdb_ok() {
            eprintln!("skipping facet_values_lists_every_value_scoped_to_other_columns: no duckdb");
            return;
        }
        let db = make_nullable_db("facet_all");
        let all = facet_values(&db, "t", "grp", None, &[]).unwrap();
        assert_eq!(all.total, 16);
        assert_eq!(all.values.len(), 16, "every distinct value is listed");
        assert_eq!(all.nulls, 20);
        assert!(all.values.iter().all(|v| v.count == 5));

        // Filters on OTHER columns scope the counts; the column's own filters
        // don't (same semantics as the facet panel). Values the filter
        // excludes stay listed with a 0 count.
        let filters = vec![vf("mkt", "A"), vf("grp", "g1")];
        let scoped = facet_values(&db, "t", "grp", None, &filters).unwrap();
        assert_eq!(scoped.nulls, 10, "even-i rows only");
        assert!(scoped.has_nulls);
        assert_eq!(scoped.values.len(), 16, "zero-count values stay listed");
        assert!(
            scoped
                .values
                .iter()
                .any(|v| v.value == "g2" && v.count == 5),
            "even groups keep their 5 rows under mkt=A"
        );
        assert!(
            scoped
                .values
                .iter()
                .any(|v| v.value == "g1" && v.count == 0),
            "odd groups show a 0 count under mkt=A"
        );
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn stats_are_scoped_to_active_filters() {
        if !duckdb_ok() {
            eprintln!("skipping stats_are_scoped_to_active_filters: no duckdb");
            return;
        }
        let db = temp_db("filtered");
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT i AS id, ('g' || (i % 37)::VARCHAR) AS grp, \
             (i * 1.0) AS amt FROM range(200) g(i);",
        );
        // grp = 'g0' selects i in {0,37,74,111,148,185} → 6 rows, max amt 185.
        let f = vf("grp", "g0");
        let s = stats(&db, "t", None, std::slice::from_ref(&f)).unwrap();
        assert_eq!(s.row_count, 6, "row count reflects the filter");
        assert_eq!(col(&s, "grp").distinct, 1, "filtered grp has one value");
        assert_eq!(col(&s, "amt").max, Some(185.0), "numeric range is filtered");
        std::fs::remove_file(&db).ok();
    }
}
