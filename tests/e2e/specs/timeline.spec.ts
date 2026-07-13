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

    // Three dependencies (s1→s3, s1→s4, s3→s4) → three orthogonal paths.
    await expect(panel.locator('svg.tl-overlay .tl-deps path')).toHaveCount(3);

    // The two paths leaving s1 have dedicated ports/elbows; they must not be
    // the same visual trunk merely because they share a parent.
    const fanout = panel.locator('svg.tl-overlay .tl-deps path[data-from="s1"]');
    await expect(fanout).toHaveCount(2);
    expect(await fanout.nth(0).getAttribute('d')).not.toBe(await fanout.nth(1).getAttribute('d'));
    // The s1→s4 route crosses the cutover time; it has a gap there rather than
    // painting across the event marker (multiple SVG move commands).
    const markerSafe = panel.locator('svg.tl-overlay .tl-deps path[data-from="s1"][data-to="s4"]');
    expect((await markerSafe.getAttribute('d'))!.match(/M/g)!.length).toBeGreaterThan(1);
    await expect(panel.locator('.tl-bar').first()).toHaveCSS('z-index', '2');

    // The --event '50|cutover' marker → a dashed line in the plot, and its label
    // is drawn in the head band above the lanes (not overprinting the bars).
    await expect(panel.locator('svg.tl-event-overlay .tl-events line')).toHaveCount(1);
    await expect(panel.locator('.tl-head-plot .tl-ev-lab', { hasText: 'cutover' })).toBeVisible();

    // Hovering the plot shows the time cursor plus a readout in the head band.
    await panel.locator('.tl-plot').hover();
    const cursor = panel.locator('.tl-cursor');
    await expect(cursor).toHaveClass(/\bshow\b/);
    const readout = panel.locator('.tl-head-readout');
    await expect(readout).toHaveClass(/\bshow\b/);
    await expect(readout).not.toBeEmpty();
  });

  test('bottom axis has regular tick marks and non-overlapping labels', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await expect(panel).toBeVisible();

    // Regularly-spaced tick marks are drawn.
    await expect(panel.locator('.tl-axis .tl-tick-mark').first()).toBeVisible();
    expect(await panel.locator('.tl-axis .tl-tick-mark').count()).toBeGreaterThanOrEqual(3);

    // Whatever labels are shown, none of them overlap (greedy drop guarantees it).
    const labels = panel.locator('.tl-axis .tl-tick');
    const n = await labels.count();
    expect(n).toBeGreaterThanOrEqual(2);
    const boxes = [];
    for (let i = 0; i < n; i++) boxes.push(await labels.nth(i).boundingBox());
    boxes.sort((a, b) => a!.x - b!.x);
    for (let i = 1; i < boxes.length; i++) {
      // each label starts at or after the previous one ends (1px tolerance).
      expect(boxes[i]!.x).toBeGreaterThanOrEqual(boxes[i - 1]!.x + boxes[i - 1]!.width - 1);
    }
  });

  test('hover readout stays within the tile when near the right edge', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline-ts"]');
    await expect(panel).toBeVisible();
    const plot = panel.locator('.tl-plot');
    const pb = await plot.boundingBox();
    // Hover very close to the right edge — where a wide absolute-time readout used
    // to clip. The readout must stay fully inside the head band's width.
    await plot.hover({ position: { x: pb!.width - 3, y: pb!.height / 2 } });
    const readout = panel.locator('.tl-head-readout');
    await expect(readout).toHaveClass(/\bshow\b/);
    const headPlot = panel.locator('.tl-head-plot');
    const hb = await headPlot.boundingBox();
    const rb = await readout.boundingBox();
    expect(rb!.x).toBeGreaterThanOrEqual(hb!.x - 1);
    expect(rb!.x + rb!.width).toBeLessThanOrEqual(hb!.x + hb!.width + 1);
  });

  test('hover readout on a local-tz timeline also shows the UTC instant', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline-ts"]');
    await expect(panel).toBeVisible();
    await panel.locator('.tl-plot').hover();
    const readout = panel.locator('.tl-head-readout');
    await expect(readout).toHaveClass(/\bshow\b/);
    await expect(readout).toContainText('UTC');
  });

  test('an absolute-time event marker renders its line and label', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline-ts"]');
    await expect(panel).toBeVisible();
    // The --event '2026-05-01 14:15|escalated' marker positions against the
    // absolute (UTC) domain: a dashed line in the plot + its label in the band.
    await expect(panel.locator('svg.tl-event-overlay .tl-events line')).toHaveCount(1);
    await expect(panel.locator('.tl-head-plot .tl-ev-lab', { hasText: 'escalated' })).toBeVisible();
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

  test('lane label hover shows its full name', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="timeline"]');
    await panel.locator('.tl-lane-label', { hasText: 'build' }).hover();
    const tip = page.locator('.wm-tip');
    await expect(tip).toBeVisible();
    await expect(tip).toContainText('Lane');
    await expect(tip).toContainText('build');
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
