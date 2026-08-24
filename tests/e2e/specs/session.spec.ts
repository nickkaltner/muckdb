import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('seeded session renders its tiles', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  // Markdown tile.
  await expect(page.locator('.panel[data-tile="summary"]')).toBeVisible();
  await expect(page.getByText('200 widgets', { exact: false })).toBeVisible();

  // Each chart tile's panel is present by title.
  await expect(page.locator('.panel', { hasText: 'By category' })).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'By day' })).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'All widgets' })).toBeVisible();
});

test('time-axis labels leave breathing room and reveal their full timestamp on hover', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const canvas = page.locator('.panel[data-tile="by-day"] canvas');
  await expect(canvas).toBeVisible();
  await canvas.scrollIntoViewIfNeeded();
  // Derive the pointer position from Chart.js, keeping the test independent of
  // browser size and the chart panel's position in the page.
  const tick = await canvas.evaluate((node) => {
    const chart = (window as any).Chart.getChart(node as HTMLCanvasElement);
    const x = chart.scales.x, rect = (node as HTMLCanvasElement).getBoundingClientRect();
    const i = Math.floor(x.ticks.length / 2);
    return {
      x: rect.left + x.getPixelForTick(i) * rect.width / chart.width,
      y: rect.top + ((x.top + x.bottom) / 2) * rect.height / chart.height,
      count: x.ticks.length,
    };
  });
  expect(tick.count).toBeLessThanOrEqual(9);
  await page.mouse.move(tick.x, tick.y);
  const tip = page.locator('.chart-time-tip');
  await expect(tip).toBeVisible();
  await expect(tip).toContainText('2026-');
  await expect(tip).toContainText('UTC');
});
