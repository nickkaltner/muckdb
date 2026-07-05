# Using muckdb (guide for Claude / coding agents)

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
        [--chart bar|stacked|line|area|scatter|pie|table|heatmap|box] [--x COL] [--y C1,C2] [--title T] [--caption C]
        [--value COL]  (heatmap: the cell value; --x/--y name the two axes)
        [--no-values]  (heatmap: colour cells only — hover shows the figure)
        [--desc COL]   (box: a per-box note column; --y is min,q1,median,q3,max)
        [--xlabel L] [--ylabel L] [--bars gradient|solid]
        [--target 'VAL|label'] [--threshold 'VAL|label'] [--event 'X|label'] [--trend]
muckdb session screenshot <name> [--tile TILE] [--out FILE.png] [--width W] [--height H]
muckdb session export <name> [--out FILE.muckdb]
muckdb session import <file.muckdb>
muckdb session rm <name> [--tile TILE]
```

- **Link the session to your conversation** with `--claude "$CLAUDE_CODE_SESSION_ID"`
  on `create`. The UUID is shown at the top of the session view and returned by
  `muckdb ls session <id>` (`claude_session`).
- **Tiles are keyed by `--name`** within a session — re-posting the same name
  replaces that panel (upsert). Use stable names so updates land in place.
- **Posts are validated against the database.** A missing view, unparseable
  `--sql`, or a `--x`/`--y` that isn't a column of the result fails immediately
  with a "did you mean" suggestion and the available names — fix and re-post.
  `--no-validate` skips the check (e.g. posting before the view exists).
- `--md -` reads the markdown from stdin (good for long/heredoc content). An
  inline `--md "..."` honours `\n`/`\t` escapes (shells leave them literal
  inside double quotes), so `--md "# Title\n\nBody"` renders as real lines.
- A tile is a **view** (`--view`, references a named duckdb view) or **inline
  SQL** (`--sql`). Prefer `--view` for anything the human should be able to drill
  into — view tiles get an **explore** button that opens the faceted table
  explorer; inline-SQL tiles get a **sql** button that shows the formatted query.
- Chart kinds: `bar | stacked | line | area | scatter | pie | table | heatmap | box`. For
  `bar`/`line`/etc, put aggregation in the view/SQL (one row per x). If the `--x`
  column is a date/timestamp, the chart uses a real time axis automatically, drawn
  on a **UTC wall-clock** so daily/hourly buckets stay on their boundaries (a
  `DATE` day won't skew by the viewer's timezone).
- **Axis labels**: `--xlabel`/`--ylabel` set the x/y axis titles on any chart.
- **Pick the chart that packs in the most information** — don't default everything
  to single-series bars. `stacked` bars show a total *and* its composition in one
  panel; `area` (and stacked areas with multiple `--y`) show volume and how parts
  evolve over time; `line` is the go-to for **temporal data** (carries trend,
  seasonality, and multiple series on one time axis — prefer it over bars when x is
  continuous time and the *shape* matters); `scatter` shows every raw point;
  `heatmap` crosses two categorical columns and shades each cell by a value
  column — the densest way to show a metric over every (x, y) combination
  (e.g. sites per country x port speed). Shape the view with one row per pair
  (`GROUP BY x, y`) and control axis order with ORDER BY (axes follow row
  order of appearance); `box` draws one box-and-whisker per row on a shared
  scale — the way to compare distributions across groups. `--x` labels each
  box, `--y` takes exactly five columns in order `min,q1,median,q3,max`
  (aggregate in the view: `min(v), quantile_cont(v,0.25), median(v),
  quantile_cont(v,0.75), max(v)`), and `--desc` names a text column whose
  note renders under each label so every box explains itself.
- **Bar fill**: `--bars solid` gives each bar its own palette colour — use it for
  categorical x (methods, status codes, regions). `--bars gradient` (default for a
  single series) suits continuous/over-time data. Colours come from the theme.
- **Caption every chart** with `--caption` (required, not optional) — one line on
  what it shows and the so-what. Pair with `--title` and `--xlabel`/`--ylabel` so
  the panel is self-explanatory; add an adjacent markdown panel for a longer
  description.
- **Daily reporting from timestamps**: bucket to a `DATE` in the view (e.g.
  `ts::DATE AS day` or `date_trunc('week', ts)::DATE`) so there's one row per day
  and bars land on day boundaries — don't plot raw timestamps for per-day charts.
- **Event markers update live**: re-post a tile with the same `--name` plus new
  `--event`/`--target` flags to add or change its markers in place.
- **Sessions are portable**: `session export` bundles the dashboard, full
  snapshots of every database its tiles reference, and their column formats
  into `<name>.muckdb` (a zip); `session import` loads one on any machine (db
  copies land under the data dir in `imports/<id>/`; a name collision gets a
  `-2` suffix). The web UI has an **export** button on the session view and an
  **import** button in the top header.
- `stacked` is a stacked bar: pass multiple `--y` columns (one per series) and
  one row per `--x`; the series stack into each bar's total. Shape the view so
  each series is its own column (e.g. `sum(amount) FILTER (category = 'X')`).
- **Reference lines & markers — use them to draw the eye.** `--target`/`--threshold`
  draw horizontal lines at a y-value (anchor a series against an SLA/budget/limit);
  `--event` draws a vertical line at an x-position (timestamp or category) and is
  the best way to flag an important moment — a deploy, incident, or campaign. Add
  one to essentially every time series. Each is `VALUE` or `VALUE|label`, e.g.
  `--threshold '30|max'`.
- **Trendline — `--trend`** overlays a smoothed trendline (locally-weighted
  regression) on a single-series `bar`/`line`/`area`/`scatter` tile — add it by
  default to records/metric-over-time charts. Ignored on stacked/multi-series tiles.

## Column display formats — set them, always

**Format every numeric column that has a unit.** A bare `4343.33` makes the human
guess (dollars? ms? a count?); `$4,343.33 USD` answers it. Make it a standard step
right after you build your views and before posting tiles — one command per column,
applies everywhere it appears.

Attach a unit/currency/decimal format to a column so facets, charts, stats and
tables render it nicely (e.g. `$4,343.33 USD`):

```
muckdb format <db> <column> [--table T] [--currency USD | --prefix $ --suffix ' USD' --decimals N --thousands --percent] [--tz local|utc|Area/City] [--epoch s|ms|us] [--link URL] [--link-title T] [--clear]
muckdb format list [<db>]
```

A registry entry keyed by column name applies wherever that column appears
(including derived view columns charts plot); `--table` scopes it to one
relation — use the name a tile actually queries (the **view** you post, not the
base table under it). Formats also read from a DuckDB column comment
(`COMMENT ON COLUMN t.c IS 'muckdb:{"prefix":"$","decimals":2,"group":true}'`),
which travels with the database; the registry overrides the comment.

**Timestamps.** Naive DB timestamps are treated as UTC instants. `--tz local`
(or `utc`, or an IANA zone like `Australia/Brisbane`) shows a timestamp column
in that zone in tables, facets and stats — and a chart with that column on x
draws its time axis in the same zone. Without a `--tz`, time axes render the
UTC wall clock so daily/hourly buckets stay on their boundaries. `--epoch
s|ms|us` marks a numeric column as an epoch so it displays and charts as time
(columns with time-ish names and plausible epoch values are auto-detected).
Time axes are granularity-aware: DATE or midnight-truncated columns never show
hour ticks, first-of-month data ticks monthly, and hourly axes get a bold date
label at each day boundary.

**Links — `--link` / `--link-title`.** Turn a column's cells into hyperlinks
(rendered in the rows view, query results and session `table` tiles; visible in
the schema tab's format column). Both flags take a **template** with the same
substitution system:

```sh
muckdb format app.db user_uuid \
  --link 'https://admin.example.com/companies/{company_uuid}/users/{value}' \
  --link-title 'user {value}'
```

- `{value}` — this column's value; `{any_column}` — the value of **any other
  column in the same row** (e.g. inject a company uuid *and* a user uuid into
  one URL, as above).
- **Encoding**: in `--link` every substitution is percent-encoded by default —
  append `:raw` (`{path:raw}`) to inject verbatim (a column that already holds
  a path/query fragment). In `--link-title` substitutions are verbatim by
  default — append `:url` (`{q:url}`) to percent-encode. Both modifiers work
  in both templates.
- A `{name}` matching no column is left as literal text; NULLs substitute as
  empty strings. `--link-title` is optional — the link text defaults to the
  column's (formatted) value, so `--currency USD --link ...` shows a clickable
  `$1,234.56 USD`.
- As a column comment: `muckdb:{"link":"https://…/{value}","link_title":"open {name}"}`.

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

## Screenshots — see what you built

`muckdb session screenshot <name> [--tile TILE] [--out F.png]` renders the
session (or one tile) exactly as the web UI shows it and writes a PNG — read the
file to check your dashboard looks right. Omit `--tile` for the whole dashboard;
the height auto-fits the content. Needs a Chromium-based browser (chromium/
chrome/brave/edge, or `MUCKDB_BROWSER=/path`). The same render is available as
`GET /api/shot?session=<id>&tile=<name>` (`image/png`) and behind the copy-image
button on every panel in the web UI.

## Good habits

- **Keep session databases somewhere durable.** Tiles keep pointing at the
  `--db` path they were posted with; a db created under `/tmp` or a session
  scratchpad breaks the dashboard ("database does not exist") when that dir is
  cleaned. Use the project dir or a stable data dir (e.g.
  `~/.local/share/muckdb/data/`) — `session tile` warns on temp paths.
- **Aggregate in SQL, not in the chart.** A tile plots rows as-is, so write the
  view to return exactly the series you want (`GROUP BY`, `ORDER BY`, a sensible
  `LIMIT`).
- **Caption and label every tile.** Always pass `--caption`, and on charts
  `--title`/`--xlabel`/`--ylabel` — an unlabelled panel isn't done.
- **Format numeric columns before posting tiles.** Set a `muckdb format` for every
  money/duration/rate/count column a panel will show — see "Column display formats".
- **Markdown for narrative, charts for data.** Lead with a markdown summary tile
  (prose + a markdown table of the key figures), then supporting chart tiles.
  Never dump raw rows into chat — summarise in a markdown panel and put the data
  in a chart or explorable view tile beside it.
- **Update, don't duplicate.** Keep `--name`s stable across a task; the dashboard
  updates live (WebSocket) each time you post.
- **Respect the trash.** The human can trash a panel in the UI; the flag persists
  on the tile (`trashed: true` in `muckdb ls session`) and survives re-posts —
  updating a trashed tile does not resurface it. Delete for real with
  `muckdb session rm <session> --tile <name>`.
- **Look at what you built.** `muckdb session screenshot <id> [--tile T]` gives
  you a PNG of the rendered dashboard — read it and check the charts say what
  you think they say before telling the human it's done.
- **Give the human the link.** `http://localhost:11000/session/<id>/` — deep-links
  to a specific table/view/query also work, e.g.
  `/db/<id>/<table>/?view=stats` (stats tabs: `&tab=correlation|timeseries|junk`).
- Queries the daemon runs (introspection, tiles, the editor) are **read-only**.

## Where state lives

Sessions are JSON files under the muckdb data dir
(`~/.local/share/muckdb/sessions/` on Linux,
`~/Library/Application Support/muckdb/sessions/` on macOS); the command ledger is
`history.jsonl` beside it. The CLI writes; the daemon watches and pushes updates.

## Developing muckdb (keep CI green)

CI (`.github/workflows/ci.yml`) runs, in order, on every push/PR across
ubuntu + macos: `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings`
→ `cargo build` → `cargo test`. `fmt --check` is strict and is the most common
cause of red — committing code that hasn't been run through `cargo fmt` fails the
whole job.

**Before committing any Rust change, always run these and only commit once they
pass clean:**

```sh
cargo fmt              # not --check — actually format the tree
cargo clippy --all-targets -- -D warnings
cargo test
```

A tracked pre-commit hook (`.githooks/pre-commit`) runs `cargo fmt` and re-stages
any reformatted `.rs` files so the format check can't go red. `core.hooksPath` is
per-clone local config, so enable it once after cloning:

```sh
git config core.hooksPath .githooks
```

## Cutting a release (binaries + Homebrew)

**Plain pushes to `main` do NOT build a release** — they only run CI. The release
workflow (`.github/workflows/release.yml`: build macOS+Linux binaries → create the
GitHub release → bump the Homebrew tap formula) fires **only on a pushed `v*`
tag**. So after merging changes you want to ship, you must cut a tagged release —
it won't happen on its own.

Use **`cargo release`** (the `cargo-release` crate, configured by `release.toml`).
It does the whole flow in one command — bump `Cargo.toml` + `Cargo.lock`, make the
`chore: release X.Y.Z` commit, tag it `vX.Y.Z`, and push the branch + tag (which is
what fires `release.yml`). `release.toml` sets `publish = false` so it never
touches crates.io.

```sh
cargo install cargo-release      # one-time, if not already installed

# Working tree must be clean first (cargo release refuses with uncommitted changes).
cargo release patch              # dry-run: shows the plan, changes nothing
cargo release patch --execute    # actually bump + commit + tag + push
#            ^ patch | minor | major, or an exact version e.g. `cargo release 0.2.0`
```

Then confirm the build started with `gh run list --workflow=release.yml`.

The convention is **one bump commit per release, and that commit is what gets
tagged** (the `vX.Y.Z` tag points at the commit that sets `version = "X.Y.Z"`) —
`cargo release` produces exactly that. Commit any other changes *before* releasing
so the release commit contains only the version bump.

Manual fallback (if `cargo release` is unavailable) — same end result:

```sh
# 1. Bump the version in BOTH Cargo.toml and Cargo.lock (the [[package]] muckdb entry).
git commit -am "chore: release X.Y.Z"        # 2. commit just the bump
git tag vX.Y.Z && git push origin main && git push origin vX.Y.Z   # 3. tag + push
```

`git push` alone does not push tags — push the tag explicitly. Check the latest
released version with `git tag --sort=-creatordate | head` (and `git describe
--tags` shows how many commits HEAD is ahead of the last tag).
