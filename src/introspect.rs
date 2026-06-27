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
}

/// A bounded preview of a table's rows, with columns in table order.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Run a read-only query against `db` and parse `duckdb -json` output into rows.
fn query_json(db: &str, sql: &str) -> Result<Vec<Value>> {
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

/// One facet filter: a column constrained to an exact (string-compared) value.
#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    pub column: String,
    pub value: String,
}

/// Build a SQL `WHERE` body (without the `WHERE` keyword) from a free-text
/// search and a set of facet filters, or `None` when neither is present.
///
/// `q` matches any column cast to text (case-insensitive); each filter pins a
/// column to a value compared as text. Pure and side-effect free so it can be
/// unit-tested without duckdb.
pub fn build_where(columns: &[String], q: Option<&str>, filters: &[Filter]) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();

    for f in filters {
        clauses.push(format!(
            "CAST({} AS VARCHAR) = '{}'",
            quote_ident(&f.column),
            escape_literal(&f.value)
        ));
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
    pub nulls: i64,
    pub distinct: i64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub avg: Option<f64>,
    pub histogram: Vec<HistBin>,
    pub top: Vec<TopValue>,
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
        "SELECT schema_name, table_name, column_count, estimated_size \
         FROM duckdb_tables() ORDER BY schema_name, table_name",
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
                .get("table_name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            column_count: r.get("column_count").and_then(Value::as_i64).unwrap_or(0),
            estimated_size: r.get("estimated_size").and_then(Value::as_i64),
        })
        .collect();
    Ok(tables)
}

/// Preview up to `limit` rows of a table, optionally filtered by a free-text
/// search and facet filters.
pub fn preview(
    db: &str,
    table: &str,
    limit: u32,
    q: Option<&str>,
    filters: &[Filter],
) -> Result<Preview> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    // Column order is authoritative from DESCRIBE (also drives text search and
    // the empty-result case).
    let columns: Vec<String> = describe(db, table)?.into_iter().map(|(n, _)| n).collect();

    let where_sql = build_where(&columns, q, filters)
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();
    let sql = format!(
        "SELECT * FROM {}{} LIMIT {}",
        quote_ident(table),
        where_sql,
        limit
    );
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
    })
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
                ", min({q}) AS mn{i}, max({q}) AS mx{i}, avg({q}) AS av{i}"
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
        let (min, max, avg) = if numeric {
            (
                getf(&format!("mn{i}")),
                getf(&format!("mx{i}")),
                getf(&format!("av{i}")),
            )
        } else {
            (None, None, None)
        };

        let mut histogram = Vec::new();
        let mut top = Vec::new();
        let q = quote_ident(name);

        if numeric {
            if let (Some(lo), Some(hi)) = (min, max) {
                let raw = histogram_buckets(db, &tbl, &q, lo, hi)?;
                histogram = bucketize(lo, hi, HIST_BINS, &raw);
            }
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
            nulls,
            distinct,
            min,
            max,
            avg,
            histogram,
            top,
        });
    }

    Ok(TableStats { row_count, columns })
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
        let filters = vec![Filter {
            column: "name".into(),
            value: "O'Hare".into(),
        }];
        assert_eq!(
            build_where(&cols, None, &filters).unwrap(),
            "CAST(\"name\" AS VARCHAR) = 'O''Hare'"
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
        let filters = vec![Filter {
            column: "a".into(),
            value: "1".into(),
        }];
        let w = build_where(&cols, Some("z"), &filters).unwrap();
        assert_eq!(
            w,
            "CAST(\"a\" AS VARCHAR) = '1' AND (CAST(\"a\" AS VARCHAR) ILIKE '%z%')"
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
