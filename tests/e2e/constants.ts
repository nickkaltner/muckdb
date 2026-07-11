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
