import { test, expect } from '@playwright/test';
import { readState } from '../constants';

test.describe('query editor intelligence', () => {
  test('formats DuckDB SQL, highlights through Tree-sitter, and completes live schema names', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/query/`);
    const input = page.locator('#sql-input');
    await expect(input).toBeVisible();

    await input.fill('select category from widgets');
    await input.press('Control+Space');
    const menu = page.locator('.sql-complete');
    await expect(menu).toBeVisible();
    await expect(menu).toContainText('widgets');
    await input.press('Tab');
    await expect(input).toHaveValue(/widgets/);

    // An identifier may already be quoted while the user is typing it. The
    // completion must replace that opening quote too, not leave `""widgets`.
    await input.fill('select category from "widgets');
    await input.press('Control+Space');
    await input.press('Tab');
    await expect(input).toHaveValue('select category from "widgets"');

    await input.fill('sum');
    await input.press('Control+Space');
    await expect(menu).toContainText('sum');
    await input.press('Tab');
    await expect(input).toHaveValue('sum()');

    await input.fill('select category,count(*) from widgets group by category');
    await page.locator('[data-format]').click();
    await expect(input).toHaveValue(/SELECT\n\s+category,/);
    await expect(page.locator('.sql-hl .s-kw').first()).toHaveText('SELECT');
  });

  test('saves a query as a view and offers a safe name when it already exists', async ({ page }) => {
    const { dbId } = readState();
    await page.goto(`/db/${dbId}/query/`);
    const input = page.locator('#sql-input');
    await input.fill('select category from widgets');

    page.once('dialog', async (dialog) => {
      expect(dialog.type()).toBe('prompt');
      await dialog.accept('saved_widget_categories');
    });
    await page.locator('[data-saveview]').click();
    await expect(page.locator('.toast')).toContainText('saved view saved_widget_categories');

    page.once('dialog', async (dialog) => {
      await dialog.accept('saved_widget_categories');
    });
    await page.locator('[data-saveview]').click();
    const chooser = page.locator('.pick-overlay');
    await expect(chooser).toBeVisible();
    await expect(chooser).toContainText('save as saved_widget_categories_2');
    await expect(chooser).toContainText('overwrite saved_widget_categories');
    await expect(chooser).toContainText('cancel');
    await chooser.getByText('cancel', { exact: true }).click();
    await expect(chooser).toBeHidden();
  });
});
