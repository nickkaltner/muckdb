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

    // The toggle lives in the tile's header actions (excluded from poster /
    // screenshot captures), not in the map body.
    await expect(panel.locator('.panel-actions .wm-toggle')).toHaveCount(1);
    await expect(panel.locator('.panel-body .wm-toggle')).toHaveCount(0);

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

  test('connections map draws arcs (bottom), dots, and labels (top layer)', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="flows"]');
    // Switch this tile's map to hi-fi (arcs only render on the SVG map).
    await panel.locator('.wm-mode[data-mapmode="svg"]').click();
    await expect(panel.locator('.wm-svg')).toBeVisible();

    // At least one arc per connection row (a wrapped arc adds a second copy),
    // plus endpoint dots.
    expect(await panel.locator('.wm-svg .wm-arcs .wm-arc').count()).toBeGreaterThanOrEqual(3);
    expect(await panel.locator('.wm-svg .wm-dots circle').count()).toBeGreaterThan(0);
    // Labels for each connection, and they're the LAST group (drawn on top).
    await expect(panel.locator('.wm-svg .wm-conn-labels .wm-arc-label')).toHaveCount(3);
    const lastGroupClass = await panel.locator('.wm-svg > g').last().getAttribute('class');
    expect(lastGroupClass).toBe('wm-conn-labels');

    // Arcs are semi-transparent (a real overlay, not opaque).
    const op = await panel.locator('.wm-svg .wm-arc').first().evaluate((el) => parseFloat(getComputedStyle(el).strokeOpacity));
    expect(op).toBeGreaterThan(0);
    expect(op).toBeLessThan(1);

    // The arc stops short of the endpoints (a margin) — its path start isn't the
    // raw projected endpoint. Just assert it's a quadratic curve (M…Q…).
    const d = await panel.locator('.wm-svg .wm-arc').first().getAttribute('d');
    expect(d).toMatch(/^M[\d.\s-]+Q/);

    // The fixture's New York → Sydney spans >180° of longitude, so it takes the
    // shorter way round the globe: a second copy shifted a full map-width (a
    // translate transform) renders the piece that wraps to the other edge.
    const translated = await panel.locator('.wm-svg .wm-arc[transform*="translate"]').count();
    expect(translated).toBeGreaterThanOrEqual(1);

    // Labels carry a subtle rounded background pill (one per connection).
    await expect(panel.locator('.wm-svg .wm-conn-labels .wm-label-bg')).toHaveCount(3);

    // Hovering a label shows the same tooltip as hovering its arc: the label
    // carries the same data-tip and triggers the shared marker tooltip.
    const label = panel.locator('.wm-svg .wm-conn-labels .wm-arc-label').first();
    const labelTip = await label.getAttribute('data-tip');
    expect(labelTip).toBeTruthy();
    await label.hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();
    const labelText = (await label.textContent())!.trim();
    await expect(tip).toContainText(labelText);
  });

  test('connections also render as a fluid SVG overlay on the ASCII backdrop', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="flows"]');
    // ASCII is the default mode; the fluid overlay hydrates over the <pre>.
    const overlay = panel.locator('.wm-ascii-overlay');
    await expect(overlay).toBeVisible();
    await expect(panel.locator('.wm-ascii-stage pre.worldmap')).toBeVisible();
    expect(await overlay.locator('.wm-arcs .wm-arc').count()).toBeGreaterThanOrEqual(3);
    await expect(overlay.locator('.wm-conn-labels .wm-arc-label')).toHaveCount(3);
    expect(await overlay.locator('.wm-dots circle').count()).toBeGreaterThan(0);
  });

  test('hi-fi land renders faded, never solid black (self-contained fill)', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="map"]');
    await panel.locator('.wm-mode[data-mapmode="svg"]').click();
    await expect(panel.locator('.wm-svg')).toBeVisible();
    // The country group must resolve to a real (non-black) fill even though the
    // colour comes from currentColor — a regression guard against the cached-SVG
    // black-map bug. The faintness comes from the #land wrapper's group opacity.
    const fill = await panel.locator('.wm-svg #polygons').evaluate((el) => getComputedStyle(el).fill);
    expect(fill).not.toBe('rgb(0, 0, 0)');
    const op = await panel.locator('.wm-svg #land').evaluate((el) => parseFloat(getComputedStyle(el).opacity));
    expect(op).toBeLessThan(0.5);
  });

  test('the hi-fi sea has a subtle animated static layer, masked to open water', async ({ page }) => {
    // A dark theme so the sea static is enabled (light themes default it off).
    await page.addInitScript(() => { try { localStorage.setItem('muckdb.theme', 'carbon'); } catch (_) {} });
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="map"]');
    await panel.locator('.wm-mode[data-mapmode="svg"]').click();
    await expect(panel.locator('.wm-svg')).toBeVisible();
    // Animated noise (feTurbulence with an <animate>), rendered subtly and clipped
    // to the sea via the land mask.
    await expect(panel.locator('.wm-svg #wm-static feTurbulence animate')).toHaveCount(1);
    const stat = panel.locator('.wm-svg .wm-static');
    await expect(stat).toHaveAttribute('mask', /wm-sea/);
    const op = await stat.evaluate((el) => parseFloat(getComputedStyle(el).opacity));
    expect(op).toBeGreaterThan(0);      // on for dark themes
    expect(op).toBeLessThan(0.6);       // still subtle
  });

  test('the sea static is stronger on light themes (low-contrast grey needs it)', async ({ page }) => {
    await page.addInitScript(() => { try { localStorage.setItem('muckdb.theme', 'paper'); } catch (_) {} });
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="map"]');
    await panel.locator('.wm-mode[data-mapmode="svg"]').click();
    await expect(panel.locator('.wm-svg')).toBeVisible();
    const op = await panel.locator('.wm-svg .wm-static').evaluate((el) => parseFloat(getComputedStyle(el).opacity));
    expect(op).toBeGreaterThan(0.2);   // boosted on light themes so it reads
  });

});
