import { test, expect } from '@playwright/test';
import { SESSION_ID } from '../constants';

test('sunroom applies its light ground and vivid chart palette', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/?theme=sunroom`);

  await expect(page.locator('body')).toBeVisible();
  const vars = await page.locator('html').evaluate((el) => {
    const style = getComputedStyle(el);
    return {
      bg: style.getPropertyValue('--bg').trim(),
      surface: style.getPropertyValue('--surface').trim(),
      accent: style.getPropertyValue('--primary').trim(),
    };
  });

  expect(vars).toEqual({ bg: '#fff1dc', surface: '#fffaf2', accent: '#e04f71' });
  await expect(page.locator('.panel', { hasText: 'By category' }).locator('canvas')).toBeVisible();
});
