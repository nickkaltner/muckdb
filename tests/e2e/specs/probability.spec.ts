import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('probability tile', () => {
  test('estimates raw distributions on a shared scale', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="probability"]');

    await expect(panel.locator('.pr-row')).toHaveCount(3);
    await expect(panel.locator('.pr-curve')).toHaveCount(3);
    await expect(panel.locator('.pr-mean')).toHaveCount(3);
    await expect(panel).toContainText('Fast path');
    await expect(panel).toContainText('Long right tail');
    await expect(panel.locator('.pr-stats').first()).toHaveText(/n 7/);
  });

  test('uses the standard height grip and stretches its curves', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="probability"]');
    const chart = panel.locator('.panel-chart');
    const plot = panel.locator('.pr-plot').first();
    const grip = panel.locator('[data-grip="probability"]');
    await expect(grip).toBeVisible();
    await grip.scrollIntoViewIfNeeded();
    const before = await plot.boundingBox();
    const beforeChartHeight = await chart.evaluate((el) => el.getBoundingClientRect().height);
    const box = await grip.boundingBox();
    await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await page.mouse.down();
    await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2 + 180);
    await page.mouse.up();
    await expect.poll(() => chart.evaluate((el) => el.getBoundingClientRect().height)).toBeGreaterThan(beforeChartHeight + 150);
    expect((await plot.boundingBox())!.height).toBeGreaterThan(before!.height);
  });
});
