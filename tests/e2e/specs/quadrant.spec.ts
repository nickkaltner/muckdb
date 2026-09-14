import { expect, test } from '../fixtures/test';
import { SESSION_ID } from '../constants';

test('quadrant structural lines snap cleanly and clustered labels avoid overlap', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  const svg = page.locator('.panel[data-tile="quadrant"] .quad-svg');
  await expect(svg).toBeVisible();
  await expect(svg.locator('.quad-label')).toHaveCount(4);
  await expect(svg.locator('.quad-axis-end')).toHaveCount(4);
  await expect(svg.locator('.quad-axis-cue')).toHaveCount(0);
  await expect(svg.locator('.quad-gutter')).toHaveCount(4);
  await expect(svg.locator('.quad-gutter-divider')).toHaveCount(2);
  await expect(svg.locator('.quad-axis-tag')).toHaveCount(0);
  const result = await svg.evaluate((el) => {
    const labels = [...el.querySelectorAll('.quad-label')].map((label) => {
      const rect = label.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
    });
    const overlaps = labels.some((a, i) => labels.slice(i + 1).some((b) =>
      Math.min(a.right, b.right) > Math.max(a.left, b.left)
      && Math.min(a.bottom, b.bottom) > Math.max(a.top, b.top)));
    return {
      border: getComputedStyle(el.querySelector('.quad-bg')!).shapeRendering,
      axes: getComputedStyle(el.querySelector('.quad-axis')!).shapeRendering,
      grid: getComputedStyle(el.querySelector('.quad-grid')!).shapeRendering,
      overlaps,
    };
  });
  expect(result).toEqual({ border: 'crispedges', axes: 'crispedges', grid: 'crispedges', overlaps: false });
});
