import { test, expect } from '@playwright/test';
import { readState } from '../constants';

test.describe('cell value filters', () => {
  test('+ pins to a value, ≠ excludes it', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/widgets/`);

    // Baseline: 200 rows. `#tp-results` is the canonical result-count hook,
    // rendering "<N> results".
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);

    // Hover a cell whose category is "Alpha", then click its + button.
    // NOTE: the .cellf button lives inside the value <td>, so the cell text is
    // "Alpha" + the button glyph — use a substring match, not an anchored regex.
    const alphaCell = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alphaCell.hover();
    await alphaCell.locator('.cellf:not([data-fnot])').click();

    // 40 of 200 rows are Alpha; an "=" chip appears.
    await expect(page.locator('#tp-results')).toHaveText(/40 results/);
    await expect(page.locator('.active-filters .fchip')).toContainText('=');

    // Remove the filter (click its chip) → back to 200.
    await page.locator('.active-filters .fchip').first().click();
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);

    // Now the ≠ button on an Alpha cell → complement (160 rows) with a "≠" chip.
    const alphaCell2 = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alphaCell2.hover();
    await alphaCell2.locator('.cellf[data-fnot]').click();
    await expect(page.locator('#tp-results')).toHaveText(/160 results/);
    await expect(page.locator('.active-filters .fchip')).toContainText('≠');
  });
});
