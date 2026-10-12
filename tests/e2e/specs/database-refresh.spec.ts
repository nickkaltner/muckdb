import { execFile, execFileSync } from 'node:child_process';
import { mkdirSync, renameSync, rmSync, symlinkSync } from 'node:fs';
import { join } from 'node:path';
import { createServer } from 'node:http';
import { promisify } from 'node:util';
import type { Page } from '@playwright/test';
import { test, expect } from '../fixtures/test';
import { BINARY } from '../constants';

function runner(tmpDir: string, port: number) {
  const env = { ...process.env, XDG_DATA_HOME: join(tmpDir, 'data'), XDG_STATE_HOME: join(tmpDir, 'state') };
  return (...args: string[]) => execFileSync(BINARY, ['--port', String(port), ...args], { env, encoding: 'utf8' });
}

async function graphValues(page: Page) {
  return page.evaluate(() => {
    const canvas = document.querySelector('[data-tile="graph"] canvas');
    const chart = canvas && (window as any).Chart.getChart(canvas);
    return chart?.data.datasets[0].data.map((point: any) => typeof point === 'number' ? point : point.y);
  });
}

test('committed batches refresh a view graph and markdown, preserve panels, and catch up after reconnect', async ({ page, e2eState }) => {
  const run = runner(e2eState.tmpDir, e2eState.port);
  const db = join(e2eState.tmpDir, 'transactions.duckdb');
  const other = join(e2eState.tmpDir, 'unrelated.duckdb');
  run(db, '-c', 'CREATE TABLE samples(step INTEGER, n INTEGER); INSERT INTO samples VALUES (1, 1); CREATE VIEW totals AS SELECT step, sum(n) AS total FROM samples GROUP BY step ORDER BY step');
  run(other, '-c', 'CREATE TABLE counts AS SELECT 99 AS n');
  run('session', 'tile', 'transactions', '--name', 'graph', '--db', db, '--view', 'totals', '--chart', 'line', '--x', 'step', '--y', 'total', '--caption', 'Totals by step.');
  run('session', 'post', 'transactions', '--name', 'summary', '--db', db, '--md', 'Total {{sql: SELECT sum(n) FROM samples}}');
  run('session', 'tile', 'transactions', '--name', 'other', '--db', other, '--view', 'counts', '--caption', 'Unrelated values.');
  await page.addInitScript(() => {
    (window as any).__sockets = [];
    const Native = window.WebSocket;
    window.WebSocket = class extends Native {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols);
        (window as any).__sockets.push(this);
      }
    };
  });
  const queries: string[] = [];
  page.on('request', request => {
    const url = new URL(request.url());
    if (url.pathname === '/api/query') queries.push(url.searchParams.get('db') || '');
  });
  await page.goto('/session/transactions/');
  await expect.poll(() => graphValues(page)).toEqual([1]);
  await expect(page.locator('[data-tile="other"]')).toContainText('99');
  await page.waitForLoadState('networkidle');
  const otherQueries = queries.filter(path => path === other).length;
  await page.evaluate(() => { (window as any).__graphPanel = document.querySelector('[data-tile="graph"]'); });
  run(db, '-c', 'BEGIN TRANSACTION; INSERT INTO samples VALUES (1, 2); INSERT INTO samples VALUES (2, 4); COMMIT;');
  await expect.poll(() => graphValues(page)).toEqual([3, 4]);
  await expect(page.locator('[data-tile="summary"]')).toContainText('Total 7');
  expect(await page.evaluate(() => (window as any).__graphPanel === document.querySelector('[data-tile="graph"]'))).toBe(true);
  // Close this browser socket; the command commits before the delayed reconnect.
  await page.evaluate(() => (window as any).__sockets.at(-1).close());
  await expect(page.locator('#status-text')).toHaveText('reconnecting');
  for (const n of [5, 6, 7]) run(db, '-c', `BEGIN; UPDATE samples SET n = ${n} WHERE step = 2; COMMIT;`);
  await expect.poll(() => graphValues(page)).toEqual([3, 7]);
  await expect(page.locator('[data-tile="summary"]')).toContainText('Total 10');
  expect(queries.filter(path => path === other).length).toBe(otherQueries);
});

test('success signals survive ledger failure; failed transactions do not increment revisions; duplicates are ignored', async ({ page, request, e2eState }) => {
  const run = runner(e2eState.tmpDir, e2eState.port);
  const db = join(e2eState.tmpDir, 'ledger-failure.duckdb');
  run(db, '-c', 'CREATE TABLE samples AS SELECT 1 AS n');
  run('session', 'tile', 'ledger-failure', '--name', 'values', '--db', db, '--view', 'samples', '--caption', 'Current value.');
  await page.goto('/session/ledger-failure/');
  const panel = page.locator('[data-tile="values"] .panel-chart');
  await expect(panel).toContainText('1');
  const revision = async () => (await (await request.get('/api/state')).json()).database_revisions[db];
  const beforeFailure = await revision();
  expect(() => run(db, '-c', 'BEGIN; UPDATE samples SET n = 9; SELECT * FROM missing_table; COMMIT;')).toThrow();
  expect(await revision()).toBe(beforeFailure);
  const history = join(e2eState.tmpDir, 'data', 'muckdb', 'history.jsonl');
  renameSync(history, history + '.backup');
  mkdirSync(history);
  try {
    run(db, '-c', 'BEGIN; UPDATE samples SET n = 42; COMMIT;');
    await expect(panel).toContainText('42');
    expect(await revision()).toBe(beforeFailure + 1);
  } finally {
    rmSync(history, { recursive: true });
    renameSync(history + '.backup', history);
  }
  const alias = db + '.alias';
  symlinkSync(db, alias);
  const invocation = 123;
  await request.post('/api/database/refresh', { data: { db: alias, invocation } });
  const after = await revision();
  await request.post('/api/database/refresh', { data: { db, invocation } });
  expect(await revision()).toBe(after);
});


test('an unavailable daemon preserves SQL output and exit status', async ({ e2eState }) => {
  const server = createServer((_request, response) => {
    response.writeHead(503);
    response.end('UI unavailable');
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const address = server.address();
    if (!address || typeof address === 'string') throw new Error('missing listener address');
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const db = join(e2eState.tmpDir, 'no-daemon.duckdb');
    const args = [db, '-csv', '-c', 'CREATE TABLE answer AS SELECT 42 AS n; SELECT n FROM answer;'];
    const result = await promisify(execFile)(BINARY, ['--port', String(address.port), ...args], { env });
    expect(result.stdout).toBe('n\n42\n');
    expect(result.stderr).toContain('UI unavailable; continuing database command');
    expect(result.stderr).toContain('could not refresh dashboard');
  } finally {
    await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
});
