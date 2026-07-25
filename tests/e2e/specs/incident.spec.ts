import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('incident timeline tile', () => {
  test('orders events, groups dates, renders optional narrative and severity', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="incident"]');
    await expect(panel).toBeVisible();

    const rows = panel.locator('.inc-row');
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText('Incident declared');
    await expect(rows.nth(0).locator('.inc-date')).toHaveText('2026-05-01');
    await expect(rows.nth(1)).toContainText('Traffic shifted');
    await expect(rows.nth(1).locator('.inc-date')).toHaveText('2026-05-02');
    await expect(rows.nth(2)).toContainText('Mitigation verified');
    await expect(rows.nth(2).locator('.inc-date')).toHaveCount(0);
    await expect(rows.nth(1).locator('.inc-desc')).toHaveCount(0);
    await expect(rows.nth(0).locator('.inc-desc')).toContainText('On-call began coordinating');
    await expect(panel.locator('.inc-legend .inc-leg')).toHaveCount(3);
    await expect(panel.locator('[data-widen]')).toHaveCount(1);
  });
});
