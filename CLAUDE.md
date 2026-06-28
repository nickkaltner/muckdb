# Using muckdb (guide for Claude / coding agents)

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

## When to use it

When you've analysed data in duckdb and want to **show the human the result**,
build a session dashboard instead of dumping text. They open one URL and get
live, interactive charts + notes that update as you post more.

## The core workflow

```sh
# 1. Tag your work so commands are grouped under a session in the ledger.
export MUCKDB_SESSION=pond-analysis

# 2. Create the session (optional --title).
muckdb session create pond-analysis --title "Pond analysis"

# 3. Do your duckdb work as normal, creating VIEWS for anything you want to chart
#    or let the human explore. (muckdb == duckdb here.)
muckdb ~/data/ponds.duckdb -c "
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
        [--xlabel L] [--ylabel L]
        [--target 'VAL|label'] [--threshold 'VAL|label'] [--event 'X|label']
muckdb session rm <name> [--tile TILE]
```

- **Link the session to your conversation** with `--claude "$CLAUDE_CODE_SESSION_ID"`
  on `create`. The UUID is shown at the top of the session view and returned by
  `muckdb ls session <id>` (`claude_session`).
- **Tiles are keyed by `--name`** within a session — re-posting the same name
  replaces that panel (upsert). Use stable names so updates land in place.
- `--md -` reads the markdown from stdin (good for long/heredoc content).
- A tile is a **view** (`--view`, references a named duckdb view) or **inline
  SQL** (`--sql`). Prefer `--view` for anything the human should be able to drill
  into — view tiles get an **explore** button that opens the faceted table
  explorer; inline-SQL tiles get a **sql** button that shows the formatted query.
- Chart kinds: `bar | stacked | line | area | scatter | pie | table`. For
  `bar`/`line`/etc, put aggregation in the view/SQL (one row per x). If the `--x`
  column is a date/timestamp, the chart uses a real time axis automatically, drawn
  on a **UTC wall-clock** so daily/hourly buckets stay on their boundaries (a
  `DATE` day won't skew by the viewer's timezone).
- **Axis labels**: `--xlabel`/`--ylabel` set the x/y axis titles on any chart.
- `stacked` is a stacked bar: pass multiple `--y` columns (one per series) and
  one row per `--x`; the series stack into each bar's total. Shape the view so
  each series is its own column (e.g. `sum(amount) FILTER (category = 'X')`).
- **Reference lines** (repeatable): `--target`/`--threshold` draw horizontal lines
  at a y-value; `--event` draws a vertical line at an x-position (timestamp or
  category). Each is `VALUE` or `VALUE|label`, e.g. `--threshold '30|max'`.

## Column display formats

Attach a unit/currency/decimal format to a column so facets, charts, stats and
tables render it nicely (e.g. `$4,343.33 USD`):

```
muckdb format <db> <column> [--table T] [--currency USD | --prefix $ --suffix ' USD' --decimals N --thousands --percent] [--clear]
muckdb format list [<db>]
```

A registry entry keyed by column name applies wherever that column appears
(including derived view columns charts plot); `--table` scopes it to one
relation. Formats also read from a DuckDB column comment (`COMMENT ON COLUMN
t.c IS 'muckdb:{"prefix":"$","decimals":2,"group":true}'`), which travels with
the database; the registry overrides the comment.

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

- **Aggregate in SQL, not in the chart.** A tile plots rows as-is, so write the
  view to return exactly the series you want (`GROUP BY`, `ORDER BY`, a sensible
  `LIMIT`).
- **Markdown for narrative, charts for data.** Lead with a markdown summary tile
  (prose + a markdown table of the key figures), then supporting chart tiles.
  Never dump raw rows into chat — summarise in a markdown panel and put the data
  in a chart or explorable view tile beside it.
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
