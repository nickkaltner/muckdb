import { test as base, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { BASE_PORT, BINARY, E2EState } from '../constants';
import { seed } from './seed';

type WorkerFixtures = { e2eState: E2EState };

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
      if ((await fetch(url)).ok) return;
    } catch {
      // The worker-local daemon is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`muckdb daemon did not serve ${url} within ${timeoutMs}ms`);
}

export const test = base.extend<{}, WorkerFixtures>({
  e2eState: [async ({}, use, workerInfo) => {
    const port = BASE_PORT + workerInfo.parallelIndex;
    const tmpDir = mkdtempSync(join(tmpdir(), `muckdb-e2e-w${workerInfo.parallelIndex}-`));
    const env = isolatedEnv(tmpDir);
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
      const state = { tmpDir, port, dbId: entry.id, sessionId: 'e2e' };
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
