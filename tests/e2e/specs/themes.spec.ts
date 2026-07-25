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

test('strong paper keeps warm stock while raising structural contrast', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/?theme=strong-paper`);

  await expect(page.locator('body')).toBeVisible();
  const vars = await page.locator('html').evaluate((el) => {
    const style = getComputedStyle(el);
    return {
      bg: style.getPropertyValue('--bg').trim(),
      surface: style.getPropertyValue('--surface').trim(),
      ink: style.getPropertyValue('--fg').trim(),
      accent: style.getPropertyValue('--primary').trim(),
      land: style.getPropertyValue('--wm-land-opacity').trim(),
    };
  });

  expect(vars).toEqual({
    bg: '#e6dac0', surface: '#f7f0df', ink: '#202a2b', accent: '#b8481c', land: '0.34',
  });
  await expect(page.locator('.panel', { hasText: 'By category' }).locator('canvas')).toBeVisible();
});

test('ink uses a flat monochrome shell with signal colours', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/?theme=ink`);

  await expect(page.locator('body')).toBeVisible();
  const vars = await page.locator('html').evaluate((el) => {
    const style = getComputedStyle(el);
    return {
      bg: style.getPropertyValue('--bg').trim(),
      surface: style.getPropertyValue('--surface').trim(),
      accent: style.getPropertyValue('--primary').trim(),
      annotation: style.getPropertyValue('--anno-event').trim(),
      panel: style.getPropertyValue('--grad-panel').trim(),
    };
  });

  expect(vars).toEqual({
    bg: '#0c0c0b', surface: '#151514', accent: '#ff5a1f', annotation: '#20d7d2', panel: 'none',
  });
  await expect(page.locator('.panel', { hasText: 'By category' }).locator('canvas')).toBeVisible();
});

test('arctic keeps mono restrained while giving categorical charts a blue palette', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/?theme=arctic`);

  const vars = await page.locator('html').evaluate((el) => {
    const style = getComputedStyle(el);
    return {
      bg: style.getPropertyValue('--bg').trim(),
      surface: style.getPropertyValue('--surface').trim(),
      accent: style.getPropertyValue('--primary').trim(),
      annotation: style.getPropertyValue('--anno-event').trim(),
    };
  });

  expect(vars).toEqual({ bg: '#101216', surface: '#181c23', accent: '#6eb6ff', annotation: '#55d3d1' });
  await expect(page.locator('.panel', { hasText: 'By category' }).locator('canvas')).toBeVisible();
});
