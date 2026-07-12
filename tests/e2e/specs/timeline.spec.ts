import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('timeline tile', () => {
  test('renders lanes, bars, a sublane for overlap, and a legend', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await expect(panel).toBeVisible();

    // Two lanes → two lane labels in the gutter.
    await expect(panel.locator('.tl-lane-label')).toHaveCount(2);
    await expect(panel.locator('.tl-lane-label', { hasText: 'build' })).toBeVisible();

    // Four bars.
    await expect(panel.locator('.tl-bar')).toHaveCount(4);

    // The two 'build' bars overlap → different top offsets (a sublane).
    const compile = panel.locator('.tl-bar', { hasText: 'compile' });
    const lint = panel.locator('.tl-bar', { hasText: 'lint' });
    const topOf = (loc) => loc.evaluate((el) => parseFloat((el as HTMLElement).style.top));
    expect(await topOf(compile)).not.toBe(await topOf(lint));

    // Colour-by-status legend has entries.
    await expect(panel.locator('.tl-legend .tl-leg')).toHaveCount(2);

    // Full-width toggle is offered (timeline is in the widen gate).
    await expect(panel.locator('[data-widen]')).toHaveCount(1);
  });

  test('draws dependency connectors, an event marker, and a hover cursor', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await expect(panel).toBeVisible();

    // Two dependencies (s1→s3, s3→s4) → two orthogonal connector paths.
    await expect(panel.locator('svg.tl-overlay .tl-deps path')).toHaveCount(2);

    // The --event '50|cutover' marker → a dashed line + its label.
    await expect(panel.locator('svg.tl-overlay .tl-events line')).toHaveCount(1);
    await expect(panel.locator('svg.tl-overlay .tl-events text')).toContainText('cutover');

    // Hovering the plot shows the time cursor with a readout.
    await panel.locator('.tl-plot').hover();
    const cursor = panel.locator('.tl-cursor');
    await expect(cursor).toHaveClass(/\bshow\b/);
    await expect(panel.locator('.tl-readout')).not.toBeEmpty();
  });

  test('bar hover shows a rich tooltip with core fields and extra columns', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await panel.locator('.tl-bar', { hasText: 'migrate' }).hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toContainText('lane: deploy');
    await expect(tip).toContainText('status: failed');  // colour category, an extra column
  });

  test('bar tooltip escapes a hostile link_title instead of injecting HTML', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await panel.locator('.tl-bar', { hasText: 'migrate' }).hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();

    // The `sid` column carries a link_title template of `<img src=x onerror=alert(1)>`
    // (set in seed.ts). It must show up as literal text in the tooltip's link,
    // never parsed as a live element.
    await expect(tip.locator('img')).toHaveCount(0);
    await expect(tip).toContainText('onerror');
    await expect(tip.locator('a[href="https://example.test/s4"]')).toContainText(
      '<img src=x onerror=alert(1)>'
    );
  });
});
