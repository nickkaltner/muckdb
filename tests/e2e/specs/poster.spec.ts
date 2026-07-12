import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

// Regression: the poster / copy-image export (html2canvas) must not choke on
// color-mix() styles. Chrome serialises a computed color-mix() as `color(srgb …)`,
// which html2canvas can't parse ("unsupported color function 'color'") — the
// onclone hook rewrites those to rgba() before capture. The e2e session has
// timeline + map tiles that use color-mix in their styles.
test('dashboard poster exports despite color-mix styles', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  await expect(page.locator('.panel[data-tile="timeline"]')).toBeVisible();

  const [download] = await Promise.all([
    page.waitForEvent('download', { timeout: 30000 }),
    page.click('#poster-btn'),
  ]);
  // A download only fires on success; an html2canvas parse error would instead
  // surface an error toast and no download.
  expect(download.suggestedFilename()).toBe(`${SESSION_ID}.png`);
});
