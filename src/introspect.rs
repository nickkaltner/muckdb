//! Reading database contents by shelling out to `duckdb -json`, staying true to
//! the "facade over the duckdb CLI" design rather than linking libduckdb.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;
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

/// Preview the first `limit` rows of a table.
pub fn preview(db: &str, table: &str, limit: u32) -> Result<Preview> {
    if !Path::new(db).exists() {
        bail!("database file does not exist");
    }
    let sql = format!("SELECT * FROM {} LIMIT {}", quote_ident(table), limit);
    let rows = query_json(db, &sql)?;

    // Column order comes from the first row (serde_json preserves insertion
    // order, and duckdb -json emits columns in SELECT order). Fall back to
    // DESCRIBE when the table is empty.
    let columns: Vec<String> = if let Some(Value::Object(map)) = rows.first() {
        map.keys().cloned().collect()
    } else {
        query_json(db, &format!("DESCRIBE {}", quote_ident(table)))?
            .into_iter()
            .filter_map(|r| {
                r.get("column_name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    };

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
