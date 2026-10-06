import { test, expect } from '../fixtures/test';
import { SESSION_ID } from '../constants';

test.describe('flame tile', () => {
  test('merges stacks into frames, zooms on click and resets', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('.panel[data-tile="flame"]');
    await expect(panel).toBeVisible();

    // all, main, run, init, a, b — shared prefixes merge into one frame each.
    const frames = panel.locator('.fl-frame');
    await expect(frames).toHaveCount(6);
    const frame = (name: string) => panel.locator(`.fl-frame[data-name="${name}"]`);
    const width = async (name: string) => {
      const plot = await panel.locator('.fl-plot').boundingBox();
      const box = await frame(name).boundingBox();
      return box!.width / plot!.width;
    };
    // run holds 90 of 100 samples; a holds 60.
    expect(await width('run')).toBeCloseTo(0.9, 1);
    expect(await width('a')).toBeCloseTo(0.6, 1);

    // Flame orientation: the root sits below its children.
    const rootY = (await frame('all').boundingBox())!.y;
    const mainY = (await frame('main').boundingBox())!.y;
    expect(rootY).toBeGreaterThan(mainY);

    const reset = panel.locator('.fl .tl-reset');
    await expect(reset).toBeHidden();

    // Zoom into run: it and its descendants fill the width, a sibling hides,
    // ancestors stretch full width (dimmed) and the reset button appears.
    await frame('run').click();
    await expect(frame('init')).toBeHidden();
    await expect(frame('main')).toHaveClass(/fl-anc/);
    await expect.poll(() => width('run')).toBeGreaterThan(0.98);
    await expect.poll(() => width('a')).toBeCloseTo(0.667, 1);
    await expect(reset).toBeVisible();
    await expect(panel.locator('.fl-path')).toContainText('run');

    await reset.click();
    await expect(frame('init')).toBeVisible();
    await expect(reset).toBeHidden();
    await expect.poll(() => width('run')).toBeCloseTo(0.9, 1);
  });
});
