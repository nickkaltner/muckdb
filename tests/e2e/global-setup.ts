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
