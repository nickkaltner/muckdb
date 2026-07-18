import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test.describe('presentation mode', () => {
  test('opens with pp, advances live tiles, and returns on Escape', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    await expect(page.locator('#panels > .panel')).not.toHaveCount(0);

    await page.keyboard.press('p');
    await page.keyboard.press('p');
    const deck = page.locator('.presentation-overlay');
    await expect(deck).toBeVisible();
    await expect(deck.locator('.presentation-stage > .panel')).toHaveCount(1);
    await expect(deck.locator('.presentation-page')).toHaveText(/^1 \/ \d+$/);

    await page.keyboard.press('ArrowRight');
    await expect(deck.locator('.presentation-page')).toHaveText(/^2 \/ \d+$/);
    const section = deck.locator('.presentation-stage > .section-panel');
    await expect(section).toBeVisible();
    await expect(deck).toHaveClass(/\bpresentation-section\b/);
    expect(await deck.evaluate((el) => getComputedStyle(el, '::after').animationName))
      .toBe('presentation-section-bloom');
    expect(await deck.evaluate((el) => getComputedStyle(el, '::after').animationDuration))
      .toBe('1s');
    expect(await section.locator('.section-bar').evaluate((el) => getComputedStyle(el).animationName))
      .toBe('presentation-section-title');
    await page.keyboard.press('ArrowLeft');
    await expect(deck.locator('.presentation-page')).toHaveText(/^1 \/ \d+$/);

    // A Chart.js tile replays its native draw animation on presentation entry.
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowRight');
    const chartCanvas = deck.locator('.presentation-stage > .panel[data-tile="by-cat"] canvas');
    await expect(chartCanvas).toBeVisible();
    await expect.poll(() => chartCanvas.evaluate((canvas) => (window as any).Chart.getChart(canvas).options.animation.duration))
      .toBe(900);

    // Maps use a wider stage (96vw) than ordinary presentation slides.
    for (let i = 0; i < 4; i++) await page.keyboard.press('ArrowRight');
    const stage = deck.locator('.presentation-stage');
    await expect(stage).toHaveClass(/\bpresentation-map\b/);
    const mapBox = await deck.locator('.presentation-stage > .panel').boundingBox();
    expect(mapBox!.width).toBeGreaterThan(page.viewportSize()!.width * 0.9);
    const worldMapBox = await deck.locator('.worldmap-wrap').boundingBox();
    const captionBox = await deck.locator('.panel-caption').boundingBox();
    expect(captionBox!.y - (worldMapBox!.y + worldMapBox!.height)).toBeLessThanOrEqual(30);
    expect(mapBox!.y + mapBox!.height - (captionBox!.y + captionBox!.height)).toBeLessThanOrEqual(22);

    // The table's "more rows" exploration hint is useful in the dashboard,
    // but not to an audience. The reusable opt-out class hides it in slides.
    for (let i = 0; i < 4; i++) await page.keyboard.press('ArrowRight');
    await expect(deck.locator('.presentation-stage > .panel[data-tile="all"]')).toBeVisible();
    await expect(deck.locator('.hide-presentation')).toBeHidden();

    // Every deck closes on a dedicated, non-tile final slide.
    for (let i = 0; i < 20; i++) await page.keyboard.press('ArrowRight');
    const fin = deck.locator('.presentation-stage > .presentation-fin');
    await expect(fin).toBeVisible();
    await expect(fin).toHaveText('Fin.');
    const [current, total] = (await deck.locator('.presentation-page').textContent())!.split(' / ');
    expect(current).toBe(total);

    await page.keyboard.press('Escape');
    await expect(deck).toHaveCount(0);
    await expect(page.locator('#panels > .panel')).not.toHaveCount(0);
  });
});
