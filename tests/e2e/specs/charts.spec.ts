import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('chart tiles render canvases; table tile renders a table', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  // Bar and line tiles draw with Chart.js → a <canvas> inside their panel.
  await expect(page.locator('.panel', { hasText: 'By category' }).locator('canvas')).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'By day' }).locator('canvas')).toBeVisible();

  // The table tile renders an HTML table (miniTable), not a canvas.
  await expect(page.locator('.panel', { hasText: 'All widgets' }).locator('table')).toBeVisible();
});

test('Cartesian charts brush-zoom their x-range and can reset', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const panel = page.locator('.panel[data-tile="by-day"]');
  const canvas = panel.locator('canvas');
  const box = await canvas.boundingBox();
  if (!box) throw new Error('chart canvas has no bounds');

  await canvas.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const y = r.top + r.height / 2;
    const fire = (type: string, x: number) => el.dispatchEvent(new MouseEvent(type, {
      bubbles: true, button: 0, clientX: x, clientY: y,
    }));
    fire('mousedown', r.left + r.width * 0.2);
    fire('mousemove', r.left + r.width * 0.65);
    fire('mouseup', r.left + r.width * 0.65);
  });

  const reset = panel.locator('.chart-reset');
  await expect(reset).toBeVisible();
  const zoomed = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    return { min: chart.options.scales.x.min, max: chart.options.scales.x.max };
  });
  expect(zoomed.min).not.toBeUndefined();
  expect(zoomed.max).not.toBeUndefined();

  await reset.click();
  await expect(reset).toBeHidden();
});

test('legend clicks fade a dataset rather than brushing or hiding it', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const canvas = page.locator('.panel[data-tile="by-day"] canvas');
  const faded = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    chart.options.plugins.legend.display = true;
    chart.update();
    const hit = chart.legend.legendHitBoxes[0];
    const r = el.getBoundingClientRect();
    const x = r.left + (hit.left + hit.width / 2) * r.width / chart.width;
    const y = r.top + (hit.top + hit.height / 2) * r.height / chart.height;
    const fire = (type: string) => el.dispatchEvent(new MouseEvent(type, {
      bubbles: true, button: 0, clientX: x, clientY: y,
    }));
    fire('mousedown'); fire('mouseup'); fire('click');
    return {
      faded: chart.$muckFadedDatasets[0],
      hidden: chart.getDatasetMeta(0).hidden,
      resetVisible: !chart.$muckBrushZoom.reset.hidden,
    };
  });
  expect(faded.faded).toBe(true);
  expect(faded.hidden).not.toBe(true);
  expect(faded.resetVisible).toBe(false);

  const restored = await canvas.evaluate((el) => {
    const chart = (window as any).Chart.getChart(el as HTMLCanvasElement);
    const hit = chart.legend.legendHitBoxes[0], r = el.getBoundingClientRect();
    const x = r.left + (hit.left + hit.width / 2) * r.width / chart.width;
    const y = r.top + (hit.top + hit.height / 2) * r.height / chart.height;
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0, clientX: x, clientY: y }));
    return !!chart.$muckFadedDatasets[0];
  });
  expect(restored).toBe(false);

});
