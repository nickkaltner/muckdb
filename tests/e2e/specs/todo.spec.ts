import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('todo tile renders states, updates DuckDB, and can leave presentation mode', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/#t=todos`);
  const panel = page.locator('.panel[data-tile="todos"]');
  await expect(panel.locator('.todo-item')).toHaveCount(4);
  await expect(panel.locator('.todo-item[data-status="success"] .todo-text')).toHaveCSS('text-decoration-line', 'line-through');
  await expect(panel).not.toContainText('Success means every chart');
  await page.locator('.panel[data-tile="all"]').evaluate((element: any) => { element.identityMarker = true; });

  const pending = panel.locator('.todo-item', { hasText: 'Review the dashboard' });
  await pending.hover();
  await pending.locator('[data-todo-status="success"]').click();
  await expect(pending).toHaveAttribute('data-status', 'success');
  expect(await page.locator('.panel[data-tile="all"]').evaluate((element: any) => element.identityMarker)).toBe(true);

  const session = await (await page.request.get(`/api/session?id=${SESSION_ID}`)).json();
  const db = session.tiles.find((tile: { name: string; db?: string }) => tile.name === 'todos').db;
  const row = await page.request.get('/api/query?' + new URLSearchParams({
    db,
    sql: "SELECT status, completed_at IS NOT NULL AS done FROM collaborative_todos WHERE item_description = 'Review the dashboard'",
  }));
  expect(await row.json()).toMatchObject({ rows: [['success', true]] });

  const toggle = panel.locator('[data-presentation="todos"]');
  await toggle.click();
  await expect(toggle).toHaveClass(/active/);
  await expect.poll(async () => (await page.request.get(`/api/session?id=${SESSION_ID}`)).json())
    .toMatchObject({ tiles: expect.arrayContaining([expect.objectContaining({ name: 'todos', skip_presentation: true })]) });

  await page.keyboard.press('p');
  await page.keyboard.press('p');
  await expect(page.locator('.presentation-overlay')).toBeVisible();
  await expect(page.locator('.presentation-overlay [data-tile="todos"]')).toHaveCount(0);
  await page.keyboard.press('Escape');
});
