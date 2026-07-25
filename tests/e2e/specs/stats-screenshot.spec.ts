import { expect, test } from '@playwright/test';
import { readState } from '../constants';

test('analysis tabs provide a screenshot download', async ({ page }) => {
  const { dbId } = readState();
  await page.goto(`/db/${dbId}/widgets/?view=stats&tab=correlation`);

  const button = page.locator('[data-stats-shot="correlation"]');
  await expect(button).toBeVisible();
  await expect(button).toHaveAttribute('title', /Download correlation screenshot/);

  const [download] = await Promise.all([
    page.waitForEvent('download'),
    button.click(),
  ]);
  expect(download.suggestedFilename()).toBe('widgets-correlation.png');
});
