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

  test('expanded map goes near full-screen (wide modal) with a copy-image button', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    await page.locator('.panel[data-tile="map"] [data-zoom]').click();
    const box = page.locator('.zoom-box');
    // Maps get the "mapzoom" treatment: the box widens toward the viewport so
    // the map scales up instead of hugging the 1280px cap.
    await expect(box).toHaveClass(/\bmapzoom\b/);
    await expect(page.locator('.zoom-overlay .worldmap')).toBeVisible();
    const boxW = (await box.boundingBox())!.width;
    const vw = page.viewportSize()!.width;
    expect(boxW).toBeGreaterThan(vw * 0.9);
    // Every expanded tile offers a copy-image action.
    await expect(page.locator('.zoom-overlay .zoom-copyimg')).toBeVisible();
  });

  test('hi-fi land renders faded, never solid black (self-contained fill)', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="map"]');
    await panel.locator('.wm-mode[data-mapmode="svg"]').click();
    await expect(panel.locator('.wm-svg')).toBeVisible();
    // The country group must resolve to a real (non-black) fill even though the
    // colour comes from currentColor — a regression guard against the cached-SVG
    // black-map bug. Opacity keeps it faint; the resolved colour is the fg.
    const grp = panel.locator('.wm-svg #polygons');
    const fill = await grp.evaluate((el) => getComputedStyle(el).fill);
    expect(fill).not.toBe('rgb(0, 0, 0)');
    const op = await grp.evaluate((el) => parseFloat(getComputedStyle(el).fillOpacity));
    expect(op).toBeLessThan(0.5);
  });
});
