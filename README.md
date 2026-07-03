# muckdb

muckdb runs the [DuckDB](https://duckdb.org) CLI for you and adds a live web
view. Run `muckdb` exactly like you'd run `duckdb` — every argument is passed
straight through — and muckdb quietly does two extra things:

1. **Self-serves a live web view.** The first time you run it (or any time you
   run `muckdb --display`), it launches a background daemon that serves a web UI
   on **port 11000**. It binds **127.0.0.1** by default (the console exposes every
   database muckdb has touched); set `MUCKDB_BIND=0.0.0.0` on the daemon to open
   it to your LAN, where it's advertised over **mDNS** for discovery.
2. **Keeps a live ledger.** Every invocation is recorded, and whenever a command
   touches a database, the web view presents that database's tables — rows
   (with search, facets, sorting, pagination), per-column stats with histograms,
   schema, a SQL query editor, and CSV/JSON export — updating in real time over a
   WebSocket. It also hosts **sessions** (see below).

It's a drop-in for the duckdb CLI, so exit codes, the interactive shell, and
piping all behave exactly as before.

## Install

```sh
brew install nickkaltner/muckdb/muckdb
```

This taps `nickkaltner/homebrew-muckdb` and installs a prebuilt binary. The
`duckdb` CLI is pulled in as a dependency (muckdb shells out to it).

### Claude skill

muckdb bundles a Claude Code skill that teaches coding agents how to drive it
(sessions, tiles, JSON introspection). Install it into your skills directory:

```sh
muckdb skill install     # → ~/.claude/skills/muckdb/SKILL.md (--force to update)
muckdb skill uninstall   # remove it again
```

### From source

```sh
cargo install --path .
# or
cargo build --release   # binary at target/release/muckdb
```

Requires the `duckdb` binary on your `PATH`.

## Usage

```sh
muckdb                       # interactive duckdb shell (+ starts the daemon)
muckdb mydata.db             # open a database (it appears in the web view)
muckdb mydata.db -c "SELECT 42"
muckdb -json :memory: -c "SELECT 1"

muckdb --display             # ensure the daemon is up and open the web view
muckdb --status              # is the daemon running?
muckdb --stop                # stop the daemon

muckdb ls databases          # read state back as JSON (also: tables <db>,
muckdb ls sessions           #   sessions, session <id>, history [--limit N])
```

Then open <http://localhost:11000>. (With `MUCKDB_BIND=0.0.0.0`, other devices
can find `muckdb` via mDNS: `avahi-browse _muckdb._tcp` on Linux,
`dns-sd -B _muckdb._tcp` on macOS.)

## Sessions (agent dashboards)

A **session** is a named dashboard of **tiles** (panels) that a tool like Claude
Code can post to from the CLI and update by name. Tiles are markdown notes or
**data views** — backed by a duckdb view or inline SQL — rendered as a chart and
explorable as a faceted search. Set `MUCKDB_SESSION` and your commands are also
grouped under that session in the ledger.

```sh
muckdb session create analysis --title "Pond analysis"

# a markdown panel (text or - for stdin)
muckdb session post analysis --name notes --title Notes \
  --md "# Findings\n\n- pH trends **down** over time"

# a data panel from a duckdb view, charted as a bar
muckdb mydb.db -c "CREATE VIEW by_species AS SELECT species, count(*) n FROM readings GROUP BY 1"
muckdb session tile analysis --name species --db mydb.db --view by_species \
  --chart bar --x species --y n --title "By species"

# or straight from inline SQL, as a scatter
muckdb session tile analysis --name temp --db mydb.db \
  --sql "SELECT temp_c, ph FROM readings" --chart scatter --x temp_c --y ph

muckdb session list
muckdb session rm analysis --tile temp     # or: rm analysis (whole session)

# capture the dashboard (or one tile) as a PNG — lets an agent *see* the result
muckdb session screenshot analysis --tile species --out species.png

# move a dashboard between machines: a .muckdb is a zip of the session plus
# full snapshots of every database its tiles reference (and their formats)
muckdb session export analysis                # writes ./analysis.muckdb
muckdb session import analysis.muckdb        # imports; name collisions get -2
```

Re-running `post`/`tile` with the same `--name` updates that tile in place; the
dashboard updates live. Charts: `bar | line | area | scatter | pie | table`.
Each data tile has an **explore** button that opens the view in the faceted
table explorer, and every panel has a **copy-image** button that puts a PNG of
the rendered panel on the clipboard, plus an **✕** that hides it into a *trash*
section of the contents sidebar (click it there to restore it — a per-browser
preference; other viewers and screenshots still see every panel).

`session screenshot` (and the copy-image button) render through a local headless
Chromium — install chromium/chrome/brave/edge, or point `MUCKDB_BROWSER` at a
browser binary. The image auto-fits the rendered content height.

**Try it:** `./demo.sh` seeds sample data (sales, a regular sensor series, and an
irregular event stream) and builds a demo dashboard, then prints the URL.

## How it works

- **CLI role** (the default): muckdb ensures the daemon is running, appends a
  record of the invocation to a shared on-disk log, then runs `duckdb` with your
  arguments, inheriting stdin/stdout/stderr.
- **Daemon role**: a detached background process (`nohup`-style) running an
  [axum](https://github.com/tokio-rs/axum) HTTP + WebSocket server, advertising
  itself over mDNS, watching the log file, and pushing updates to the browser.
- **Shared store**: an append-only JSONL file under your data directory
  (`~/.local/share/muckdb/history.jsonl` on Linux,
  `~/Library/Application Support/muckdb/` on macOS). The CLI and daemon are
  decoupled — the CLI never talks to the daemon directly.
- **Database views**: the daemon reads databases by shelling out to
  `duckdb -readonly -json`, so reads go through the same CLI you'd use by hand.

### API

The daemon also exposes JSON endpoints (handy for other mDNS clients):

| Endpoint | Description |
|----------|-------------|
| `GET /api/state` | history + known databases |
| `GET /api/databases` | databases with ids + existence flags |
| `GET /api/tables?db=PATH` | tables/views in a database |
| `GET /api/preview?db&table&limit&offset&q&filter&sort&dir` | a page of rows (filtered/sorted) |
| `GET /api/facets?db&table&q&filter` | per-column facets (values, numeric range, date range) |
| `GET /api/stats?db&table` | per-column stats + histograms |
| `GET /api/schema?db&table` | column definitions |
| `GET /api/query?db&sql` | run a read-only query |
| `GET /api/export?db&table&format=csv\|json&q&filter` | download the filtered set |
| `GET /api/sessions` | session dashboards (summaries) |
| `GET /api/session?id=ID` | one session with its tiles |
| `GET /api/session/export?id=ID` | download a session as a `.muckdb` archive (session + db snapshots) |
| `POST /api/session/import` | install a `.muckdb` archive (raw zip body) |
| `GET /api/shot?session=ID&tile=NAME&width=W&height=H` | render a session (or one tile) to PNG via headless Chromium |
| `GET /ws` | WebSocket; pushes history + databases + sessions on every change |

The web UI deep-links via clean paths like `/db/<id>/<table>/?view=stats&sort=...`.

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## License

MIT © Nick Kaltner
