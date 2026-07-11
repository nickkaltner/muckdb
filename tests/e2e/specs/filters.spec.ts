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
    // This is also safe across columns: the seed fixture's category values
    // (Alpha/Beta/Gamma/Delta/Epsilon) are unique, so no other column's cell
    // text contains "Alpha".
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

  test('active "=" shows a bordered "+", never swaps to "−"', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/widgets/`);
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);

    const alpha = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alpha.hover();
    await alpha.locator('.cellf:not([data-fnot])').click();
    await expect(page.locator('#tp-results')).toHaveText(/40 results/);

    // Re-locate an Alpha cell (the table re-rendered) and inspect its include
    // button: it stays "+" and gains the active class (bordered), rather than
    // turning into a "−".
    const cell = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await cell.hover();
    const plus = cell.locator('.cellf:not([data-fnot])');
    await expect(plus).toHaveText('+');
    await expect(plus).toHaveClass(/\bon\b/);
  });

  test('"=" and "≠" on the same value are mutually exclusive', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/widgets/`);
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);

    // Apply "= Alpha" → 40 rows, one chip.
    let alpha = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alpha.hover();
    await alpha.locator('.cellf:not([data-fnot])').click();
    await expect(page.locator('#tp-results')).toHaveText(/40 results/);
    await expect(page.locator('.active-filters .fchip')).toHaveCount(1);

    // Click "≠" on an Alpha cell: it must REPLACE the "=" (not stack both, which
    // would be contradictory → 0 rows). Result: only "≠ Alpha" → 160 rows.
    alpha = page.locator('table.preview td', { hasText: 'Alpha' }).first();
    await alpha.hover();
    await alpha.locator('.cellf[data-fnot]').click();
    await expect(page.locator('#tp-results')).toHaveText(/160 results/);
    const chips = page.locator('.active-filters .fchip');
    await expect(chips).toHaveCount(1);
    await expect(chips.first()).toContainText('≠');
  });
});
