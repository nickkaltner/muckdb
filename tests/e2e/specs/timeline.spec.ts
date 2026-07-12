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
});
