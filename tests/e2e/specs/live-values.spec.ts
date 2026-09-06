import { test, expect, Response } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { BINARY, PORT, readState } from '../constants';

function cli(...args: string[]) {
  const { tmpDir } = readState();
  return execFileSync(BINARY, ['--port', String(PORT), ...args], {
    env: { ...process.env, XDG_DATA_HOME: join(tmpDir, 'data'), XDG_STATE_HOME: join(tmpDir, 'state') },
    encoding: 'utf8',
  });
}

test('live markdown formats scalars, escapes text, reports errors and refreshes after SQL', async ({ page }) => {
  const db = join(readState().tmpDir, 'live.duckdb');
  cli(db, '-c', 'CREATE TABLE totals AS SELECT 12.5 AS revenue');
  cli('format', db, 'revenue', '--currency', 'USD');
  cli('session', 'post', 'live', '--name', 'summary', '--db', db, '--md',
    '**Revenue {{sql: SELECT revenue FROM totals}}**\n\n' +
    '| Kind | Value |\n|---|---|\n| Null | {{sql: SELECT NULL}} |\n\n' +
    '{{sql: SELECT \'<img src=x onerror=alert(1)> **literal**\'}}\n\n' +
    '{{sql: SELECT * FROM range(2)}}\n\n' +
    '{{sql: SELECT 1, 2}}\n\n{{sql: SELECT 1 WHERE false}}\n\n' +
    '{{sql: DELETE FROM totals}}\n\n`{{sql: SELECT 999}}`');
  await page.goto('/session/live/');
  const md = page.locator('[data-tile="summary"] .md');
  await expect(md.locator('strong').first()).toHaveText('Revenue $12.50 USD');
  await expect(md).toContainText('<img src=x onerror=alert(1)> **literal**');
  await expect(md.locator('img')).toHaveCount(0);
  await expect(md.locator('td').last()).toHaveText('—');
  await expect(md).toContainText('Expected exactly one row and one column');
  await expect(md.locator('code')).toHaveText('{{sql: SELECT 999}}');
  cli(db, '-c', 'UPDATE totals SET revenue = 25');
  await expect(md.locator('strong').first()).toHaveText('Revenue $25.00 USD');
  await page.screenshot({ path: '/tmp/muckdb-live-summary.png', fullPage: true });
  // A markdown-only dashboard must carry its database across export/import.
  const archive = join(readState().tmpDir, 'live.muckdb');
  cli('session', 'export', 'live', '--out', archive);
  cli('session', 'import', archive);
  await page.goto('/session/live-2/');
  await expect(md.locator('strong').first()).toHaveText('Revenue $25.00 USD');
});

test('view and SQL tiles honor default and explicit limits and identify truncation', async ({ page }) => {
  const db = join(readState().tmpDir, 'limits.duckdb');
  cli(db, '-c', 'CREATE VIEW many AS SELECT range AS n FROM range(10002) ORDER BY n');
  cli('session', 'tile', 'limits', '--name', 'default', '--db', db, '--view', 'many');
  cli('session', 'tile', 'limits', '--name', 'small', '--db', db, '--sql', 'SELECT * FROM many', '--limit', '3');
  cli('session', 'tile', 'limits', '--name', 'full', '--db', db, '--view', 'many', '--limit', '10002');
  const returned: Record<string, number> = {};
  const collect = async (response: Response) => {
    const url = new URL(response.url());
    if (url.pathname === '/api/query') {
      const data = await response.json();
      returned[url.searchParams.get('sql')!] = data.rows?.length;
    }
  };
  page.on('response', collect);
  await page.goto('/session/limits/');
  await expect(page.locator('[data-tile="default"] .tile-limit-note')).toContainText('10,000');
  await expect(page.locator('[data-tile="small"] .tile-limit-note')).toContainText('first 3 rows');
  await expect(page.locator('[data-tile="full"] table')).toBeVisible();
  await expect(page.locator('[data-tile="full"] .tile-limit-note')).toHaveCount(0);
  expect(Object.values(returned)).toContain(10001);
  expect(Object.values(returned)).toContain(4);
  expect(Object.values(returned)).toContain(10002);
  page.off('response', collect);
  expect(() => cli('session', 'tile', 'limits', '--name', 'bad', '--db', db, '--view', 'many', '--limit', '0')).toThrow();
  expect(() => cli('session', 'post', 'limits', '--md', '{{sql: SELECT 1}}')).toThrow();
  cli('session', 'post', 'limits', '--name', 'docs', '--md', '`{{sql: SELECT 1}}`');
  await expect(page.locator('[data-tile="docs"] code')).toHaveText('{{sql: SELECT 1}}');
});

test('format updates refresh values without replacing panels or moving the reader', async ({ page }) => {
  const db = join(readState().tmpDir, 'format-scroll.duckdb');
  cli(db, '-c', 'CREATE TABLE amounts AS SELECT 12.5 AS amount');
  cli('session', 'post', 'format-scroll', '--name', 'intro', '--md',
    Array.from({ length: 45 }, (_, i) => `Paragraph ${i}: dashboard context.`).join('\n\n'));
  cli('session', 'tile', 'format-scroll', '--name', 'values', '--db', db, '--view', 'amounts');
  await page.goto('/session/format-scroll/');
  const panel = page.locator('[data-tile="values"]');
  await expect(panel.locator('td').first()).toHaveText('12.5');
  await panel.scrollIntoViewIfNeeded();
  const before = await page.evaluate(() => {
    (document.querySelector('[data-tile="values"]') as any).identityMarker = true;
    return document.getElementById('panels-scroll')!.scrollTop;
  });
  expect(before).toBeGreaterThan(500);
  cli('format', db, 'amount', '--currency', 'USD');
  await expect(panel.locator('td').first()).toHaveText('$12.50 USD');
  expect(await panel.evaluate((p: any) => p.identityMarker)).toBe(true);
  const after = await page.evaluate(() => document.getElementById('panels-scroll')!.scrollTop);
  expect(Math.abs(after - before)).toBeLessThan(3);
});
