import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('seeded session renders its tiles', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  // Markdown tile.
  await expect(page.locator('.panel', { hasText: 'Summary' })).toBeVisible();
  await expect(page.getByText('200 widgets', { exact: false })).toBeVisible();

  // Each chart tile's panel is present by title.
  await expect(page.locator('.panel', { hasText: 'By category' })).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'By day' })).toBeVisible();
  await expect(page.locator('.panel', { hasText: 'All widgets' })).toBeVisible();
});
