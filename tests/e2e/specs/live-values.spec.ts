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

test('live markdown renders DuckDB BLOBs and data-image values', async ({ page }) => {
  const db = join(readState().tmpDir, 'markdown-images.duckdb');
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>';
  const dataUri = `data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`;
  cli(db, '-c', `CREATE TABLE images AS SELECT CAST('${svg}' AS BLOB) AS blob_value, '${dataUri}' AS data_value`);
  cli('session', 'post', 'image-live', '--name', 'images', '--db', db, '--md',
    '![Blob image]({{image: SELECT blob_value FROM images}})\n\n' +
    '![Data image]({{image: SELECT data_value FROM images}})');

  await page.goto('/session/image-live/');
  const md = page.locator('[data-tile="images"] .md');
  await expect(md.locator('img.md-db-image')).toHaveCount(2);
  await expect(md.locator('img[alt="Blob image"]')).toHaveJSProperty('naturalWidth', 8);
  await expect(md.locator('img[alt="Data image"]')).toHaveJSProperty('naturalWidth', 8);
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

test('live chart refreshes retain faded legend series', async ({ page }) => {
  const db = join(readState().tmpDir, 'legend-refresh.duckdb');
  cli(db, '-c', 'CREATE TABLE readings AS SELECT 1 AS epoch, 0.7 AS alpha, 0.6 AS beta UNION ALL SELECT 2, 0.8, 0.7');
  cli('session', 'tile', 'legend-refresh', '--name', 'comparison', '--db', db, '--view', 'readings',
    '--chart', 'line', '--x', 'epoch', '--y', 'alpha,beta', '--caption', 'Two series updated live.');

  await page.goto('/session/legend-refresh/');
  const canvas = page.locator('[data-tile="comparison"] canvas');
  const xAxis = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    return {
      autoSkip: chart.scales.x.options.ticks.autoSkip,
      labelPadding: chart.scales.x.options.ticks.padding,
      tickLength: chart.scales.x.options.grid.tickLength,
    };
  });
  expect(xAxis).toEqual({ autoSkip: false, labelPadding: 3, tickLength: 4 });
  await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    const hit = chart.legend.legendHitBoxes[0], r = el.getBoundingClientRect();
    const x = r.left + (hit.left + hit.width / 2) * r.width / chart.width;
    const y = r.top + (hit.top + hit.height / 2) * r.height / chart.height;
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0, clientX: x, clientY: y }));
  });
  await expect.poll(() => canvas.evaluate((el) =>
    !!(window as any).Chart.getChart(el as HTMLCanvasElement).$muckFadedDatasets[0])).toBe(true);

  await canvas.evaluate((el: any) => { el.identityMarker = true; });
  // A session edit reloads the dashboard's panels, creating a new Chart.js
  // instance. The reader's legend choice must survive that replacement.
  cli('session', 'post', 'legend-refresh', '--name', 'refresh-note', '--md', 'The comparison was refreshed.');
  await expect.poll(() => canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    return !!chart && !!chart.$muckFadedDatasets[0] && !(el as any).identityMarker;
  })).toBe(true);
});

test('CLI session updates keep the reader on the same panel', async ({ page }) => {
  const session = 'cli-scroll';
  const intro = Array.from({ length: 45 }, (_, i) => `Original context ${i}.`).join('\n\n');
  cli('session', 'post', session, '--name', 'intro', '--md', intro);
  cli('session', 'post', session, '--name', 'detail', '--md', '# Detail\n\nThe reader stays here.');
  // Keep enough content below the target that it can remain at the same
  // viewport offset after the intro grows (rather than hitting scroll bottom).
  cli('session', 'post', session, '--name', 'tail', '--md', Array.from({ length: 80 }, (_, i) => `Tail ${i}.`).join('\n\n'));

  await page.goto(`/session/${session}/`);
  const detail = page.locator('[data-tile="detail"]');
  await detail.evaluate((el) => el.scrollIntoView({ block: 'start' }));
  const before = await detail.evaluate((el) => {
    const scroller = document.getElementById('panels-scroll')!;
    return {
      scrollTop: scroller.scrollTop,
      offset: el.getBoundingClientRect().top - scroller.getBoundingClientRect().top,
    };
  });
  expect(before.scrollTop).toBeGreaterThan(500);

  // Updating the CLI session changes the session revision and makes the
  // WebSocket client refetch/rebuild every panel. Grow content above detail to
  // ensure preserving a raw scrollTop would not be sufficient.
  cli('session', 'post', session, '--name', 'intro', '--md',
    Array.from({ length: 65 }, (_, i) => `Updated context ${i}.`).join('\n\n'));
  await expect(page.locator('[data-tile="intro"]')).toContainText('Updated context 64.');

  await expect.poll(() => detail.evaluate((el) => {
    const scroller = document.getElementById('panels-scroll')!;
    return el.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
  })).toBeLessThan(100);
});

test('an update to another session does not refresh the open session', async ({ page }) => {
  await page.goto('/session/e2e/#t=todos');
  const list = page.locator('[data-tile="todos"] .todo-list');
  await expect(list).toBeVisible();
  await list.evaluate((element: any) => { element.identityMarker = true; });
  const countBefore = Number((await page.locator('#session-combo .cv-sub').textContent())?.match(/\d+/)?.[0]);

  cli('session', 'post', 'unrelated-live-update', '--name', 'note', '--md', 'Changed elsewhere.');
  await expect(page.locator('#session-combo .cv-sub')).toContainText(`${countBefore + 1} sessions`);

  expect(await list.evaluate((element: any) => element.identityMarker)).toBe(true);
});
