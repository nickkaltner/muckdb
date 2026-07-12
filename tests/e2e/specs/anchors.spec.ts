import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

// The session view keeps a #t=<tile> anchor in the URL hash pointing at the tile
// you're reading, so a reload (e.g. a theme switch, which reloads the page)
// restores you to the same panel.
test.describe('tile anchors', () => {
  const TARGET = 'timeline'; // a tall panel well down the page

  test('scrolling updates the URL hash to the tile in view', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    await expect(page.locator(`.panel[data-tile="${TARGET}"]`)).toBeVisible();

    await page.locator(`.panel[data-tile="${TARGET}"]`).evaluate((el) =>
      el.scrollIntoView({ block: 'start' }));

    await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#t=${TARGET}`);
  });

  test('the anchor survives a reload and restores scroll position', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    await page.locator(`.panel[data-tile="${TARGET}"]`).evaluate((el) =>
      el.scrollIntoView({ block: 'start' }));
    await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#t=${TARGET}`);

    // A reload keeps the hash and lands back on the same tile (not the top).
    await page.reload();
    await expect(page).toHaveURL(new RegExp(`#t=${TARGET}$`));

    // The target panel ends up at (near) the top of the scroller, and the
    // scroller is genuinely scrolled down — position was restored.
    await expect
      .poll(async () =>
        page.locator(`.panel[data-tile="${TARGET}"]`).evaluate((el) => {
          const scroller = document.getElementById('panels-scroll')!;
          return el.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
        }),
      )
      .toBeLessThan(60);
    expect(
      await page.evaluate(() => document.getElementById('panels-scroll')!.scrollTop),
    ).toBeGreaterThan(100);
  });
});
