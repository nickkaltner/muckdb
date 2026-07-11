import { test, expect } from '@playwright/test';
import { readState } from '../constants';

test.describe('array cell formatting', () => {
  test('empty array shows "—", never a bare unit suffix', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/widgets/`);
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);

    // `sizes` carries a " Gbps" format; every 4th row is an empty array. An
    // empty list must render a muted em dash (td.null with "—"), never the bare
    // format suffix — the fixture has no NULLs, so td.null == empty lists here.
    const empty = page.locator('table.preview td.null', { hasText: '—' }).first();
    await expect(empty).toBeVisible();
    await expect(empty).toHaveText('—');

    // Non-empty cells apply the suffix once, after the values ("10, 100 Gbps").
    const filled = page.locator('table.preview .cell-json', { hasText: 'Gbps' }).first();
    await expect(filled).toContainText(/\d[\d,\s]*Gbps/);

    // Regression guard: no cell renders as a bare " Gbps" with no values.
    await expect(page.locator('table.preview td', { hasText: /^\s*Gbps\s*$/ })).toHaveCount(0);
  });

  test('list-column stats value filters by equality, not containment (no collapse)', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/widgets/?view=stats`);
    // The `sizes` stat card shows whole-array values ("[10, 100]" on 150 rows).
    // Clicking one must filter by equality (CAST = '[10, 100]' → 150 rows), not
    // list-element containment (which matched 0 and collapsed the whole page).
    const row = page.locator('.stat-card .toprow[data-fcol="sizes"]', { hasText: '10, 100' }).first();
    await expect(row).toBeVisible();
    await row.click();
    // Equality chip (=), not containment (∋).
    const chip = page.locator('.active-filters .fchip').first();
    await expect(chip).toContainText('=');
    await expect(chip).not.toContainText('∋');
    // The stats still cover the 150 matching rows — not collapsed to 0.
    await expect(page.locator('.stat-card', { hasText: /rows 150\b/ }).first()).toBeVisible();
  });
});
