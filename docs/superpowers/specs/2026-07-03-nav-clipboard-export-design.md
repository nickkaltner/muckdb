# Design: pushState navigation, macOS copy-image fix, session export/import

Date: 2026-07-03. Status: approved.

Three independent changes to muckdb's web UI + daemon.

## 1. pushState navigation

**Bug.** `exploreTile()` calls `setTab("databases")`, which pushes an
intermediate URL built from stale state (`/` or the previously selected db)
before `selectDb → selectTable` pushes the real `/db/<id>/<view>/` URL. Back
from explore therefore lands on `/`, which the router parses as "sessions tab,
no session" and auto-loads the *first* session — not the one the user came
from. The ledger's "open db" click has the same compound-navigation shape.

**Rule.** Real navigation pushes exactly one history entry per user action:
tab switches, session opens, db/table selection, subview (rows/stats/schema),
panel expand, explore. Ephemeral in-table state — search text, facet filters,
sort, pagination, page size — keeps updating the URL via `replaceState` so the
URL is always current but Back never steps through keystrokes.

**Changes** (all in `src/assets/index.html`):

- Compound navigations (explore, ledger open-db, anything that switches tab
  *and* selects a db) suppress the intermediate `syncUrl` from `setTab` and let
  the final state push once. Mechanism: `setTab(tab, {sync:false})` (or an
  equivalent suppress flag) used by `exploreTile`/open-db; plain header tab
  clicks keep pushing.
- Audit every `syncUrl(...)` call site against the rule above; most already
  comply.
- `popstate → restoreFromNav` already restores; no server changes.

## 2. Copy image to clipboard on macOS (Chrome)

**Symptom.** Error flash in Chrome on macOS. The error path only triggers when
both the clipboard write *and* the download fallback fail, and both depend on
the server-side `/api/shot` headless-Chromium render — so the shot itself
likely fails on that machine, and the current code hides the reason in
`console.warn` and re-fetches (re-renders) the PNG for the fallback.

**Changes** (in `copyPanelImage`, `src/assets/index.html`):

1. Fetch the PNG **once** (a shared promise) instead of twice.
2. Try in order:
   a. promise-based `ClipboardItem` (required by Safari — keeps the user
      gesture fresh across the multi-second render),
   b. plain-blob `ClipboardItem` after awaiting the fetch (what Chrome
      prefers; Chrome auto-grants `clipboard-write` to the focused tab so the
      elapsed await is fine),
   c. download-the-PNG fallback.
3. On total failure, show a small toast containing the **actual error
   message** (e.g. the `/api/shot` JSON error: "no Chromium-based browser
   found…", browser stderr) instead of a bare red flash, so remaining macOS
   failures are self-diagnosing.

No server changes expected; `/api/shot` already returns a JSON error body.

## 3. Session export / import (`<session>.muckdb`)

A `.muckdb` file is a zip:

```
pond-analysis.muckdb
├── manifest.json     # {format: 1, session id/title, muckdb version, exported ts,
│                     #  dbs: [{id, original_path, file}], formats: [registry entries
│                     #  for those dbs]}
├── session.json      # the session JSON verbatim (original db paths)
└── dbs/<id>.duckdb   # full snapshot of each database referenced by tiles
```

**Export.**

- DB snapshot via the wrapped `duckdb` CLI: `ATTACH '<dst>' AS out; COPY FROM
  DATABASE <cur> TO out;` — clean, checkpointed, compacted, safe against
  WAL/mid-write state. Fallback: `fs::copy` of the `.duckdb` (+ `.wal` if
  present) if the CLI copy errors.
- Zip written with the `zip` crate (new dependency) to a temp file, then
  streamed.
- Web: export button in the session view header →
  `GET /api/session/export?session=<id>` responds with the zip,
  `Content-Disposition: attachment; filename="<name>.muckdb"`.
- CLI: `muckdb session export <name> [--out FILE]` (defaults to
  `./<name>.muckdb`), same Rust function.

**Import.**

- Web: import button in the top header → file picker →
  `POST /api/session/import` with the raw zip body.
- CLI: `muckdb session import <file.muckdb>`.
- The daemon/CLI unpacks db files to `<data-dir>/imports/<session-id>/`,
  rewrites tile `db` paths to those locations, re-keys the imported format
  entries to the new paths' `db_id`s and merges them into the local registry,
  writes the session JSON, and appends a ledger record per imported db so they
  appear in the databases tab.
- Session-id collision → numeric suffix (`pond-analysis-2`), never overwrite.
- Format-version field in the manifest guards future changes; importing an
  unknown version fails with a clear error.

**Placement.** Export/import zip+manifest logic in a new `src/export.rs`;
session-view export button and header import button in `src/assets/index.html`;
routes in `src/server.rs`; CLI subcommands wired in `src/session.rs`'s
dispatcher.

## 4. `muckdb --start` (added after approval)

Start the background daemon without opening a browser: exactly `--display`
minus the `open_browser` call. Sits beside `--status`/`--stop`/`--display` in
`main.rs` and the help/docs.

## Testing

- Rust unit tests: manifest round-trip, session-id collision suffixing, format
  re-keying, zip pack/unpack round-trip (small temp duckdb built via the
  `duckdb` CLI in a tempdir).
- Manual/E2E: build a session, export, import, verify dashboard renders and
  explore works against the imported db; browser Back/Forward across
  session → explore → back.
- Existing gates: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`.
