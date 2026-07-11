import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('map tile', () => {
  test('marker tooltip shows above the expand overlay', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);

    // Expand the map tile into the zoom overlay.
    await page.locator('.panel[data-tile="map"] [data-zoom]').click();
    const overlay = page.locator('.zoom-overlay');
    await expect(overlay).toBeVisible();

    // Hover a marker inside the zoomed map → the rich tooltip appears.
    await page.locator('.zoom-overlay .wm-x').first().hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toContainText(/point/i);

    // Regression: the tooltip must stack ABOVE the overlay (the bug was
    // z-index 60 < the overlay's 70, so it rendered behind it).
    const tipZ = await tip.evaluate((el) => parseInt(getComputedStyle(el).zIndex, 10) || 0);
    const overlayZ = await overlay.evaluate((el) => parseInt(getComputedStyle(el).zIndex, 10) || 0);
    expect(tipZ).toBeGreaterThanOrEqual(overlayZ);
  });

  test('hi-fi toggle swaps the ASCII map for the SVG map', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="map"]');

    // Defaults to ASCII: the <pre> map is shown, the SVG host is hidden.
    const wrap = panel.locator('.worldmap-wrap');
    await expect(wrap).toHaveClass(/\bmode-ascii\b/);
    await expect(panel.locator('.worldmap')).toBeVisible();
    await expect(panel.locator('.wm-svg')).toHaveCount(0);

    // Flip to hi-fi → the SVG map hydrates with one dot per plotted cell.
    await panel.locator('.wm-mode[data-mapmode="svg"]').click();
    await expect(wrap).toHaveClass(/\bmode-svg\b/);
    await expect(panel.locator('.wm-mode[data-mapmode="svg"]')).toHaveClass(/\bon\b/);
    await expect(panel.locator('.wm-svg')).toBeVisible();
    const dots = panel.locator('.wm-svg .wm-dots circle.wm-x');
    expect(await dots.count()).toBeGreaterThan(0);

    // The preference persists: a fresh load comes up in hi-fi mode.
    await page.reload();
    await expect(panel.locator('.worldmap-wrap')).toHaveClass(/\bmode-svg\b/);
    await expect(panel.locator('.wm-svg')).toBeVisible();

    // Flip back to ASCII for later tests / other viewers.
    await panel.locator('.wm-mode[data-mapmode="ascii"]').click();
    await expect(panel.locator('.worldmap-wrap')).toHaveClass(/\bmode-ascii\b/);
  });

  test('expanded map shrinks to its content (no full-height whitespace)', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    await page.locator('.panel[data-tile="map"] [data-zoom]').click();
    const box = page.locator('.zoom-box');
    // Content-sized → the modal gets the "fit" treatment and renders the map.
    await expect(box).toHaveClass(/\bfit\b/);
    await expect(page.locator('.zoom-overlay .worldmap')).toBeVisible();
    // The box hugs its content rather than filling the modal height.
    const boxH = (await box.boundingBox())!.height;
    const vh = page.viewportSize()!.height;
    expect(boxH).toBeLessThan(vh * 0.85);
  });
});
