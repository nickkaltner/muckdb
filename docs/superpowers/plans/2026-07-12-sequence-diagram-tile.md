# Sequence Diagram Tile Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `--chart sequence` session tile that renders a hand-rolled SVG sequence diagram (one view, one row per message) for showing interactions between microservices, with a toolbar button that exports the diagram as mermaid.js `sequenceDiagram` text.

**Architecture:** Mirror the existing `timeline` tile end-to-end. Backend: a group of `Option<String>`/`bool` fields on the `Chart` struct, a CLI flag mapping, and a `kind == "sequence"` branch in `validate_tile`. Frontend: a self-contained `sequenceHtml()` SVG renderer (layout is intrinsic to the data — one column per participant, one row per message — so, unlike timeline, it needs **no** responsive hydrate step) dispatched from `loadTileChart`/`zoomTile`, plus a client-side `seqToMermaid()` generator behind a toolbar button. Mermaid is an **export format only**, not the renderer.

**Tech Stack:** Rust (hand-rolled CLI arg parser, serde, anyhow, duckdb CLI for validation); a single embedded `src/assets/index.html` (vanilla JS + inline SVG, no framework/build); Playwright + Vitest-style e2e.

**Reference spec:** `docs/superpowers/specs/2026-07-12-sequence-diagram-tile-design.md`.

## Global Constraints

- CI gates must stay green, in order: `cargo fmt` (run it, not just `--check`) → `cargo clippy --all-targets -- -D warnings` → `cargo build` → `cargo test`. Plus the Playwright e2e suite (`cd tests/e2e && npm test`, which builds the release binary).
- New `Chart` fields use `Option<String>` + `#[serde(default, skip_serializing_if = "Option::is_none")]` (and `bool` + `#[serde(default, skip_serializing_if = "is_false")]` for `autonumber`) so no other tile kind's serialized JSON gains keys.
- `Chart` does **not** derive `Default`; every full `Chart {..}` literal must list all fields. Full literals live at: `src/session.rs:971` (CLI constructor), `src/export.rs:396` (test fixture), and the test literals `src/session.rs:1171`, `:1211`, `:1257`, `:1434`. (The `..c`/`..spread` variants at `:1246` and `:1294` compile unchanged.)
- Every data tile keeps requiring `--caption` (existing convention; nothing to add — it already applies).
- Participant type vocabulary: `participant` (default) · `actor` · `database` · `boundary`. Message-type vocabulary: `sync` (default) · `reply` · `async` · `lost`.
- Mermaid arrow mapping: `sync`→`->>`, `reply`→`-->>`, `async`→`-)`, `lost`→`-x`. Participant mapping: `actor`→`actor`, everything else→`participant`, with a `%% database` / `%% boundary` comment line preceding db/boundary declarations.
- Follow the timeline tile's naming/structure wherever a parallel exists.

**Commit trailer** for every commit:
```
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 1: Chart fields + CLI mapping + serde round-trip

**Files:**
- Modify: `src/session.rs` — `Chart` struct (after line 113), CLI constructor (`:971-999`), and the full test literals (`:1171`, `:1211`, `:1257`, `:1434`).
- Modify: `src/export.rs:396-424` — the test-fixture `Chart` literal (compile fix).
- Test: `src/session.rs` `mod tests` — new `sequence_chart_serde_roundtrips_fields`.

**Interfaces:**
- Produces (Chart fields, exact names): `from_participant: Option<String>`, `to_participant: Option<String>`, `message_type: Option<String>`, `from_type: Option<String>`, `to_type: Option<String>`, `group: Option<String>`, `group_branch: Option<String>`, `autonumber: bool`. (`label` is reused for the message text.)
- Produces (CLI flags): `--from`, `--to`, `--message-type`, `--from-type`, `--to-type`, `--group`, `--group-branch`, `--autonumber` (valueless).
- The serialized JSON keys equal the field names (no serde rename), so the frontend reads `spec.from_participant`, `spec.to_participant`, `spec.message_type`, `spec.from_type`, `spec.to_type`, `spec.group`, `spec.group_branch`, `spec.autonumber`.

- [ ] **Step 1: Add the fields to the `Chart` struct**

Insert immediately after the `depends_on` field (currently `src/session.rs:113`, before the closing `}` at `:114`):

```rust
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
```

- [ ] **Step 2: Map the CLI flags in the constructor**

In the `Chart { .. }` literal in the `"tile"` handler, add these lines immediately after `depends_on: p.get("depends-on").map(str::to_string),` (currently `src/session.rs:998`):

```rust
                    from_participant: p.get("from").map(str::to_string),
                    to_participant: p.get("to").map(str::to_string),
                    message_type: p.get("message-type").map(str::to_string),
                    from_type: p.get("from-type").map(str::to_string),
                    to_type: p.get("to-type").map(str::to_string),
                    group: p.get("group").map(str::to_string),
                    group_branch: p.get("group-branch").map(str::to_string),
                    autonumber: p.get("autonumber").is_some(),
```

- [ ] **Step 3: Register `--autonumber` as a valueless flag**

Change `BOOL_FLAGS` (`src/session.rs:488`) to include it:

```rust
const BOOL_FLAGS: &[&str] = &["no-validate", "up", "down", "trend", "autonumber"];
```

> Note: `--trend` is added here too. It was previously handled via `p.get("trend").is_some()` and worked only because it was the last flag; adding it to `BOOL_FLAGS` makes it robust regardless of flag order and is a correct no-op otherwise. If `trend` is already present in `BOOL_FLAGS` when you read the file, leave it and just add `"autonumber"`.

- [ ] **Step 4: Add the new fields to every full `Chart {..}` literal so the crate compiles**

Append these eight lines to each of the full literals — in `src/export.rs` (the fixture ending at `:423` with `depends_on: None,`) and in `src/session.rs` at `:1171` (heatmap test), `:1211` (map test), `:1257` (timeline test), and `:1434` (the `base()` closure in the validation test) — immediately after their `depends_on: None,` line:

```rust
            from_participant: None,
            to_participant: None,
            message_type: None,
            from_type: None,
            to_type: None,
            group: None,
            group_branch: None,
            autonumber: false,
```

(Match the surrounding indentation of each literal — `src/export.rs` uses more indentation than the session tests.)

- [ ] **Step 5: Write the serde round-trip test**

Add to `src/session.rs` `mod tests`, right after `timeline_chart_serde_roundtrips_fields` (ends `:1307`):

```rust
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
```

- [ ] **Step 6: Verify it compiles and the test passes**

Run: `cargo test sequence_chart_serde_roundtrips_fields`
Expected: PASS (and the whole crate compiles — the literal edits are complete).

- [ ] **Step 7: fmt + clippy + commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/session.rs src/export.rs
git commit -m "feat(session): sequence-tile Chart fields + CLI mapping"
```

---

### Task 2: `validate_tile` sequence branch + validation test + usage strings

**Files:**
- Modify: `src/session.rs` — `validate_tile` (add a branch after the timeline block, currently ends `:797`); the two usage strings (`:1120` and `:1125`).
- Test: `src/session.rs` `mod tests` — new `sequence_validation_requires_core_flags`.

**Interfaces:**
- Consumes: the `check(what, col)` closure (`src/session.rs:641`) and the `cols` vector already built in `validate_tile`.
- Produces: `validate_tile` rejects a sequence tile missing `--from`, `--to`, or `--label`, and rejects any named column that doesn't exist (with a "did you mean" hint via the existing `check`).

- [ ] **Step 1: Add the validation branch**

Insert immediately before the final `Ok(())` of `validate_tile` (currently `src/session.rs:798`):

```rust
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
```

- [ ] **Step 2: Write the validation test**

Add to `src/session.rs` `mod tests`, right after `timeline_validation_requires_core_flags` (ends `:1486`). It reuses the `duckdb_ok()` and `run_sql()` helpers already defined above it:

```rust
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
```

- [ ] **Step 3: Update the usage strings**

In `src/session.rs`, add `sequence` to the `--chart` kind list in the usage block. Change the `tile` line (`:1120`) so its `[--chart …]` reads:

```
[--chart bar|stacked|line|area|scatter|pie|table|heatmap|box|map|timeline|sequence]
```

And add a dedicated help line immediately after the `--chart timeline:` line (`:1125`), matching the surrounding `\n                       \` continuation style:

```
                 --chart sequence: --from COL --to COL --label COL (one row per message); optional --message-type sync|reply|async|lost, --from-type/--to-type participant|actor|database|boundary, --group 'kind:label', --group-branch COL, --autonumber\n                       \
```

- [ ] **Step 4: Run the tests**

Run: `cargo test sequence_validation_requires_core_flags`
Expected: PASS (or a skip line if `duckdb` isn't on PATH — it is in CI).

- [ ] **Step 5: fmt + clippy + commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
git add src/session.rs
git commit -m "feat(session): validate sequence tiles; usage strings"
```

---

### Task 3: Frontend SVG renderer + dispatch wiring + CSS

**Files:**
- Modify: `src/assets/index.html` — add renderer helpers (near the timeline block, after `hydrateTimelines`, ~`:3960`); dispatch in `zoomTile` (`:3021`, `:3030`) and `loadTileChart` (`:3548`); the `panelHtml` grip/widen gates (`:3494`, `:3503`); and CSS (near the `.tl-*` rules, ~`:513`).

**Interfaces:**
- Consumes: `esc`, `attr` (`:1261-1262`); `colFmt`, `applyFmtHtml`, `linkSubst` (used by `tlTip`, `:3663`); the `.wm-x` + `data-tip` delegated tooltip (`:5207`).
- Produces: `sequenceHtml(cols, rows, spec, db, table) -> string` (a complete `<svg>…</svg>` markup string; no hydrate needed), plus module-level helpers `seqType`, `seqGroup`, `seqTip`. These are also consumed by Task 4's `seqToMermaid`.

> **This task is screenshot-verified, not pixel-specified.** The code below is a complete, correct first cut; refine spacing/shape polish by rebuilding, restarting the daemon, and reading `muckdb session screenshot`. The e2e spec (Task 6) asserts *structure* (element counts, classes, tooltip text), which is the acceptance gate.

- [ ] **Step 1: Add the renderer helpers**

Insert after `hydrateTimelines(...)` closes (~`src/assets/index.html:3960`; find the line after that function's final `}`):

```js
  // ---- sequence diagram tile ------------------------------------------------
  // A hand-rolled SVG sequence diagram (microservice comms / UML). Layout is
  // intrinsic to the data — one column per participant, one row per message — so
  // sequenceHtml() returns a complete self-contained SVG string with no
  // responsive hydrate step. Tooltips use the delegated .wm-x/data-tip
  // mechanism; the mermaid-export button (see the click handler) regenerates
  // from the rows on demand.
  const SEQ_COL_W = 160, SEQ_MSG_H = 46, SEQ_SELF_H = 60, SEQ_HEAD_H = 64, SEQ_PAD = 26, SEQ_GPAD = 12;
  const SEQ_PART_TYPES = ["actor", "database", "boundary", "participant"];
  const SEQ_MSG_TYPES = ["sync", "reply", "async", "lost"];

  function seqType(v) {
    const t = (v == null ? "" : String(v)).trim().toLowerCase();
    return SEQ_MSG_TYPES.includes(t) ? t : "sync"; // blank/unknown → sync
  }
  // "kind:label" → {kind,label,raw}; an unrecognised/blank kind keeps the whole
  // value as the label with kind "group".
  function seqGroup(v) {
    if (v == null || String(v).trim() === "") return null;
    const s = String(v).trim(), i = s.indexOf(":");
    const kind = i > 0 ? s.slice(0, i).trim().toLowerCase() : "";
    const label = i > 0 ? s.slice(i + 1).trim() : s;
    return { kind: ["loop", "opt", "alt", "par"].includes(kind) ? kind : "group", label, raw: s };
  }
  // Participants in first-appearance order (from then to, scanning rows top to
  // bottom); each participant's type comes from the first row where it appears.
  function seqParticipants(cols, rows, spec) {
    const fi = cols.indexOf(spec.from_participant), ti = cols.indexOf(spec.to_participant);
    const fti = spec.from_type ? cols.indexOf(spec.from_type) : -1;
    const tti = spec.to_type ? cols.indexOf(spec.to_type) : -1;
    const order = [], typeOf = {};
    const seen = (n, tv) => {
      if (n === "") return;
      if (!order.includes(n)) order.push(n);
      if (typeOf[n] == null && tv != null && String(tv).trim() !== "") {
        const t = String(tv).trim().toLowerCase();
        if (SEQ_PART_TYPES.includes(t)) typeOf[n] = t;
      }
    };
    rows.forEach((r) => {
      seen(String(r[fi] ?? ""), fti >= 0 ? r[fti] : null);
      seen(String(r[ti] ?? ""), tti >= 0 ? r[tti] : null);
    });
    return { order, typeOf };
  }
  // Rich hover tooltip (mirrors tlTip): from → to, label, type, then every other
  // row column through its format/link.
  function seqTip(m, cols, rows, spec, db, table) {
    const row = rows[m.ri];
    const core = [
      `<b>${esc(m.label)}</b>`,
      `${esc(m.from)} → ${esc(m.to)}`,
      `type: ${esc(m.type)}`,
    ];
    const shown = new Set([spec.from_participant, spec.to_participant, spec.label, spec.message_type].filter(Boolean));
    const extra = [];
    cols.forEach((c, i) => {
      if (shown.has(c)) return;
      const v = row[i]; if (v === null || v === undefined || v === "") return;
      const f = colFmt(db, c, table);
      let disp = f ? applyFmtHtml(f, v) : esc(typeof v === "object" ? JSON.stringify(v) : String(v));
      if (f && f.link) {
        const href = linkSubst(f.link, v, row, cols, true);
        const txt = f.link_title ? esc(linkSubst(f.link_title, v, row, cols, false)) : disp;
        disp = `<a href="${attr(href)}" target="_blank" rel="noopener noreferrer">${txt}</a>`;
      }
      extra.push(`${esc(c)}: ${disp}`);
    });
    const extraHtml = extra.length ? `<div class="tl-tip-sep"></div>${extra.join("<br>")}` : "";
    return `${core.join("<br>")}${extraHtml}`;
  }
  // A small SVG arrowhead at (x,y) pointing in dir (+1 right, -1 left) for a
  // message of the given type. Filled triangle for sync/reply; open for async;
  // an X for lost (drawn at the line end instead of a head).
  function seqArrow(x, y, dir, type) {
    if (type === "lost") {
      const s = 4;
      return `<path class="seq-line" d="M${x - s},${y - s} L${x + s},${y + s} M${x + s},${y - s} L${x - s},${y + s}"/>`;
    }
    const w = 9 * dir, h = 4;
    if (type === "async") {
      return `<path class="seq-head-open" d="M${x - w},${y - h} L${x},${y} L${x - w},${y + h}"/>`;
    }
    return `<path class="seq-head" d="M${x},${y} L${x - w},${y - h} L${x - w},${y + h} Z"/>`;
  }

  function sequenceHtml(cols, rows, spec, db, table) {
    const fi = cols.indexOf(spec.from_participant), ti = cols.indexOf(spec.to_participant), lbi = cols.indexOf(spec.label);
    if (fi < 0 || ti < 0 || lbi < 0) return '<div class="note">sequence needs --from, --to and --label.</div>';
    const mti = spec.message_type ? cols.indexOf(spec.message_type) : -1;
    const gi = spec.group ? cols.indexOf(spec.group) : -1;
    const gbi = spec.group_branch ? cols.indexOf(spec.group_branch) : -1;

    const msgs = rows.map((r, ri) => ({
      ri,
      from: r[fi] == null ? "" : String(r[fi]),
      to: r[ti] == null ? "" : String(r[ti]),
      label: r[lbi] == null ? "" : String(r[lbi]),
      type: mti >= 0 ? seqType(r[mti]) : "sync",
      grp: gi >= 0 ? seqGroup(r[gi]) : null,
      branch: gbi >= 0 && r[gbi] != null ? String(r[gbi]) : null,
    })).filter((m) => m.from !== "" && m.to !== "");
    if (!msgs.length) return '<div class="note">no sequence rows.</div>';

    const { order, typeOf } = seqParticipants(cols, rows, spec);
    const parts = order.filter((n) => n !== "");
    const xOf = {};
    parts.forEach((p, i) => { xOf[p] = SEQ_PAD + i * SEQ_COL_W + SEQ_COL_W / 2; });

    // Per-message vertical slots (self-messages are taller); line drawn 22px
    // below the slot top, label 10px below the top.
    let cur = SEQ_HEAD_H + SEQ_PAD;
    const ys = msgs.map((m) => { const top = cur; cur += (m.from === m.to ? SEQ_SELF_H : SEQ_MSG_H); return top; });
    const bottomY = cur + 6;
    const W = SEQ_PAD * 2 + parts.length * SEQ_COL_W;
    const H = bottomY + SEQ_PAD;

    // Group frames: contiguous runs of equal grp.raw.
    let framesSvg = "";
    for (let i = 0; i < msgs.length;) {
      const g = msgs[i].grp;
      if (!g) { i++; continue; }
      let j = i;
      while (j < msgs.length && msgs[j].grp && msgs[j].grp.raw === g.raw) j++;
      let lo = Infinity, hi = -Infinity;
      for (let k = i; k < j; k++) {
        [msgs[k].from, msgs[k].to].forEach((p) => { const idx = parts.indexOf(p); if (idx >= 0) { lo = Math.min(lo, idx); hi = Math.max(hi, idx); } });
      }
      if (lo === Infinity) { i = j; continue; }
      const x1 = SEQ_PAD + lo * SEQ_COL_W + 10, x2 = SEQ_PAD + (hi + 1) * SEQ_COL_W - 10;
      const y1 = ys[i] - 4, y2 = (j < msgs.length ? ys[j] : bottomY) - 8;
      const tab = `${esc(g.kind)}${g.label ? " " + esc(g.label) : ""}`;
      const tabW = tab.length * 6 + 14;
      framesSvg += `<rect class="seq-frame" x="${x1}" y="${y1}" width="${x2 - x1}" height="${y2 - y1}" rx="3"/>`
        + `<path class="seq-frame-tab" d="M${x1},${y1} h${tabW} v12 l-6,6 h-${tabW - 6} Z"/>`
        + `<text class="seq-frame-lbl" x="${x1 + 6}" y="${y1 + 13}">${tab}</text>`;
      // Compartment dividers where --group-branch changes within the frame.
      for (let k = i + 1; k < j; k++) {
        if (msgs[k].branch && (msgs[k].branch || "") !== (msgs[k - 1].branch || "")) {
          const dy = ys[k] - 6;
          framesSvg += `<line class="seq-div" x1="${x1}" y1="${dy}" x2="${x2}" y2="${dy}"/>`
            + `<text class="seq-frame-lbl" x="${x1 + 6}" y="${dy - 3}">${esc(msgs[k].branch)}</text>`;
        }
      }
      i = j;
    }

    // Participant headers + lifelines.
    let headSvg = "", lifeSvg = "";
    parts.forEach((p) => {
      const x = xOf[p], ty = typeOf[p] || "participant";
      const lblY = SEQ_HEAD_H - 6;
      lifeSvg += `<line class="seq-life" x1="${x}" y1="${SEQ_HEAD_H + 2}" x2="${x}" y2="${bottomY}"/>`;
      if (ty === "actor") {
        headSvg += `<circle class="seq-part" cx="${x}" cy="16" r="7"/>`
          + `<path class="seq-line" d="M${x},23 v13 M${x - 9},29 h18 M${x},36 l-8,10 M${x},36 l8,10"/>`;
      } else if (ty === "database") {
        headSvg += `<path class="seq-part" d="M${x - 26},14 v22 a26,7 0 0 0 52,0 v-22"/>`
          + `<ellipse class="seq-part" cx="${x}" cy="14" rx="26" ry="7"/>`;
      } else if (ty === "boundary") {
        headSvg += `<line class="seq-line" x1="${x - 30}" y1="10" x2="${x - 30}" y2="42"/>`
          + `<line class="seq-line" x1="${x - 30}" y1="26" x2="${x - 14}" y2="26"/>`
          + `<circle class="seq-part" cx="${x}" cy="26" r="14"/>`;
      } else {
        const bw = Math.min(SEQ_COL_W - 24, Math.max(70, p.length * 8 + 16));
        headSvg += `<rect class="seq-part" x="${x - bw / 2}" y="10" width="${bw}" height="30" rx="4"/>`;
      }
      headSvg += `<text class="seq-part-lbl" x="${x}" y="${lblY}" text-anchor="middle">${esc(p.length > 20 ? p.slice(0, 19) + "…" : p)}</text>`;
    });

    // Messages.
    let msgSvg = "";
    msgs.forEach((m, k) => {
      const ly = ys[k] + 22, lty = ys[k] + 12;
      const tip = attr(seqTip(m, cols, rows, spec, db, table));
      const num = spec.autonumber ? k + 1 : null;
      if (m.from === m.to) {
        const x = xOf[m.from];
        msgSvg += `<path class="seq-line${m.type === "reply" ? " reply" : ""}" d="M${x},${ly} h34 v16 h-34"/>`
          + seqArrow(x, ly + 16, -1, m.type)
          + `<text class="seq-msg-lbl" x="${x + 40}" y="${ly + 6}">${esc(m.label)}</text>`;
        if (num != null) msgSvg += `<circle class="seq-num" cx="${x - 12}" cy="${ly}" r="8"/><text class="seq-num-txt" x="${x - 12}" y="${ly + 3}" text-anchor="middle">${num}</text>`;
        msgSvg += `<rect class="seq-hit wm-x" x="${x - 6}" y="${ys[k]}" width="90" height="${SEQ_SELF_H}" data-tip="${tip}"/>`;
      } else {
        const x1 = xOf[m.from], x2 = xOf[m.to], dir = x2 > x1 ? 1 : -1;
        const ex = x2 - dir * 1; // stop at the lifeline
        msgSvg += `<line class="seq-line${m.type === "reply" ? " reply" : ""}" x1="${x1}" y1="${ly}" x2="${ex}" y2="${ly}"/>`
          + seqArrow(ex, ly, dir, m.type)
          + `<text class="seq-msg-lbl" x="${(x1 + x2) / 2}" y="${lty}" text-anchor="middle">${esc(m.label)}</text>`;
        if (num != null) msgSvg += `<circle class="seq-num" cx="${x1 + dir * 12}" cy="${ly}" r="8"/><text class="seq-num-txt" x="${x1 + dir * 12}" y="${ly + 3}" text-anchor="middle">${num}</text>`;
        const hx = Math.min(x1, x2), hw = Math.abs(x2 - x1);
        msgSvg += `<rect class="seq-hit wm-x" x="${hx}" y="${ys[k]}" width="${hw}" height="${SEQ_MSG_H}" data-tip="${tip}"/>`;
      }
    });

    return `<div class="seq-wrap"><svg class="seq-svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img">`
      + framesSvg + lifeSvg + headSvg + msgSvg + `</svg></div>`;
  }
```

- [ ] **Step 2: Add the CSS**

Insert near the timeline CSS (after the `.tl-*` rules, ~`src/assets/index.html:521`):

```css
  .seq-wrap { overflow-x: auto; padding: 4px 0; }
  .seq-svg { display: block; max-width: 100%; height: auto; font-family: inherit; }
  .seq-part { fill: var(--raised); stroke: var(--line); stroke-width: 1.2; }
  .seq-part-lbl { fill: var(--fg); font-size: 12px; }
  .seq-life { stroke: var(--line); stroke-width: 1; stroke-dasharray: 3 4; }
  .seq-line { stroke: var(--fg); stroke-width: 1.4; fill: none; }
  .seq-line.reply { stroke-dasharray: 5 4; }
  .seq-head { fill: var(--fg); stroke: none; }
  .seq-head-open { stroke: var(--fg); stroke-width: 1.4; fill: none; }
  .seq-msg-lbl { fill: var(--fg); font-size: 11px; }
  .seq-num { fill: var(--teal); }
  .seq-num-txt { fill: var(--on-accent); font-size: 9px; }
  .seq-frame { fill: none; stroke: var(--muted); stroke-width: 1; stroke-dasharray: 4 3; }
  .seq-frame-tab { fill: var(--surface); stroke: var(--muted); stroke-width: 1; }
  .seq-frame-lbl { fill: var(--muted); font-size: 10px; font-weight: 600; }
  .seq-div { stroke: var(--muted); stroke-width: 1; stroke-dasharray: 4 3; }
  .seq-hit { fill: transparent; }
```

- [ ] **Step 3: Wire the three dispatch points**

In `zoomTile`, extend the `fit`-class gate (`:3021`) to include sequence:

```js
      if (kind === "heatmap" || kind === "box" || kind === "timeline" || kind === "sequence") layer.querySelector(".zoom-box").classList.add("fit");
```

Add a dispatch line right after the timeline one (`:3030`):

```js
      if (kind === "sequence") { slot.innerHTML = sequenceHtml(cols, rows, t.chart || {}, t.db, t.view || null); return; }
```

In `loadTileChart`, add after the timeline line (`:3548`):

```js
    if (kind === "sequence") { slot.style.height = "auto"; slot.innerHTML = sequenceHtml(cols, rows, t.chart || {}, t.db, t.view || null); return; }
```

- [ ] **Step 4: Extend the panelHtml gates**

Add `sequence` to the full-width toggle gate (`:3494`) and the no-grip gate (`:3503`):

```js
    const widenBtn = kind === "table" || kind === "timeline" || kind === "sequence"
```
```js
    const grip = kind === "heatmap" || kind === "box" || kind === "map" || kind === "timeline" || kind === "sequence" ? "" : gripHtml(t.name);
```

- [ ] **Step 5: Build, restart the daemon, screenshot, and eyeball it**

```bash
cargo build
./target/debug/muckdb --stop 2>/dev/null; ./target/debug/muckdb --status >/dev/null 2>&1 || true
# Seed a quick throwaway session against a durable db (see Task 6's SQL for a fuller one):
DB=~/.local/share/muckdb/data/seqdemo.duckdb
./target/debug/muckdb "$DB" -c "CREATE OR REPLACE VIEW calls AS SELECT * FROM (VALUES
  ('user','gateway','GET /orders','sync','actor','participant', NULL, NULL),
  ('gateway','auth','verify token','sync','participant','boundary','alt:auth ok','ok'),
  ('auth','gateway','200 ok','reply','boundary','participant','alt:auth ok','ok'),
  ('gateway','orders','fetch','sync','participant','participant','alt:auth ok','ok'),
  ('orders','db','SELECT','sync','participant','database','alt:auth ok','ok'),
  ('gateway','user','401','reply','participant','actor','alt:auth ok','denied'),
  ('orders','orders','retry','async','participant','participant',NULL,NULL)
) t(src,dst,msg,mtype,st,dt,grp,branch);"
./target/debug/muckdb session create seqdemo
./target/debug/muckdb session tile seqdemo --name seq --db "$DB" --view calls --chart sequence \
  --from src --to dst --label msg --message-type mtype --from-type st --to-type dt \
  --group grp --group-branch branch --autonumber --caption "demo"
./target/debug/muckdb session screenshot seqdemo --tile seq --out /tmp/seq.png
```

Read `/tmp/seq.png`. Verify: 4 participant shapes drawn (actor, box, boundary, database) with lifelines; sync arrows filled, reply dashed, async open, self-message loop; an `alt` frame with an `else`/denied divider; autonumber badges. Adjust the geometry constants / shape paths until it reads cleanly. (This is the visual-iteration step — the numbers above are a good starting point, not gospel.)

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/assets/index.html
git commit -m "feat(web): sequence diagram tile renderer (hand-rolled SVG)"
```

---

### Task 4: Mermaid export — button + generator + click handler

**Files:**
- Modify: `src/assets/index.html` — `seqToMermaid()` (beside the sequence helpers); the export button in `panelHtml` (`:3505-3506`); a delegated `data-mermaid` branch in the click handler (~`:3548`... the click listener at `:5235`).

**Interfaces:**
- Consumes: `seqType`, `seqGroup`, `seqParticipants` (Task 3); `toast` (`:3285`); `sessionTile`, `ensureFormats`; the `/api/query` endpoint.
- Produces: `seqToMermaid(cols, rows, spec) -> string` (a valid mermaid `sequenceDiagram` document).

- [ ] **Step 1: Add the mermaid generator**

Insert beside the Task 3 helpers (after `sequenceHtml`):

```js
  // A mermaid-safe participant id, plus whether an "as <original>" alias is needed.
  function seqMermaidId(name) {
    const id = name.replace(/[^A-Za-z0-9_]/g, "_").replace(/^_+|_+$/g, "") || "p";
    return { id, alias: id !== name };
  }
  // Generate a valid mermaid `sequenceDiagram` document from the same rows/spec.
  // db/boundary participants map to `participant` with a preceding %% comment
  // (mermaid has no such shape); arrows map sync/reply/async/lost →
  // ->>/-->>/-)/-x; single-level groups → loop/opt/alt/par … end.
  function seqToMermaid(cols, rows, spec) {
    const fi = cols.indexOf(spec.from_participant), ti = cols.indexOf(spec.to_participant), lbi = cols.indexOf(spec.label);
    if (fi < 0 || ti < 0 || lbi < 0) return "sequenceDiagram\n";
    const mti = spec.message_type ? cols.indexOf(spec.message_type) : -1;
    const gi = spec.group ? cols.indexOf(spec.group) : -1;
    const gbi = spec.group_branch ? cols.indexOf(spec.group_branch) : -1;
    const arrow = { sync: "->>", reply: "-->>", async: "-)", lost: "-x" };

    const lines = ["sequenceDiagram"];
    if (spec.autonumber) lines.push("  autonumber");

    const { order, typeOf } = seqParticipants(cols, rows, spec);
    const idOf = {};
    order.filter((n) => n !== "").forEach((n) => {
      const { id, alias } = seqMermaidId(n);
      idOf[n] = id;
      const ty = typeOf[n] || "participant";
      if (ty === "database") lines.push("  %% database");
      else if (ty === "boundary") lines.push("  %% boundary");
      const kw = ty === "actor" ? "actor" : "participant";
      lines.push(alias ? `  ${kw} ${id} as ${n}` : `  ${kw} ${id}`);
    });

    let curRaw = null, curKind = null, curBranch = null;
    const close = () => { if (curRaw) { lines.push("  end"); curRaw = null; curKind = null; curBranch = null; } };
    rows.forEach((r) => {
      const from = String(r[fi] ?? ""), to = String(r[ti] ?? "");
      if (from === "" || to === "") return;
      const raw = gi >= 0 && r[gi] != null && String(r[gi]).trim() !== "" ? String(r[gi]).trim() : null;
      if (raw !== curRaw) {
        close();
        if (raw) {
          const g = seqGroup(raw);
          curKind = g.kind === "group" ? "loop" : g.kind; // unknown kind → a loop frame
          lines.push(`  ${curKind} ${g.label}`);
          curRaw = raw;
          curBranch = gbi >= 0 && r[gbi] != null ? String(r[gbi]) : null;
        }
      } else if (curRaw && gbi >= 0) {
        const br = r[gbi] != null ? String(r[gbi]) : null;
        if (br && br !== curBranch) {
          lines.push(`  ${curKind === "par" ? "and" : "else"} ${br}`);
          curBranch = br;
        }
      }
      const ty = mti >= 0 ? seqType(r[mti]) : "sync";
      const indent = curRaw ? "    " : "  ";
      const label = String(r[lbi] ?? "").replace(/\s*\n\s*/g, " ");
      lines.push(`${indent}${idOf[from] || seqMermaidId(from).id}${arrow[ty]}${idOf[to] || seqMermaidId(to).id}: ${label}`);
    });
    close();
    return lines.join("\n") + "\n";
  }
```

- [ ] **Step 2: Add the toolbar button**

In `panelHtml`, add a mermaid button (gated to sequence tiles) and place it in the actions row. After the `mapToggle` line (`:3499`), add:

```js
    const mermaidBtn = kind === "sequence" ? `<button class="panel-act" data-mermaid="${attr(t.name)}" title="Copy as mermaid.js">mermaid</button>` : "";
```

Then include `${mermaidBtn}` in the `panel-actions` span (`:3506`), right after `${sqlBtn}`:

```js
        <span class="panel-actions">${mapToggle}${sqlBtn}${mermaidBtn}${widenBtn}${contractBtn}${copyImgBtn(t.name)}${zoomBtn}${explore}${trashBtn(t.name)}</span></div>
```

- [ ] **Step 3: Handle the click**

In the delegated click listener (`:5235`), add a branch alongside the other `data-*` branches (e.g. right after the `data-copyimg` branch at `:5251`):

```js
    const mmd = ev.target.closest("[data-mermaid]");
    if (mmd) {
      const t = sessionTile(mmd.dataset.mermaid);
      if (!t || t.type !== "view") return;
      const sql = t.sql || `SELECT * FROM "${(t.view || "").replace(/"/g, '""')}" LIMIT 5000`;
      fetch("/api/query?" + new URLSearchParams({ db: t.db, sql }))
        .then((r) => r.json())
        .then((data) => {
          if (data.error) throw new Error(data.error);
          const text = seqToMermaid(data.columns || [], data.rows || [], t.chart || {});
          const done = () => toast("mermaid copied to clipboard");
          if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).then(done).catch(() => toast("copy failed", true));
          else { const ta = document.createElement("textarea"); ta.value = text; document.body.appendChild(ta); ta.select(); try { document.execCommand("copy"); done(); } catch (_) { toast("copy failed", true); } ta.remove(); }
        })
        .catch((e) => toast(String(e), true));
      return;
    }
```

- [ ] **Step 4: Build, restart, verify by hand**

```bash
cargo build && ./target/debug/muckdb --stop 2>/dev/null; true
```
Reload `http://localhost:11000/session/seqdemo/`, click **mermaid** on the sequence panel, paste the clipboard into <https://mermaid.live> and confirm it renders (participants incl. the `%% database`/`%% boundary` comments, arrows, the `alt … else … end` frame, `autonumber`). This is the manual acceptance; the automated check is in Task 6.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/assets/index.html
git commit -m "feat(web): export sequence tile as mermaid.js (copy button)"
```

---

### Task 5: Documentation — SKILL.md + CLAUDE.md

**Files:**
- Modify: `src/assets/skill/SKILL.md` — `session tile` usage block (~`:187`), the kind list (~`:248`), and a new `## Sequence tiles` section (model it on `## Timeline tiles`, ~`:338`).
- Modify: `CLAUDE.md` — the usage block (~`:71`) and the kind list (~`:115`).

**Interfaces:** none (docs only). Must match the flags/vocabulary defined in Tasks 1–4 exactly.

- [ ] **Step 1: Add `sequence` to the command reference**

In both `SKILL.md` and `CLAUDE.md`, add `sequence` to the `[--chart …]` kind list wherever the full list appears, and add these flag lines to the `session tile` reference block (matching the surrounding style):

```
        [--chart sequence]  (sequence diagram — service comms; one row per message)
        [--from COL] [--to COL]  (sequence: source/destination participant; --from == --to is a self-message; message text = --label)
        [--message-type COL]  (sequence: sync (default) | reply | async | lost)
        [--from-type COL] [--to-type COL]  (sequence: participant (default) | actor | database | boundary)
        [--group COL]  (sequence: 'kind:label' — loop|opt|alt|par; contiguous equal values = one frame)
        [--group-branch COL]  (sequence: else/and compartment label within a frame)
        [--autonumber]  (sequence: number the messages)
```

- [ ] **Step 2: Add the `## Sequence tiles` section to SKILL.md**

Add after the `## Timeline tiles` section. Include, in prose + a worked example:

- **Lead sentence:** sequence tiles show **interactions between microservices** (request/response, async fan-out, retries, fallbacks) — one row per message.
- The flag table (from/to/label required; message-type/from-type/to-type/group/group-branch/autonumber optional) with the two vocabularies.
- Message order = row order (`ORDER BY`); participant order = first appearance; participant type = the type on the row where it first appears.
- The `--group` `kind:label` grammar, compartments via `--group-branch`, single-level only.
- The **mermaid export** button and the mapping caveat: mermaid has no database/boundary shape, so those export as `participant` with a `%% database`/`%% boundary` comment; `actor` exports as `actor`.
- A concrete example:

````markdown
```sh
muckdb ~/data/trace.duckdb -c "
  CREATE OR REPLACE VIEW login_flow AS SELECT * FROM (VALUES
    (1,'user','gateway','GET /orders','sync','actor','participant',NULL,NULL),
    (2,'gateway','auth','verify','sync','participant','boundary','alt:token valid','valid'),
    (3,'auth','db','SELECT session','sync','boundary','database','alt:token valid','valid'),
    (4,'gateway','orders','list orders','sync','participant','participant','alt:token valid','valid'),
    (5,'gateway','user','401','reply','participant','actor','alt:token valid','expired')
  ) t(seq,src,dst,msg,mtype,st,dt,grp,branch)
  ORDER BY seq;"

muckdb session tile trace --name login --db ~/data/trace.duckdb --view login_flow \
  --chart sequence --from src --to dst --label msg --message-type mtype \
  --from-type st --to-type dt --group grp --group-branch branch --autonumber \
  --caption "Login flow across gateway/auth/orders — click 'mermaid' to export."
```
````

- [ ] **Step 3: Commit**

```bash
git add src/assets/skill/SKILL.md CLAUDE.md
git commit -m "docs: sequence diagram tile (skill + CLAUDE)"
```

> Note (asset cache-busting): changes are inside `index.html` itself, not a separately-fetched asset, so no `?v=` bump is needed. `SKILL.md` is served fresh.

---

### Task 6: E2E — seed fixture + `sequence.spec.ts`

**Files:**
- Modify: `tests/e2e/fixtures/seed.ts` — add a `messages` view to `CREATE_SQL` and post a `sequence` tile (with a `--link` format for tooltip-link coverage and a hostile value for XSS coverage).
- Create: `tests/e2e/specs/sequence.spec.ts`.

**Interfaces:**
- Consumes: `SESSION_ID` (`tests/e2e/constants.ts`), the seeded `e2e` session, `run(binary, env, args)` in `seed.ts`.
- Produces: a `sequence` tile named `sequence` in the `e2e` session.

- [ ] **Step 1: Extend the seed fixture**

In `tests/e2e/fixtures/seed.ts`, append to `CREATE_SQL` (after the `ts_timeline` view, before the closing backtick at `:52`):

```sql
-- Sequence diagram fixture: one row per message, each participant type, all four
-- arrow kinds, a self-message, and an alt/else group. `trace` carries a --link
-- format (tooltip-link coverage); `note` carries a hostile value (XSS coverage).
CREATE VIEW messages AS SELECT * FROM (VALUES
  (1,'user','gateway','GET /orders','sync','actor','participant',NULL,NULL,'t-1','<img src=x onerror=alert(1)>'),
  (2,'gateway','auth','verify','sync','participant','boundary','alt:token valid','valid','t-2','ok'),
  (3,'auth','db','SELECT session','sync','boundary','database','alt:token valid','valid','t-3','ok'),
  (4,'gateway','user','401','reply','participant','actor','alt:token valid','expired','t-4','denied'),
  (5,'orders','orders','retry','async','participant','participant',NULL,NULL,'t-5','backoff'),
  (6,'gateway','cache','ping','lost','participant','participant',NULL,NULL,'t-6','timeout')
) t(seq,src,dst,msg,mtype,st,dt,grp,branch,trace,note)
ORDER BY seq;
```

Then in `seed()`, after the timeline tiles (after `:103`), add a `--link` format on `trace` and post the tile:

```ts
  run(binary, env, [
    'format', dbPath, 'trace', '--table', 'messages',
    '--link', 'https://trace.example.test/{value}',
  ]);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'sequence', '--title', 'Service comms',
    '--db', dbPath, '--view', 'messages', '--chart', 'sequence',
    '--from', 'src', '--to', 'dst', '--label', 'msg', '--message-type', 'mtype',
    '--from-type', 'st', '--to-type', 'dt', '--group', 'grp', '--group-branch', 'branch',
    '--autonumber',
    '--caption', 'A sequence diagram: participant types, arrow kinds, a self-message, an alt group.']);
```

- [ ] **Step 2: Write the spec**

Create `tests/e2e/specs/sequence.spec.ts`:

```ts
import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('sequence tile', () => {
  test('renders participants, lifelines, messages, a self-message and a group frame', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="sequence"]');
    await expect(panel).toBeVisible();

    // Five participants (user, gateway, auth, db, orders, cache) → a lifeline each.
    // (user, gateway, auth, db, orders, cache = 6.)
    await expect(panel.locator('.seq-life')).toHaveCount(6);

    // Six messages → six hit areas.
    await expect(panel.locator('.seq-hit')).toHaveCount(6);

    // Different arrow styles are present: at least one dashed (reply) line.
    await expect(panel.locator('.seq-line.reply').first()).toBeVisible();

    // The alt group frame + its else/expired compartment divider.
    await expect(panel.locator('.seq-frame').first()).toBeVisible();
    await expect(panel.locator('.seq-div').first()).toBeVisible();

    // Autonumber badges.
    await expect(panel.locator('.seq-num').first()).toBeVisible();

    // Full-width toggle offered.
    await expect(panel.locator('[data-widen]')).toHaveCount(1);
  });

  test('message hover shows a rich tooltip with core fields + a formatted link, escaping HTML', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="sequence"]');
    await expect(panel).toBeVisible();
    // Hover the first message's hit area.
    await panel.locator('.seq-hit').first().hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toContainText('user → gateway');
    await expect(tip).toContainText('type: sync');
    // The `trace` column has a --link format → a clickable link in the tooltip.
    await expect(tip.locator('a[href="https://trace.example.test/t-1"]')).toBeVisible();
    // The hostile `note` value is shown as text, never parsed as an element.
    await expect(tip.locator('img')).toHaveCount(0);
    await expect(tip).toContainText('onerror');
  });

  test('the mermaid button copies a valid sequenceDiagram to the clipboard', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="sequence"]');
    await expect(panel).toBeVisible();
    await panel.locator('[data-mermaid]').click();
    // The toast confirms the copy.
    await expect(page.locator('#toast')).toContainText('mermaid');
    const text = await page.evaluate(() => navigator.clipboard.readText());
    expect(text).toContain('sequenceDiagram');
    expect(text).toContain('autonumber');
    expect(text).toContain('->>');       // a sync arrow
    expect(text).toContain('-->>');      // a reply arrow
    expect(text).toContain('%% database'); // db participant annotation
    expect(text).toMatch(/\n\s*alt token valid/); // the group frame
    expect(text).toMatch(/\n\s*end/);
  });
});
```

- [ ] **Step 3: Run the e2e suite**

```bash
cd tests/e2e && npm test -- sequence.spec.ts
```
Expected: all three tests PASS. (If clipboard-read is flaky in the runner, the toast assertion is the primary gate; keep the clipboard content assertions.)

- [ ] **Step 4: Run the full suite to confirm nothing regressed**

```bash
cd tests/e2e && npm test
```
Expected: the full suite (existing specs + the new one) passes.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/fixtures/seed.ts tests/e2e/specs/sequence.spec.ts
git commit -m "test(e2e): sequence tile — render, tooltip, mermaid export"
```

---

## Self-Review

**1. Spec coverage** — every spec section maps to a task:
- One-view/one-row-per-message model + all CLI flags → Task 1. ✔
- Validation (required from/to/label, column existence) → Task 2. ✔
- Participant types (participant/actor/database/boundary) + inference/order → Task 3 (`seqParticipants`, header shapes). ✔
- Message arrow types (sync/reply/async/lost) + self-messages → Task 3 (`seqType`, `seqArrow`, self-loop). ✔
- Groups (`kind:label`, single-level, compartments) → Task 3 (frames) + Task 4 (mermaid) + Task 2 (validation). ✔
- Autonumber → Tasks 1/3/4. ✔
- Rich tooltip with formats/links + XSS escaping → Task 3 (`seqTip`) + Task 6 (test). ✔
- Mermaid export button + mapping (arrows, actor/participant, `%% database`/`%% boundary`, loop/opt/alt/par + else/and) → Task 4. ✔
- Docs incl. "shows interactions between microservices" → Task 5. ✔
- Tests (serde, validation, e2e render/tooltip/export) → Tasks 1, 2, 6. ✔
- Deferred (activations, notes, nesting, mermaid import) → not implemented (correct). ✔

**2. Placeholder scan** — no TBD/TODO; every code step carries complete code. Task 3 is explicitly flagged as screenshot-refined for pixel polish, with e2e structural assertions as the hard gate — the code given is complete and runnable, not a stub.

**3. Type/name consistency** — Chart field names (`from_participant`, `to_participant`, `message_type`, `from_type`, `to_type`, `group`, `group_branch`, `autonumber`) are identical across Task 1 (Rust), and the frontend reads them as `spec.*` in Tasks 3–4. CLI flags (`--from`, `--to`, `--message-type`, `--from-type`, `--to-type`, `--group`, `--group-branch`, `--autonumber`) are consistent across Tasks 1, 2, 5, 6. `seqType`/`seqGroup`/`seqParticipants` are defined in Task 3 and consumed in Task 4. Arrow mapping (`->>`/`-->>`/`-)`/`-x`) matches the Global Constraints. ✔

## Notes for the executor

- **Model selection:** Task 1 is near-transcription (cheapest tier ok). Tasks 2 and 4–6 are standard-tier. Task 3 needs judgment + browser iteration (standard tier, and expect a screenshot loop).
- **Do not `git add -A`.** `FUTURE_DIRECTIONS.md` is a private untracked doc — stage files explicitly as shown.
- **Do not push or release.** Stop after Task 6 + the final review; the human will say when to ship (then: merge to `main` + `cargo release patch --execute`).
- After all tasks: run the full gate (`cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`, then the e2e suite) before the final whole-branch review.
