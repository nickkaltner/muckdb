import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('box tile', () => {
  test('ignores incomplete summaries when calculating the shared range', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="boxes"]');

    // NULL statistics must not become Number(null) === 0: only the two
    // complete summaries render, and their global whisker range is 40–90.
    await expect(panel.locator('.bx-row')).toHaveCount(2);
    await expect(panel.locator('.bx-axis-line')).toHaveText(/40\s*90/);
    await expect(panel).not.toContainText('Incomplete summary');
  });
});
