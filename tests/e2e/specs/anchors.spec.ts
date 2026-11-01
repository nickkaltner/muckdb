import { test, expect } from '../fixtures/test';
import { SESSION_ID } from '../constants';
import type { Page } from '@playwright/test';

// Keep enough content on either side to align the target at the scroller top,
// without loading expensive charts unrelated to anchor navigation.
test.use({ sessionTiles: ['summary', 'by-cat', 'map', 'timeline', 'all', 'sequence'] });

async function waitForDashboardLayout(page: Page) {
  // Panels exist before their queries return. Scrolling then lets late content
  // above the target move it and change the active hash to a neighbouring tile.
  await expect(page.locator('#panels > .panel')).toHaveCount(6);
  await expect(page.locator('#panels .panel-chart')).toHaveCount(5);
  await expect(page.locator('#panels .panel-chart:empty')).toHaveCount(0);
  await expect(page.locator('#panels .panel-chart .err')).toHaveCount(0);
  await page.locator('#panels').evaluate(async (panels) => {
    await document.fonts.ready;
    await Promise.all(panels.getAnimations({ subtree: true }).map((animation) => animation.finished));
  });
}

// The session view keeps a #t=<tile> anchor in the URL hash pointing at the tile
// you're reading, so a reload (e.g. a theme switch, which reloads the page)
// restores you to the same panel.
for (const queryDelay of [0, 750]) {
  test.describe(`tile anchors (query delay ${queryDelay}ms)`, () => {
    test.beforeEach(async ({ page }) => {
      if (!queryDelay) return;
      // Exercise layout changes above the target after other tiles have loaded.
      // This is injected network latency, not a sleep used to synchronize the test.
      await page.route('**/api/query?*', async (route) => {
        const sql = new URL(route.request().url()).searchParams.get('sql') || '';
        if (!sql.includes('by_category')) return route.fallback();
        const response = await route.fetch();
        await new Promise((resolve) => setTimeout(resolve, queryDelay));
        await route.fulfill({ response });
      });
    });
    const TARGET = 'timeline'; // a tall panel well down the page

    test('scrolling updates the URL hash to the tile in view', async ({ page }) => {
      await page.goto(`/session/${SESSION_ID}/`);
      await waitForDashboardLayout(page);

      await page.locator(`.panel[data-tile="${TARGET}"]`).evaluate((el) =>
        el.scrollIntoView({ block: 'start' }));

      await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#t=${TARGET}`);
    });

    test('the anchor survives a reload and restores scroll position', async ({ page }) => {
      await page.goto(`/session/${SESSION_ID}/`);
      await waitForDashboardLayout(page);
      await page.locator(`.panel[data-tile="${TARGET}"]`).evaluate((el) =>
        el.scrollIntoView({ block: 'start' }));
      await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#t=${TARGET}`);

      // A reload keeps the hash and lands back on the same tile (not the top).
      await page.reload();
      await waitForDashboardLayout(page);
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
        .toBeGreaterThanOrEqual(-1);
      await expect.poll(() => page.locator(`.panel[data-tile="${TARGET}"]`).evaluate((el) =>
        el.getBoundingClientRect().top - document.getElementById('panels-scroll')!.getBoundingClientRect().top))
        .toBeLessThan(60);
      expect(
        await page.evaluate(() => document.getElementById('panels-scroll')!.scrollTop),
      ).toBeGreaterThan(100);
    });
  });
}
