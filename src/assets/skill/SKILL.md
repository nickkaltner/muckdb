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
muckdb session tile <name> --name TILE --db <db> (--view V | --sql "SQL")
        [--chart bar|stacked|line|area|scatter|pie|table] [--x COL] [--y C1,C2] [--title T] [--caption C]
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
- Chart kinds: `bar | stacked | line | area | scatter | pie | table`. For
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

## Good habits

- **Get it into duckdb first.** Whatever the source or format, land it in a table,
  then analyse and chart from there — don't compute results outside SQL and paste
  them in.
- **Aggregate in SQL, not in the chart.** A tile plots rows as-is, so write the
  view to return exactly the series you want (`GROUP BY`, `ORDER BY`, a sensible
  `LIMIT`).
- **Caption and label every tile.** Always pass `--caption` (what it shows + the
  takeaway) and, on charts, `--title`/`--xlabel`/`--ylabel`. An unlabelled panel
  isn't done — see the caption note in the command reference above.
- **Format numeric columns before posting tiles.** Set a `muckdb format` (currency,
  unit suffix, percent, thousands) for every money/duration/rate/count column a
  panel will display, so values render as `$1,234.56 USD` / `42 ms` / `12.5%`
  everywhere — see "Column display formats" above. Treat it as a standard step, not
  an afterthought.
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
