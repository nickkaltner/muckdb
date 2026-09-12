import { test, expect } from '@playwright/test';
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
});
