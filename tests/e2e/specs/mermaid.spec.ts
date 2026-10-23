import { test, expect } from '../fixtures/test';
import { SESSION_ID } from '../constants';

test.describe('authored Mermaid tile', () => {
  test('renders offline and edits with validated live preview', async ({ page }) => {
    await page.goto(`/session/${SESSION_ID}/`);
    const panel = page.locator('[data-tile="authored-tree"]');
    await expect(panel.locator('.mermaid-host svg')).toBeVisible();
    await expect(panel.locator('.mermaid-host')).toContainText('Application');
    expect(await panel.evaluate((el) => {
      const svg = el.querySelector('svg')!.getBoundingClientRect();
      const box = el.getBoundingClientRect();
      return svg.bottom <= box.bottom + 1;
    })).toBe(true);

    await panel.getByRole('button', { name: 'edit' }).click();
    const editor = page.getByRole('dialog', { name: 'Edit Authored service tree' });
    const source = editor.getByRole('textbox', { name: 'Mermaid source' });
    await expect(source).toHaveValue(/flowchart TD/);
    await expect(editor.locator('.mermaid-editor-status')).toHaveText('valid');

    await source.fill('this is not a Mermaid diagram');
    await expect(editor.locator('.mermaid-editor-status')).toHaveText('fix syntax to save');
    await expect(editor.getByRole('button', { name: 'save changes' })).toBeDisabled();

    const updated = 'flowchart TD\n  app[Application] --> api[API]\n  api --> cache[(Cache)]';
    await source.fill(updated);
    await expect(editor.locator('.mermaid-editor-status')).toHaveText('valid');
    await expect(editor.locator('.mermaid-editor-preview')).toContainText('Cache');
    await editor.getByRole('button', { name: 'save changes' }).click();
    await expect(editor).toHaveCount(0);
    await expect(panel.locator('.mermaid-host')).toContainText('Cache');

    const saved = await page.evaluate(async (sessionId) =>
      (await fetch(`/api/session?id=${sessionId}`)).json(), SESSION_ID);
    expect(saved.tiles.find((tile: any) => tile.name === 'authored-tree').source).toBe(updated);
  });

  for (const direction of ['LR', 'TD']) {
    test(`expands a large ${direction} diagram without clipping and restores the tile`, async ({ page }) => {
      await page.setViewportSize({ width: 1280, height: 800 });
      const source = `flowchart ${direction}\n` + Array.from({ length: 20 }, (_, i) =>
        `  n${i}[Service ${i}] --> n${i + 1}[Service ${i + 1}]`).join('\n');
      await page.route('/api/session?id=e2e', async (route) => {
        const response = await route.fetch();
        const session = await response.json();
        session.tiles = session.tiles.filter((tile: { name: string }) => tile.name === 'authored-tree');
        session.tiles[0].source = source;
        await route.fulfill({ response, json: session });
      });
      await page.goto(`/session/${SESSION_ID}/`);
      const panel = page.locator('[data-tile="authored-tree"]');
      await expect(panel.locator('.mermaid-host svg')).toBeVisible();
      await panel.getByRole('button', { name: 'Expand', exact: true }).click();
      await expect(page).toHaveURL(/\/expand\/authored-tree/);
      const viewer = page.locator('.zoom-overlay');
      const svg = viewer.locator('.mermaid-host svg');
      await expect(svg).toBeVisible();
      await expect(viewer.locator('.mermaid-host')).toContainText('Service 20');
      const size = await svg.evaluate((el: SVGSVGElement) => ({
        width: el.getBoundingClientRect().width,
        height: el.getBoundingClientRect().height,
        naturalWidth: el.viewBox.baseVal.width,
        naturalHeight: el.viewBox.baseVal.height,
      }));
      const uiScale = await page.evaluate(() => Number.parseFloat(getComputedStyle(document.documentElement).zoom) || 1);
      expect(size.width / uiScale).toBeCloseTo(Math.floor(size.naturalWidth), 0);
      expect(size.height / uiScale).toBeCloseTo(Math.floor(size.naturalHeight), 0);
      const slot = viewer.locator('.zoom-chart');
      expect(await slot.evaluate((el) => el.scrollWidth > el.clientWidth || el.scrollHeight > el.clientHeight)).toBe(true);
      await slot.evaluate((el) => { el.scrollLeft = el.scrollWidth; el.scrollTop = el.scrollHeight; });
      expect(await slot.evaluate((el) => el.scrollLeft > 0 || el.scrollTop > 0)).toBe(true);
      await page.keyboard.press('Escape');
      await expect(viewer).toHaveCount(0);
      await expect(page).toHaveURL(new RegExp(`/session/${SESSION_ID}/$`));
      await expect(panel.locator('.mermaid-host svg')).toBeVisible();
    });
  }

});
