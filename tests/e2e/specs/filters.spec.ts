import { test, expect } from '@playwright/test';
import { readState } from '../constants';

test.describe('cell value filters', () => {
  test('+ pins to a value, ≠ excludes it', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/widgets/`);

    // Baseline: 200 rows. NOTE: the app has no `#tp-results` element (the
    // Selector Reference is stale on this point) — the pager's `.range`
    // element ("showing 1–13 of 200") is the actual result-count hook.
    await expect(page.locator('.range')).toHaveText(/of 200\b/);

    // Hover a cell whose category is "Alpha", then click its + button.
    // NOTE: the .cellf button lives inside the value <td>, so the cell text is
    // "Alpha" + the button glyph — use a substring match, not an anchored regex.
    const alphaCell = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alphaCell.hover();
    await alphaCell.locator('.cellf:not([data-fnot])').click();

    // 40 of 200 rows are Alpha; an "=" chip appears.
    await expect(page.locator('.range')).toHaveText(/of 40\b/);
    await expect(page.locator('.active-filters .fchip')).toContainText('=');

    // Remove the filter (click its chip) → back to 200.
    await page.locator('.active-filters .fchip').first().click();
    await expect(page.locator('.range')).toHaveText(/of 200\b/);

    // Now the ≠ button on an Alpha cell → complement (160 rows) with a "≠" chip.
    const alphaCell2 = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alphaCell2.hover();
    await alphaCell2.locator('.cellf[data-fnot]').click();
    await expect(page.locator('.range')).toHaveText(/of 160\b/);
    await expect(page.locator('.active-filters .fchip')).toContainText('≠');
  });
});
