import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

export const BASE_PORT = 12700;
export const SESSION_ID = 'e2e';

export const REPO_ROOT = resolve(__dirname, '..', '..');
export const BINARY = join(REPO_ROOT, 'target', 'release', 'muckdb');

export interface E2EState {
  tmpDir: string;
  port: number;
  dbId: string;
  sessionId: string;
}

export function readState(): E2EState {
  const file = process.env.MUCKDB_E2E_STATE;
  if (!file) throw new Error('MUCKDB_E2E_STATE is unavailable outside a Playwright worker');
  return JSON.parse(readFileSync(file, 'utf8')) as E2EState;
}
