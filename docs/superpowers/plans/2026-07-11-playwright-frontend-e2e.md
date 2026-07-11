# Playwright Frontend E2E Tests + CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Playwright end-to-end suite that drives the real muckdb web UI in Chromium, plus a blocking `e2e.yml` CI workflow that runs it on every PR and push to `main`.

**Architecture:** A Node/TypeScript Playwright harness under `tests/e2e/`. A `global-setup` seeds a deterministic duckdb database + session and starts a real muckdb daemon on a fixed non-default port (`12700`) with its XDG state redirected to a temp dir (so it never touches the developer's real muckdb state); `global-teardown` stops the daemon and removes the temp dir. Specs navigate the served app by URL and assert on existing DOM hooks — no production-source changes. CI installs a pinned duckdb, builds muckdb release, installs Chromium, and runs the suite.

**Tech Stack:** `@playwright/test` (TypeScript), Node 22, npm; the muckdb release binary; duckdb CLI `v1.5.3`; GitHub Actions.

## Global Constraints

- **No production-source changes.** Specs must use existing selectors only (see the Selector Reference below). If a selector proves insufficient during implementation, STOP and flag it — do not silently edit `src/assets/index.html`.
- **State isolation is mandatory.** Every `muckdb` invocation in the harness (seed, start, stop, ls) runs with `XDG_DATA_HOME`/`XDG_STATE_HOME` pointed at the run's temp dir and `MUCKDB_BIND=127.0.0.1`. Never run a harness `muckdb` command without that env.
- **Fixed test port: `12700`.** Well away from the default `11000` so it never collides with a developer's or user's running daemon.
- **duckdb pinned to `v1.5.3`** in CI (matches local dev). Seed SQL uses only stable core features (`range()`, list literals, basic types).
- **The release binary path is `target/release/muckdb`** relative to the repo root. The harness resolves it from the repo root (two levels up from `tests/e2e/`).
- Commit messages end with:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- The worktree is `../muckdb-playwright` on branch `playwright-frontend-tests`. All paths below are relative to the repo root of that worktree.

## Selector Reference (verified against `src/assets/index.html`)

- Titlebar buttons: `#theme-btn`, `#credits-btn` (theme sits immediately before credits).
- Tabs: `#tabs .tab[data-tab="databases"|"sessions"|"ledger"]`.
- Table rows view: `table.preview`; result count element `#tp-results` (text like `"200 results"`); pagination range `.range` (`"showing 1–25 of 200"`).
- Cell filter buttons: within a `td`, `.cellf` (revealed on `td:hover`). The `+` button is `.cellf:not([data-fnot])`; the `≠` button is `.cellf[data-fnot]`. Both carry `data-fcol`/`data-fval`.
- Active filter chips: `.active-filters .fchip` (text contains `=`, `≠`, `∋`, `∌`, or `is NULL`).
- Facet panel (visible at desktop viewport ≥ the breakpoint): `.facet-panel`; facet value buttons `.facet-val[data-fcol][data-fval]`.
- CSV/JSON export: `.exlink[data-export="csv"]`, `.exlink[data-export="json"]` (in the rows view export span).
- Session dashboard: panels `.panel`; chart tiles contain a `<canvas>` (Chart.js); table/heatmap tiles render an HTML `<table>` (`miniTable`). Target a specific tile by its title text: `page.locator('.panel', { hasText: '<title>' })`.
- Routing is path-based via `history.pushState`: table view = `/db/<dbId>/<table>/`, session = `/session/<sessionId>/`.

## Seed Fixture (deterministic)

Database `widgets.duckdb`, built with these statements (each `category` value appears exactly 40 times across 200 rows; complement of one category = 160):

```sql
CREATE TABLE widgets AS
SELECT i AS id,
       (['Alpha','Beta','Gamma','Delta','Epsilon'])[(i % 5) + 1] AS category,
       (['US','EU','APAC'])[(i % 3) + 1]                        AS region,
       round((i * 7) % 100 + 0.5, 2)                            AS price,
       TIMESTAMP '2026-01-01 00:00:00' + (i * INTERVAL 6 HOUR)  AS created
FROM range(200) t(i);
CREATE VIEW widgets_all  AS SELECT * FROM widgets;
CREATE VIEW by_category  AS SELECT category, count(*) AS n FROM widgets GROUP BY 1 ORDER BY n DESC;
CREATE VIEW by_day       AS SELECT created::DATE AS day, count(*) AS n FROM widgets GROUP BY 1 ORDER BY 1;
```

Session `e2e` (slug `e2e`) with tiles: a markdown `summary`, a bar tile `by-cat` (view `by_category`, `--x category --y n`), a line tile `by-day` (view `by_day`, `--x day --y n`), and a table tile `all` (view `widgets_all`, `--chart table`).

---

### Task 1: Harness scaffold, app bring-up, and smoke test

**Files:**
- Create: `tests/e2e/package.json`
- Create: `tests/e2e/tsconfig.json`
- Create: `tests/e2e/playwright.config.ts`
- Create: `tests/e2e/constants.ts`
- Create: `tests/e2e/fixtures/seed.ts`
- Create: `tests/e2e/global-setup.ts`
- Create: `tests/e2e/global-teardown.ts`
- Create: `tests/e2e/specs/smoke.spec.ts`
- Create: `tests/e2e/.gitignore`

**Interfaces:**
- Produces:
  - `constants.ts`: `export const PORT = 12700; export const BASE_URL = 'http://127.0.0.1:12700'; export const SESSION_ID = 'e2e';`
  - `constants.ts`: `export function readState(): { tmpDir: string; port: number; dbId: string; sessionId: string }` — reads `tests/e2e/.e2e-state.json`.
  - `seed.ts`: `export function seed(env: NodeJS.ProcessEnv, binary: string, dbPath: string): void` — runs all `muckdb` seed commands synchronously; throws on any non-zero exit.
  - `.e2e-state.json` (written by global-setup, git-ignored): `{ tmpDir, port, dbId, sessionId }`.

- [ ] **Step 1: Create `tests/e2e/.gitignore`**

```
node_modules/
.e2e-state.json
test-results/
playwright-report/
```

- [ ] **Step 2: Create `tests/e2e/package.json`**

```json
{
  "name": "muckdb-e2e",
  "private": true,
  "version": "0.0.0",
  "scripts": {
    "test": "playwright test"
  },
  "devDependencies": {
    "@playwright/test": "^1.48.0",
    "@types/node": "^22.0.0",
    "typescript": "^5.6.0"
  }
}
```

- [ ] **Step 3: Create `tests/e2e/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "CommonJS",
    "moduleResolution": "node",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "types": ["node"]
  }
}
```

- [ ] **Step 4: Create `tests/e2e/constants.ts`**

```ts
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

export const PORT = 12700;
export const BASE_URL = `http://127.0.0.1:${PORT}`;
export const SESSION_ID = 'e2e';

export interface E2EState {
  tmpDir: string;
  port: number;
  dbId: string;
  sessionId: string;
}

export const STATE_FILE = join(__dirname, '.e2e-state.json');

export function readState(): E2EState {
  return JSON.parse(readFileSync(STATE_FILE, 'utf8')) as E2EState;
}
```

- [ ] **Step 5: Create `tests/e2e/fixtures/seed.ts`**

```ts
import { execFileSync } from 'node:child_process';
import { PORT } from '../constants';

// Run one `muckdb` command with the isolated env; throws on failure.
function run(binary: string, env: NodeJS.ProcessEnv, args: string[]): void {
  execFileSync(binary, ['--port', String(PORT), ...args], {
    env,
    stdio: 'pipe',
  });
}

const CREATE_SQL = `
CREATE TABLE widgets AS
SELECT i AS id,
       (['Alpha','Beta','Gamma','Delta','Epsilon'])[(i % 5) + 1] AS category,
       (['US','EU','APAC'])[(i % 3) + 1]                        AS region,
       round((i * 7) % 100 + 0.5, 2)                            AS price,
       TIMESTAMP '2026-01-01 00:00:00' + (i * INTERVAL 6 HOUR)  AS created
FROM range(200) t(i);
CREATE VIEW widgets_all AS SELECT * FROM widgets;
CREATE VIEW by_category AS SELECT category, count(*) AS n FROM widgets GROUP BY 1 ORDER BY n DESC;
CREATE VIEW by_day AS SELECT created::DATE AS day, count(*) AS n FROM widgets GROUP BY 1 ORDER BY 1;
`;

// Build the seed database + session. `dbPath` must live under the run's temp dir.
export function seed(env: NodeJS.ProcessEnv, binary: string, dbPath: string): void {
  // 1. Build the database (this also registers it in the ledger so the daemon can browse it).
  run(binary, env, [dbPath, '-c', CREATE_SQL]);

  // 2. Build the dashboard session.
  run(binary, env, ['session', 'create', 'e2e', '--title', 'E2E fixtures']);
  run(binary, env, ['session', 'post', 'e2e', '--name', 'summary', '--title', 'Summary',
    '--md', '# E2E\n\n**200 widgets**, 5 categories.']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'by-cat', '--title', 'By category',
    '--db', dbPath, '--view', 'by_category', '--chart', 'bar', '--x', 'category', '--y', 'n',
    '--caption', 'Widgets per category (deterministic: 40 each).']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'by-day', '--title', 'By day',
    '--db', dbPath, '--view', 'by_day', '--chart', 'line', '--x', 'day', '--y', 'n',
    '--caption', 'Widgets created per day.']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'all', '--title', 'All widgets',
    '--db', dbPath, '--view', 'widgets_all', '--chart', 'table',
    '--caption', 'The full flattened list.']);
}
```

- [ ] **Step 6: Create `tests/e2e/global-setup.ts`**

```ts
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { PORT, STATE_FILE, E2EState } from './constants';
import { seed } from './fixtures/seed';

const REPO_ROOT = resolve(__dirname, '..', '..');
const BINARY = join(REPO_ROOT, 'target', 'release', 'muckdb');

function isolatedEnv(tmpDir: string): NodeJS.ProcessEnv {
  return {
    ...process.env,
    XDG_DATA_HOME: join(tmpDir, 'data'),
    XDG_STATE_HOME: join(tmpDir, 'state'),
    MUCKDB_BIND: '127.0.0.1',
  };
}

async function waitForServer(url: string, timeoutMs = 15000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`muckdb daemon did not serve ${url} within ${timeoutMs}ms`);
}

export default async function globalSetup(): Promise<void> {
  const tmpDir = mkdtempSync(join(tmpdir(), 'muckdb-e2e-'));
  mkdirSync(join(tmpDir, 'data'), { recursive: true });
  mkdirSync(join(tmpDir, 'state'), { recursive: true });
  const env = isolatedEnv(tmpDir);
  const dbPath = join(tmpDir, 'widgets.duckdb');

  // Start the daemon on the isolated port, then seed (seed's first passthrough would
  // also start it, but starting explicitly makes readiness deterministic).
  execFileSync(BINARY, ['--port', String(PORT), 'start'], { env, stdio: 'pipe' });
  await waitForServer(`http://127.0.0.1:${PORT}/`);

  seed(env, BINARY, dbPath);

  // Resolve the db id the daemon assigned (needed for /db/<id>/... URLs).
  const dbsJson = execFileSync(BINARY, ['--port', String(PORT), 'ls', 'databases'], {
    env,
    encoding: 'utf8',
  });
  const dbs = JSON.parse(dbsJson) as Array<{ id: string; path: string }>;
  // Exact match first; fall back to basename in case muckdb canonicalized a
  // symlinked temp path (safe — state is isolated so only our db is registered).
  const entry =
    dbs.find((d) => d.path === dbPath) ?? dbs.find((d) => d.path.endsWith('widgets.duckdb'));
  if (!entry) throw new Error(`seeded db ${dbPath} not found in ls databases`);

  const state: E2EState = { tmpDir, port: PORT, dbId: entry.id, sessionId: 'e2e' };
  writeFileSync(STATE_FILE, JSON.stringify(state, null, 2));
}
```

- [ ] **Step 7: Create `tests/e2e/global-teardown.ts`**

```ts
import { execFileSync } from 'node:child_process';
import { rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { readState } from './constants';

const BINARY = join(resolve(__dirname, '..', '..'), 'target', 'release', 'muckdb');

export default async function globalTeardown(): Promise<void> {
  let state;
  try {
    state = readState();
  } catch {
    return; // setup never completed; nothing to clean.
  }
  const env = {
    ...process.env,
    XDG_DATA_HOME: join(state.tmpDir, 'data'),
    XDG_STATE_HOME: join(state.tmpDir, 'state'),
  };
  try {
    execFileSync(BINARY, ['--port', String(state.port), '--stop'], { env, stdio: 'pipe' });
  } catch {
    // best-effort; the temp dir removal below still isolates state.
  }
  rmSync(state.tmpDir, { recursive: true, force: true });
}
```

- [ ] **Step 8: Create `tests/e2e/playwright.config.ts`**

```ts
import { defineConfig, devices } from '@playwright/test';
import { BASE_URL } from './constants';

export default defineConfig({
  testDir: './specs',
  globalSetup: require.resolve('./global-setup'),
  globalTeardown: require.resolve('./global-teardown'),
  timeout: 30000,
  fullyParallel: false,
  workers: 1,
  reporter: [['html', { open: 'never' }], ['list']],
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
```

- [ ] **Step 9: Create `tests/e2e/specs/smoke.spec.ts`**

```ts
import { test, expect } from '@playwright/test';

// Shared helper: fail a test if the page logs any uncaught error, with a small
// allowlist for benign noise.
const ALLOW = [/favicon/i];
function guardConsole(page: import('@playwright/test').Page, errors: string[]): void {
  page.on('console', (m) => {
    if (m.type() === 'error' && !ALLOW.some((re) => re.test(m.text()))) errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push(String(e)));
}

test('page loads with tabs and no console errors', async ({ page }) => {
  const errors: string[] = [];
  guardConsole(page, errors);

  await page.goto('/');
  await expect(page.locator('#tabs .tab[data-tab="databases"]')).toBeVisible();
  await expect(page.locator('#tabs .tab[data-tab="sessions"]')).toBeVisible();
  await expect(page.locator('#tabs .tab[data-tab="ledger"]')).toBeVisible();

  // Theme button sits immediately before the credits (?) button (guards the recent move).
  const ids = await page.locator('.titlebar button.kbtn').evaluateAll((els) =>
    els.map((e) => e.id),
  );
  expect(ids.indexOf('theme-btn')).toBe(ids.indexOf('credits-btn') - 1);

  await page.waitForTimeout(300);
  expect(errors, `console errors:\n${errors.join('\n')}`).toEqual([]);
});
```

- [ ] **Step 10: Install dependencies and Chromium**

Run (from `tests/e2e/`):
```bash
cd tests/e2e && npm install && npx playwright install chromium
```
Expected: dependencies install; Chromium downloads. (Also ensure `cargo build --release` has been run at the repo root so `target/release/muckdb` exists, and `duckdb` is on PATH.)

- [ ] **Step 11: Run the smoke test to verify the whole harness**

Run (from `tests/e2e/`):
```bash
npx playwright test smoke
```
Expected: PASS (1 test). The daemon starts on 12700 with isolated state, seeds, serves, the page renders, and teardown stops it. Confirm no daemon is left: `./target/release/muckdb --port 12700 --status` should report not running.

- [ ] **Step 12: Commit**

```bash
git add tests/e2e/ docs/superpowers/plans/2026-07-11-playwright-frontend-e2e.md
git commit -m "test(e2e): scaffold Playwright harness, app bring-up, smoke test

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Filter round-trip spec (`+` and `≠`)

**Files:**
- Create: `tests/e2e/specs/filters.spec.ts`

**Interfaces:**
- Consumes: `readState()` (`dbId`), `.range`/`#tp-results`, `.cellf`, `.fchip` from Task 1's Selector Reference.

- [ ] **Step 1: Write the spec**

```ts
import { test, expect } from '@playwright/test';
import { readState } from '../constants';

test.describe('cell value filters', () => {
  test('+ pins to a value, ≠ excludes it', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/widgets/`);

    // Baseline: 200 rows.
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);

    // Hover a cell whose category is "Alpha", then click its + button.
    // NOTE: the .cellf button lives inside the value <td>, so the cell text is
    // "Alpha" + the button glyph — use a substring match, not an anchored regex.
    const alphaCell = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alphaCell.hover();
    await alphaCell.locator('.cellf:not([data-fnot])').click();

    // 40 of 200 rows are Alpha; an "=" chip appears.
    await expect(page.locator('#tp-results')).toHaveText(/40 results/);
    await expect(page.locator('.active-filters .fchip')).toContainText('=');

    // Remove the filter (click its chip) → back to 200.
    await page.locator('.active-filters .fchip').first().click();
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);

    // Now the ≠ button on an Alpha cell → complement (160 rows) with a "≠" chip.
    const alphaCell2 = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alphaCell2.hover();
    await alphaCell2.locator('.cellf[data-fnot]').click();
    await expect(page.locator('#tp-results')).toHaveText(/160 results/);
    await expect(page.locator('.active-filters .fchip')).toContainText('≠');
  });
});
```

- [ ] **Step 2: Run to verify it passes**

Run: `cd tests/e2e && npx playwright test filters`
Expected: PASS. (If the ≠ selector or count is wrong, the failure trace shows the rendered DOM — do NOT edit index.html; re-check the Selector Reference.)

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/specs/filters.spec.ts
git commit -m "test(e2e): + and ≠ cell-filter round-trip

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Session dashboard spec

**Files:**
- Create: `tests/e2e/specs/session.spec.ts`

**Interfaces:**
- Consumes: `SESSION_ID`, `.panel` locator by title text.

- [ ] **Step 1: Write the spec**

```ts
import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('seeded session renders its tiles', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  // Markdown tile.
  await expect(page.locator('.panel', { hasText: 'Summary' })).toBeVisible();
  await expect(page.getByText('200 widgets', { exact: false })).toBeVisible();

  // Each chart tile's panel is present by title.
  await expect(page.locator('.panel', { hasText: 'By category' })).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'By day' })).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'All widgets' })).toBeVisible();
});
```

- [ ] **Step 2: Run to verify it passes**

Run: `cd tests/e2e && npx playwright test session`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/specs/session.spec.ts
git commit -m "test(e2e): seeded session dashboard renders tiles

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Chart rendering spec

**Files:**
- Create: `tests/e2e/specs/charts.spec.ts`

**Interfaces:**
- Consumes: `SESSION_ID`; chart tiles → `<canvas>`, table tile → `<table>`.

- [ ] **Step 1: Write the spec**

```ts
import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('chart tiles render canvases; table tile renders a table', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  // Bar and line tiles draw with Chart.js → a <canvas> inside their panel.
  await expect(page.locator('.panel', { hasText: 'By category' }).locator('canvas')).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'By day' }).locator('canvas')).toBeVisible();

  // The table tile renders an HTML table (miniTable), not a canvas.
  await expect(page.locator('.panel', { hasText: 'All widgets' }).locator('table')).toBeVisible();
});
```

- [ ] **Step 2: Run to verify it passes**

Run: `cd tests/e2e && npx playwright test charts`
Expected: PASS. (Canvas needs a moment to draw; `toBeVisible` auto-waits up to the timeout.)

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/specs/charts.spec.ts
git commit -m "test(e2e): session chart/table tiles render

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Explore / facet-drawer spec

**Files:**
- Create: `tests/e2e/specs/explore.spec.ts`

**Interfaces:**
- Consumes: `readState()` (`dbId`), `.facet-panel .facet-val`, `#tp-results`, `.active-filters .fchip`.

- [ ] **Step 1: Write the spec**

```ts
import { test, expect } from '@playwright/test';
import { readState } from '../constants';

test('facet panel filters the table', async ({ page }) => {
  const { dbId } = readState();
  await page.goto(`/db/${dbId}/widgets/`);
  await expect(page.locator('#tp-results')).toHaveText(/200 results/);

  // The facet panel is visible at desktop viewport; click the "region = US" facet value.
  const usFacet = page
    .locator('.facet-panel .facet-val[data-fcol="region"][data-fval="US"]')
    .first();
  await expect(usFacet).toBeVisible();
  await usFacet.click();

  // region cycles US/EU/APAC over 200 rows: US = ids where i % 3 == 0 → 67 rows.
  await expect(page.locator('#tp-results')).toHaveText(/67 results/);
  await expect(page.locator('.active-filters .fchip')).toContainText('region');
});
```

Note on the expected count: `range(200)` yields `i` 0..199; `i % 3 == 0` → 0,3,…,198 = **67** rows. Verify against the rendered `#tp-results`; if the app labels differently, read the trace — do not change index.html.

- [ ] **Step 2: Run to verify it passes**

Run: `cd tests/e2e && npx playwright test explore`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/specs/explore.spec.ts
git commit -m "test(e2e): facet-panel filtering

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Export (CSV / JSON) spec

**Files:**
- Create: `tests/e2e/specs/export.spec.ts`

**Interfaces:**
- Consumes: `readState()` (`dbId`), `.exlink[data-export="csv"|"json"]`, Playwright `download` event.

- [ ] **Step 1: Write the spec**

```ts
import { test, expect } from '@playwright/test';
import { readState } from '../constants';
import { readFileSync } from 'node:fs';

test('CSV and JSON export produce non-empty downloads', async ({ page }) => {
  const { dbId } = readState();
  await page.goto(`/db/${dbId}/widgets/`);
  await expect(page.locator('#tp-results')).toHaveText(/200 results/);

  // CSV.
  const [csv] = await Promise.all([
    page.waitForEvent('download'),
    page.locator('.exlink[data-export="csv"]').click(),
  ]);
  const csvPath = await csv.path();
  const csvText = readFileSync(csvPath, 'utf8');
  expect(csvText.split('\n').length).toBeGreaterThan(1);
  expect(csvText).toContain('category');

  // JSON.
  const [json] = await Promise.all([
    page.waitForEvent('download'),
    page.locator('.exlink[data-export="json"]').click(),
  ]);
  const jsonText = readFileSync(await json.path(), 'utf8');
  const parsed = JSON.parse(jsonText);
  expect(Array.isArray(parsed) ? parsed.length : Object.keys(parsed).length).toBeGreaterThan(0);
});
```

- [ ] **Step 2: Run to verify it passes**

Run: `cd tests/e2e && npx playwright test export`
Expected: PASS. (If the export downloads all rows vs the current page, the assertions only require non-empty/parseable output, so either is fine.)

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/specs/export.spec.ts
git commit -m "test(e2e): CSV/JSON export downloads

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: CI workflow (`e2e.yml`)

**Files:**
- Create: `.github/workflows/e2e.yml`

**Interfaces:**
- Consumes: everything above; the release binary and `duckdb` on PATH.

- [ ] **Step 1: Write the workflow**

```yaml
name: E2E

on:
  push:
    branches: [main]
  pull_request:

jobs:
  e2e:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5

      - name: Install duckdb CLI (pinned)
        run: |
          curl -L -o /tmp/duckdb.zip \
            https://github.com/duckdb/duckdb/releases/download/v1.5.3/duckdb_cli-linux-amd64.zip
          unzip -o /tmp/duckdb.zip -d "$HOME/.local/bin"
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"

      - name: Verify duckdb
        run: duckdb --version

      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Build muckdb (release)
        run: cargo build --release

      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: npm
          cache-dependency-path: tests/e2e/package-lock.json

      - name: Install e2e deps
        working-directory: tests/e2e
        run: npm ci

      - name: Install Chromium
        working-directory: tests/e2e
        run: npx playwright install --with-deps chromium

      - name: Run Playwright tests
        working-directory: tests/e2e
        run: npx playwright test

      - name: Upload report on failure
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: playwright-report
          path: |
            tests/e2e/playwright-report/
            tests/e2e/test-results/
          retention-days: 7
```

- [ ] **Step 2: Generate the lockfile npm ci needs**

Run (from `tests/e2e/`):
```bash
cd tests/e2e && npm install
```
Expected: `tests/e2e/package-lock.json` is created/updated (Task 1 Step 10 already ran `npm install`; confirm the lockfile is present and tracked — it must NOT be git-ignored).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/e2e.yml tests/e2e/package-lock.json
git commit -m "ci: blocking Playwright E2E workflow

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 4: Full-suite sanity run**

Run (from `tests/e2e/`):
```bash
npx playwright test
```
Expected: all specs PASS; `./target/release/muckdb --port 12700 --status` reports not running afterward (teardown cleaned up).

---

## Notes for the implementer

- **Never** run a harness `muckdb` command without the isolated `XDG_DATA_HOME`/`XDG_STATE_HOME` env — you would read/write the developer's real muckdb state and could disturb a running daemon.
- The default daemon on port 11000 (if the developer has one) must stay untouched. Everything here uses port 12700 with a separate pidfile (per the configurable-port feature already on `main`).
- If any spec's expected row count disagrees with the rendered `#tp-results`, the seed arithmetic is the source of truth: 200 rows, 5 categories × 40, region US = 67 (`i % 3 == 0`), EU = 67 (`i % 3 == 1`: 1..199 → 67), APAC = 66. Re-derive rather than guess.
- `package-lock.json` MUST be committed (CI uses `npm ci`); the `.gitignore` excludes only `node_modules/`, `.e2e-state.json`, and report dirs.
