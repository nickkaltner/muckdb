import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { SESSION_ID } from '../constants';

// Regression: the poster / copy-image export (html2canvas) must not choke on
// color-mix() styles. Chrome serialises a computed color-mix() as `color(srgb …)`,
// which html2canvas can't parse ("unsupported color function 'color'") — the
// onclone hook rewrites those to rgba() before capture. The e2e session has
// timeline + map tiles that use color-mix in their styles.
test('dashboard poster exports despite color-mix styles', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  await expect(page.locator('.panel[data-tile="timeline"]')).toBeVisible();

  // Capture must neutralise the app's root CSS zoom while measuring/cloning and
  // carry it in the raster scale instead. Mixing the two coordinate systems
  // makes macOS captures progressively widen the spaces between text runs.
  await page.evaluate(() => {
    const win = window as typeof window & {
      html2canvas: typeof window.html2canvas;
      captureProbe?: { zoom: string; scale: number };
    };
    const render = win.html2canvas;
    win.html2canvas = (element, options = {}) => {
      const onclone = options.onclone;
      return render(element, {
        ...options,
        onclone: (doc, cloned) => {
          onclone?.(doc, cloned);
          win.captureProbe = {
            zoom: doc.documentElement.style.zoom,
            scale: Number(options.scale),
          };
        },
      });
    };
  });

  const [download] = await Promise.all([
    page.waitForEvent('download', { timeout: 30000 }),
    page.click('#poster-btn'),
  ]);
  // A download only fires on success; an html2canvas parse error would instead
  // surface an error toast and no download.
  expect(download.suggestedFilename()).toBe(`${SESSION_ID}.png`);
  const png = readFileSync(await download.path());
  // Metadata is injected after IHDR, before the compressed image data. Check
  // both the standard creation-time marker and the human-readable fields.
  expect(png.toString('latin1')).toMatch(/Software\0muckdb \d+\.\d+\.\d+ by Nick Kaltner/);
  expect(png.toString('latin1')).toContain('Creation Time\0');
  expect(png.subarray(37, 41).toString('ascii')).toBe('tIME');
  await expect.poll(() => page.evaluate(() => (window as any).captureProbe)).toEqual({
    zoom: '1',
    scale: 2.5,
  });
});
