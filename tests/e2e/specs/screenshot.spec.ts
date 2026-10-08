import { test, expect } from '../fixtures/test';
import { BINARY } from '../constants';
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

function seedCapture(state: { tmpDir: string; browserPath: string; port: number }, kind: string, session = `capture-${kind}`, tile = 'chart') {
  const env = { ...process.env, XDG_DATA_HOME: join(state.tmpDir, 'data'),
    XDG_STATE_HOME: join(state.tmpDir, 'state'), MUCKDB_BROWSER: state.browserPath };
  const run = (args: string[]) => execFileSync(BINARY, ['--port', String(state.port), ...args], { env, stdio: 'pipe' });
  const temporal = ['line', 'area', 'stacked-time'].includes(kind);
  const sql = temporal
    ? "SELECT TIMESTAMP '2026-10-01' + i * INTERVAL 1 HOUR AS x, CASE WHEN i >= 160 THEN 80 ELSE 20 END AS a, 10 AS b FROM range(169) t(i) ORDER BY i"
    : "SELECT 'group-' || i AS x, i AS a, 10 AS b FROM range(1,8) t(i) ORDER BY i";
  run(['session', 'tile', session, '--name', tile, '--db', join(state.tmpDir, 'widgets.duckdb'),
    '--sql', sql, '--chart', kind === 'grouped' ? 'bar' : kind === 'stacked-time' ? 'stacked' : kind, '--x', 'x',
    '--y', kind === 'bar' || kind === 'area' ? 'a' : 'a,b', '--title', `${kind} capture`,
    '--caption', 'All categories, the final step, axis labels and legend must fit.',
    '--xlabel', temporal ? 'Hour (UTC)' : 'Category', '--ylabel', 'Value',
    ...(temporal ? ['--event', '2026-10-08T00:00|last point'] : [])]);
  return { session, run };
}

// Exercise the shared capture layout once with every chart variant. A full
// dashboard covers all six canvases; an area-only capture checks tile filtering.
// Keep native virtual-time Chrome: normal ResizeObserver timing alone missed
// the original oversized-canvas regression.
test('captures fit every chart before any window resize', async ({ page, e2eState }) => {
  test.setTimeout(60000);
  const kinds = ['bar', 'grouped', 'stacked', 'stacked-time', 'line', 'area'];
  const session = 'capture-layout';
  for (const kind of kinds) seedCapture(e2eState, kind, session, kind);
  await page.addInitScript(() => {
    const drawText = CanvasRenderingContext2D.prototype.fillText;
    CanvasRenderingContext2D.prototype.fillText = function(text, x, y, maxWidth) {
      if (text === 'last point') {
        const width = this.measureText(text).width;
        const left = this.textAlign === 'right' ? x - width : x;
        (this.canvas as any).captureLabel = { left, right: left + width };
      }
      if (maxWidth === undefined) drawText.call(this, text, x, y);
      else drawText.call(this, text, x, y, maxWidth);
    };
  });
  for (const width of [1200, 1800]) {
    await page.setViewportSize({ width, height: 900 });
    for (const tile of ['', '&tile=area']) {
      await page.goto(`/session/${session}/?shot=1${tile}`);
      await expect.poll(() => page.locator('html').getAttribute('data-shot-ready')).toBe('1');
      const canvases = page.locator('.panel-chart canvas');
      await expect(canvases).toHaveCount(tile ? 1 : kinds.length);
      const dimensions = await canvases.evaluateAll((canvases) => canvases.map((canvas) => {
        const chart = (window as any).Chart.getChart(canvas);
        const host = canvas.parentElement!;
        const rect = canvas.getBoundingClientRect(), box = host.getBoundingClientRect();
        const caption = host.nextElementSibling!.getBoundingClientRect();
        return { kind: canvas.closest<HTMLElement>('.panel')!.dataset.tile,
          width: chart.width, height: chart.height, hostWidth: host.clientWidth, hostHeight: host.clientHeight,
          right: rect.right, hostRight: box.right, bottom: rect.bottom, captionTop: caption.top,
          lastX: chart.getDatasetMeta(0).data.at(-1).x, areaRight: chart.chartArea.right,
          legendBottom: chart.legend.bottom, xBottom: chart.scales.x.bottom,
          stamped: Number(document.documentElement.dataset.shotH), captionBottom: caption.bottom,
          label: (canvas as any).captureLabel };
      }));
      for (const dims of dimensions) {
        if (['line', 'area', 'stacked-time'].includes(dims.kind!)) {
          expect(dims.label.left).toBeGreaterThanOrEqual(0);
          expect(dims.label.right).toBeLessThanOrEqual(dims.width);
        }
        expect(dims.width).toBeCloseTo(dims.hostWidth, 0);
        expect(dims.height).toBeCloseTo(dims.hostHeight, 0);
        expect(dims.right).toBeLessThanOrEqual(dims.hostRight + 1);
        expect(dims.bottom).toBeLessThanOrEqual(dims.captionTop + 1);
        expect(dims.lastX).toBeLessThanOrEqual(dims.areaRight + 1);
        expect(dims.legendBottom).toBeLessThanOrEqual(dims.height);
        expect(dims.xBottom).toBeLessThanOrEqual(dims.height);
        expect(dims.stamped).toBeGreaterThanOrEqual(Math.ceil(dims.captionBottom));
      }
      const dom = execFileSync(chromium.executablePath(), ['--headless', '--no-sandbox',
        '--disable-gpu', '--hide-scrollbars', '--no-first-run', '--disable-extensions',
        `--window-size=${width},900`, '--virtual-time-budget=10000', '--dump-dom',
        `http://127.0.0.1:${e2eState.port}/session/${session}/?shot=1${tile}`],
        { encoding: 'utf8', timeout: 25000, maxBuffer: 8 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
      expect(dom).toContain('data-shot-ready="1"');
      // Ignore literal canvas markup inside bundled script/template source.
      const nativeCanvases = [...dom.matchAll(/<canvas[^>]*>/g)]
        .map((match) => match[0]).filter((canvas) => canvas.includes('style='));
      expect(nativeCanvases).toHaveLength(dimensions.length);
      for (const [index, canvas] of nativeCanvases.entries()) {
        expect(canvas).toMatch(/height: 300px/);
        const renderedWidth = Number(canvas.match(/width: ([\d.]+)px/)?.[1]);
        expect(renderedWidth).toBeCloseTo(dimensions[index].hostWidth, 0);
      }
    }
  }
});

test('CLI and API screenshot captures honor the requested theme', async ({ page, e2eState }) => {
  test.setTimeout(90000);
  const { session, run } = seedCapture(e2eState, 'area');
  await page.goto(`/session/${session}/?shot=1&theme=paper`);
  await expect.poll(() => page.locator('html').getAttribute('data-shot-ready')).toBe('1');
  const pixel = async (bytes: Buffer) => page.evaluate(async (data) => {
    const blob = await (await fetch(`data:image/png;base64,${data}`)).blob();
    const bitmap = await createImageBitmap(blob);
    const canvas = document.createElement('canvas'); canvas.width = bitmap.width; canvas.height = bitmap.height;
    const ctx = canvas.getContext('2d')!; ctx.drawImage(bitmap, 0, 0);
    return [...ctx.getImageData(0, 0, 1, 1).data];
  }, bytes.toString('base64'));
  const api = await page.request.get(`/api/shot?session=${session}&tile=chart&theme=paper`);
  expect(api.ok()).toBe(true);
  const apiBody = await api.body();
  expect(api.headers()['content-type'], apiBody.toString('utf8')).toBe('image/png');
  const apiPixel = await pixel(apiBody);
  const out = test.info().outputPath('paper-cli.png');
  mkdirSync(dirname(out), { recursive: true });
  run(['session', 'screenshot', session, '--tile', 'chart', '--theme', 'paper', '--out', out]);
  expect(await pixel(readFileSync(out))).toEqual(apiPixel);
  // Pale paper must differ from the dark default hearth capture.
  expect(apiPixel[0]).toBeGreaterThan(180);
  expect(apiPixel[1]).toBeGreaterThan(180);
});
