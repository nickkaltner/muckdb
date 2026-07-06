//! The "predict" subview backend: for every ordered pair of usable columns
//! (X, Y), a score in [0, 1] for how well knowing X predicts Y — Spearman ρ²
//! for numeric pairs, Theil's uncertainty coefficient for categorical targets,
//! and the adjusted correlation ratio η² for category → number. Directional by
//! design: U(country | metro) ≈ 1 while U(metro | country) is much lower.
//!
//! muckdb holds no duckdb connection — every query is one `duckdb` process —
//! so the whole computation is a single invocation: `SET threads TO 1`, a temp
//! sample table with positional `c0..cN` aliases, then the pair scores in
//! UNION ALL batches. Numeric columns with distinct ≤ 10 (codes, flags) stay
//! numeric: Spearman handles ordinal/binary fine.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::introspect::{
    Filter, build_where, describe, is_numeric, is_temporal, json_f64, quote_ident,
};

/// Usable columns beyond this are excluded (`column_cap`) — 20 columns is
/// already 380 directed pairs.
const MAX_COLUMNS: usize = 20;
/// Tables larger than this are reservoir-sampled to exactly this many rows.
const SAMPLE_ROWS: i64 = 50_000;
/// Fixed seed so the same table gives the same sample (with threads=1).
const SAMPLE_SEED: u32 = 42;
/// Pairs with fewer both-non-null rows than this are dropped as noise.
const MIN_PAIR_N: i64 = 30;
/// Hard ceiling on a categorical column's distinct values. What makes a text
/// column useless isn't width, it's uniqueness — see CAT_UNIQUE_RATIO — so
/// this is just a sanity cap for very wide tables.
const MAX_CAT_DISTINCT: i64 = 500;
/// A text column whose values mostly don't repeat (distinct/non-null above
/// this) is a label, not a category — grouping by it is meaningless. A wide
/// but genuine category (164 operators over 610 rows) stays in; the weak flag
/// covers its statistical shakiness.
const CAT_UNIQUE_RATIO: f64 = 0.5;
/// Integer columns whose distinct/non-null ratio exceeds this are id-like:
/// grouping by them "predicts" everything trivially. Floats are exempt — a
/// continuous measure is *supposed* to be nearly unique.
const ID_RATIO: f64 = 0.95;
/// Belt-and-braces ceiling on |X| × |Y| for grouped measures — statistical
/// garbage territory more than a performance limit; weak-flagging handles the
/// grey zone below it.
const MAX_JOINT_CELLS: i64 = 50_000;
/// UNION ALL branches per SELECT statement — keeps statements bounded and
/// makes each completed batch an independently parseable result set.
const PAIRS_PER_BATCH: usize = 60;
/// Wall-clock budget for the scoring process; on expiry we keep what finished.
const BUDGET_MS: u64 = 10_000;
/// Quantile bins when a numeric predictor targets a category.
const BINS: i64 = 10;
/// A grouped score with n below 20× the predictor's group count is flagged
/// weak (entropy/variance estimates inflate at small n per group).
const WEAK_N_PER_GROUP: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Num,
    Cat,
    Excluded,
}

#[derive(Debug, Serialize)]
pub struct PredictColumn {
    pub name: String,
    pub col_type: String,
    pub role: Role,
    pub distinct: i64,
    pub nulls: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Pair {
    pub x: String,
    pub y: String,
    pub score: f64,
    pub method: &'static str,
    pub n: i64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub weak: bool,
}

#[derive(Debug, Serialize)]
pub struct Prediction {
    pub row_count: i64,
    pub sampled: bool,
    pub sample_n: i64,
    pub truncated: bool,
    pub columns: Vec<PredictColumn>,
    pub pairs: Vec<Pair>,
}

/// A usable column's slot in the sample table (`c<idx>`).
#[derive(Debug, Clone)]
struct Usable {
    idx: usize,
    name: String,
    role: Role,
    temporal: bool,
    distinct: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Method {
    /// Symmetric — computed once per unordered pair, mirrored on collect.
    SpearmanSq,
    TheilU,
    EtaSq,
    BinnedTheilU,
}

impl Method {
    fn label(self) -> &'static str {
        match self {
            Method::SpearmanSq => "spearman_sq",
            Method::TheilU => "theil_u",
            Method::EtaSq => "eta_sq",
            Method::BinnedTheilU => "binned_theil_u",
        }
    }
}

#[derive(Debug, Clone)]
struct PairPlan {
    x: usize, // index into the usable vec == c-alias number
    y: usize,
    method: Method,
    /// Predictor group count for the weak-n heuristic (0 = not grouped).
    groups: i64,
}

/// Column classification carried out of the profiling pass.
struct ColProfile {
    name: String,
    col_type: String,
    supported: bool,
    duplicate: bool,
    non_null: i64,
    distinct: i64,
}

/// Compute the pairwise prediction matrix for a table, honouring the same
/// search/filter scoping as the stats view.
pub fn predict(db: &str, table: &str, q: Option<&str>, filters: &[Filter]) -> Result<Prediction> {
    if !Path::new(db).exists() {
        bail!("database file does not exist: {db}");
    }
    let cols = describe(db, table)?;
    let col_names: Vec<String> = cols.iter().map(|(n, _)| n.clone()).collect();
    let filters = crate::introspect::normalize_contains(&cols, filters);
    let where_cond = build_where(&col_names, q, &filters);

    let (row_count, profiles) = profile(db, table, &cols, where_cond.as_deref())?;
    let mut columns = assign_roles(&profiles, row_count);

    let usable = usable_columns(&columns, &cols);
    let plans = plan_pairs(&usable);
    let sampled = row_count > SAMPLE_ROWS;
    let sample_n = if sampled { SAMPLE_ROWS } else { row_count };

    if plans.is_empty() || row_count == 0 {
        columns.sort_by_key(|c| c.role == Role::Excluded);
        return Ok(Prediction {
            row_count,
            sampled,
            sample_n,
            truncated: false,
            columns,
            pairs: Vec::new(),
        });
    }

    let ctas = sample_ctas_sql(table, &usable, where_cond.as_deref(), sampled);
    let branches: Vec<String> = plans.iter().map(pair_select_sql).collect();
    let script = build_script(&ctas, &branches);
    let (rows, truncated) = run_script(db, &script, Duration::from_millis(BUDGET_MS))?;
    let pairs = collect_pairs(&rows, &usable, &plans);

    columns.sort_by_key(|c| c.role == Role::Excluded);
    Ok(Prediction {
        row_count,
        sampled,
        sample_n,
        truncated,
        columns,
        pairs,
    })
}

/// One aggregate pass over the (filtered) table: row count plus per-column
/// non-null and distinct counts, with an exact recount where the HLL estimate
/// is small — the same shape as the stats profiling pass.
fn profile(
    db: &str,
    table: &str,
    cols: &[(String, String)],
    where_cond: Option<&str>,
) -> Result<(i64, Vec<ColProfile>)> {
    let tbl = quote_ident(table);
    let where_sql = where_cond
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();

    let mut seen = std::collections::HashSet::new();
    let mut profiles: Vec<ColProfile> = cols
        .iter()
        .map(|(name, ty)| ColProfile {
            name: name.clone(),
            col_type: ty.clone(),
            supported: type_supported(ty),
            duplicate: !seen.insert(name.clone()),
            non_null: 0,
            distinct: 0,
        })
        .collect();

    let mut sel = String::from("SELECT count(*) AS n");
    for (i, p) in profiles.iter().enumerate() {
        if p.supported && !p.duplicate {
            let q = quote_ident(&p.name);
            sel.push_str(&format!(
                ", count({q}) AS nn{i}, approx_count_distinct({q}) AS nd{i}"
            ));
        }
    }
    sel.push_str(&format!(" FROM {tbl}{where_sql}"));
    let agg = crate::introspect::query_json(db, &sel)?;
    let agg = agg.first().cloned().unwrap_or(Value::Null);
    let geti = |k: &str| {
        agg.get(k)
            .and_then(|v| v.as_i64().or_else(|| json_f64(v).map(|f| f as i64)))
            .unwrap_or(0)
    };
    let row_count = geti("n");
    for (i, p) in profiles.iter_mut().enumerate() {
        if p.supported && !p.duplicate {
            p.non_null = geti(&format!("nn{i}"));
            p.distinct = geti(&format!("nd{i}"));
        }
    }

    // Exact recount for low-cardinality columns: the HLL estimate can be off
    // by a few, and the cat/constant thresholds want exact numbers.
    let small: Vec<usize> = profiles
        .iter()
        .enumerate()
        .filter(|(_, p)| p.supported && !p.duplicate && p.distinct <= MAX_CAT_DISTINCT * 2)
        .map(|(i, _)| i)
        .collect();
    if !small.is_empty() {
        let proj = small
            .iter()
            .map(|&i| format!("count(DISTINCT {}) AS d{i}", quote_ident(&profiles[i].name)))
            .collect::<Vec<_>>()
            .join(", ");
        let exact =
            crate::introspect::query_json(db, &format!("SELECT {proj} FROM {tbl}{where_sql}"))?;
        if let Some(row) = exact.first() {
            for &i in &small {
                if let Some(d) = row
                    .get(format!("d{i}"))
                    .and_then(|v| v.as_i64().or_else(|| json_f64(v).map(|f| f as i64)))
                {
                    profiles[i].distinct = d;
                }
            }
        }
    }
    Ok((row_count, profiles))
}

/// Numeric and temporal types plus VARCHAR-ish ones participate; TIME,
/// INTERVAL, BLOB and nested types don't.
fn type_supported(ty: &str) -> bool {
    let t = ty.to_ascii_uppercase();
    if t.contains("STRUCT") || t.contains("LIST") || t.contains("MAP") || t.contains('[') {
        return false;
    }
    if is_numeric(&t) {
        return true;
    }
    if t.contains("TIMESTAMP") || t.contains("DATE") {
        return true;
    }
    if t.contains("TIME") || t.contains("INTERVAL") || t.contains("BLOB") {
        return false;
    }
    t.contains("VARCHAR") || t.contains("BOOL") || t.contains("ENUM") || t.contains("UUID")
}

/// Integer-family types (id-like exclusion applies to these, not to floats —
/// a continuous measure is naturally near-unique).
fn is_integer_type(ty: &str) -> bool {
    let t = ty.to_ascii_uppercase();
    !t.contains("INTERVAL")
        && (t.contains("INT") && !t.contains("POINT"))
        && !t.contains("DOUBLE")
        && !t.contains("FLOAT")
        && !t.contains("REAL")
        && !t.contains("DEC")
        && !t.contains("NUMERIC")
}

/// The exclusion ladder, in order; survivors get Num or Cat.
fn assign_roles(profiles: &[ColProfile], row_count: i64) -> Vec<PredictColumn> {
    let mut out = Vec::with_capacity(profiles.len());
    let mut kept = 0usize;
    for p in profiles {
        let nulls = row_count - p.non_null;
        let excluded = |reason: &'static str| PredictColumn {
            name: p.name.clone(),
            col_type: p.col_type.clone(),
            role: Role::Excluded,
            distinct: p.distinct,
            nulls,
            reason: Some(reason),
        };
        let col = if !p.supported {
            excluded("unsupported_type")
        } else if p.duplicate {
            excluded("duplicate_name")
        } else if p.non_null == 0 {
            excluded("all_null")
        } else if p.distinct <= 1 {
            excluded("constant")
        } else {
            let numeric = is_numeric(&p.col_type);
            let temporal = !numeric && is_temporal(&p.col_type);
            let id_like = is_integer_type(&p.col_type)
                && p.non_null > 0
                && (p.distinct as f64 / p.non_null as f64) > ID_RATIO;
            if id_like {
                excluded("id_like")
            } else if !numeric
                && !temporal
                && (p.distinct > MAX_CAT_DISTINCT
                    || (p.distinct as f64 / p.non_null as f64) > CAT_UNIQUE_RATIO)
            {
                excluded("high_cardinality")
            } else if kept >= MAX_COLUMNS {
                excluded("column_cap")
            } else {
                kept += 1;
                PredictColumn {
                    name: p.name.clone(),
                    col_type: p.col_type.clone(),
                    role: if numeric || temporal {
                        Role::Num
                    } else {
                        Role::Cat
                    },
                    distinct: p.distinct,
                    nulls,
                    reason: None,
                }
            }
        };
        out.push(col);
    }
    out
}

/// The usable columns in table order; their position is the `c<idx>` alias.
fn usable_columns(columns: &[PredictColumn], types: &[(String, String)]) -> Vec<Usable> {
    columns
        .iter()
        .filter(|c| c.role != Role::Excluded)
        .enumerate()
        .map(|(idx, c)| {
            let ty = types
                .iter()
                .find(|(n, _)| *n == c.name)
                .map(|(_, t)| t.as_str())
                .unwrap_or("");
            Usable {
                idx,
                name: c.name.clone(),
                role: c.role,
                temporal: !is_numeric(ty) && is_temporal(ty),
                distinct: c.distinct,
            }
        })
        .collect()
}

/// Every scoreable ordered pair. Spearman is symmetric, so num-num pairs are
/// planned once (x < y) and mirrored when collected.
fn plan_pairs(usable: &[Usable]) -> Vec<PairPlan> {
    let mut plans = Vec::new();
    for x in usable {
        for y in usable {
            if x.idx == y.idx {
                continue;
            }
            let plan = match (x.role, y.role) {
                (Role::Num, Role::Num) => {
                    if x.idx > y.idx {
                        continue; // mirrored from the (y, x) plan
                    }
                    PairPlan {
                        x: x.idx,
                        y: y.idx,
                        method: Method::SpearmanSq,
                        groups: 0,
                    }
                }
                (Role::Cat, Role::Cat) => {
                    if x.distinct * y.distinct > MAX_JOINT_CELLS {
                        continue;
                    }
                    PairPlan {
                        x: x.idx,
                        y: y.idx,
                        method: Method::TheilU,
                        groups: x.distinct,
                    }
                }
                (Role::Cat, Role::Num) => PairPlan {
                    x: x.idx,
                    y: y.idx,
                    method: Method::EtaSq,
                    groups: x.distinct,
                },
                (Role::Num, Role::Cat) => {
                    if BINS * y.distinct > MAX_JOINT_CELLS {
                        continue;
                    }
                    PairPlan {
                        x: x.idx,
                        y: y.idx,
                        method: Method::BinnedTheilU,
                        groups: BINS,
                    }
                }
                _ => continue,
            };
            plans.push(plan);
        }
    }
    plans
}

/// The temp sample table: usable columns projected once as `c0..cN` (so
/// identifier quoting and casts happen exactly once), filtered like the stats
/// view, reservoir-sampled only when the table is large.
fn sample_ctas_sql(
    table: &str,
    usable: &[Usable],
    where_cond: Option<&str>,
    sampled: bool,
) -> String {
    let proj = usable
        .iter()
        .map(|u| {
            let q = quote_ident(&u.name);
            let i = u.idx;
            match (u.role, u.temporal) {
                (Role::Num, true) => format!("epoch(CAST({q} AS TIMESTAMP)) AS c{i}"),
                (Role::Num, false) => format!("CAST({q} AS DOUBLE) AS c{i}"),
                _ => format!("CAST({q} AS VARCHAR) AS c{i}"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let where_sql = where_cond
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();
    let sample = if sampled {
        format!(" USING SAMPLE reservoir({SAMPLE_ROWS} ROWS) REPEATABLE ({SAMPLE_SEED})")
    } else {
        String::new()
    };
    format!(
        "CREATE TEMP TABLE __mp AS SELECT * FROM (SELECT {proj} FROM {} {where_sql}){sample}",
        quote_ident(table)
    )
}

/// One UNION ALL branch: (xi, yi, method, n, score). Every score expression is
/// NaN/Inf-guarded — duckdb's `-json` emits bare `nan`, which is invalid JSON
/// and would poison the whole result array.
fn pair_select_sql(plan: &PairPlan) -> String {
    let (x, y) = (plan.x, plan.y);
    let label = plan.method.label();
    match plan.method {
        Method::SpearmanSq => format!(
            "SELECT {x} AS xi, {y} AS yi, '{label}' AS method, count(*) AS n, \
             CASE WHEN isnan(pow(corr(rx, ry), 2)) OR isinf(pow(corr(rx, ry), 2)) THEN NULL \
                  ELSE least(1.0, pow(corr(rx, ry), 2)) END AS score \
             FROM (SELECT rank() OVER (ORDER BY c{x}) + (count(*) OVER (PARTITION BY c{x}) - 1) / 2.0 AS rx, \
                          rank() OVER (ORDER BY c{y}) + (count(*) OVER (PARTITION BY c{y}) - 1) / 2.0 AS ry \
                   FROM __mp WHERE c{x} IS NOT NULL AND c{y} IS NOT NULL) __r"
        ),
        Method::TheilU | Method::BinnedTheilU => {
            let d = if plan.method == Method::TheilU {
                format!(
                    "SELECT c{x} AS x, c{y} AS y FROM __mp \
                     WHERE c{x} IS NOT NULL AND c{y} IS NOT NULL"
                )
            } else {
                format!(
                    "SELECT least({top}, CAST(floor(percent_rank() OVER (ORDER BY c{x}) * {BINS}) AS INT)) AS x, \
                            c{y} AS y \
                     FROM __mp WHERE c{x} IS NOT NULL AND c{y} IS NOT NULL",
                    top = BINS - 1
                )
            };
            format!(
                "SELECT * FROM (WITH d AS ({d}), \
                 t AS (SELECT count(*) AS n, entropy(y) AS hy FROM d), \
                 g AS (SELECT count(*)::DOUBLE AS cn, entropy(y) AS hyx FROM d GROUP BY x) \
                 SELECT {x} AS xi, {y} AS yi, '{label}' AS method, t.n AS n, \
                        CASE WHEN t.hy > 0 \
                             THEN greatest(0.0, least(1.0, 1.0 - (SELECT sum(cn * hyx) FROM g) / t.n / t.hy)) \
                        END AS score \
                 FROM t) __s"
            )
        }
        Method::EtaSq => format!(
            "SELECT * FROM (WITH d AS (SELECT c{x} AS x, c{y} AS y FROM __mp \
                                       WHERE c{x} IS NOT NULL AND c{y} IS NOT NULL), \
             t AS (SELECT count(*)::DOUBLE AS n, avg(y) AS gm, var_pop(y) AS vt FROM d), \
             g AS (SELECT count(*)::DOUBLE AS cn, avg(y) AS m FROM d GROUP BY x), \
             k AS (SELECT count(*)::DOUBLE AS k FROM g) \
             SELECT {x} AS xi, {y} AS yi, '{label}' AS method, t.n::BIGINT AS n, \
                    CASE WHEN t.vt > 0 AND t.n > k.k \
                         THEN greatest(0.0, least(1.0, \
                              1.0 - (1.0 - (SELECT sum(cn * pow(m - t.gm, 2)) FROM g) / (t.n * t.vt)) \
                                    * (t.n - 1.0) / (t.n - k.k))) \
                    END AS score \
             FROM t, k) __s"
        ),
    }
}

/// The full script: deterministic single-threaded execution, the sample CTAS,
/// then the branches in UNION ALL batches (one statement per batch, so each
/// completed batch is one independently parseable JSON array on stdout).
fn build_script(ctas: &str, branches: &[String]) -> String {
    let mut script = String::from("SET threads TO 1;\n");
    script.push_str(ctas);
    script.push_str(";\n");
    for chunk in branches.chunks(PAIRS_PER_BATCH) {
        script.push_str(&chunk.join("\nUNION ALL\n"));
        script.push_str(";\n");
    }
    script
}

/// Run the script in one duckdb process with a wall-clock deadline. Stdout is
/// one JSON array per completed SELECT; parse leniently so a deadline kill
/// still yields every batch that finished (`truncated` reports the kill).
fn run_script(db: &str, script: &str, budget: Duration) -> Result<(Vec<Value>, bool)> {
    let mut child = Command::new("duckdb")
        .arg("-readonly")
        .arg("-json")
        .arg(db)
        .arg("-c")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run `duckdb` — is it installed and on PATH?")?;

    // Drain stdout on a thread so a large result can't deadlock the pipe.
    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout_pipe.read_to_string(&mut buf);
        buf
    });

    let deadline = Instant::now() + budget;
    let mut killed = false;
    let status = loop {
        if let Some(st) = child.try_wait().context("waiting for duckdb")? {
            break st;
        }
        if Instant::now() >= deadline {
            killed = true;
            let _ = child.kill();
            break child.wait().context("waiting for killed duckdb")?;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout = reader.join().unwrap_or_default();
    if !killed && !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        bail!("duckdb error: {}", err.trim());
    }

    // Each SELECT emitted one JSON array; a killed process may leave a
    // truncated tail — keep every array that parsed.
    let mut rows = Vec::new();
    let mut stream = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
    let mut clean_end = true;
    for item in &mut stream {
        match item {
            Ok(Value::Array(a)) => rows.extend(a),
            Ok(_) => {}
            Err(_) => {
                clean_end = false;
                break;
            }
        }
    }
    Ok((rows, killed || !clean_end))
}

/// Turn result rows back into named pairs: drop null/non-finite scores and
/// tiny-n pairs, mirror the symmetric Spearman rows, flag weak grouped scores.
fn collect_pairs(rows: &[Value], usable: &[Usable], plans: &[PairPlan]) -> Vec<Pair> {
    let geti = |v: &Value, k: &str| {
        v.get(k)
            .and_then(|x| x.as_i64().or_else(|| json_f64(x).map(|f| f as i64)))
    };
    let plan_for = |x: usize, y: usize| plans.iter().find(|p| p.x == x && p.y == y);

    let mut pairs = Vec::new();
    for row in rows {
        let (Some(xi), Some(yi)) = (geti(row, "xi"), geti(row, "yi")) else {
            continue;
        };
        let (xi, yi) = (xi as usize, yi as usize);
        let Some(score) = row.get("score").and_then(json_f64) else {
            continue;
        };
        if !score.is_finite() {
            continue;
        }
        let n = geti(row, "n").unwrap_or(0);
        if n < MIN_PAIR_N {
            continue;
        }
        let (Some(xc), Some(yc)) = (usable.get(xi), usable.get(yi)) else {
            continue;
        };
        let Some(plan) = plan_for(xi, yi) else {
            continue;
        };
        let weak = plan.groups > 0 && n < WEAK_N_PER_GROUP * plan.groups;
        pairs.push(Pair {
            x: xc.name.clone(),
            y: yc.name.clone(),
            score,
            method: plan.method.label(),
            n,
            weak,
        });
        if plan.method == Method::SpearmanSq {
            pairs.push(Pair {
                x: yc.name.clone(),
                y: xc.name.clone(),
                score,
                method: plan.method.label(),
                n,
                weak,
            });
        }
    }
    pairs.sort_by(|a, b| (&a.x, &a.y).cmp(&(&b.x, &b.y)));
    pairs
}

// ---- junk data: constants, all-NULLs, sparse columns, exact duplicates ----

/// A column's health metrics for the junk tab. Everything is computed on the
/// (sampled) VARCHAR-cast values — equality on strings is exactly what "these
/// two columns hold the same data" means for junk detection.
#[derive(Debug, Serialize)]
pub struct JunkColumn {
    pub name: String,
    pub col_type: String,
    pub non_null: i64,
    pub distinct: i64,
    /// The most frequent value and how many (sampled) rows hold it.
    pub top_value: Option<String>,
    pub top_count: i64,
}

/// Two columns whose values match on every (sampled) row.
#[derive(Debug, Serialize)]
pub struct JunkDuplicate {
    pub a: String,
    pub b: String,
}

#[derive(Debug, Serialize)]
pub struct JunkReport {
    pub row_count: i64,
    pub sampled: bool,
    pub sample_n: i64,
    pub columns: Vec<JunkColumn>,
    pub duplicates: Vec<JunkDuplicate>,
}

/// Anything castable to VARCHAR participates in junk analysis (wider than
/// prediction: TIME and high-cardinality text are still junk-checkable).
fn junk_supported(ty: &str) -> bool {
    let t = ty.to_ascii_uppercase();
    !(t.contains("STRUCT")
        || t.contains("LIST")
        || t.contains("MAP")
        || t.contains('[')
        || t.contains("BLOB"))
}

/// Column-health metrics + exact-duplicate detection for a table, honouring
/// the same search/filter scoping and sampling as the predict matrix.
pub fn junk(db: &str, table: &str, q: Option<&str>, filters: &[Filter]) -> Result<JunkReport> {
    if !Path::new(db).exists() {
        bail!("database file does not exist: {db}");
    }
    let cols = describe(db, table)?;
    let col_names: Vec<String> = cols.iter().map(|(n, _)| n.clone()).collect();
    let filters = crate::introspect::normalize_contains(&cols, filters);
    let where_cond = build_where(&col_names, q, &filters);
    let where_sql = where_cond
        .as_ref()
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();

    // First-occurrence, castable columns only (a view can expose duplicate
    // names; JSON keys would collide anyway).
    let mut seen = std::collections::HashSet::new();
    let usable: Vec<(String, String)> = cols
        .into_iter()
        .filter(|(n, t)| junk_supported(t) && seen.insert(n.clone()))
        .collect();

    let count = crate::introspect::query_json(
        db,
        &format!(
            "SELECT count(*) AS n FROM {}{where_sql}",
            quote_ident(table)
        ),
    )?;
    let row_count = count
        .first()
        .and_then(|r| r.get("n"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let sampled = row_count > SAMPLE_ROWS;
    let sample_n = if sampled { SAMPLE_ROWS } else { row_count };

    if usable.is_empty() || row_count == 0 {
        return Ok(JunkReport {
            row_count,
            sampled,
            sample_n,
            columns: Vec::new(),
            duplicates: Vec::new(),
        });
    }

    // One process: VARCHAR-cast sample table, then three result sets — basic
    // counts, top values, and pairwise equal-on-every-row counts.
    let proj = usable
        .iter()
        .enumerate()
        .map(|(i, (n, _))| format!("CAST({} AS VARCHAR) AS j{i}", quote_ident(n)))
        .collect::<Vec<_>>()
        .join(", ");
    let sample = if sampled {
        format!(" USING SAMPLE reservoir({SAMPLE_ROWS} ROWS) REPEATABLE ({SAMPLE_SEED})")
    } else {
        String::new()
    };
    let mut script = String::from("SET threads TO 1;\n");
    script.push_str(&format!(
        "CREATE TEMP TABLE __mj AS SELECT * FROM (SELECT {proj} FROM {}{where_sql}){sample};\n",
        quote_ident(table)
    ));
    // Exact distincts are affordable here — this runs on the ≤50k sample.
    let counts = (0..usable.len())
        .map(|i| format!("count(j{i}) AS nn{i}, count(DISTINCT j{i}) AS nd{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    script.push_str(&format!("SELECT count(*) AS n, {counts} FROM __mj;\n"));
    let tops = (0..usable.len())
        .map(|i| {
            format!(
                "(SELECT max(c) FROM (SELECT count(*) AS c FROM __mj WHERE j{i} IS NOT NULL GROUP BY j{i})) AS tc{i}, \
                 (SELECT j{i} FROM __mj WHERE j{i} IS NOT NULL GROUP BY j{i} ORDER BY count(*) DESC, j{i} LIMIT 1) AS tv{i}"
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    script.push_str(&format!("SELECT {tops} FROM (SELECT 1);\n"));
    let mut dup_keys = Vec::new();
    let dups = (0..usable.len())
        .flat_map(|i| (i + 1..usable.len()).map(move |j| (i, j)))
        .map(|(i, j)| {
            dup_keys.push((i, j));
            format!("count(*) FILTER (j{i} IS NOT DISTINCT FROM j{j}) AS eq{i}_{j}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    if !dups.is_empty() {
        script.push_str(&format!("SELECT {dups} FROM __mj;\n"));
    }

    let (rows, _truncated) = run_script(db, &script, Duration::from_millis(BUDGET_MS))?;
    let geti = |row: &Value, k: &str| {
        row.get(k)
            .and_then(|v| v.as_i64().or_else(|| json_f64(v).map(|f| f as i64)))
            .unwrap_or(0)
    };
    // Result rows arrive flattened in order: counts row, tops row, dups row.
    let counts_row = rows.first().cloned().unwrap_or(Value::Null);
    let tops_row = rows.get(1).cloned().unwrap_or(Value::Null);
    let dups_row = rows.get(2).cloned().unwrap_or(Value::Null);
    let n = geti(&counts_row, "n");

    let columns: Vec<JunkColumn> = usable
        .iter()
        .enumerate()
        .map(|(i, (name, ty))| JunkColumn {
            name: name.clone(),
            col_type: ty.clone(),
            non_null: geti(&counts_row, &format!("nn{i}")),
            distinct: geti(&counts_row, &format!("nd{i}")),
            top_value: tops_row
                .get(format!("tv{i}"))
                .and_then(Value::as_str)
                .map(String::from),
            top_count: geti(&tops_row, &format!("tc{i}")),
        })
        .collect();

    let duplicates = dup_keys
        .iter()
        .filter(|(i, j)| {
            // Equal on every sampled row (NULLs matching NULLs), and not two
            // all-NULL columns pretending to agree.
            n > 0
                && geti(&dups_row, &format!("eq{i}_{j}")) == n
                && (columns[*i].non_null > 0 || columns[*j].non_null > 0)
        })
        .map(|&(i, j)| JunkDuplicate {
            a: columns[i].name.clone(),
            b: columns[j].name.clone(),
        })
        .collect();

    Ok(JunkReport {
        row_count,
        sampled,
        sample_n,
        columns,
        duplicates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prof(name: &str, ty: &str, non_null: i64, distinct: i64) -> ColProfile {
        ColProfile {
            name: name.into(),
            col_type: ty.into(),
            supported: type_supported(ty),
            duplicate: false,
            non_null,
            distinct,
        }
    }

    #[test]
    fn roles_apply_the_exclusion_ladder() {
        let profiles = vec![
            prof("id", "BIGINT", 1000, 1000),     // id_like
            prof("amount", "DOUBLE", 1000, 990),  // stays num: floats exempt
            prof("grp", "VARCHAR", 1000, 12),     // cat
            prof("op", "VARCHAR", 1000, 164),     // cat: wide but values repeat
            prof("note", "VARCHAR", 1000, 800),   // high_cardinality (mostly unique)
            prof("flag", "BOOLEAN", 1000, 2),     // cat
            prof("blank", "VARCHAR", 0, 0),       // all_null
            prof("one", "VARCHAR", 1000, 1),      // constant
            prof("seen", "TIMESTAMP", 1000, 400), // num via epoch
            prof("blob", "BLOB", 1000, 5),        // unsupported_type
        ];
        let cols = assign_roles(&profiles, 1000);
        let by = |n: &str| cols.iter().find(|c| c.name == n).unwrap();
        assert_eq!(by("id").reason, Some("id_like"));
        assert_eq!(by("amount").role, Role::Num);
        assert_eq!(by("grp").role, Role::Cat);
        assert_eq!(by("op").role, Role::Cat, "wide genuine category stays in");
        assert_eq!(by("note").reason, Some("high_cardinality"));
        assert_eq!(by("flag").role, Role::Cat);
        assert_eq!(by("blank").reason, Some("all_null"));
        assert_eq!(by("one").reason, Some("constant"));
        assert_eq!(by("seen").role, Role::Num);
        assert_eq!(by("blob").reason, Some("unsupported_type"));
    }

    #[test]
    fn roles_cap_usable_columns() {
        let profiles: Vec<ColProfile> = (0..25)
            .map(|i| prof(&format!("c{i}"), "DOUBLE", 100, 90))
            .collect();
        let cols = assign_roles(&profiles, 100);
        let kept = cols.iter().filter(|c| c.role != Role::Excluded).count();
        let capped = cols
            .iter()
            .filter(|c| c.reason == Some("column_cap"))
            .count();
        assert_eq!(kept, MAX_COLUMNS);
        assert_eq!(capped, 5);
    }

    #[test]
    fn pair_plans_cover_all_role_combinations_once() {
        let usable = vec![
            Usable {
                idx: 0,
                name: "a".into(),
                role: Role::Num,
                temporal: false,
                distinct: 100,
            },
            Usable {
                idx: 1,
                name: "b".into(),
                role: Role::Num,
                temporal: false,
                distinct: 100,
            },
            Usable {
                idx: 2,
                name: "c".into(),
                role: Role::Cat,
                temporal: false,
                distinct: 5,
            },
        ];
        let plans = plan_pairs(&usable);
        let m = |x: usize, y: usize| {
            plans
                .iter()
                .find(|p| p.x == x && p.y == y)
                .map(|p| p.method)
        };
        assert_eq!(m(0, 1), Some(Method::SpearmanSq));
        assert_eq!(m(1, 0), None, "spearman planned once per unordered pair");
        assert_eq!(m(2, 0), Some(Method::EtaSq));
        assert_eq!(m(0, 2), Some(Method::BinnedTheilU));
        assert_eq!(plans.len(), 5); // a-b spearman, c→a, c→b eta, a→c, b→c binned
    }

    #[test]
    fn ctas_quotes_and_casts_once() {
        let usable = vec![
            Usable {
                idx: 0,
                name: "weird\"name".into(),
                role: Role::Num,
                temporal: false,
                distinct: 9,
            },
            Usable {
                idx: 1,
                name: "seen".into(),
                role: Role::Num,
                temporal: true,
                distinct: 9,
            },
            Usable {
                idx: 2,
                name: "grp".into(),
                role: Role::Cat,
                temporal: false,
                distinct: 3,
            },
        ];
        let sql = sample_ctas_sql("t", &usable, Some("\"grp\" = 'a'"), true);
        assert!(sql.contains("CAST(\"weird\"\"name\" AS DOUBLE) AS c0"));
        assert!(sql.contains("epoch(CAST(\"seen\" AS TIMESTAMP)) AS c1"));
        assert!(sql.contains("CAST(\"grp\" AS VARCHAR) AS c2"));
        assert!(sql.contains("WHERE \"grp\" = 'a'"));
        assert!(sql.contains(&format!(
            "USING SAMPLE reservoir({SAMPLE_ROWS} ROWS) REPEATABLE ({SAMPLE_SEED})"
        )));
        let unsampled = sample_ctas_sql("t", &usable, None, false);
        assert!(!unsampled.contains("USING SAMPLE"));
    }

    #[test]
    fn script_batches_branches_into_statements() {
        let branches: Vec<String> = (0..130).map(|i| format!("SELECT {i}")).collect();
        let script = build_script("CREATE TEMP TABLE __mp AS SELECT 1 AS c0", &branches);
        // 130 branches / 60 per batch = 3 statements + SET + CTAS.
        assert_eq!(script.matches(";\n").count(), 5);
        assert_eq!(script.matches("UNION ALL").count(), 127);
        assert!(script.starts_with("SET threads TO 1;\n"));
    }

    // ---- duckdb-backed integration tests (skipped when duckdb is absent) ---

    fn duckdb_ok() -> bool {
        Command::new("duckdb")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    fn temp_db(tag: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "muckdb_predict_{}_{tag}.duckdb",
            std::process::id()
        ));
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
            "setup sql failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn pair<'a>(p: &'a Prediction, x: &str, y: &str) -> &'a Pair {
        p.pairs
            .iter()
            .find(|q| q.x == x && q.y == y)
            .unwrap_or_else(|| panic!("pair {x}->{y} missing: {:?}", p.pairs))
    }

    #[test]
    fn predicts_planted_dependencies() {
        if !duckdb_ok() {
            eprintln!("skipping predicts_planted_dependencies: no duckdb");
            return;
        }
        let db = temp_db("planted");
        // grp determines bucket exactly; y is a monotone function of x;
        // noise is independent of everything (seeded hash, not random()).
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT \
               ('g' || (i % 5)::VARCHAR) AS grp, \
               ('b' || (i % 5)::VARCHAR) AS bucket, \
               (i * 1.0) AS x, \
               (i * i * 1.0) AS y, \
               (hash(i) % 1000) * 1.0 AS noise \
             FROM range(2000) r(i);",
        );
        let p = predict(&db, "t", None, &[]).unwrap();
        assert_eq!(p.row_count, 2000);
        assert!(!p.sampled);
        assert!(!p.truncated);
        // Deterministic mapping: U ≈ 1 in both directions.
        assert!(pair(&p, "grp", "bucket").score > 0.99);
        assert!(pair(&p, "bucket", "grp").score > 0.99);
        // Monotone numeric: Spearman ρ² ≈ 1, mirrored.
        assert!(pair(&p, "x", "y").score > 0.99);
        assert!(pair(&p, "y", "x").score > 0.99);
        assert_eq!(pair(&p, "x", "y").method, "spearman_sq");
        // Independent noise: near zero against the category.
        assert!(pair(&p, "grp", "noise").score < 0.05);
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn excludes_and_scopes_by_filter() {
        if !duckdb_ok() {
            eprintln!("skipping excludes_and_scopes_by_filter: no duckdb");
            return;
        }
        let db = temp_db("scoped");
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT \
               i AS id, \
               ('v' || i::VARCHAR) AS uniq_text, \
               'same' AS always, \
               ('g' || (i % 4)::VARCHAR) AS grp, \
               (i % 4) * 10.0 + (hash(i) % 10) AS val \
             FROM range(400) r(i);",
        );
        let p = predict(&db, "t", None, &[]).unwrap();
        let col = |n: &str| p.columns.iter().find(|c| c.name == n).unwrap();
        assert_eq!(col("id").reason, Some("id_like"));
        assert_eq!(col("uniq_text").reason, Some("high_cardinality"));
        assert_eq!(col("always").reason, Some("constant"));
        assert_eq!(col("grp").role, Role::Cat);
        // grp determines val's decile almost exactly → strong η².
        assert!(pair(&p, "grp", "val").score > 0.8);

        // A filter that pins grp to one value leaves too little signal — and
        // the scoped row count reflects the filter, like the stats view.
        let f = Filter {
            column: "grp".into(),
            value: Some("g0".into()),
            min: None,
            max: None,
            tmin: None,
            tmax: None,
            contains: false,
            is_null: false,
        };
        let scoped = predict(&db, "t", None, std::slice::from_ref(&f)).unwrap();
        assert_eq!(scoped.row_count, 100);
        let grp_col = scoped.columns.iter().find(|c| c.name == "grp").unwrap();
        assert_eq!(grp_col.reason, Some("constant"), "filtered grp is constant");
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn junk_finds_constants_nulls_sparse_and_duplicates() {
        if !duckdb_ok() {
            eprintln!("skipping junk_finds_constants_nulls_sparse_and_duplicates: no duckdb");
            return;
        }
        let db = temp_db("junk");
        run_sql(
            &db,
            "CREATE TABLE t AS SELECT \
               i AS id, \
               'fixed' AS always, \
               CAST(NULL AS VARCHAR) AS empty, \
               CASE WHEN i < 10 THEN 'rare' ELSE NULL END AS sparse, \
               ('g' || (i % 4)::VARCHAR) AS grp, \
               ('g' || (i % 4)::VARCHAR) AS grp_copy, \
               CASE WHEN i = 0 THEN 'odd' ELSE 'usual' END AS nearly \
             FROM range(200) r(i);",
        );
        let j = junk(&db, "t", None, &[]).unwrap();
        assert_eq!(j.row_count, 200);
        let col = |n: &str| j.columns.iter().find(|c| c.name == n).unwrap();
        assert_eq!(col("always").non_null, 200);
        assert_eq!(col("always").top_count, 200, "constant: one value fills it");
        assert_eq!(col("empty").non_null, 0, "all NULL");
        assert_eq!(col("sparse").non_null, 10, "sparse: 5% filled");
        assert_eq!(col("nearly").top_count, 199, "near-constant");
        assert_eq!(col("nearly").top_value.as_deref(), Some("usual"));
        assert_eq!(j.duplicates.len(), 1, "exactly one duplicate pair");
        assert_eq!(
            (j.duplicates[0].a.as_str(), j.duplicates[0].b.as_str()),
            ("grp", "grp_copy")
        );
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn survives_exotic_types_and_nulls() {
        if !duckdb_ok() {
            eprintln!("skipping survives_exotic_types_and_nulls: no duckdb");
            return;
        }
        let db = temp_db("exotic");
        run_sql(
            &db,
            "CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy'); \
             CREATE TABLE t AS SELECT \
               (i % 3)::INT AS code, \
               (CASE i % 3 WHEN 0 THEN 'sad' WHEN 1 THEN 'ok' ELSE 'happy' END)::mood AS m, \
               (TIMESTAMP '2026-01-01 00:00:00' + i * INTERVAL 1 HOUR)::TIMESTAMPTZ AS ts, \
               CASE WHEN i % 7 = 0 THEN NULL ELSE (i * 1.0) END AS holey, \
               [1, 2] AS arr \
             FROM range(300) r(i);",
        );
        let p = predict(&db, "t", None, &[]).unwrap();
        let col = |n: &str| p.columns.iter().find(|c| c.name == n).unwrap();
        assert_eq!(col("arr").reason, Some("unsupported_type"));
        assert_eq!(col("m").role, Role::Cat, "ENUM treated as categorical");
        assert_eq!(col("ts").role, Role::Num, "TIMESTAMPTZ becomes epoch");
        // code ↔ m is a bijection: binned-U and η² both near 1.
        assert!(pair(&p, "m", "code").score > 0.9);
        // NULL-y column still pairs (n = rows where both non-null).
        assert!(pair(&p, "ts", "holey").n >= 250);
        std::fs::remove_file(&db).ok();
    }
}
