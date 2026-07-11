# Playwright frontend E2E tests + CI

**Date:** 2026-07-11
**Branch:** `playwright-frontend-tests` (worktree off `main` @ c1221c9)
**Status:** approved design

## Problem

muckdb has 98 Rust unit/integration tests but **zero automated coverage of the
frontend**. The entire UI — filters, chart adapters, session dashboards, the
explore facet drawer, export — lives in one large `src/assets/index.html` with
inline JS. Regressions there surface only when a human clicks. The recently
shipped NOT (`≠`) cell filter, for example, was verified only by "the JS parses"
plus backend SQL unit tests; the buttons were never exercised.

This adds a Playwright end-to-end suite that drives the real app in a browser,
and a blocking CI workflow that runs it on every PR and push to `main`.

## Goals

- Catch frontend regressions automatically (rendering, filtering, sessions,
  charts, explore, export).
- Exercise the app **full-stack** — a real daemon serving real data — because
  most UI logic depends on live API responses.
- Run in CI as a **separate, blocking** workflow, isolated from the fast Rust
  matrix.

## Non-goals (explicit, for this PR)

- Cross-browser (firefox/webkit) — Chromium only.
- macOS / Windows runners — ubuntu-latest only.
- Visual-regression / screenshot snapshots.
- Testing the CLI or daemon internals (covered by Rust tests).

These are easy follow-ons once the base suite is green.

## Approach: minimal-but-real full-stack E2E

A Node/Playwright (TypeScript, npm) harness that seeds a deterministic database
+ session, starts a real muckdb daemon on an isolated port, and drives it in
Chromium.

### Layout

```
tests/e2e/
  package.json            # @playwright/test devDependency, "test" script
  tsconfig.json
  playwright.config.ts    # chromium project, baseURL, html reporter, trace+screenshot on failure
  global-setup.ts         # build + seed + start daemon; expose port/baseURL
  global-teardown.ts      # stop daemon, remove temp state dir
  fixtures/
    seed.ts               # deterministic dataset + session builder (shells out to the built binary)
  specs/
    smoke.spec.ts         # page loads, no uncaught console errors, tabs/nav render
    filters.spec.ts       # + value filter drops rows; ≠ filter excludes (NOT-filter feature)
    session.spec.ts       # seeded dashboard renders tiles + markdown
    charts.spec.ts        # chart tiles render (bar / line / table)
    explore.spec.ts       # facet drawer filters a table
    export.spec.ts        # CSV / JSON download from a view tile
.github/workflows/e2e.yml
```

### App bring-up & state isolation (the crux)

The daemon self-daemonizes (`fork` + `setsid`) and stores state in an XDG data /
state dir via the `directories` crate. Two consequences shape the harness:

1. **No `webServer` auto-management.** Playwright's built-in `webServer` kills
   the launched process (group) on teardown; it cannot reliably reach a
   detached daemon. Instead the daemon lifecycle is owned by
   `global-setup`/`global-teardown`.
2. **State must be isolated** so tests never read or clobber the developer's real
   sessions/databases. On Linux `directories` honors `XDG_DATA_HOME` /
   `XDG_STATE_HOME`, so `global-setup` points both at a throwaway temp dir and
   passes that env to every spawned muckdb process (seed + daemon). **No source
   change required.** (CI is ubuntu-only, so the XDG path is sufficient; a
   portable `HOME` override is a fallback if we ever add non-Linux runners.)

`global-setup` steps:
- Resolve the release binary path (`target/release/muckdb`); assume `cargo build
  --release` has run (CI does it explicitly; a local npm `pretest` can too).
- Create a temp state dir; set `XDG_DATA_HOME`/`XDG_STATE_HOME` in the env used
  for all muckdb invocations.
- Seed data (`fixtures/seed.ts`): a `widgets` table with a categorical column
  (`category`, ~5 distinct values — for `+`/`≠` filters and facets), a second
  categorical (`region`), a numeric column (`price` — stats, range filter,
  charts), and a `created TIMESTAMP` (time axis), ~200 deterministic rows via
  `range()`. Create views for chart/explore tiles.
- Build a session (`session create` + `post` markdown + `tile` for a bar chart,
  a view/table tile, and a line chart) so `session.spec`, `charts.spec`,
  `explore.spec`, and `export.spec` have fixtures.
- Start the daemon: `muckdb --port <TEST_PORT> start` with
  `MUCKDB_BIND=127.0.0.1`. `TEST_PORT` is a fixed uncommon port (e.g. `12700`)
  well away from the default 11000.
- Poll `--port <TEST_PORT> --status` (or an HTTP GET) until it serves, then
  stash `baseURL` for the config.

`global-teardown`: `muckdb --port <TEST_PORT> --stop`; remove the temp dir.

### Selectors

Prefer existing stable hooks: element ids (`#tabs`, `#view-databases`,
`#view-sessions`), classes (`.cellf`, `.facet-val`, `.fchip`, `.panel`), and
visible text / ARIA roles. Where a target is genuinely ambiguous, add a minimal
`data-testid` to `index.html` — kept to the smallest set necessary and called
out in the implementation plan. This is the only anticipated production-source
change.

### Console-error gate

Each spec attaches `page.on('console')` + `page.on('pageerror')` listeners and
fails the test on any uncaught error / `console.error`, with a small allowlist
for benign noise (e.g. favicon 404, and — under a real daemon this shouldn't
occur — any third-party library warning we explicitly document).

### What each spec asserts

- **smoke:** navigating to `/` renders the titlebar and the three tabs
  (databases / sessions / ledger); no uncaught console errors; the theme button
  sits immediately before the `?` button (guards the recent move).
- **filters:** open the `widgets` table rows view; record total row count; click
  a cell's `+` for a known `category` value → assert rows reduce to that
  category's count and a `=` chip appears. Remove it; click the sibling `≠`
  button → assert rows reduce to the complement (total − that category) and a
  `≠` chip appears. (Directly validates the NOT-filter feature.)
- **session:** open the seeded session; assert the markdown tile and each chart
  tile panel are present with their titles.
- **charts:** assert bar / line / table tiles each render their expected DOM
  (canvas/svg for charts, a table for the table tile).
- **explore:** open a view tile's explore / the facet drawer; toggle a facet
  value; assert the row set changes and the active-filter chip shows.
- **export:** trigger CSV and JSON export on a view; assert a download starts and
  the payload is non-empty / parses.

### CI — `.github/workflows/e2e.yml`

- Triggers: `pull_request`, `push` to `main`. Required check (blocking).
- Runner: `ubuntu-latest`.
- Steps:
  1. `actions/checkout@v5`
  2. Install a **pinned** duckdb CLI — download the official Linux release zip
     (pin the version, e.g. matching the developer's local duckdb), unzip onto
     `PATH`. (duckdb is not in apt.)
  3. `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache@v2`; `cargo build
     --release`.
  4. `actions/setup-node` (LTS) with npm cache; `npm ci` in `tests/e2e`.
  5. `npx playwright install --with-deps chromium`.
  6. `npx playwright test` (cwd `tests/e2e`), with the release binary path and
     `TEST_PORT` in env.
  7. On failure: `actions/upload-artifact` for the Playwright HTML report +
     `test-results/` traces.

## Risks / open questions

- **duckdb version drift** between local and CI — mitigated by pinning the CI
  version; the seed SQL uses only stable core features (`range()`, basic types).
- **Detached-daemon teardown flakiness** — mitigated by owning lifecycle in
  setup/teardown and a bounded readiness poll; if `--stop` ever races, the temp
  XDG dir still isolates state so a leaked process can't corrupt anything real.
- **Download-assert reliability** for export — use Playwright's `waitForEvent
  ('download')` with a timeout and assert on the saved file.
- **`data-testid` creep** — cap it; reassess if more than a handful are needed
  (a sign a spec is over-coupled to markup).

## Success criteria

- `npx playwright test` passes locally (with a built binary + duckdb on PATH)
  and in CI.
- The suite fails if: the page throws, a filter (incl. `≠`) stops working, a
  session/chart tile stops rendering, explore filtering breaks, or export stops
  producing output.
- CI runs isolated from developer/user muckdb state and leaves no daemon behind.
