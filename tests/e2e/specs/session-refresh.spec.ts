import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { test, expect } from '../fixtures/test';
import { BINARY } from '../constants';

test('database updates refresh only the matching session tiles', async ({ page, e2eState }) => {
  const env = {
    ...process.env,
    XDG_DATA_HOME: join(e2eState.tmpDir, 'data'),
    XDG_STATE_HOME: join(e2eState.tmpDir, 'state'),
  };
  const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env, stdio: 'pipe' });
  const firstDb = join(e2eState.tmpDir, 'refresh-a.duckdb');
  const secondDb = join(e2eState.tmpDir, 'refresh-b.duckdb');
  for (const db of [firstDb, secondDb]) run([db, '-c', 'CREATE TABLE counts (n INTEGER); INSERT INTO counts VALUES (1)']);
  run(['session', 'create', 'refresh-scope']);
  for (const [name, db] of [['a', firstDb], ['b', secondDb]]) {
    run(['session', 'tile', 'refresh-scope', '--name', name, '--db', db,
      '--sql', 'SELECT sum(n) AS total FROM counts', '--chart', 'table', '--caption', `Sum for ${name}.`]);
  }

  const queries: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (url.pathname === '/api/query') queries.push(url.searchParams.get('db') || '');
  });
  await page.goto('/session/refresh-scope/');
  await expect(page.locator('.panel[data-tile="a"] .panel-chart')).toContainText('1');
  await expect(page.locator('.panel[data-tile="b"] .panel-chart')).toContainText('1');
  await page.waitForLoadState('networkidle');
  const firstBefore = queries.filter((db) => db === firstDb).length;
  const secondBefore = queries.filter((db) => db === secondDb).length;

  run([secondDb, '-c', 'INSERT INTO counts VALUES (2)']);
  await expect(page.locator('.panel[data-tile="b"] .panel-chart')).toContainText('3');
  expect(queries.filter((db) => db === secondDb).length).toBeGreaterThan(secondBefore);
  expect(queries.filter((db) => db === firstDb).length).toBe(firstBefore);

  await page.waitForLoadState('networkidle');
  run([firstDb, '-c', 'INSERT INTO counts VALUES (4)']);
  await expect(page.locator('.panel[data-tile="a"] .panel-chart')).toContainText('5');
  expect(queries.filter((db) => db === firstDb).length).toBeGreaterThan(firstBefore);
});
