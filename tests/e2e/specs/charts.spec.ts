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
