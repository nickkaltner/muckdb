---
name: muckdb
description: Use whenever you work with data in any way — data analysis, charts, plots, graphs, tables, metrics, or numbers you'd otherwise just state in prose. muckdb is a drop-in duckdb CLI wrapper with a live web UI. Get the data into duckdb (from CSV, JSON, Parquet, Excel, an API, another skill's output, or any format you can save to a file), analyse it in SQL, and present the result as interactive, drill-down dashboards the human can verify for themselves. It is the default tool for any chart/plot/graph and for any analysis expressible in SQL — prefer it over matplotlib, ASCII charts, ad-hoc tables, or printing numbers into chat.
---

# muckdb

`muckdb` is a drop-in wrapper around the `duckdb` CLI that also runs a live web
UI (default <http://localhost:11000>). Anything you'd run with `duckdb`, run with
`muckdb` instead — same arguments, same stdout/exit codes — and it additionally:

- records every invocation in a live **ledger**,
- lets you browse any database it has touched (rows, search, facets, sorting,
  stats with histograms — plus correlation, time-series and junk-data tabs,
  schema, a SQL query editor, CSV/JSON export), and
- hosts **sessions**: named dashboards of panels you build from the CLI to
  present results to the human.

The first `muckdb` call starts the background server automatically. You don't
need to manage it (`muckdb start` / `--status` / `--stop` / `--display` exist if needed).

## Use it by default for data work

Reach for muckdb **any time you touch data**, not just for big analyses. In
particular:

- **Any chart, plot, or graph.** muckdb is the go-to. Don't generate matplotlib
  PNGs, ASCII bar charts, or paste a table into chat — build a tile.
- **Any analysis you can express in SQL.** Aggregations, joins, filtering,
  windowing, ranking, time bucketing — do it in muckdb (it's duckdb) rather than
  hand-rolling it in Python/pandas or in your head.
- **Any numbers you'd otherwise assert in prose.** "There are 240 readings across
  5 species" is a claim the human has to trust. Put it in a view and a tile and
  it becomes something they can see and check.
- **Data arriving from anywhere** — a file, an API response, a command's stdout,
  another skill's output, the clipboard. Get it into duckdb first (see below),
  then analyse and present it.

## Make the result indisputable

The goal is that nothing you report is "take my word for it." Instead of
summarising data in text, land it in duckdb and present it so the human can
**verify it themselves**:

- Every figure you cite should be backed by a **view** the human can open and a
  **tile** they can drill into (view tiles get an **explore** button → faceted
  table browser; inline-SQL tiles show the exact query).
- Keep the source query visible and the data live — they can re-sort, filter,
  facet, check the row count, and export CSV/JSON. The dashboard is the evidence,
  the prose is just the headline.
- When you state a conclusion, link to the dashboard/tile that proves it rather
  than restating the numbers.

## Summarise tabular data with markdown panels — always

**Never dump a table of numbers into chat.** When you have tabular results to
report, post a **markdown panel** (`muckdb session post`) that summarises them,
and put the rows themselves in a chart or an explorable view tile beside it. A
markdown panel is the headline; the chart/view is the evidence.

Make this your default reflex for *any* result set:

- **Lead every dashboard with a markdown summary panel.** Open with the headline
  finding in prose, then a compact **markdown table** of the key figures. The
  human should understand the result from the top panel alone, before scrolling.
- **Render small result sets as a markdown table, not raw rows.** If a query
  returns a handful of rows (totals, top-N, a breakdown), format them as a
  GitHub pipe table in a markdown panel — right-align numbers, add a units/`%`
  column — instead of pasting CLI output. Use a `table` chart or a view tile when
  the human needs to sort/filter/export the full set.
- **Pair every chart with words.** Each chart tile should sit under (or beside) a
  markdown panel that says what it shows and why it matters — the trend, the
  outlier, the so-what. A chart with no narrative is half a result.
- **Keep the headline numbers in markdown.** Totals, deltas, and rates belong in
  a markdown panel (bold them, show the change) so the takeaway is unmissable;
  the chart shows the shape, the table shows the exact values.
- **If the chart has no shape, don't chart it.** When a bar chart would show many
  bars at (nearly) the same height — e.g. every category has the same count, or a
  column's `avg` equals its `max` so there's no variation — the chart conveys
  nothing. Replace that panel with a **markdown summary** that states the finding
  in words ("all 5 event types occur ~48 times — uniform, no outliers"). A chart
  earns its place by showing variation; a flat one just wastes the space.

```sh
# Headline panel: prose + a markdown table of the exact figures.
muckdb session post sales --name summary --title "Q2 summary" --md - <<'MD'
# Q2 sales

Revenue **$1.2M (+18% QoQ)** across **4 regions**; Northland leads.

| Region    | Revenue   | Orders | Share |
|:----------|----------:|-------:|------:|
| Northland | $412,000  |  1,204 |  34%  |
| Eastvale  | $356,000  |  1,011 |  29%  |
| Southport | $241,000  |    832 |  20%  |
| Westend   | $203,000  |    789 |  17%  |

See **By region** below to sort/filter/export the full breakdown.
MD

# Evidence: the chart (and a view tile the human can drill into).
muckdb session tile sales --name by_region --title "By region" \
  --db sales.duckdb --view revenue_by_region --chart bar --x region --y revenue \
  --xlabel Region --ylabel Revenue
```

## Get any data into duckdb

duckdb reads most formats directly — so the move for *any* incoming data is to
load it into a table, then work from there. Save whatever you have to a file (or
pipe it) and ingest:

```sh
# CSV / TSV (auto-detects types, header, delimiter)
muckdb data.duckdb -c "CREATE OR REPLACE TABLE t AS SELECT * FROM read_csv_auto('in.csv');"

# JSON / NDJSON (records, nested objects, arrays)
muckdb data.duckdb -c "CREATE OR REPLACE TABLE t AS SELECT * FROM read_json_auto('in.json');"

# Parquet
muckdb data.duckdb -c "CREATE OR REPLACE TABLE t AS SELECT * FROM read_parquet('in.parquet');"

# Excel (.xlsx) — load the extension once, then read a sheet
muckdb data.duckdb -c "INSTALL excel; LOAD excel; CREATE OR REPLACE TABLE t AS SELECT * FROM read_xlsx('in.xlsx');"

# Remote files work too (https / s3 with the httpfs extension)
muckdb data.duckdb -c "CREATE OR REPLACE TABLE t AS SELECT * FROM read_csv_auto('https://example.com/data.csv');"

# Data from another tool's stdout, an API, or a skill: write it to a file first,
# then ingest. e.g. some_command > /tmp/out.json && muckdb ... read_json_auto('/tmp/out.json')

# Small/structured data you already have in hand: inline it as VALUES
muckdb data.duckdb -c "CREATE OR REPLACE TABLE t(label TEXT, n INT) AS VALUES ('a',3),('b',7);"
```

Once it's a table, the rest is normal SQL: build **views** for anything you want
to chart or let the human explore, then post tiles. If a format isn't directly
readable, convert it to CSV/JSON/Parquet first, then load that.

**Put the database somewhere durable — never in a temp/scratchpad dir.** The
dashboard outlives your session, and its tiles keep pointing at the `--db` path
you posted: a database created under `/tmp` or a session scratchpad disappears
when that dir is cleaned, and every explore/chart on it then fails with
"database does not exist". Create session databases in the project directory or
a stable data dir (e.g. `~/.local/share/muckdb/data/`) instead — `muckdb
session tile` warns when a `--db` lives in a temp dir.

## The core workflow

```sh
# 1. Tag your work so commands are grouped under a session in the ledger.
export MUCKDB_SESSION=pond-analysis

# 2. Create the session, linked to THIS Claude session via its UUID so the
#    dashboard is tied to the conversation that built it.
muckdb session create pond-analysis --title "Pond analysis" \
  --claude "$CLAUDE_CODE_SESSION_ID"

# 3. Ingest + analyse. Create VIEWS for anything you want to chart or let the
#    human explore. (muckdb == duckdb here.)
muckdb ~/data/ponds.duckdb -c "
  CREATE OR REPLACE TABLE readings AS SELECT * FROM read_csv_auto('~/data/readings.csv');
  CREATE OR REPLACE VIEW by_species AS
    SELECT species, count(*) AS n FROM readings GROUP BY 1 ORDER BY n DESC;
"

# 4. Post panels. Re-run with the same --name to UPDATE a panel in place.
muckdb session post pond-analysis --name summary --title Summary \
  --md "# Pond analysis\n\n**5 species**, ~240 readings. pH trends **down**."

muckdb session tile pond-analysis --name species --title "By species" \
  --db ~/data/ponds.duckdb --view by_species --chart bar --x species --y n

# 5. Tell the human to open the dashboard:
#    http://localhost:11000/session/pond-analysis/
```

## Command reference

```
muckdb session create <name> [--title T] [--claude UUID]
muckdb session list
muckdb session post <name> --md <text|->  [--name TILE] [--title T]
muckdb session section <name> --name TILE --title HEADING
muckdb session move <name> --tile TILE (--up | --down | --to N | --before TILE | --after TILE)
muckdb session tile <name> --name TILE --db <db> (--view V | --sql "SQL")
        [--chart bar|stacked|line|area|scatter|pie|table|heatmap|box|map|timeline|sequence] [--x COL] [--y C1,C2] [--title T] [--caption C]
        [--value COL]  (heatmap: the cell value; --x/--y name the two axes)
        [--no-values]  (heatmap: colour cells only — hover shows the figure)
        [--lat COL] [--lon COL]  (map: latitude/longitude columns; auto-detected from lat/latitude & lon/lng/longitude if omitted)
        [--label COL]  (map: per-point label shown in the hover tooltip; timeline: the text drawn in each bar; sequence: the message text)
        [--desc COL]   (box: a per-box note column; --y is min,q1,median,q3,max)
        [--lane COL]   (timeline: the horizontal lane/row each bar belongs to)
        [--start COL] [--end COL] [--duration COL]  (timeline: bar start, and its end OR a numeric-seconds duration)
        [--color COL]  (timeline: colour bars by this category value, adds a legend)
        [--id COL] [--depends-on COL]  (timeline: unique bar id + comma-separated parent id(s) → dependency connectors)
        [--chart sequence]  (sequence diagram — service comms; one row per message)
        [--from COL] [--to COL]  (sequence: source/destination participant; --from == --to is a self-message; message text = --label)
        [--message-type COL]  (sequence: sync (default) | reply | async | lost)
        [--from-type COL] [--to-type COL]  (sequence: participant (default) | actor | database | boundary)
        [--group COL]  (sequence: 'kind:label' — loop|opt|alt|par; contiguous equal values = one frame)
        [--group-branch COL]  (sequence: else/and compartment label within a frame)
        [--autonumber]  (sequence: number the messages)
        [--xlabel L] [--ylabel L] [--bars gradient|solid]
        [--target 'VAL|label'] [--threshold 'VAL|label'] [--event 'X|label'] [--trend]
muckdb session screenshot <name> [--tile TILE] [--out FILE.png] [--width W] [--height H]
muckdb session export <name> [--out FILE.muckdb]
muckdb session import <file.muckdb>
muckdb session rm <name> [--tile TILE]
```

- **Link the session to your conversation.** Pass `--claude "$CLAUDE_CODE_SESSION_ID"`
  on `create` to record the Claude Code session UUID on the dashboard. It's shown
  at the top of the session view and returned by `muckdb ls session <id>`
  (`claude_session`), so a human can tell which conversation produced a dashboard.
- **Posts are validated against the database.** A missing view, unparseable
  `--sql`, or a `--x`/`--y` that isn't a column of the result fails immediately
  with a "did you mean" suggestion and the available names — fix and re-post.
  `--no-validate` skips the check (e.g. posting before the view exists).
- **Tiles are keyed by `--name`** within a session — re-posting the same name
  replaces that panel (upsert). Use stable names so updates land in place.
- **Lay the report out, and keep it laid out.** Tiles render in post order;
  `session move` reorders one (`--up`/`--down`, `--to N` for a 1-based position,
  or `--before`/`--after TILE`). `session section --name S --title "Heading"` adds
  a heading-only tile that renders as a divider in the dashboard and a section
  header in the contents, grouping the panels after it. Use sections to structure
  a long report and `move` to sequence it.
  - **A dashboard is a document, not an append-only log.** Every time you add
    tiles, **re-evaluate the section structure and reorganise** so it still reads
    top-to-bottom: does the new tile belong under an existing section or does it
    need a new one? Are related tiles adjacent? Is the order still a sensible
    narrative (summary/overview first, then supporting detail)? Use `session
    section` and `session move` to fix it — `move --before`/`--after` to slot a
    tile beside its section, a new `section` when a group of tiles has formed.
    Don't just append to the end and leave the layout to drift; readability is
    part of the deliverable, so tidy the structure as the report grows.
- `--md -` reads the markdown from stdin (good for long/heredoc content). An
  inline `--md "..."` honours `\n`/`\t` escapes (shells leave them literal
  inside double quotes), so `--md "# Title\n\nBody"` renders as real lines.
- **Respect the trash.** The human can trash a panel in the UI; the flag
  persists on the tile (`trashed: true` in `muckdb ls session`) and survives
  re-posts — updating a trashed tile does not resurface it. Delete for real
  with `muckdb session rm <session> --tile <name>`.
- **Sessions are portable.** `session export` bundles the dashboard, a full
  snapshot of every database its tiles reference, and their column formats
  into `<name>.muckdb` (a zip); `session import` loads one on any machine —
  the db copies land under muckdb's data dir (`imports/<id>/`), and a name
  collision gets a numeric suffix (`name-2`) rather than overwriting. The web
  UI has the same pair: an **export** button on the session view and an
  **import** button in the top header.
- A tile is a **view** (`--view`, references a named duckdb view) or **inline
  SQL** (`--sql`). Prefer `--view` for anything the human should be able to drill
  into — view tiles get an **explore** button that opens the faceted table
  explorer; inline-SQL tiles get a **sql** button that shows the formatted query.
- Chart kinds: `bar | stacked | line | area | scatter | pie | table | heatmap | box | map | timeline | sequence`. For
  `bar`/`line`/etc, put aggregation in the view/SQL (one row per x). If the `--x`
  column is a date/timestamp, the chart uses a real time axis automatically, drawn
  on a **UTC wall-clock** so daily/hourly buckets sit on their boundaries instead
  of skewing by the viewer's timezone.
- **Label your axes** with `--xlabel`/`--ylabel` so a chart is readable on its own.
- **Caption every chart** with `--caption` — this is required, not optional. One
  line on what it shows and the so-what (the trend, the outlier, the takeaway). A
  chart with no caption makes the human guess what they're looking at; treat
  writing the caption as part of building the tile. Pair it with `--title` (the
  panel heading) and `--xlabel`/`--ylabel` so the panel is self-explanatory on its
  own. If a one-liner isn't enough, add an adjacent markdown panel for the full
  description.
- **Pick the chart that packs in the most information.** Don't default everything
  to a single-series bar chart — choose the kind that shows the most per panel:
  - **`stacked` bars** when each x has a breakdown that sums to a meaningful total
    (revenue split by category, errors by type per day). One panel then shows both
    the total *and* its composition — far denser than one bar per total or a
    separate chart per series.
  - **`area`** for a cumulative or volume-over-time feel, and **stacked areas**
    (multiple `--y`) to show how a total and its parts evolve together over time —
    the temporal analogue of a stacked bar.
  - **`line`** is the go-to for **temporal data** — anything with a `TIMESTAMP`/
    `DATE` x. It carries trend, seasonality, and multiple series at once cleanly;
    plot several `--y` columns to compare measures on one time axis. Prefer it over
    bars when the x is continuous time and you care about the *shape* of the change.
  - **`scatter`** when you want every raw point (density/clusters), not an aggregate.
  - **`heatmap`** to cross two categorical columns and shade each cell by a
    value column — the densest way to show a metric over every (x, y)
    combination (e.g. sites per country x port speed). Pass `--x`, `--y` and
    `--value`; shape the view with one row per pair (`GROUP BY x, y`) and
    control axis order with ORDER BY (axes follow row order of appearance).
    Cell values render through the value column's display format; pass
    `--no-values` to colour cells only (hover still shows the exact figure) —
    better for large grids where the numbers would be noise.
  - **`box`** to compare distributions across groups — one horizontal
    box-and-whisker per row, all on a shared scale. `--x` labels each box;
    `--y` takes exactly five columns in order `min,q1,median,q3,max`
    (aggregate in the view: `min(v), quantile_cont(v,0.25), median(v),
    quantile_cont(v,0.75), max(v)`); `--desc` names a text column rendered
    under each label so every box carries its own explanation. Values render
    through the median column's display format.
  - **`map`** to plot geographic points on a compact ASCII world map. Give it
    `--lat`/`--lon` columns (or name them lat/latitude & lon/lng/longitude and
    they're auto-detected), one row per point — don't pre-aggregate; the tile
    bins points into grid cells itself. Each cell with data gets a coloured `x`
    shaded by how many points land in it, or by the average of `--value` when
    given. Points are projected onto a true equirectangular grid, so markers sit
    on the right coastline. `--label COL` names each point in a rich hover
    tooltip (coords, count, aggregate value, and the labels in that cell).
  - **`timeline`** to lay events out as bars over a shared time axis, arranged
    into labelled lanes — muckdb's Gantt / trace / incident view. See the
    dedicated **Timeline** section below for the full flag set and worked
    examples.
  - **`sequence`** to show **interactions between microservices** — one row
    per message, drawn as an arrow between two participant lifelines. See the
    dedicated **Sequence** section below for the full flag set and a worked
    example.
- **Bar fill — `--bars gradient|solid`** — match the fill to the data:
  - **`--bars solid`** for **categorical** x (HTTP methods GET/POST/PUT, status
    codes, regions, product names, error types). Each bar gets its own solid
    colour from the theme palette, so distinct categories read as distinct.
  - **`--bars gradient`** (the default for a single series) for **continuous**
    data — counts/amounts **over time**, a numeric progression. One smooth
    gradient signals "these belong to one continuous measure."
- **Daily/weekly reporting from timestamps → bucket to a DATE in the view.** If
  your data has fine-grained `TIMESTAMP`s but you want per-day (or per-week) bars,
  don't plot raw timestamps — aggregate to a date column in the view so there's
  one row per day: `SELECT ts::DATE AS day, count(*) AS n FROM events GROUP BY 1`
  (or `date_trunc('week', ts)::DATE`). Then `--x day`. The chart's time axis is a
  UTC wall-clock, so each `DATE` bar sits squarely on its day.
- `stacked` is a stacked bar: pass multiple `--y` columns (one per series) and
  one row per `--x`; the series stack into each bar's total. Shape the view so
  each series is its own column (e.g. `sum(amount) FILTER (category = 'X')`).
- **Reference lines & events — use them to draw the eye to what matters.**
  `--target` and `--threshold` draw horizontal lines at a y-value (target =
  accent/dotted, threshold = warning/dashed) — use them to anchor a series against
  an SLA, budget, or limit so "good vs bad" is visible at a glance. **`--event`**
  draws a vertical line at an x-position (a timestamp on a time axis, or a category
  label) and is the best way to **draw attention to an important moment** — a
  deploy, an incident, a campaign, a config change. **Add one to essentially every
  time series**: it turns "the line jumped" into "the line jumped *when we shipped
  X*," which is usually the whole point of the chart. Each takes `VALUE` or
  `VALUE|label`, e.g.
  `--target '20|SLA' --threshold '30|max' --event '2026-05-15T00:00|deploy'`.
  Markers are part of the tile, so **add or update them anytime** by re-posting the
  tile with the same `--name` (and the new `--event`/`--target` flags) — it
  replaces the panel in place and the dashboard updates live.
- **Trendline — `--trend`.** Overlays a smoothed trendline (locally-weighted
  regression, so it tracks the series' actual level — edges included) on a
  single-series `bar`/`line`/`area`/`scatter` tile. The quickest way to make a
  time series' direction unmistakable — add it by default to records-over-time
  and metric-over-time charts. Ignored on stacked/multi-series tiles.

## Timeline tiles (Gantt / trace / incident view)

A `timeline` tile draws each **row of your view as one horizontal bar** on a
shared time axis, grouped into labelled **lanes**. Reach for it whenever the
story is *what happened when, and on which resource* — the one chart kind that
carries lanes, overlap, dependencies, and markers together. **One row = one
bar.** Shape the view so each bar is a row; lane order follows first appearance,
so control it with `ORDER BY`.

**Flags:**

| Flag | Required | Meaning |
|:-----|:--------:|:--------|
| `--lane COL` | yes | The lane (row/resource) the bar belongs to; its label is drawn in the left gutter. |
| `--label COL` | yes | The text drawn inside the bar. |
| `--start COL` | yes | Bar start — **numeric** (relative seconds) *or* a **timestamp/date**. |
| `--end COL` | one of | Bar end (same type as `--start`). |
| `--duration COL` | one of | Numeric **seconds**; the bar ends at `start + duration`. Give exactly one of `--end`/`--duration`. |
| `--color COL` | no | Colour bars by this **category** value and add a legend (else each lane gets one palette colour). |
| `--id COL` | no | A unique bar id — enables dependencies. |
| `--depends-on COL` | no | Comma-separated parent id(s); each draws a right-angle connector from the parent's end to this bar's start (crosses lanes when the parent is in another lane). |
| `--event 'X\|label'` | no, repeatable | A dashed vertical marker at a time `X` (a number on a relative axis, or a timestamp), labelled above the lanes. |
| `--title` / `--caption` / `--xlabel` | no | As other charts (caption still required). |

**Axis is auto-detected — you don't flag it:**

- **Numeric `--start`/`--end`** → a **relative-seconds** axis from the data's min
  (usually `0s`), humanised (`0s`, `30s`, `2m 15s`, `1h 05m`). Use `--duration`
  here for "N seconds long."
- **Timestamp/date `--start`/`--end`** → an **absolute** axis. Naive timestamps
  are treated as **UTC** and the axis shows the **UTC wall-clock by default** — it
  is *not* auto-converted to local. Opt into a zone with a column format:
  `muckdb format <db> <startcol> --tz local` (or `--tz Australia/Brisbane`); the
  axis then renders in that zone **and the hover readout also shows the UTC
  instant**, so a local-time timeline stays unambiguous.
- Mixed/ambiguous start/end types fail validation with a clear message.
- The bottom axis places **tick marks on regular round intervals** sized to the
  tile width, and drops any label that would overlap — so it stays legible at any
  width or zoom.

**Colour, sublanes, dependencies, markers:**

- **Default** (no `--color`): each lane is one palette colour. **With `--color`:**
  bars are coloured by that column's category (with a legend) — use it for
  status/severity/outcome.
- **Overlap → sublanes.** Bars in a lane whose times overlap are packed into
  stacked sublanes so none overlap visually; the lane grows to fit. Concurrent
  work is therefore visible automatically.
- **Dependencies** (`--id` + `--depends-on`): thin right-angle connectors. A
  parent id that matches no bar is silently ignored.
- **Markers** (`--event`): dashed vertical lines with a small triangle at the top
  and their labels in a band **above** the lanes (stacked if they'd collide) —
  distinct from the orange hover cursor. Re-post with new `--event` flags to
  update markers live. Marker colour/dash/width/opacity are themeable via the
  `--tl-marker-*` CSS vars.
- **Hover** anywhere for a vertical cursor + the time at that point (local **and**
  UTC when a `--tz` is set); hover a bar for a rich tooltip — label, lane,
  `start → end`, computed duration, the `--color` category, then **every other
  column in the row**, each rendered through its column format (so a `--link`
  column becomes a clickable link — see below). Timeline tiles support the
  **full-width** toggle like tables.

**The four canonical uses — shape the view like this:**

1. **Resource allocation** — lanes = machines/people/rooms, bars = the tasks each
   is busy with. `--lane resource --label task --start t0 --end t1`.
2. **Incident timeline** — lanes = systems/actors, bars = phases, colour by
   severity, `--event` for key moments:
   ```sh
   muckdb format inc.db started --table incident_tl --tz local   # local axis + UTC on hover
   muckdb session tile ops --name incident --db inc.db --view incident_tl --chart timeline \
     --lane system --label phase --start started --end ended --color severity \
     --event '2026-05-01 14:18|outage declared' --event '2026-05-01 14:41|resolved' \
     --caption "Incident phases per system; severity by colour, markers for the key moments."
   ```
3. **OpenTelemetry / trace view** — lanes = services, bars = spans, `--duration`
   for span length, `--id`/`--depends-on` for parent→child causality; span
   attributes appear in the tooltip:
   ```sh
   muckdb session tile trace --name t --db trace.db --view spans --chart timeline \
     --lane service --label op --start start_s --duration dur_s \
     --id span_id --depends-on parent_id \
     --caption "Trace waterfall: spans per service; arrows are parent→child."
   ```
4. **Investigation / CI pipeline sequencing** — lanes = workstreams/runners, bars
   = steps, overlaps stack into sublanes, dependencies show the chain:
   ```sh
   muckdb session tile ci --name pipeline --db ci.db --view pipeline_tl --chart timeline \
     --lane runner --label step --start t0 --end t1 --color outcome \
     --id sid --depends-on parent --event '95|tests start' \
     --caption "CI pipeline (0→seconds): sublanes for concurrent steps, arrows for deps."
   ```

**Clickable columns in the tooltip.** Because the tooltip renders every extra
column through its format, a `--link` on an id/reference column turns it into a
launchpad — set it in the same pass as your other formats:

```sh
muckdb format ci.db sid --table pipeline_tl \
  --link 'https://ci.example.com/builds/{value}' --link-title 'build {value} · {step}'
```

## Sequence tiles (service comms / request-response / trace narrative)

A `sequence` tile shows **interactions between microservices** — request/
response, async fan-out, retries, fallbacks — as a classic UML sequence
diagram: one vertical **lifeline** per participant, one horizontal **arrow**
per message. **One row = one message.** Shape the view so each row is a
message in the flow it sends/receives; message order follows the row order,
so control it with `ORDER BY`.

**Flags:**

| Flag | Required | Meaning |
|:-----|:--------:|:--------|
| `--from COL` | yes | Source participant. |
| `--to COL` | yes | Destination participant. `--from == --to` on a row draws a **self-message**. |
| `--label COL` | yes | The message text drawn on the arrow. |
| `--message-type COL` | no | Arrow kind: `sync` (default, solid) \| `reply` (dashed) \| `async` (open head) \| `lost` (dead-ends with an `x`). |
| `--from-type COL` / `--to-type COL` | no | Participant shape: `participant` (default) \| `actor` \| `database` \| `boundary`. |
| `--group COL` | no | `'kind:label'` where `kind` ∈ `loop \| opt \| alt \| par` — wraps contiguous rows sharing the same value in one labelled frame. |
| `--group-branch COL` | no | Names an else/and compartment **within** the current frame (an `alt`'s else-branch, a `par`'s and-branch). |
| `--autonumber` | no | Numbers the messages 1, 2, 3… down the lifelines. |
| `--title` / `--caption` / `--xlabel` | no | As other charts (caption still required). |

**Ordering rules:**

- **Message order = row order** — the diagram lays arrows out top-to-bottom in
  the order the view returns them, so `ORDER BY` the column that carries the
  real sequence (a timestamp, a step counter).
- **Participant order = first appearance.** A participant's lifeline is added
  the first time it shows up as a `--from` or `--to` value; there's no
  separate participant list to maintain.
- **Participant type = the type on the row where it first appears.** If
  `gateway` first shows up with `--from-type participant`, it's drawn as a
  plain participant even if a later row (inconsistently) tags it `actor` —
  keep a participant's type consistent across rows.

**Groups are single-level in this version** — a `loop`/`opt`/`alt`/`par` frame
can't nest another frame inside it. Contiguous rows with the same `--group`
value merge into one frame; a new value (or a gap) closes the current frame
and opens the next. Use `--group-branch` to add else/and compartments inside
a frame without starting a new one (e.g. an `alt` with a `valid`/`expired`
branch, still one frame).

**Mermaid export.** Every sequence tile gets a **mermaid** toolbar button that
copies a valid mermaid.js `sequenceDiagram` to the clipboard — paste it
straight into a mermaid live editor or a markdown doc that renders mermaid.
One mapping caveat: **mermaid has no database or boundary participant
shape**, so `database`/`boundary` participants export as plain `participant`
with a preceding `%% database` / `%% boundary` comment marking what they
really are; `actor` exports as mermaid's `actor` (its one distinct shape).

**Worked example** — a login flow across gateway/auth/orders, with an `alt`
frame for the valid-vs-expired-token branches:

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

## Column display formats (units, currency, decimals) — set them, always

**Format every numeric column that has a unit.** A bare `4343.33` makes the human
guess (dollars? ms? a count?); `$4,343.33 USD` answers it. This is not a nicety —
an unformatted money/duration/percentage column is a half-finished panel. Make
setting formats a standard step right after you build your views, *before* you
post tiles: for each numeric column a chart or table will show, attach a currency,
a unit suffix, a percent, or a thousands separator. It costs one command per
column and applies everywhere that column appears (facets, charts, stats, tables).

Attach a display format to a column so facets, charts, stats and tables show
`$4,343.33 USD` instead of `4343.33`:

```sh
# muckdb registry (applies by column name across tables/views, incl. derived
# columns like a `revenue` produced by sum(amount) — and works on read-only DBs)
muckdb format <db> revenue --currency USD          # → $1,234.56 USD
muckdb format <db> latency_ms --suffix ' ms' --decimals 0
muckdb format <db> rate --percent                  # → 12.5%
muckdb format <db> amount --table sales --prefix '$' --thousands   # scope to one table
muckdb format <db> revenue --clear                 # remove
muckdb format list [<db>]

# or store it WITH the data via a DuckDB column comment (travels in the .duckdb):
muckdb <db> -c "COMMENT ON COLUMN sales.amount IS 'muckdb:{\"prefix\":\"\$\",\"suffix\":\" USD\",\"decimals\":2,\"group\":true}'"
```

The registry overrides the comment. Flags: `--currency CODE`, `--prefix`,
`--suffix`, `--decimals N`, `--thousands`, `--percent`, `--tz Z`, `--epoch U`,
`--link URL`, `--link-title T`, `--clear`. A registry entry keyed by column name
(no `--table`) is the easy win —
it formats that column everywhere it appears, including the derived columns your
chart views produce. When you do scope with `--table`, use the name a tile
actually queries (the **view** name you post, not the base table under it).

### Timestamps: `--tz` and `--epoch`

Naive DB timestamps are treated as **UTC instants** everywhere. Two flags make
time columns readable:

```sh
muckdb format <db> created_at --tz local      # show in the viewer's timezone
muckdb format <db> ts --tz Australia/Brisbane # or a fixed IANA zone / utc
muckdb format <db> ts_ms --epoch ms           # numeric epoch column: s | ms | us
```

`--tz` converts the column's display (tables, facets, stats) to that zone —
`2026-06-28 10:00:00 GMT+10` — and a chart with that column on the x-axis draws
its **time axis in the same zone**. Without a `--tz`, chart time axes render the
UTC wall clock so daily/hourly buckets stay on their boundaries. `--epoch` marks
a numeric column as an epoch so it renders (and charts) as time; columns with
time-ish names (`ts`, `*_at`, `*_ms`, `epoch`, …) and plausible epoch values are
detected automatically, so the flag is mostly for odd names or overrides.

Time axes are granularity-aware on their own: a `DATE` (or midnight-truncated)
column never shows hour ticks, first-of-month data ticks by month, hour ticks
render `HH:mm` with a bold date label at each day boundary.

### Links: `--link` and `--link-title` — turn a column into a hyperlink

**Add a link to every column where it makes sense — do this as routinely as you
format numeric columns.** Any column that identifies or references something a
human would want to open belongs behind a `--link`: ids/uuids/slugs that map to
an admin portal, a ticket, a repo, a PR, a profile, a dashboard, an S3 object, a
build. A clickable id turns a flat table into a launchpad into the real systems,
and it costs one `muckdb format` per column. When you build a table's formats,
ask of each column "does this point at something openable?" and wire the link if
so.

Point a column at an external system (an admin portal, a dashboard, a ticket
tracker) and its cells become clickable links in the rows view, query results
and session `table` tiles. Both flags are **templates** sharing one
substitution system:

```sh
# Inject BOTH a company uuid and a user uuid from the same row into the URL:
muckdb format app.db user_uuid \
  --link 'https://admin.example.com/companies/{company_uuid}/users/{value}' \
  --link-title 'user {value}'

# Search link built from another column, explicitly percent-encoded in a title:
muckdb format app.db order_id \
  --link 'https://tickets.example.com/search?q={customer_name}' \
  --link-title 'tickets for {customer_name}'
```

- `{value}` is this column's value; `{any_column}` pulls **any other column of
  the same row** into the URL or the title.
- **Encoding rules**: inside `--link` every substitution is percent-encoded by
  default; append `:raw` (e.g. `{path_fragment:raw}`) to inject verbatim when a
  column already holds URL-ready text. Inside `--link-title` substitutions are
  verbatim by default; append `:url` to percent-encode. Both modifiers work in
  both templates.
- A `{name}` that matches no column stays as literal text; NULL values
  substitute as empty strings.
- `--link-title` is optional — without it the link text is the column's
  (formatted) value, so it composes with the numeric flags: `--currency USD
  --link ...` renders a clickable `$1,234.56 USD`.
- Comment form (travels with the db):
  `muckdb:{"link":"https://…/{value}","link_title":"open {name}"}`.
- The schema tab's format column shows the link template, so a human can see
  where a column points.

## Inspecting state (read it back as JSON)

To understand what muckdb is currently showing — without starting the server —
use the read-only `ls` commands. They print JSON, so you can parse them:

```
muckdb ls databases          # [{id, path, exists}] for every db muckdb has seen
muckdb ls tables <db>         # tables + views in a database ({name, is_view, ...})
muckdb ls sessions           # every session with its tiles
muckdb ls session <id>       # one session (tiles: names, types, charts, views)
muckdb ls history [--limit N]  # the command ledger (args, exit codes, session tag)
```

Use these to check a session's current tiles before updating one, to find a
database's `id` (for building a `/db/<id>/…` link), or to see what the human has
been running.

**See what the human actually looks at.** Session JSON from `ls sessions` /
`ls session <id>` includes an `activity` block recorded from the web UI:
per-session `views`/`last_viewed`, and per-tile `zooms`/`explores`/`last`.
Use it to adapt: a session with many views is worth keeping polished; a tile
the human zooms or explores repeatedly deserves more depth; a tile with zero
interactions across many views is a hint to present that data differently.

## See what you built — screenshot a panel

`muckdb session screenshot` renders a session (or one tile) exactly as the web
UI shows it and writes a PNG — so **you can look at your own dashboard** instead
of guessing how a chart came out:

```sh
muckdb session screenshot pond-analysis --tile species --out species.png
# then Read species.png to view it
```

- Omit `--tile` to capture the whole dashboard; the image auto-fits the content
  height. `--out` defaults to `muckdb-<session>[-<tile>].png` in the cwd.
- **Verify visually after building.** After posting tiles, screenshot the
  session and look at it — wrong chart kind, an empty series, or unreadable
  labels are obvious in the image and invisible in the CLI output.
- Needs a Chromium-based browser installed (chromium/chrome/brave/edge; override
  with `MUCKDB_BROWSER=/path/to/browser`). Renders in ~1s.
- The same render backs `GET /api/shot?session=<id>&tile=<name>` (returns
  `image/png`) and the **copy-image button** on every panel in the web UI.
- The web UI header also has a **poster** button that downloads one PNG of the
  whole dashboard (every tile, rendered in-browser), and **table tiles have a
  full-width toggle** (horizontal-expand icon) that breaks the tile out of the
  centred column so all columns show at once.

## Good habits

- **Get it into duckdb first.** Whatever the source or format, land it in a table,
  then analyse and chart from there — don't compute results outside SQL and paste
  them in.
- **Aggregate in SQL, not in the chart.** A tile plots rows as-is, so write the
  view to return exactly the series you want (`GROUP BY`, `ORDER BY`, a sensible
  `LIMIT`).
- **Order columns by how filterable they are — most-filtered first, view-only
  last.** In the `SELECT` list of a view (and base tables), lead with the columns
  a human will actually facet, search, and filter on — status, category, region,
  name, dates, amounts — because the explorer shows and facets columns in order.
  Push columns that can't be meaningfully filtered to the end: `latitude`/
  `longitude` are only useful on a map (you can't sensibly filter a raw coordinate
  by hand), and a bare `id`/`uuid` is for viewing/linking, not filtering. So a
  natural order is *filter dimensions → measures → id/coords last*, e.g.
  `SELECT status, region, plan, mrr, created, id, latitude, longitude FROM …` —
  not `id` first out of habit.
- **Caption and label every tile.** Always pass `--caption` (what it shows + the
  takeaway) and, on charts, `--title`/`--xlabel`/`--ylabel`. An unlabelled panel
  isn't done — see the caption note in the command reference above.
- **Format numeric columns before posting tiles.** Set a `muckdb format` (currency,
  unit suffix, percent, thousands) for every money/duration/rate/count column a
  panel will display, so values render as `$1,234.56 USD` / `42 ms` / `12.5%`
  everywhere — see "Column display formats" above. Treat it as a standard step, not
  an afterthought.
- **Link id/reference columns.** In the same pass, add a `--link` to every column
  that identifies or references something openable (uuid → admin portal, ticket
  id → tracker, repo/PR, object key → storage, etc.), so its cells are clickable —
  see "Links" above. A table of bare ids is a dead end; linked ids are a launchpad.
- **Markdown for narrative, charts for data.** Lead with a markdown summary tile
  (prose + a markdown table of the key figures), then supporting chart tiles.
  Never dump raw rows into chat — summarise in markdown, evidence in a chart/view
  (see "Summarise tabular data with markdown panels" above).
- **Update, don't duplicate.** Keep `--name`s stable across a task; the dashboard
  updates live (WebSocket) each time you post.
- **Look at what you built.** `muckdb session screenshot <id> [--tile T]` gives
  you a PNG of the rendered dashboard — read it and check the charts say what you
  think they say before telling the human it's done.
- **Give the human the link.** `http://localhost:11000/session/<id>/` — deep-links
  to a specific table/view/query also work, e.g.
  `/db/<id>/<table>/?view=stats` (stats tabs: `&tab=correlation|timeseries|junk`).
- Queries the daemon runs (introspection, tiles, the editor) are **read-only**.

## Where state lives

Sessions are JSON files under the muckdb data dir
(`~/.local/share/muckdb/sessions/` on Linux,
`~/Library/Application Support/muckdb/sessions/` on macOS); the command ledger is
`history.jsonl` beside it. The CLI writes; the daemon watches and pushes updates.
