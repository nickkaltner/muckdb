import { test, expect } from '../fixtures/test';
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

test('view badge opens its source table', async ({ page }) => {
  const { dbId } = readState();
  await page.goto(`/db/${dbId}/widgets_all/`);
  await expect(page.locator('#tp-results')).toHaveText(/200 results/);
  await page.locator('#view-source').click();
  await expect(page.locator('.tp-name')).toHaveText('widgets');
  await expect(page).toHaveURL(new RegExp(`/db/${dbId}/widgets/`));
});

test('auto rows keep pagination visible at the bottom across viewport sizes', async ({ page }) => {
  const { dbId } = readState();
  for (const height of [900, 600, 750]) {
    await page.setViewportSize({ width: 1280, height });
    await page.goto(`/db/${dbId}/widgets/`);
    await expect(page.locator('#tp-results')).toHaveText(/200 results/);
    await expect.poll(async () => page.evaluate(() => {
      const results = document.getElementById('results')!;
      const pager = document.querySelector('.pager')!.getBoundingClientRect();
      const bottom = document.querySelector('.statusline')!.getBoundingClientRect().top;
      return results.scrollHeight <= results.clientHeight + 1 && pager.bottom <= bottom && bottom - pager.bottom < 25;
    })).toBe(true);
  }
});

test('export collapses before the view title overlaps its badge', async ({ page }) => {
  const { dbId } = readState();
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.goto(`/db/${dbId}/widgets_all/`);
  await expect(page.locator('#tp-results')).toHaveText(/200 results/);
  await expect(page.locator('#export-combo')).toBeVisible();
  await expect(page.locator('.tp-head .export')).toBeHidden();
  const title = await page.locator('.tp-name').boundingBox();
  const badge = await page.locator('#view-source').boundingBox();
  expect(title!.x + title!.width).toBeLessThanOrEqual(badge!.x);
  await page.locator('#export-combo').click();
  await expect(page.locator('.pick-row').filter({ hasText: 'csv' })).toBeVisible();
  await expect(page.locator('.pick-row').filter({ hasText: 'json' })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.setViewportSize({ width: 1100, height: 800 });
  await expect(page.locator('#tp-results')).toBeHidden();
  await expect(page.locator('.tp-name')).not.toHaveClass(/truncate/);
  await expect(page.locator('.pager .range')).toContainText('200');
  await page.setViewportSize({ width: 1800, height: 800 });
  await expect(page.locator('#tp-results')).toBeVisible();
  await expect(page.locator('.tp-head .export')).toBeVisible();
  await expect(page.locator('#export-combo')).toBeHidden();
});
