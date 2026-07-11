import { test, expect } from '@playwright/test';
import { readState } from '../constants';
import { readFileSync } from 'node:fs';

test('CSV and JSON export produce non-empty downloads', async ({ page }) => {
  const { dbId } = readState();
  await page.goto(`/db/${dbId}/widgets/`);
  await expect(page.locator('#tp-results')).toHaveText(/200 results/);

  // CSV.
  const [csv] = await Promise.all([
    page.waitForEvent('download'),
    page.locator('.exlink[data-export="csv"]').click(),
  ]);
  const csvPath = await csv.path();
  const csvText = readFileSync(csvPath, 'utf8');
  expect(csvText.split('\n').length).toBeGreaterThan(1);
  expect(csvText).toContain('category');

  // JSON.
  const [json] = await Promise.all([
    page.waitForEvent('download'),
    page.locator('.exlink[data-export="json"]').click(),
  ]);
  const jsonText = readFileSync(await json.path(), 'utf8');
  const parsed = JSON.parse(jsonText);
  expect(Array.isArray(parsed) ? parsed.length : Object.keys(parsed).length).toBeGreaterThan(0);
});
