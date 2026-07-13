# Timeline Tile Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `timeline` (Gantt-like) session chart kind that draws event rows as horizontal bars over a shared, auto-scaled time axis grouped into labelled lanes, with sublane stacking for overlaps, right-angle dependency connectors, `--event` markers, a mouse-following time cursor, a rich per-bar tooltip, and full-width support.

**Architecture:** Backend follows the existing `map` precedent — new optional `Chart` fields serialized straight into session JSON, wired in the `tile` CLI action, validated in `validate_tile`. Frontend is a hand-rolled renderer `timelineHtml()` in the single `src/assets/index.html`: bars are CSS-positioned by time-fraction (naturally horizontally responsive, like the `box` tile), and a per-plot SVG overlay (redrawn by a `ResizeObserver`, like the map's ASCII overlay) carries dependency lines, markers, and the hover cursor. Colours come from the theme palette; the shared `.wm-x`/`data-tip` delegated tooltip powers the rich hover.

**Tech Stack:** Rust (serde, anyhow) for CLI/model/validation; vanilla JS + hand-rolled SVG/CSS in `index.html` (no framework, no build step); Playwright for E2E; DuckDB SQL for demo/seed data.

## Global Constraints

- **Do NOT push or release** until the user explicitly gives the word. No `git push`, no `cargo release`, no tags.
- CI order that must stay green (per `AGENT.md`): `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo build` → `cargo test`. Always run `cargo fmt` (not `--check`), `cargo clippy --all-targets -- -D warnings`, and `cargo test` before each commit.
- The `Chart` struct has no `Default` derive: **every `Chart { … }` literal must list all fields**. New fields use `#[serde(default, skip_serializing_if = "Option::is_none")]` so other chart kinds omit them from JSON.
- Chart `kind` is a free-form `String`, not an enum — no new Rust variant is needed, but the frontend switch appears in **three** places that must all be updated: `loadTileChart`, `zoomTile`, `panelHtml`.
- New flag→field name mapping: `--depends-on` → `depends_on` (the parser keeps the hyphen in the key, so read `p.get("depends-on")`).
- Column formats/links must be honoured in tooltips via the existing `colFmt`/`applyFmtHtml`/`linkSubst` helpers.
- Every session tile in demo/seed must pass validation (real columns) and carry a `--caption`.

---

## File Structure

- `src/session.rs` — **modify**: `Chart` struct (add fields ~line 92), `tile` action constructor (~line 866), `validate_tile` (~line 692), help/usage string (~line 1008), chart-kind literal lists, unit tests (~line 1057+).
- `src/assets/index.html` — **modify**: new `timelineHtml()` renderer + overlay/tooltip helpers + CSS; dispatch in `loadTileChart` (~3435), `zoomTile` (~2972), `panelHtml` grip list (~3393) and widen gate (~3384).
- `tests/e2e/fixtures/seed.ts` — **modify**: add a timeline view + tile.
- `tests/e2e/specs/timeline.spec.ts` — **create**: DOM assertions.
- `demo.sh` — **modify**: timeline section + tiles + summary row.
- `src/assets/skill/SKILL.md` — **modify**: detailed `timeline` docs.
- `AGENT.md` — **modify**: mirror the timeline docs.

---

## Task 1: Backend data model + CLI wiring

**Files:**
- Modify: `src/session.rs` (Chart struct ~30-92; tile constructor ~866-887; existing test literals ~1058, ~1091)

**Interfaces:**
- Produces: `Chart` fields `lane`, `start`, `end`, `duration`, `color`, `id`, `depends_on` — all `Option<String>`. CLI flags `--lane`, `--start`, `--end`, `--duration`, `--color`, `--id`, `--depends-on`. These serialize to JSON keys `lane`/`start`/`end`/`duration`/`color`/`id`/`depends_on`, consumed by the frontend as `spec.<field>`.

- [ ] **Step 1: Add the new fields to the `Chart` struct.**

In `src/session.rs`, immediately after the `trend` field (the last field, ~line 91) and before the closing `}` of `pub struct Chart`, add:

```rust
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
```

- [ ] **Step 2: Wire the flags in the `tile` action constructor.**

In `src/session.rs`, inside the `Box::new(Chart { … })` literal (~866-887), after the `trend: p.get("trend").is_some(),` line, add:

```rust
                    lane: p.get("lane").map(str::to_string),
                    start: p.get("start").map(str::to_string),
                    end: p.get("end").map(str::to_string),
                    duration: p.get("duration").map(str::to_string),
                    color: p.get("color").map(str::to_string),
                    id: p.get("id").map(str::to_string),
                    depends_on: p.get("depends-on").map(str::to_string),
```

- [ ] **Step 3: Fix the two existing test `Chart` literals so the crate compiles.**

The `Chart { … }` literals in `heatmap_chart_serde_roundtrips_value_column` (~line 1058) and `map_chart_serde_roundtrips_lat_lon` (~line 1091) list every field. In **each**, add these lines just before the closing `}` of the literal (after `trend: false,`):

```rust
            lane: None,
            start: None,
            end: None,
            duration: None,
            color: None,
            id: None,
            depends_on: None,
```

- [ ] **Step 4: Build to confirm the crate compiles.**

Run: `cargo build`
Expected: builds clean (no "missing field" errors).

- [ ] **Step 5: Write a failing serde roundtrip test for the timeline chart.**

In `src/session.rs`, in the `#[cfg(test)] mod tests`, add a new test (place it after `map_chart_serde_roundtrips_lat_lon`, ~after line 1129):

```rust
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
```

- [ ] **Step 6: Run the test to verify it passes.**

Run: `cargo test timeline_chart_serde_roundtrips_fields`
Expected: PASS.

- [ ] **Step 7: Format, lint, full test, commit.**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
git add src/session.rs
git commit -m "feat(timeline): add Chart fields + CLI flags for timeline tile

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Backend validation + help text

**Files:**
- Modify: `src/session.rs` (`validate_tile` ~692; help/usage `~1008`; chart-kind list literals)

**Interfaces:**
- Consumes: `Chart` fields from Task 1; the existing `check(what, col)` closure and `cols`/`rows` from `DESCRIBE` in `validate_tile`.
- Produces: validation that a `timeline` tile has `--lane`, `--label`, `--start`, exactly one of `--end`/`--duration`, and that named columns exist (with "did you mean"); a type-consistency check that `--start` and `--end` are both numeric or both temporal.

- [ ] **Step 1: Write failing validation unit tests.**

In `src/session.rs` tests module, add a helper-backed test. `validate_tile` needs a real DuckDB file, so build one in a temp dir using the crate's own CLI path. Add:

```rust
    #[test]
    fn timeline_validation_requires_core_flags() {
        let dir = std::env::temp_dir().join(format!("muckdb-tl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("tl.duckdb");
        let dbs = db.to_str().unwrap();
        // Build a spans view with the columns a timeline uses.
        crate::introspect::query_json(
            dbs,
            "CREATE TABLE spans AS SELECT 'web' AS lane, 'deploy' AS task, \
             0.0 AS t0, 10.0 AS t1, 'ok' AS status, 'a' AS sid, NULL AS pids",
        )
        .ok();
        // Re-open via a real write since query_json is read-only; use a raw CLI run.
        // (If query_json cannot CREATE, fall back to the duckdb crate write path
        //  already used elsewhere in tests — see store.rs test helpers.)
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
        std::fs::remove_dir_all(&dir).ok();
    }
```

> NOTE for the implementer: if `crate::introspect::query_json` is read-only and cannot run the `CREATE TABLE`, mirror the DuckDB write helper used by existing tests in `src/store.rs`/`src/export.rs` (search for `Connection::open` in tests) to create `spans` before validating. The assertions themselves do not change.

- [ ] **Step 2: Run the test to verify it fails.**

Run: `cargo test timeline_validation_requires_core_flags`
Expected: FAIL (no timeline validation yet — `mk` returns `Ok` for the bad charts).

- [ ] **Step 3: Add the timeline validation block.**

In `src/session.rs`, in `validate_tile`, after the `box` requirement block (just before the final `Ok(())`, ~line 692), add:

```rust
    if chart.kind == "timeline" {
        // Core columns must be named and must exist.
        let lane = chart
            .lane
            .as_deref()
            .context("--chart timeline needs --lane <column> (the row / resource label)")?;
        check("--lane", lane)?;
        let label = chart
            .label
            .as_deref()
            .context("--chart timeline needs --label <column> (the bar text)")?;
        check("--label", label)?;
        let start = chart
            .start
            .as_deref()
            .context("--chart timeline needs --start <column>")?;
        check("--start", start)?;
        match (&chart.end, &chart.duration) {
            (Some(_), Some(_)) => bail!(
                "--chart timeline takes either --end or --duration, not both"
            ),
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
        let is_temporal = |t: &str| {
            t.contains("TIMESTAMP") || t.contains("DATE") || t.contains("TIME")
        };
        let is_numeric = |t: &str| {
            ["INT", "DEC", "DOUBLE", "FLOAT", "REAL", "HUGEINT", "NUMERIC", "BIGINT"]
                .iter()
                .any(|k| t.contains(k))
        };
        if let Some(end) = chart.end.as_deref() {
            if let (Some(st), Some(et)) = (col_type(start), col_type(end)) {
                let st_temporal = is_temporal(&st) && !is_numeric(&st);
                let et_temporal = is_temporal(&et) && !is_numeric(&et);
                if st_temporal != et_temporal {
                    bail!(
                        "--chart timeline: --start ({start}: {st}) and --end ({end}: {et}) must both be \
                         numeric (relative seconds) or both be timestamps"
                    );
                }
            }
        }
    }
```

> NOTE: `Value` is already imported (used by the `check` closure). `.context(...)` needs `anyhow::Context` — it is already in scope in this file (used elsewhere). Confirm with the compiler; if `Option::context` is unavailable, use `.ok_or_else(|| anyhow!(...))?`.

- [ ] **Step 4: Run the validation test to verify it passes.**

Run: `cargo test timeline_validation_requires_core_flags`
Expected: PASS.

- [ ] **Step 5: Update the usage/help string and chart-kind lists.**

In `src/session.rs`, find the `tile <name> …` usage literal (~line 1008). Add `timeline` to the `--chart` kind list and add a timeline line after the box line:

Change `[--chart bar|stacked|line|area|scatter|pie|table|heatmap|box|map]` to include `|timeline`.
After the `--chart box:` help line, add:

```
                       --chart timeline: --lane COL --label COL --start COL (--end COL | --duration COL); optional --color CAT --id COL --depends-on COL; --event 'T|label' markers\n                       \
```

- [ ] **Step 6: Format, lint, full test, commit.**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
git add src/session.rs
git commit -m "feat(timeline): validate timeline tiles (required flags + type check)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Frontend renderer core (lanes, sublanes, bars, colour, legend, axis) + seed + E2E

**Files:**
- Modify: `src/assets/index.html` (add CSS; add `timelineHtml()` + helpers `tlFmtDur`, `tlFmtAbs`, `tlIsNumeric`; dispatch in `loadTileChart`, `zoomTile`; `panelHtml` grip list + widen gate)
- Modify: `tests/e2e/fixtures/seed.ts`
- Create: `tests/e2e/specs/timeline.spec.ts`

**Interfaces:**
- Consumes: JSON `t.chart.{lane,label,start,end,duration,color}`; helpers `catColors(n)`, `colFmt`, `esc`, `attr`, `cssVar`, `parseTs`, `fmtTs`.
- Produces: `timelineHtml(cols, rows, spec, db, table) -> string`; module map `tlPayloads` and a `tlSeq` counter (used by Task 4's hydrate); DOM contract: `.tl-wrap > (.tl-legend?) .tl > (.tl-gutter > .tl-lane-label*) (.tl-plot[data-tl] > .tl-band* .tl-bar* svg.tl-overlay .tl-cursor) ; .tl-axis-row > .tl-axis > .tl-tick*`. Bars carry `data-tlid` (their id) and inline `left/width%`, `top/height px`.

- [ ] **Step 1: Add the CSS block.**

In `src/assets/index.html`, near the box-plot CSS (search for `.bx {` ~line 493), add after the `.bx-axis-line` rule:

```css
  /* ---- timeline (Gantt-like) tile -------------------------------------- */
  .tl-wrap { --tl-gutter: 148px; padding: 6px 4px 2px; }
  .tl-legend { display: flex; flex-wrap: wrap; gap: 10px 16px; padding: 2px 4px 10px; font-size: 11.5px; color: var(--muted); }
  .tl-leg { display: inline-flex; align-items: center; gap: 6px; }
  .tl-leg i { width: 11px; height: 11px; border-radius: 3px; display: inline-block; }
  .tl { display: grid; grid-template-columns: var(--tl-gutter) 1fr; }
  .tl-gutter { position: relative; }
  .tl-lane-label { position: absolute; left: 0; right: 8px; display: flex; align-items: center;
    font-size: 12px; color: var(--fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    border-top: 1px solid var(--line-soft); padding-right: 6px; }
  .tl-plot { position: relative; overflow: hidden; border-left: 1px solid var(--line); }
  .tl-band { position: absolute; left: 0; right: 0; border-top: 1px solid var(--line-soft); }
  .tl-band.alt { background: color-mix(in srgb, var(--fg) 3%, transparent); }
  .tl-bar { position: absolute; border-radius: 3px; overflow: hidden; box-sizing: border-box;
    box-shadow: 0 1px 2px rgba(0,0,0,0.18); cursor: default; display: flex; align-items: center; }
  .tl-bar-label { font-size: 11px; line-height: 1; padding: 0 6px; color: var(--on-accent, #14171b);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 600; }
  .tl-overlay { position: absolute; inset: 0; pointer-events: none; }
  .tl-overlay .tl-ev-label { font-size: 10px; fill: var(--muted); }
  .tl-cursor { position: absolute; top: 0; bottom: 0; width: 0; border-left: 1px dashed var(--muted);
    pointer-events: none; opacity: 0; }
  .tl-cursor.show { opacity: 0.85; }
  .tl-readout { position: absolute; top: 0; left: 4px; font-size: 10.5px; color: var(--fg);
    background: var(--surface); border: 1px solid var(--line-soft); border-radius: 3px; padding: 1px 4px; white-space: nowrap; }
  .tl-axis-row { display: grid; grid-template-columns: var(--tl-gutter) 1fr; margin-top: 2px; }
  .tl-axis { position: relative; height: 16px; border-top: 1px solid var(--line-soft); }
  .tl-tick { position: absolute; top: 2px; transform: translateX(-50%); font-size: 10.5px; color: var(--faint); white-space: nowrap; }
  .tl-tick:first-child { transform: none; }
  .tl-tick:last-child { transform: translateX(-100%); }
  .tl-tip-cat { color: var(--muted); font-weight: 400; }
  .tl-tip-sep { border-top: 1px solid var(--line-soft); margin: 4px 0; }
```

- [ ] **Step 2: Add the renderer + helpers.**

In `src/assets/index.html`, just before `boxTileHtml` (search `function boxTileHtml`), add:

```javascript
  // ---- timeline (Gantt-like) tile ---------------------------------------
  // Bars over a shared time axis, grouped into labelled lanes; overlaps stack
  // into sublanes. Start/end are numeric (relative seconds) or timestamps —
  // auto-detected. Dependency connectors, --event markers and a hover cursor
  // are drawn by hydrateTimelines() over an SVG overlay (see below).
  const TL_ROW_H = 26, TL_BAR_H = 18, TL_LANE_PAD = 6;
  let tlSeq = 0;
  const tlPayloads = {};   // per-render overlay payloads, consumed on hydrate

  function tlFmtDur(s) {
    s = Math.round(Number(s));
    if (!isFinite(s)) return "—";
    const neg = s < 0 ? "-" : ""; s = Math.abs(s);
    if (s < 60) return neg + s + "s";
    const m = Math.floor(s / 60), sec = s % 60;
    if (m < 60) return neg + (sec ? `${m}m ${String(sec).padStart(2, "0")}s` : `${m}m`);
    const h = Math.floor(m / 60), mm = m % 60;
    return neg + `${h}h ${String(mm).padStart(2, "0")}m`;
  }
  function tlFmtAbs(t, fmt) {
    if (fmt && (fmt.tz || fmt.epoch)) {
      const r = fmtTs(fmt, new Date(t).toISOString());
      if (r) return r[0] + (r[1] ? " " + r[1] : "");
    }
    const d = new Date(t);
    return isFinite(d.getTime()) ? d.toISOString().slice(0, 16).replace("T", " ") : "—";
  }
  // Numeric (relative-seconds) axis if every start value parses as a finite
  // number and doesn't look like a date/time (no '-' or ':' or 'T').
  function tlIsNumeric(vals) {
    return vals.every((v) => v == null || v === "" ||
      (isFinite(Number(v)) && !/[-:T]/.test(String(v).trim())));
  }
  function tlFmtTime(t, pay) { return pay.numeric ? tlFmtDur(t) : tlFmtAbs(t, pay.startFmt); }

  function timelineHtml(cols, rows, spec, db, table) {
    const li = cols.indexOf(spec.lane), bi = cols.indexOf(spec.label), si = cols.indexOf(spec.start);
    const ei = spec.end ? cols.indexOf(spec.end) : -1;
    const dgi = spec.duration ? cols.indexOf(spec.duration) : -1;
    if (li < 0 || bi < 0 || si < 0 || (ei < 0 && dgi < 0)) {
      return '<div class="note">timeline needs --lane, --label, --start and --end or --duration.</div>';
    }
    const numeric = tlIsNumeric(rows.map((r) => r[si]));
    const toT = (v) => numeric ? Number(v) : parseTs(v);
    const idi = spec.id ? cols.indexOf(spec.id) : -1;
    const depi = spec.depends_on ? cols.indexOf(spec.depends_on) : -1;
    const ci = spec.color ? cols.indexOf(spec.color) : -1;
    const bars = [];
    rows.forEach((r, ri) => {
      const s = toT(r[si]);
      let e = ei >= 0 ? toT(r[ei])
        : s + (numeric ? Number(r[dgi]) : Number(r[dgi]) * 1000);
      if (!isFinite(s) || !isFinite(e)) return;
      if (e < s) e = s;
      bars.push({
        ri, lane: r[li] == null ? "" : String(r[li]),
        label: r[bi] == null ? "" : String(r[bi]), s, e,
        id: idi >= 0 && r[idi] != null ? String(r[idi]) : null,
        deps: depi >= 0 && r[depi] != null
          ? String(r[depi]).split(",").map((x) => x.trim()).filter(Boolean) : [],
        cat: ci >= 0 && r[ci] != null ? String(r[ci]) : null,
      });
    });
    if (!bars.length) return '<div class="note">no timeline rows.</div>';
    const t0 = Math.min(...bars.map((b) => b.s));
    const t1max = Math.max(...bars.map((b) => b.e));
    const t1 = t1max > t0 ? t1max : t0 + 1;
    const span = t1 - t0;
    const frac = (t) => (t - t0) / span;
    // Lanes in first-appearance order.
    const laneOrder = [], laneMap = {};
    bars.forEach((b) => { if (!(b.lane in laneMap)) { laneMap[b.lane] = []; laneOrder.push(b.lane); } laneMap[b.lane].push(b); });
    // Colours: category (with legend) or per-lane palette.
    const laneCols = catColors(laneOrder.length);
    let cats = null, catCols = null, catIdx = null;
    if (ci >= 0) {
      cats = [...new Set(bars.filter((b) => b.cat != null).map((b) => b.cat))];
      catCols = catColors(Math.max(1, cats.length));
      catIdx = Object.fromEntries(cats.map((c, i) => [c, i]));
    }
    const barColor = (b, laneIndex) =>
      (ci >= 0 && b.cat != null && b.cat in catIdx) ? catCols[catIdx[b.cat] % catCols.length]
        : laneCols[laneIndex % laneCols.length];
    // Layout: greedy sublane packing per lane; assign each bar a top px.
    let y = 0; const laneRows = [];
    laneOrder.forEach((lane, laneIndex) => {
      const list = laneMap[lane].slice().sort((a, b) => a.s - b.s);
      const subEnds = [];
      list.forEach((b) => {
        let sl = subEnds.findIndex((end) => end <= b.s);
        if (sl < 0) { sl = subEnds.length; subEnds.push(b.e); } else subEnds[sl] = b.e;
        b.sub = sl;
      });
      const subCount = Math.max(1, subEnds.length);
      const top = y, height = subCount * TL_ROW_H + TL_LANE_PAD * 2;
      list.forEach((b) => { b.topPx = top + TL_LANE_PAD + b.sub * TL_ROW_H; b.color = barColor(b, laneIndex); });
      laneRows.push({ lane, top, height });
      y += height;
    });
    const totalH = y;
    const startFmt = colFmt(db, spec.start, table);
    const fmtT = (t) => numeric ? tlFmtDur(t) : tlFmtAbs(t, startFmt);
    const fmtDur = (a, b) => numeric ? tlFmtDur(b - a) : tlFmtDur((b - a) / 1000);
    // Bars. (Rich tooltip is added in a later task; core fields go in title=.)
    const barsHtml = bars.map((b) => {
      const l = frac(b.s) * 100, w = Math.max(0.4, (frac(b.e) - frac(b.s)) * 100);
      const title = `${b.label}\n${b.lane}\n${fmtT(b.s)} → ${fmtT(b.e)} (${fmtDur(b.s, b.e)})`;
      return `<div class="tl-bar" data-tlid="${attr(b.id || "")}" title="${attr(title)}"
        style="left:${l}%;width:${w}%;top:${b.topPx}px;height:${TL_BAR_H}px;background:${b.color}">
        <span class="tl-bar-label">${esc(b.label)}</span></div>`;
    }).join("");
    const gutterHtml = laneRows.map((lr) =>
      `<div class="tl-lane-label" style="top:${lr.top}px;height:${lr.height}px" title="${attr(lr.lane)}">${esc(lr.lane)}</div>`).join("");
    const bandsHtml = laneRows.map((lr, i) =>
      `<div class="tl-band${i % 2 ? " alt" : ""}" style="top:${lr.top}px;height:${lr.height}px"></div>`).join("");
    const legendHtml = (ci >= 0 && cats.length)
      ? `<div class="tl-legend">${cats.map((c, i) => `<span class="tl-leg"><i style="background:${catCols[i % catCols.length]}"></i>${esc(c)}</span>`).join("")}</div>`
      : "";
    const ticks = [];
    for (let i = 0; i <= 4; i++) { const t = t0 + (span * i) / 4; ticks.push(`<span class="tl-tick" style="left:${(i / 4) * 100}%">${esc(fmtT(t))}</span>`); }
    const axisHtml = `<div class="tl-axis">${ticks.join("")}</div>`;
    // Overlay payload (deps + events) for hydrateTimelines().
    const id = "tl" + (++tlSeq);
    const barById = {}; bars.forEach((b) => { if (b.id) barById[b.id] = b; });
    const deps = [];
    bars.forEach((b) => b.deps.forEach((pid) => {
      const p = barById[pid];
      if (p) deps.push({ fromFrac: frac(p.e), fromY: p.topPx + TL_BAR_H / 2, toFrac: frac(b.s), toY: b.topPx + TL_BAR_H / 2 });
    }));
    const events = (spec.events || []).map((m) => ({ frac: frac(toT(m.value)), label: m.label || "" }))
      .filter((e) => isFinite(e.frac) && e.frac >= -0.001 && e.frac <= 1.001);
    tlPayloads[id] = { t0, t1, numeric, startFmt, deps, events, totalH };
    return `<div class="tl-wrap">${legendHtml}
      <div class="tl" style="height:${totalH}px">
        <div class="tl-gutter" style="height:${totalH}px">${gutterHtml}</div>
        <div class="tl-plot" data-tl="${id}" style="height:${totalH}px">
          ${bandsHtml}${barsHtml}
          <svg class="tl-overlay" xmlns="http://www.w3.org/2000/svg"></svg>
          <div class="tl-cursor"><span class="tl-readout"></span></div>
        </div>
      </div>
      <div class="tl-axis-row"><div></div>${axisHtml}</div>
    </div>`;
  }
```

> NOTE: `hydrateTimelines` is added in Task 4. For this task, dispatch calls it too (harmless no-op if defined then). Define a temporary stub now so this task's E2E works standalone — add right after `timelineHtml`:
> ```javascript
>   function hydrateTimelines(_scope) { /* overlay hydration added in Task 4 */ }
> ```

- [ ] **Step 3: Dispatch in `loadTileChart`.**

In `loadTileChart`, after the `if (kind === "map") { … }` line (~3435), add:

```javascript
    if (kind === "timeline") { slot.style.height = "auto"; slot.innerHTML = timelineHtml(cols, rows, t.chart || {}, t.db, t.view || null); hydrateTimelines(slot); return; }
```

- [ ] **Step 4: Dispatch in `zoomTile`.**

In `zoomTile`, after the `if (kind === "map") { … }` render line (~2972), add:

```javascript
      if (kind === "timeline") { slot.innerHTML = timelineHtml(cols, rows, t.chart || {}, t.db, t.view || null); hydrateTimelines(slot); return; }
```

Also add `timeline` to the fit-height branch so the modal shrinks to content: change the line `if (kind === "heatmap" || kind === "box") layer.querySelector(".zoom-box").classList.add("fit");` to `if (kind === "heatmap" || kind === "box" || kind === "timeline") layer.querySelector(".zoom-box").classList.add("fit");`.

- [ ] **Step 5: `panelHtml` — grip list + widen gate.**

In `panelHtml`:
- Change the grip line (~3393) from `kind === "heatmap" || kind === "box" || kind === "map"` to `kind === "heatmap" || kind === "box" || kind === "map" || kind === "timeline"`.
- Change the widen-button gate (~3384) from `kind === "table"` to `kind === "table" || kind === "timeline"`.

- [ ] **Step 6: Add seed data (view + tile).**

In `tests/e2e/fixtures/seed.ts`, add to `CREATE_SQL` (before the closing backtick):

```sql
-- Timeline (Gantt) fixture: a small deploy pipeline on a relative-seconds axis
-- with two lanes, an overlap (→ sublane), a colour category, and a dependency.
CREATE VIEW deploy_timeline AS SELECT * FROM (VALUES
  ('build',  'compile',   0.0,  40.0, 'ok',     's1', NULL),
  ('build',  'lint',      5.0,  30.0, 'ok',     's2', NULL),   -- overlaps compile → sublane
  ('deploy', 'push',     40.0,  70.0, 'ok',     's3', 's1'),
  ('deploy', 'migrate',  70.0,  95.0, 'failed', 's4', 's3')
) t(lane, task, t0, t1, status, sid, parent);
```

Then add a tile in `seed()` after the `flows` tile:

```typescript
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'timeline', '--title', 'Deploy timeline',
    '--db', dbPath, '--view', 'deploy_timeline', '--chart', 'timeline',
    '--lane', 'lane', '--label', 'task', '--start', 't0', '--end', 't1',
    '--color', 'status', '--id', 'sid', '--depends-on', 'parent',
    '--event', '50|cutover',
    '--caption', 'A Gantt-style timeline: lanes stack overlapping bars into sublanes; colour = status.']);
```

- [ ] **Step 7: Write the E2E spec (core assertions).**

Create `tests/e2e/specs/timeline.spec.ts`:

```typescript
import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('timeline tile', () => {
  test('renders lanes, bars, a sublane for overlap, and a legend', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await expect(panel).toBeVisible();

    // Two lanes → two lane labels in the gutter.
    await expect(panel.locator('.tl-lane-label')).toHaveCount(2);
    await expect(panel.locator('.tl-lane-label', { hasText: 'build' })).toBeVisible();

    // Four bars.
    await expect(panel.locator('.tl-bar')).toHaveCount(4);

    // The two 'build' bars overlap → different top offsets (a sublane).
    const compile = panel.locator('.tl-bar', { hasText: 'compile' });
    const lint = panel.locator('.tl-bar', { hasText: 'lint' });
    const topOf = (loc) => loc.evaluate((el) => parseFloat((el as HTMLElement).style.top));
    expect(await topOf(compile)).not.toBe(await topOf(lint));

    // Colour-by-status legend has entries.
    await expect(panel.locator('.tl-legend .tl-leg')).toHaveCount(2);

    // Full-width toggle is offered (timeline is in the widen gate).
    await expect(panel.locator('[data-widen]')).toHaveCount(1);
  });
});
```

- [ ] **Step 8: Build the binary and run the timeline spec.**

```bash
cargo build --release
cd tests/e2e && npx playwright test timeline.spec.ts; cd ../..
```
Expected: PASS. (The e2e harness re-seeds using the release binary; ensure it is built first.)

- [ ] **Step 9: Format, lint, test, commit.**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
git add src/assets/index.html tests/e2e/fixtures/seed.ts tests/e2e/specs/timeline.spec.ts
git commit -m "feat(timeline): frontend renderer core (lanes, sublanes, bars, legend) + e2e

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Overlay — dependency connectors, markers, hover cursor

**Files:**
- Modify: `src/assets/index.html` (replace the `hydrateTimelines` stub with the real one; add `renderTimelineOverlay`; add the `TL_DEP_OPACITY` theme global + wire into `applyTheme`)
- Modify: `tests/e2e/specs/timeline.spec.ts` (add assertions)

**Interfaces:**
- Consumes: `tlPayloads[id]` from Task 3 (`{t0,t1,numeric,startFmt,deps:[{fromFrac,fromY,toFrac,toY}],events:[{frac,label}],totalH}`); `.tl-plot[data-tl]`, `.tl-overlay`, `.tl-cursor`, `.tl-readout` from the rendered HTML; `cssVar`, `esc`, `tlFmtTime`.
- Produces: `hydrateTimelines(scope)` draws `svg.tl-overlay > g.tl-deps > path` (orthogonal), `g.tl-events > line` (dashed) `+ text`, and wires the mouse cursor. `renderTimelineOverlay(plot, pay)` is idempotent and re-runs on resize.

- [ ] **Step 1: Add the theme opacity global + wire into applyTheme.**

Near the ARC globals (search `let ARC_OPACITY = 0.22;` ~3638), add:

```javascript
  // Timeline dependency-connector opacity. Themes override with `depOpacity`.
  let TL_DEP_OPACITY = 0.5;
```

In `applyTheme` where arc globals are set (search `ARC_OPACITY = t.arcOpacity != null ? …` ~1167), add after that group:

```javascript
    TL_DEP_OPACITY = t.depOpacity != null ? t.depOpacity : (t.light ? 0.6 : 0.5);
```

- [ ] **Step 2: Replace the `hydrateTimelines` stub with the real implementation.**

Replace the stub `function hydrateTimelines(_scope) { … }` (from Task 3) with:

```javascript
  function renderTimelineOverlay(plot, pay) {
    const svg = plot.querySelector("svg.tl-overlay"); if (!svg) return;
    const W = plot.clientWidth, H = pay.totalH; if (!W) return;
    svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
    svg.setAttribute("width", W); svg.setAttribute("height", H);
    const line = cssVar("--muted") || "#9b958a";
    const evCol = cssVar("--anno-event") || cssVar("--teal") || "#b8b2a6";
    const deps = pay.deps.map((d) => {
      const x1 = d.fromFrac * W, y1 = d.fromY, x2 = d.toFrac * W, y2 = d.toY;
      const mx = (x1 + x2) / 2;
      return `<path d="M${x1.toFixed(1)} ${y1} H${mx.toFixed(1)} V${y2} H${x2.toFixed(1)}" fill="none" stroke="${line}" stroke-width="1" opacity="${TL_DEP_OPACITY}"/>` +
        `<circle cx="${x2.toFixed(1)}" cy="${y2}" r="2.2" fill="${line}" opacity="${TL_DEP_OPACITY}"/>`;
    }).join("");
    const events = pay.events.map((e) => {
      const x = (e.frac * W).toFixed(1);
      return `<line x1="${x}" y1="0" x2="${x}" y2="${H}" stroke="${evCol}" stroke-width="1.2" stroke-dasharray="4 3"/>` +
        (e.label ? `<text x="${(+x + 3)}" y="11" class="tl-ev-label">${esc(e.label)}</text>` : "");
    }).join("");
    svg.innerHTML = `<g class="tl-deps">${deps}</g><g class="tl-events">${events}</g>`;
  }
  function hydrateTimelines(scope) {
    (scope || document).querySelectorAll(".tl-plot[data-tl]").forEach((plot) => {
      const pay = tlPayloads[plot.dataset.tl]; if (!pay) return;
      plot._tlPay = pay;
      const draw = () => renderTimelineOverlay(plot, pay);
      draw();
      if (window.ResizeObserver && !plot._tlRO) { plot._tlRO = new ResizeObserver(draw); plot._tlRO.observe(plot); }
      const cursor = plot.querySelector(".tl-cursor");
      const readout = cursor && cursor.querySelector(".tl-readout");
      if (cursor && !plot._tlCursor) {
        plot._tlCursor = true;
        plot.addEventListener("mousemove", (ev) => {
          const rect = plot.getBoundingClientRect();
          const fx = (ev.clientX - rect.left) / rect.width;
          if (fx < 0 || fx > 1) { cursor.classList.remove("show"); return; }
          cursor.style.left = (fx * 100) + "%";
          cursor.classList.add("show");
          const t = pay.t0 + (pay.t1 - pay.t0) * fx;
          if (readout) { readout.textContent = tlFmtTime(t, pay); readout.style.left = fx > 0.85 ? "auto" : "4px"; readout.style.right = fx > 0.85 ? "4px" : "auto"; }
        });
        plot.addEventListener("mouseleave", () => cursor.classList.remove("show"));
      }
    });
  }
```

- [ ] **Step 3: Add E2E assertions for deps, marker, and cursor.**

Append to `tests/e2e/specs/timeline.spec.ts` inside the `describe`:

```typescript
  test('draws dependency connectors, an event marker, and a hover cursor', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await expect(panel).toBeVisible();

    // Two dependencies (s1→s3, s3→s4) → two orthogonal connector paths.
    await expect(panel.locator('svg.tl-overlay .tl-deps path')).toHaveCount(2);

    // The --event '50|cutover' marker → a dashed line + its label.
    await expect(panel.locator('svg.tl-overlay .tl-events line')).toHaveCount(1);
    await expect(panel.locator('svg.tl-overlay .tl-events text')).toContainText('cutover');

    // Hovering the plot shows the time cursor with a readout.
    await panel.locator('.tl-plot').hover();
    const cursor = panel.locator('.tl-cursor');
    await expect(cursor).toHaveClass(/\bshow\b/);
    await expect(panel.locator('.tl-readout')).not.toBeEmpty();
  });
```

- [ ] **Step 4: Build and run the spec.**

```bash
cargo build --release
cd tests/e2e && npx playwright test timeline.spec.ts; cd ../..
```
Expected: PASS.

- [ ] **Step 5: Format, lint, test, commit.**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/assets/index.html tests/e2e/specs/timeline.spec.ts
git commit -m "feat(timeline): dependency connectors, event markers, hover cursor

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Rich per-bar tooltip (core fields + all extra columns)

**Files:**
- Modify: `src/assets/index.html` (add `tlTip()`; switch bars from `title=` to `class="wm-x" data-tip=`)
- Modify: `tests/e2e/specs/timeline.spec.ts`

**Interfaces:**
- Consumes: the shared `.wm-x` + `data-tip` delegated tooltip (`wmTooltip` IIFE) and `.wm-tip` element; helpers `colFmt`, `applyFmtHtml`, `linkSubst`, `esc`, `attr`.
- Produces: `tlTip(b, cols, rows, spec, db, table, fmtT, fmtDur) -> htmlString` returning core fields then every non-core, non-null column formatted (links honoured); bars gain `wm-x` class + `data-tip`.

- [ ] **Step 1: Add the `tlTip` builder.**

In `src/assets/index.html`, immediately before `timelineHtml`, add:

```javascript
  function tlTip(b, cols, rows, spec, db, table, fmtT, fmtDur) {
    const row = rows[b.ri];
    const core = [];
    core.push(`<b>${esc(b.label)}</b>` + (b.cat != null ? ` <span class="tl-tip-cat">[${esc(spec.color)}: ${esc(b.cat)}]</span>` : ""));
    core.push(`lane: ${esc(b.lane)}`);
    core.push(`${esc(fmtT(b.s))} → ${esc(fmtT(b.e))} (${esc(fmtDur(b.s, b.e))})`);
    const shown = new Set([spec.lane, spec.label, spec.start, spec.end, spec.duration].filter(Boolean));
    const extra = [];
    cols.forEach((c, i) => {
      if (shown.has(c)) return;
      const v = row[i]; if (v === null || v === undefined || v === "") return;
      const f = colFmt(db, c, table);
      let disp = f ? applyFmtHtml(f, v) : esc(typeof v === "object" ? JSON.stringify(v) : String(v));
      if (f && f.link) {
        const href = linkSubst(f.link, v, row, cols, true);
        const txt = f.link_title ? linkSubst(f.link_title, v, row, cols, false) : disp;
        disp = `<a href="${attr(href)}" target="_blank" rel="noopener">${txt}</a>`;
      }
      extra.push(`${esc(c)}: ${disp}`);
    });
    const extraHtml = extra.length ? `<div class="tl-tip-sep"></div>${extra.join("<br>")}` : "";
    return `${core.join("<br>")}${extraHtml}`;
  }
```

- [ ] **Step 2: Switch bars to the rich tooltip.**

In `timelineHtml`, replace the `barsHtml` mapping body (the `title=` version from Task 3) with:

```javascript
    const barsHtml = bars.map((b) => {
      const l = frac(b.s) * 100, w = Math.max(0.4, (frac(b.e) - frac(b.s)) * 100);
      const tip = tlTip(b, cols, rows, spec, db, table, fmtT, fmtDur);
      return `<div class="tl-bar wm-x" data-tlid="${attr(b.id || "")}" data-tip="${attr(tip)}"
        style="left:${l}%;width:${w}%;top:${b.topPx}px;height:${TL_BAR_H}px;background:${b.color}">
        <span class="tl-bar-label">${esc(b.label)}</span></div>`;
    }).join("");
```

- [ ] **Step 3: Add an E2E assertion for the rich tooltip.**

Append to `timeline.spec.ts`:

```typescript
  test('bar hover shows a rich tooltip with core fields and extra columns', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await panel.locator('.tl-bar', { hasText: 'migrate' }).hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toContainText('lane: deploy');
    await expect(tip).toContainText('status: failed');  // colour category, an extra column
  });
```

- [ ] **Step 4: Build and run.**

```bash
cargo build --release
cd tests/e2e && npx playwright test timeline.spec.ts; cd ../..
```
Expected: PASS (all three timeline tests).

- [ ] **Step 5: Format, lint, test, commit.**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/assets/index.html tests/e2e/specs/timeline.spec.ts
git commit -m "feat(timeline): rich per-bar tooltip (core fields + all row columns)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Demo — timeline section in `demo.sh`

**Files:**
- Modify: `demo.sh` (sample data, a new section + tiles, summary row)

**Interfaces:**
- Consumes: the `--chart timeline` CLI from Tasks 1-2.
- Produces: a "Timeline" section in the `demo` session with a relative-seconds Gantt (deps + sublanes) and an absolute-time incident timeline (markers + colour-by-status).

- [ ] **Step 1: Add sample data.**

In `demo.sh`, in the big `"$MUCKDB" "$DB" -c " … "` data block (after the `events` table, before its closing `";`), add two tables:

```sql
-- A CI/deploy pipeline for a relative-seconds Gantt: several lanes, an overlap
-- (build/test run concurrently → sublane), colour by outcome, and a dependency
-- chain (checkout → build → {test, package} → deploy).
CREATE OR REPLACE TABLE pipeline AS SELECT * FROM (VALUES
  ('runner-1', 'checkout',  0.0,   12.0, 'ok',      'p1', NULL),
  ('runner-1', 'build',     12.0,  95.0, 'ok',      'p2', 'p1'),
  ('runner-2', 'unit tests',95.0, 180.0, 'ok',      'p3', 'p2'),
  ('runner-2', 'lint',      95.0, 130.0, 'warn',    'p4', 'p2'),   -- overlaps unit tests
  ('runner-1', 'package',  180.0, 210.0, 'ok',      'p5', 'p3'),
  ('runner-1', 'deploy',   210.0, 260.0, 'failed',  'p6', 'p5')
) t(resource, step, t0, t1, outcome, sid, parent);

-- An incident timeline on an absolute time axis: phases per system, with
-- colour by severity and event markers for the key moments.
CREATE OR REPLACE TABLE incident AS SELECT * FROM (VALUES
  ('api',      'elevated errors', TIMESTAMP '2026-05-01 14:02:00', TIMESTAMP '2026-05-01 14:18:00', 'warning'),
  ('api',      'outage',          TIMESTAMP '2026-05-01 14:18:00', TIMESTAMP '2026-05-01 14:41:00', 'critical'),
  ('database', 'failover',        TIMESTAMP '2026-05-01 14:22:00', TIMESTAMP '2026-05-01 14:35:00', 'critical'),
  ('oncall',   'investigate',     TIMESTAMP '2026-05-01 14:10:00', TIMESTAMP '2026-05-01 14:30:00', 'info'),
  ('oncall',   'mitigate',        TIMESTAMP '2026-05-01 14:30:00', TIMESTAMP '2026-05-01 14:41:00', 'info')
) t(system, phase, started, ended, severity);
```

- [ ] **Step 2: Add views for the two timelines.**

In `demo.sh`, in the section where views are created (search for `CREATE OR REPLACE VIEW` blocks; add near the other view definitions or in the same data block), add:

```sql
CREATE OR REPLACE VIEW pipeline_timeline AS
  SELECT resource, step, t0, t1, outcome, sid, parent FROM pipeline;
CREATE OR REPLACE VIEW incident_timeline AS
  SELECT system, phase, started, ended, severity FROM incident ORDER BY started;
```

- [ ] **Step 3: Add the section and tiles.**

In `demo.sh`, after the Geography section's tiles (after the `flows` tile, before the closing `summary` post ~line 285), add:

```bash
"$MUCKDB" session section "$SESSION" --name sec-timeline --title "Timelines" >/dev/null

"$MUCKDB" session tile "$SESSION" --name pipeline --title "CI/CD pipeline (relative seconds)" \
  --db "$DB" --view pipeline_timeline --chart timeline \
  --lane resource --label step --start t0 --end t1 \
  --color outcome --id sid --depends-on parent \
  --event '95|tests start' \
  --caption "A Gantt-style pipeline on a 0→seconds axis: lanes are runners, bars are steps; lint overlaps unit tests so it stacks into a sublane; right-angle connectors show the dependency chain and colour encodes the step outcome." >/dev/null

"$MUCKDB" session tile "$SESSION" --name incident --title "Incident timeline (absolute time)" \
  --db "$DB" --view incident_timeline --chart timeline \
  --lane system --label phase --start started --end ended \
  --color severity \
  --event '2026-05-01 14:18|outage declared' --event '2026-05-01 14:41|resolved' \
  --caption "The same tile on an absolute time axis: each system's phases over the incident, coloured by severity, with dashed markers for when the outage was declared and resolved. Hover any bar for its exact window and details." >/dev/null
```

- [ ] **Step 4: Add rows to the summary table.**

In `demo.sh`, in the `summary` markdown table, add two rows (keep the existing alignment):

```
| **CI pipeline**     | timeline (relative seconds)    | tasks over time, sublanes for overlap, dep arrows |
| **Incident**        | timeline (absolute time)       | phases per system, severity colour, event markers |
```

Update the sentence listing section headers to include **Timelines**.

- [ ] **Step 5: Run the demo end-to-end and screenshot it.**

```bash
cargo build --release
MUCKDB=./target/release/muckdb ./demo.sh /tmp/claude-1000/-home-anko-code-rust-muckdb/28d1096f-97e4-401c-a544-8df7d408c3d1/scratchpad/demo.duckdb
MUCKDB=./target/release/muckdb ./target/release/muckdb session screenshot demo --tile pipeline --out /tmp/claude-1000/-home-anko-code-rust-muckdb/28d1096f-97e4-401c-a544-8df7d408c3d1/scratchpad/pipeline.png
MUCKDB=./target/release/muckdb ./target/release/muckdb session screenshot demo --tile incident --out /tmp/claude-1000/-home-anko-code-rust-muckdb/28d1096f-97e4-401c-a544-8df7d408c3d1/scratchpad/incident.png
```

Read both PNGs and confirm: lanes labelled, bars placed by time, the lint/unit-tests overlap shows two sublanes, dependency connectors are right-angled, the marker lines and legend render. Fix any layout issues in `timelineHtml`/CSS before committing.

- [ ] **Step 6: Commit.**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add demo.sh
git commit -m "demo: add Timelines section (CI pipeline + incident timeline)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Documentation — SKILL.md (detailed) + AGENT.md mirror

**Files:**
- Modify: `src/assets/skill/SKILL.md` (command reference ~187; chart-kind list ~244; new detailed `timeline` subsection; option rows)
- Modify: `AGENT.md` (mirror the chart-kind list + timeline docs where the other kinds are documented)

**Interfaces:** none (docs only).

- [ ] **Step 1: SKILL.md — command reference + option lines.**

In `src/assets/skill/SKILL.md`, in the `session tile` command reference block (~line 187), add `|timeline` to the `--chart` list, and add these option lines under the existing per-kind option lines:

```
        [--lane COL] [--start COL] [--end COL | --duration COL]  (timeline: lane label, bar start, bar end or numeric-seconds duration)
        [--color COL]  (timeline: colour bars by a category value + legend)
        [--id COL] [--depends-on COL]  (timeline: unique bar id + comma-separated parent id(s) → right-angle dependency connectors)
```

- [ ] **Step 2: SKILL.md — chart-kind prose list.**

In the prose list (~line 244) change `bar | stacked | line | area | scatter | pie | table | heatmap | box | map` to append `| timeline`.

- [ ] **Step 3: SKILL.md — detailed `timeline` subsection.**

In the "Pick the chart that packs in the most information" list, after the `map` bullet (~line 286+), add a `timeline` bullet with the full detail:

````markdown
  - **`timeline`** to show events as horizontal bars over a shared time axis,
    grouped into labelled **lanes** (a Gantt chart). Reach for it for **resource
    allocation**, an **incident timeline**, an **OpenTelemetry trace**, or
    **investigation sequencing** — anything where *what happened, in which lane,
    over what time* is the point. Each row is one bar.
    - **Columns:** `--lane COL` (the row/resource label), `--label COL` (bar
      text), `--start COL`, and either `--end COL` or `--duration COL` (numeric
      seconds; `end = start + duration`). Start/end may be **numeric** (a
      relative "0 → N seconds" axis, auto-humanised to `2m 10s`) or
      **timestamps/dates** (an absolute time axis) — the kind is auto-detected;
      don't mix the two.
    - **Colour:** by default each lane gets its own palette colour. Pass
      `--color COL` to colour every bar by that column's **category** (e.g.
      status `ok`/`failed`), which adds a legend — colour then encodes the
      category, not the lane.
    - **Overlaps → sublanes:** bars in the same lane whose times overlap stack
      into sublanes automatically, so nothing is hidden. Lane order follows the
      view's row order — use `ORDER BY` to control it.
    - **Dependencies:** `--id COL` gives each bar a unique id; `--depends-on COL`
      holds comma-separated parent id(s). Each parent is drawn to this bar with a
      thin right-angle connector (multiple parents = multiple connectors). An id
      that matches nothing is ignored.
    - **Markers:** `--event 'T|label'` (repeatable) draws a dashed vertical line
      at time `T` (a number for the relative axis, or a timestamp) — flag a
      deploy, an alert, a cutover.
    - **Hover:** a vertical cursor follows the mouse with a time readout; the
      bottom axis shows the full range. Hovering a bar shows a rich tooltip —
      label, lane, start → end, duration, the colour category, and **every other
      column** in the row (formatted, with links if you set a `--link` format).
    - **Full width:** timeline tiles get the full-width toggle (like tables) —
      dense timelines benefit from breaking out of the centred column.

    Worked examples (shape the view so each row is one bar; set formats first):

    ```sh
    # Resource allocation — machines as lanes, jobs as bars, colour by job type.
    muckdb session tile ops --name alloc --title "Cluster allocation" \
      --db ops.db --view job_spans --chart timeline \
      --lane host --label job --start started --end ended --color job_type \
      --caption "Which host ran which job, when — colour = job type."

    # Incident timeline — systems as lanes, phases as bars, markers for key moments.
    muckdb session tile inc --name incident --title "Incident 4821" \
      --db inc.db --view phases --chart timeline \
      --lane system --label phase --start started --end ended --color severity \
      --event '2026-05-01 14:18|outage' --event '2026-05-01 14:41|resolved' \
      --caption "Phases per system, coloured by severity."

    # OpenTelemetry trace — services as lanes, spans as bars, parent→child deps.
    # Relative-seconds axis: start_ms/dur_ms are numbers from the trace.
    muckdb session tile trc --name trace --title "Trace 7f3a…" \
      --db trace.db --view spans --chart timeline \
      --lane service --label name --start start_s --duration dur_s \
      --id span_id --depends-on parent_id --color status \
      --caption "Spans on a 0→seconds axis; arrows show parent→child."

    # Investigation sequencing — workstreams as lanes, steps as bars.
    muckdb session tile inv --name seq --title "Investigation" \
      --db inv.db --view steps --chart timeline \
      --lane workstream --label step --start began --end finished \
      --id step_id --depends-on blocks_on \
      --caption "Concurrent steps stack into sublanes; deps show ordering."
    ```
````

- [ ] **Step 4: Mirror into AGENT.md.**

In the repo `AGENT.md`, apply the same two edits (chart-kind list + a condensed version of the timeline bullet with the four use cases, the options, and at least the trace + incident worked examples) in the chart-docs section where `map`/`box` are documented, so the checked-in guide matches the skill.

- [ ] **Step 5: Commit.**

```bash
git add src/assets/skill/SKILL.md AGENT.md
git commit -m "docs(timeline): detailed skill + AGENT.md docs with use cases

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (before reporting done — do NOT push/release)

- [ ] `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo build && cargo test` all clean.
- [ ] `cd tests/e2e && npx playwright test timeline.spec.ts map.spec.ts charts.spec.ts` all pass (timeline didn't regress the others).
- [ ] `./demo.sh` runs clean; `muckdb session screenshot demo --tile pipeline` and `--tile incident` PNGs look correct (read them).
- [ ] Manually verify both time modes render and full-width toggles, then report to the user for the release decision. **Leave unpushed and unreleased.**

---

## Self-Review notes (author)

- **Spec coverage:** data model + auto-detected axis (Task 1/3), validation incl. type check (Task 2), colour default+category+legend (Task 3), dependencies (Task 3 payload + Task 4 draw), markers via `--event` (Task 3/4), hover cursor + range axis (Task 3/4), rich tooltip (Task 5), sublanes (Task 3), full-width + grip (Task 3), demo (Task 6), detailed skill + AGENT.md (Task 7), tests throughout. All spec sections mapped.
- **Type consistency:** `tlPayloads`/`tlSeq` defined in Task 3, consumed in Task 4; payload shape `{t0,t1,numeric,startFmt,deps,events,totalH}` is identical at produce and consume; `tlTip` signature matches its Task-5 call site; DOM class names (`.tl-plot`,`.tl-overlay`,`.tl-deps path`,`.tl-events line/text`,`.tl-cursor`,`.tl-readout`,`.tl-bar`,`.tl-lane-label`,`.tl-legend .tl-leg`) match between renderer, overlay, CSS, and specs.
- **Placeholders:** none — all steps carry real code/commands. The two `NOTE`s (test DB write helper; `Option::context` fallback) are compiler-verifiable branch points, not deferred work.
