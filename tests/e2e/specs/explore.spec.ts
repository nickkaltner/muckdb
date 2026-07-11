import { test, expect } from '@playwright/test';
import { readState } from '../constants';

test('facet panel filters the table', async ({ page }) => {
  const { dbId } = readState();
  await page.goto(`/db/${dbId}/widgets/`);
  await expect(page.locator('#tp-results')).toHaveText(/200 results/);

  // The facet panel is visible at desktop viewport; click the "region = US" facet value.
  const usFacet = page
    .locator('.facet-panel .facet-val[data-fcol="region"][data-fval="US"]')
    .first();
  await expect(usFacet).toBeVisible();
  await usFacet.click();

  // region cycles US/EU/APAC over 200 rows: US = ids where i % 3 == 0 → 67 rows.
  await expect(page.locator('#tp-results')).toHaveText(/67 results/);
  await expect(page.locator('.active-filters .fchip')).toContainText('region');
});
