import { test, expect } from '../fixtures/test';
import { BINARY } from '../constants';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';

for (const kind of ['line', 'area']) {
  test(`dense numeric ${kind} uses readable ticks and retains every point`, async ({ page, e2eState }) => {
    const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
    const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env, stdio: 'pipe' });
    const db = join(e2eState.tmpDir, 'widgets.duckdb');
    run(['format', db, 'optimizer_step', '--suffix', ' steps', '--decimals', '0', '--thousands']);
    run(['session', 'tile', `dense-${kind}`, '--name', 'training', '--db', db,
      '--sql', 'SELECT i AS optimizer_step, 2.5/(i+1) AS loss FROM range(1500) t(i) ORDER BY i',
      '--chart', kind, '--x', 'optimizer_step', '--y', 'loss', '--title', 'Within-epoch training',
      '--xlabel', 'Optimizer step', '--ylabel', 'Loss', '--caption', 'All training steps with readable axis ticks.']);
    await page.goto(`/session/dense-${kind}/?shot=1&tile=training`);
    await expect.poll(() => page.locator('html').getAttribute('data-shot-ready')).toBe('1');
    const canvas = page.locator('.panel-chart canvas');
    const axis = await canvas.evaluate(el => {
      const c = (window as any).Chart.getChart(el as HTMLCanvasElement);
      return { type: c.scales.x.type, ticks: c.scales.x.ticks.map((t: any) => t.label),
        points: c.getDatasetMeta(0).data.length, lastX: c.getDatasetMeta(0)._parsed.at(-1).x,
        labels: c.scales.x.getLabelItems().map((item: any) => ({ x: item.options.translation[0], width: c.ctx.measureText(item.label).width })),
        rotation: c.scales.x.labelRotation };
    });
    expect(axis.type).toBe('linear');
    expect(axis.ticks.length).toBeLessThanOrEqual(9);
    expect(axis.ticks.every((t: string) => t.endsWith(' steps'))).toBe(true);
    expect(axis.points).toBe(1500);
    expect(axis.lastX).toBe(1499);
    expect(axis.rotation).toBe(0);
    for (let i = 1; i < axis.labels.length; i++) {
      const prev = axis.labels[i-1], next = axis.labels[i];
      expect(next.x - next.width / 2).toBeGreaterThan(prev.x + prev.width / 2);
    }
    await page.screenshot({ path: test.info().outputPath(`${kind}.png`), fullPage: true });
    await page.goto(`/session/dense-${kind}/`);
    await expect(canvas).toBeVisible();
    await canvas.scrollIntoViewIfNeeded();
    await page.waitForTimeout(1000);
    const point = await canvas.evaluate(el => {
      const c = (window as any).Chart.getChart(el as HTMLCanvasElement);
      const p = c.getDatasetMeta(0).data[1000].getCenterPoint(true);
      const r = el.getBoundingClientRect();
      return { x: r.left + p.x * r.width / c.width, y: r.top + p.y * r.height / c.height };
    });
    await page.mouse.move(point.x, point.y);
    await expect.poll(() => canvas.evaluate(el => {
      const c = (window as any).Chart.getChart(el as HTMLCanvasElement);
      return c.tooltip.getActiveElements()[0]?.index;
    })).toBe(1000);
    const title = await canvas.evaluate(el => (window as any).Chart.getChart(el as HTMLCanvasElement).tooltip.title);
    expect(title.join(' ')).toContain('1,000 steps');

  });
}

test('irregular numeric line spacing, trend and confidence band share the value axis', async ({ page, e2eState }) => {
  const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
  const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env, stdio: 'pipe' });
  const db = join(e2eState.tmpDir, 'widgets.duckdb');
  run(['session', 'tile', 'irregular-line', '--name', 'training', '--db', db,
    '--sql', 'SELECT * FROM (VALUES (0,10,8,12), (1,8,6,10), (10,6,4,8), (100,4,2,6), (1000,2,1,3), (NULL,5,4,6)) t(step,loss,lo,hi)',
    '--chart', 'line', '--x', 'step', '--y', 'loss', '--band', 'lo,hi', '--trend',
    '--caption', 'Irregular optimizer steps preserve spacing, bounds and fitted values.']);
  await page.goto('/session/irregular-line/?shot=1&tile=training');
  await expect.poll(() => page.locator('html').getAttribute('data-shot-ready')).toBe('1');
  const result = await page.locator('canvas').evaluate(el => {
    const c = (window as any).Chart.getChart(el as HTMLCanvasElement);
    const x = c.scales.x;
    return { type: x.type, unitGap: x.getPixelForValue(1)-x.getPixelForValue(0), wideGap: x.getPixelForValue(1000)-x.getPixelForValue(100),
      sets: c.data.datasets.map((d: any, i: number) => ({ label: d.label, xs: d.data.map((p: any) => p.x), skipped: c.getDatasetMeta(i).data.filter((p: any) => p.skip).length })) };
  });
  expect(result.type).toBe('linear');
  expect(result.wideGap / result.unitGap).toBeCloseTo(900);
  for (const set of result.sets) {
    expect(set.xs).toEqual(expect.arrayContaining([0,1,10,100,1000]));
    if (set.label === 'trend') expect(set.xs).not.toContain(null);
    else expect(set.skipped).toBe(1);
  }
});

test('numeric-looking category labels skip crowded labels without changing their identity', async ({ page, e2eState }) => {
  const env = { ...process.env, XDG_DATA_HOME: join(e2eState.tmpDir, 'data'), XDG_STATE_HOME: join(e2eState.tmpDir, 'state') };
  const run = (args: string[]) => execFileSync(BINARY, ['--port', String(e2eState.port), ...args], { env, stdio: 'pipe' });
  const db = join(e2eState.tmpDir, 'widgets.duckdb');
  run(['format', db, 'stage', '--suffix', ' units', '--decimals', '0']);
  run(['session', 'tile', 'categorical-line', '--name', 'training', '--db', db,
    '--sql', 'SELECT CAST(1000+i AS VARCHAR) AS stage, i AS loss FROM range(100) t(i) ORDER BY i',
    '--chart', 'line', '--x', 'stage', '--y', 'loss', '--caption', 'Category identity survives sparse ticks.']);
  await page.goto('/session/categorical-line/?shot=1&tile=training');
  await expect.poll(() => page.locator('html').getAttribute('data-shot-ready')).toBe('1');
  const axis = await page.locator('canvas').evaluate(el => {
    const c = (window as any).Chart.getChart(el as HTMLCanvasElement);
    return { type: c.scales.x.type, labels: c.scales.x.ticks.map((t: any) => ({ value: t.value, label: t.label })), points: c.getDatasetMeta(0).data.length };
  });
  expect(axis.type).toBe('category');
  expect(axis.labels.length).toBeLessThanOrEqual(9);
  expect(axis.points).toBe(100);
  for (const tick of axis.labels) expect(tick.label).toBe(`${1000 + tick.value} units`);
});
