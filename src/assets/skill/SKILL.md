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
  stats with histograms, schema, a SQL query editor, CSV/JSON export), and
- hosts **sessions**: named dashboards of panels you build from the CLI to
  present results to the human.

The first `muckdb` call starts the background server automatically. You don't
need to manage it (`muckdb --status` / `--stop` / `--display` exist if needed).

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

## The core workflow

```sh
# 1. Tag your work so commands are grouped under a session in the ledger.
export MUCKDB_SESSION=pond-analysis

# 2. Create the session (optional --title).
muckdb session create pond-analysis --title "Pond analysis"

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
muckdb session create <name> [--title T]
muckdb session list
muckdb session post <name> --md <text|->  [--name TILE] [--title T]
muckdb session tile <name> --name TILE --db <db> (--view V | --sql "SQL")
        [--chart bar|stacked|line|area|scatter|pie|table] [--x COL] [--y C1,C2] [--title T] [--caption C]
muckdb session rm <name> [--tile TILE]
```

- **Tiles are keyed by `--name`** within a session — re-posting the same name
  replaces that panel (upsert). Use stable names so updates land in place.
- `--md -` reads the markdown from stdin (good for long/heredoc content).
- A tile is a **view** (`--view`, references a named duckdb view) or **inline
  SQL** (`--sql`). Prefer `--view` for anything the human should be able to drill
  into — view tiles get an **explore** button that opens the faceted table
  explorer; inline-SQL tiles get a **sql** button that shows the formatted query.
- Chart kinds: `bar | stacked | line | area | scatter | pie | table`. For
  `bar`/`line`/etc, put aggregation in the view/SQL (one row per x). If the `--x`
  column is a date/timestamp, the chart uses a real time axis automatically.
- `stacked` is a stacked bar: pass multiple `--y` columns (one per series) and
  one row per `--x`; the series stack into each bar's total. Shape the view so
  each series is its own column (e.g. `sum(amount) FILTER (category = 'X')`).

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

## Good habits

- **Get it into duckdb first.** Whatever the source or format, land it in a table,
  then analyse and chart from there — don't compute results outside SQL and paste
  them in.
- **Aggregate in SQL, not in the chart.** A tile plots rows as-is, so write the
  view to return exactly the series you want (`GROUP BY`, `ORDER BY`, a sensible
  `LIMIT`).
- **Markdown for narrative, charts for data.** Lead with a markdown summary tile,
  then supporting chart tiles. Let the charts carry the numbers.
- **Update, don't duplicate.** Keep `--name`s stable across a task; the dashboard
  updates live (WebSocket) each time you post.
- **Give the human the link.** `http://localhost:11000/session/<id>/` — deep-links
  to a specific table/view/query also work, e.g.
  `/db/<id>/<table>/?view=stats`.
- Queries the daemon runs (introspection, tiles, the editor) are **read-only**.

## Where state lives

Sessions are JSON files under the muckdb data dir
(`~/.local/share/muckdb/sessions/` on Linux,
`~/Library/Application Support/muckdb/sessions/` on macOS); the command ledger is
`history.jsonl` beside it. The CLI writes; the daemon watches and pushes updates.
