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

test('session routing help overlays content without changing toolbar overflow', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  const nav = page.locator('#session-nav');
  await expect(nav).toBeVisible();
  const overflow = await nav.evaluate((node) => ({
    horizontal: node.scrollWidth > node.clientWidth,
    vertical: node.scrollHeight > node.clientHeight,
  }));
  expect(overflow).toEqual({ horizontal: false, vertical: false });

  const button = page.locator('#session-info-btn');
  const exportButton = page.locator('#export-btn');
  await expect(button).toBeVisible();
  const [infoBox, exportBox, radius] = await Promise.all([
    button.boundingBox(),
    exportButton.boundingBox(),
    button.evaluate((node) => getComputedStyle(node).borderRadius),
  ]);
  expect(Math.abs(infoBox!.height - exportBox!.height)).toBeLessThan(1);
  expect(infoBox?.width).toBe(infoBox?.height);
  expect(Number.parseFloat(radius)).toBeLessThan((infoBox?.width ?? 0) / 2);

  await button.hover();
  const popover = page.locator('#session-info-pop');
  await expect(popover).toBeVisible();
  await expect(popover).toContainText('Session details');
  await expect(popover).toContainText('Last updated');
  await expect(popover.locator('time')).toHaveAttribute('datetime', /^\d{4}-\d{2}-\d{2}T/);
  const [popoverBox, bodyBox] = await Promise.all([
    popover.boundingBox(),
    page.locator('#session-body').boundingBox(),
  ]);
  expect(popoverBox!.y + popoverBox!.height).toBeGreaterThan(bodyBox!.y);
  expect(await popover.evaluate((node) => node.parentElement?.id)).toBe('view-sessions');
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

test('screenshot capture height includes the scaled chart x-axis', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/?shot=1&tile=by-day`);
  await expect(page.locator('.panel-chart canvas')).toBeVisible();
  await expect.poll(() => page.locator('html').getAttribute('data-shot-ready')).toBe('1');
  const dimensions = await page.evaluate(() => ({
    stamped: Number(document.documentElement.dataset.shotH),
    rendered: Math.ceil(document.body.getBoundingClientRect().height),
  }));
  expect(dimensions.stamped).toBeGreaterThanOrEqual(dimensions.rendered);
});
