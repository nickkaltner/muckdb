import { test, expect } from '../fixtures/test';
import { SESSION_ID } from '../constants';

test('clipboard PNG retains both edges of a centered topology SVG', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  await expect(page.locator('#panels .panel').first()).toBeVisible();
  const expected = await page.evaluate(() => {
    const panel = document.createElement('section');
    panel.className = 'panel'; panel.dataset.tile = 'capture-regression';
    panel.innerHTML = `<button data-copyimg="capture-regression" class="no-screenshot">copy</button>
      <div class="topo-wrap"><svg class="topo-svg" style="--topo-natural-width:600px" viewBox="0 0 600 100">
        <rect x="20" y="20" width="30" height="60" fill="#0000ff"/>
        <rect x="550" y="20" width="30" height="60" fill="#ff0000"/>
      </svg></div>`;
    document.getElementById('panels')!.append(panel);
    // Capture measures at logical zoom, then applies UI zoom to raster scale.
    const root = document.documentElement, priorZoom = root.style.zoom;
    root.style.zoom = '1';
    const box = panel.getBoundingClientRect();
    const svg = panel.querySelector('svg')!.getBoundingClientRect();
    root.style.zoom = priorZoom;
    (window as any).capturePng = null;
    navigator.clipboard.write = async (items) => {
      const blob = await (await items)[0].getType('image/png');
      const bitmap = await createImageBitmap(blob);
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width; canvas.height = bitmap.height;
      const ctx = canvas.getContext('2d')!; ctx.drawImage(bitmap, 0, 0);
      const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      let blueX = Infinity, redX = Infinity, redCount = 0;
      for (let i = 0; i < data.length; i += 4) {
        const x = (i / 4) % canvas.width;
        if (data[i] < 10 && data[i + 1] < 10 && data[i + 2] > 245) blueX = Math.min(blueX, x);
        if (data[i] > 245 && data[i + 1] < 10 && data[i + 2] < 10) { redX = Math.min(redX, x); redCount++; }
      }
      (window as any).capturePng = { width: canvas.width, blueX, redX, redCount };
      bitmap.close();
    };
    return {
      blue: (svg.left - box.left + svg.width * 20 / 600) / box.width,
      red: (svg.left - box.left + svg.width * 550 / 600) / box.width,
    };
  });
  await page.locator('[data-copyimg="capture-regression"]').click();
  await expect.poll(() => page.evaluate(() => !!(window as any).capturePng)).toBe(true);
  const png = await page.evaluate(() => (window as any).capturePng);
  expect(png.redCount).toBeGreaterThan(500);
  expect(Math.abs(png.blueX - expected.blue * png.width)).toBeLessThan(4);
  expect(Math.abs(png.redX - expected.red * png.width)).toBeLessThan(4);
});
