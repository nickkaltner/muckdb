# Timeline (Gantt-like) tile — design

**Status:** approved design, pending implementation plan.
**Do not push or release** until the user gives the word.

## Goal

Add a new session chart kind, `timeline`, that draws events as horizontal
rectangles ("bars") over a shared time axis, arranged into labelled horizontal
lanes. It is muckdb's answer to Gantt charts, incident timelines, OpenTelemetry
trace views, and investigation sequencing.

### Target use cases (drive the design + demo + skill docs)

- **Resource allocation** — lanes are resources (machines, people, rooms); bars
  are the tasks each resource is busy with over time.
- **Incident timeline** — lanes are systems/actors; bars are phases (detection,
  triage, mitigation); `--event` markers flag key moments (alert fired, deploy,
  resolved).
- **OpenTelemetry trace** — lanes are services; bars are spans; `--depends-on`
  draws parent→child span causality; the rich tooltip surfaces span attributes.
- **Investigation sequencing** — lanes are workstreams; bars are steps; overlaps
  stack into sublanes so concurrent work is visible.

## Data model

Each row of the tile's view/SQL is **one bar**. Column roles are named by
explicit flags, matching how the `map` tile named `--lat`/`--lon`/`--from-lat`.

| Flag | Required | Meaning |
|:-----|:--------:|:--------|
| `--lane COL` | yes | Vertical row / resource label the bar belongs to |
| `--label COL` | yes | Text drawn in the bar |
| `--start COL` | yes | Bar start: numeric (relative seconds) **or** timestamp/date |
| `--end COL` | one of end/duration | Bar end (same type as start) |
| `--duration COL` | one of end/duration | Numeric seconds; `end = start + duration` |
| `--color COL` | no | Colour bars by this **category** value (adds a legend) |
| `--id COL` | no | Unique bar id (enables dependencies) |
| `--depends-on COL` | no | Comma-separated parent id(s) this bar depends on |
| `--event 'X\|label'` | no, repeatable | Dashed vertical marker at a time (reuses existing marker infra) |
| `--title` / `--caption` / `--xlabel` | no | As other charts |

### Time axis: auto-detected, not flagged

- If `--start`/`--end` columns are **numeric** → a **relative** axis starting at
  the data's min (typically `0s`), tick labels humanised (`0s`, `30s`, `2m 10s`,
  `1h 05m`). This is the "seconds from start" mode.
- If they are **timestamp/date** → an **absolute** UTC time axis, reusing the
  existing time-axis code (`parseTs`, granularity-aware ticks) and honouring any
  `--tz` column format set via `muckdb format`.
- Mixed/ambiguous types fail validation with a clear message.

### Colouring

- **Default (no `--color`):** each lane gets its own palette colour (via
  `catColors`), so a lane reads as one colour band — the "colours from the theme"
  default.
- **With `--color COL`:** bars are coloured by that column's category value
  (distinct value → palette colour), with a legend row. Colour then encodes the
  category (e.g. status `ok`/`failed`/`pending`), not the lane. This is the
  user's chosen behaviour and supersedes per-lane override.

### Dependencies

- `--id` + `--depends-on` are both optional; omit them and no connectors draw.
- `--depends-on` holds zero or more parent ids, comma-separated. For each parent,
  an **orthogonal (right-angle)** connector is routed from the parent bar's right
  edge to this bar's left edge: a short horizontal stub → vertical run → short
  horizontal stub into the target. Lines are thin and semi-transparent
  (theme-driven). Multiple parents → multiple connectors. A `--depends-on` id
  that matches no `--id` in the data is ignored (no line), not an error.

### Hover / interaction

- **Vertical cursor line** follows the mouse across the plot, with a floating
  readout of the time at that x-position.
- **Bottom axis** always shows the full time range with ticks (the tile fits the
  whole range to its width in v1 — see Non-goals).
- **Box tooltip** on hovering a bar shows: `label`, `lane`, `start → end`,
  computed `duration`, the `--color` category if set, then **every other column
  in the row**, each formatted via muckdb's column formats and rendered as a link
  where a `--link` format is set. (Rich tooltip = the user's choice.)
- Tooltips reuse the existing delegated `.wm-x` + `data-tip` mechanism (or a
  parallel delegated handler in the same style).

### Sublanes (overlap handling)

Within a lane, bars whose time ranges overlap are packed into **sublanes**
(stacked rows) by a greedy interval-packing pass, so no two bars overlap
visually. A lane's height grows to fit its number of sublanes. Lane order is the
**order of first appearance** in the view's rows (control it with `ORDER BY`,
consistent with heatmap/box axes).

## Implementation

### Rust — `src/session.rs`

- Extend the `Chart` struct (around line 29) with new optional fields, all
  `Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`:
  `lane`, `start`, `end`, `duration`, `color`, `id`, `depends_on`. Reuse existing
  `label`, `events`, `xlabel`, `title`, `caption`.
- Wire the new flags into the `tile` action's `Chart { ... }` constructor
  (around line 860). Flag→field: `--depends-on` → `depends_on`.
- Add a `chart.kind == "timeline"` block to `validate_tile` (after the box block,
  ~line 692): require `--lane`, `--label`, `--start`, and **exactly one** of
  `--end`/`--duration`; `check()` each named column (incl. `--color`, `--id`,
  `--depends-on`) so typos get the "did you mean" suggestion; verify start/end (or
  duration) column types are consistent (both numeric, or both temporal).
- Update the help/usage string (~line 1008) and the chart-kind literal lists.
- Tests: serde roundtrip for a timeline chart (pattern at ~line 1057/1090);
  validation tests (missing required flag; both `--end` and `--duration`; neither;
  typo suggestion).

### Frontend — `src/assets/index.html`

- New renderer `timelineHtml(cols, rows, spec, db, table)` — hand-rolled SVG in
  the style of `boxTileHtml`/`mapHtml` (Chart.js cannot do sublanes + orthogonal
  dependency routing + a custom hover cursor).
- Dispatch it from the three parallel switch sites: `loadTileChart` (~3422),
  `zoomTile` (~2939), `panelHtml` (~3359). In `panelHtml`, add `timeline` to the
  grip-less list (like box/map) **and** to the `--widen` full-width gate (~3384)
  so Gantt charts can break out to full width like tables.
- Renderer responsibilities:
  - Compute the global time domain across all bars; build a shared x-scale.
  - Left gutter of lane labels; per-lane greedy sublane packing.
  - Bars as rounded rects; colour by lane (default) or `catColors` by `--color`
    category (with a legend); label clipped inside the bar.
  - Orthogonal dependency connectors (thin, theme-opacity).
  - `--event` markers as dashed vertical lines with a top label.
  - Mouse-following vertical cursor + time readout; bottom time axis with ticks.
  - Rich box tooltip (core fields + all extra columns) via `colFmt`/`applyFmt`
    and the `.wm-x`/`data-tip` pattern; honour `--link` formats.
  - Re-render on resize (ResizeObserver) so the SVG re-fits, like the map.
- Theme: add theme keys with defaults for dependency-line opacity and bar corner
  radius; pull bar/marker/cursor colours from CSS vars like the other custom
  charts. Bars/markers must re-render on theme change (open panels already do).

### Full-width

Add `"timeline"` to the widen gate at `index.html:3384`; the existing
`.panel.full-bleed` machinery then works unchanged.

## Demo — `demo.sh`

- Seed a small timeline-shaped table/view. Use a scenario that exercises the
  features: several lanes, overlapping bars (→ sublanes), a `--color` category,
  a couple of `--depends-on` links, and one or two `--event` markers.
  Recommended: a **deploy/CI pipeline or incident timeline** on a relative-seconds
  axis, plus (optionally) a second timeline on an absolute-timestamp axis to show
  both time modes.
- Add a new section (e.g. `sec-timeline` "Timeline") and post **a few** timeline
  tiles under it (at least: one relative-seconds Gantt with dependencies +
  sublanes, and one absolute-time incident timeline with `--event` markers and
  `--color` by status), each with a `--caption` explaining what it shows.
- Add the timeline row(s) to the closing summary markdown table so the dashboard
  still reads top-to-bottom.

## Installed skill — `src/assets/skill/SKILL.md`

- Add `timeline` to the chart-kind list in the command reference (~line 187) and
  to the prose chart-kind list (~line 244).
- Add the flag rows to the `session tile` option block (`--lane`, `--start`,
  `--end`/`--duration`, `--color`, `--id`, `--depends-on`).
- Add a dedicated **`timeline`** subsection under "Pick the chart that packs in
  the most information" that spells out, **in detail**:
  - **What it is** and when to reach for it.
  - **The four use cases** (resource allocation, incident timeline, otel trace,
    investigation sequencing) with a one-line "shape your view like this" for each.
  - **Every option** and its meaning (the table above), including the
    auto-detected relative-seconds vs absolute-time axis, the default per-lane
    colouring vs `--color`-by-category, the `--id`/`--depends-on` dependency
    model (comma-separated multiple parents), sublane overlap behaviour, `--event`
    markers, the rich hover tooltip, and full-width support.
  - **A worked example** command per use case (copy-pasteable), each showing how
    to shape the view (`GROUP BY`/`ORDER BY` for lane order) and set column
    formats.
- Mirror the same detail into the repo `AGENT.md` chart docs where the other
  kinds are documented, so both stay consistent.

## Tests

- **Rust unit:** serde roundtrip + validation cases (above).
- **E2E (Playwright):** seed a timeline tile in `tests/e2e/fixtures/seed.ts`
  (a spans view with an overlap, a dependency, a `--color` category, a marker);
  add `tests/e2e/specs/timeline.spec.ts` asserting: lanes render with labels,
  overlapping bars stack into sublanes, a dependency connector exists, an
  `--event` marker line exists, and hovering a bar shows the rich tooltip.
  Follow the thorough `map.spec.ts` pattern.

## Non-goals (v1)

- **No interactive zoom/pan** on the time axis. v1 fits the full range to the
  tile width; use the full-width toggle and the existing expand overlay for more
  room. Pan/zoom (important for very dense otel traces) is a candidate v2.
- No editing of bars in the UI (read-only, like every other tile).
- No cross-tile dependency links (dependencies are within a single tile's data).

## Definition of done

- `cargo fmt` clean, `cargo clippy --all-targets -- -D warnings` clean,
  `cargo test` green (per `AGENT.md` CI rules).
- E2E timeline spec passes.
- `demo.sh` produces the timeline section; `muckdb session screenshot demo`
  looks right (verified by reading the PNG).
- SKILL.md + AGENT.md document the tile in detail with worked examples.
- **Not** pushed or released — await the user's word.
