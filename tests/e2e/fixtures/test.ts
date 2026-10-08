import { chromium, test as base, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { BASE_PORT, BINARY, E2EState } from '../constants';
import { seed } from './seed';

type TestFixtures = { sessionTiles: string[] };

type WorkerFixtures = { e2eState: E2EState };

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

function isolatedEnv(tmpDir: string, browserPath: string): NodeJS.ProcessEnv {
  return {
    ...process.env,
    XDG_DATA_HOME: join(tmpDir, 'data'),
    XDG_STATE_HOME: join(tmpDir, 'state'),
    MUCKDB_BIND: '127.0.0.1',
    MUCKDB_BROWSER: browserPath,
  };
}

async function waitForServer(url: string, timeoutMs = 15000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // The worker-local daemon is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`muckdb daemon did not serve ${url} within ${timeoutMs}ms`);
}

export const test = base.extend<TestFixtures, WorkerFixtures>({
  sessionTiles: [[], { option: true }],
  page: async ({ page, sessionTiles }, use) => {
    // Focused tests use real daemon data without rendering unrelated panels.
    // Full-dashboard integration tests retain the default empty filter.
    if (sessionTiles.length) {
      await page.route(`/api/session?id=e2e`, async (route) => {
        const response = await route.fetch();
        const session = await response.json();
        session.tiles = session.tiles.filter((tile: { name: string }) => sessionTiles.includes(tile.name));
        expect(session.tiles.map((tile: { name: string }) => tile.name).sort()).toEqual([...sessionTiles].sort());
        await route.fulfill({ response, json: session });
      });
    }
    await use(page);
  },
  e2eState: [async ({}, use, workerInfo) => {
    const port = BASE_PORT + workerInfo.parallelIndex;
    const tmpDir = mkdtempSync(join(tmpdir(), `muckdb-e2e-w${workerInfo.parallelIndex}-`));
    const browserPath = join(tmpDir, 'chromium-no-sandbox');
    writeFileSync(browserPath, `#!/bin/sh\nexec ${shellQuote(chromium.executablePath())} --no-sandbox "$@"\n`, { mode: 0o755 });
    const env = isolatedEnv(tmpDir, browserPath);
    const stateFile = join(tmpDir, 'state.json');
    const dbPath = join(tmpDir, 'widgets.duckdb');
    process.env.MUCKDB_E2E_STATE = stateFile;
    mkdirSync(join(tmpDir, 'data', 'muckdb'), { recursive: true });
    mkdirSync(join(tmpDir, 'state'), { recursive: true });
    writeFileSync(join(tmpDir, 'data', 'muckdb', 'update-check.json'), JSON.stringify({
      last_checked_at: Date.now(), latest_version: 'v99.0.0',
    }));

    try {
      execFileSync(BINARY, ['--port', String(port), 'start'], { env, stdio: 'pipe' });
      await waitForServer(`http://127.0.0.1:${port}/`);
      seed(env, BINARY, dbPath, port);
      const dbs = JSON.parse(execFileSync(
        BINARY, ['--port', String(port), 'ls', 'databases'], { env, encoding: 'utf8' },
      )) as Array<{ id: string; path: string }>;
      const entry = dbs.find((db) => db.path === dbPath)
        ?? dbs.find((db) => db.path.endsWith('widgets.duckdb'));
      if (!entry) throw new Error(`seeded db ${dbPath} not found in ls databases`);
      const state = { tmpDir, browserPath, port, dbId: entry.id, sessionId: 'e2e' };
      writeFileSync(stateFile, JSON.stringify(state));
      await use(state);
    } finally {
      try {
        execFileSync(BINARY, ['--port', String(port), '--stop'], { env, stdio: 'pipe' });
      } catch {
        // Best effort; removing the isolated state still prevents contamination.
      }
      delete process.env.MUCKDB_E2E_STATE;
      rmSync(tmpDir, { recursive: true, force: true });
    }
  }, { scope: 'worker' }],
  baseURL: async ({ e2eState }, use) => {
    await use(`http://127.0.0.1:${e2eState.port}`);
  },
});

export { expect };
export type { Response } from '@playwright/test';
