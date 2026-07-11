import { execFileSync } from 'node:child_process';
import { rmSync } from 'node:fs';
import { join } from 'node:path';
import { readState, BINARY } from './constants';

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
    MUCKDB_BIND: '127.0.0.1',
  };
  try {
    execFileSync(BINARY, ['--port', String(state.port), '--stop'], { env, stdio: 'pipe' });
  } catch {
    // best-effort; the temp dir removal below still isolates state.
  }
  rmSync(state.tmpDir, { recursive: true, force: true });
}
