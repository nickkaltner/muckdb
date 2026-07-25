import { test, expect } from '@playwright/test';

test('newer cached release turns the version indicator red with a rich tooltip', async ({ page }) => {
  await page.goto('/');
  const version = page.locator('#sl-state');
  await expect(version).toHaveClass(/update-available/);
  await expect(version).toHaveText(/v0\.4\.19/);
  await expect(version).toHaveAttribute('data-tip', /Update available/);
  await expect(version).toHaveAttribute('data-tip', /v99\.0\.0/);
});
