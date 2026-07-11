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
