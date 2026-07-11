import { test, expect } from '@playwright/test';

// Shared helper: fail a test if the page logs any uncaught error, with a small
// allowlist for benign noise.
const ALLOW = [/favicon/i];
function guardConsole(page: import('@playwright/test').Page, errors: string[]): void {
  page.on('console', (m) => {
    if (m.type() === 'error' && !ALLOW.some((re) => re.test(m.text()))) errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push(String(e)));
}

test('page loads with tabs and no console errors', async ({ page }) => {
  const errors: string[] = [];
  guardConsole(page, errors);

  await page.goto('/');
  await expect(page.locator('#tabs .tab[data-tab="databases"]')).toBeVisible();
  await expect(page.locator('#tabs .tab[data-tab="sessions"]')).toBeVisible();
  await expect(page.locator('#tabs .tab[data-tab="ledger"]')).toBeVisible();

  // Theme button sits immediately before the credits (?) button (guards the recent move).
  const ids = await page.locator('.titlebar button.kbtn').evaluateAll((els) =>
    els.map((e) => e.id),
  );
  expect(ids.indexOf('theme-btn')).toBe(ids.indexOf('credits-btn') - 1);

  await page.waitForTimeout(300);
  expect(errors, `console errors:\n${errors.join('\n')}`).toEqual([]);
});
