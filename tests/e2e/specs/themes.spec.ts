import { test, expect } from '../fixtures/test';
import { SESSION_ID } from '../constants';

test('theme picker opens below the theme button', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);

  await page.locator('#theme-btn').click();
  const picker = page.locator('.pick-pop .pick-box');
  await expect(picker).toBeVisible();

  const [box, viewport] = await Promise.all([
    picker.boundingBox(),
    page.evaluate(() => ({ width: window.innerWidth, height: window.innerHeight })),
  ]);
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
  expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height);
});

test('theme editor labels its name and only enables save after an edit', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/?theme=custom-1`);
  await page.locator('#theme-btn').click();
  await page.locator('[data-pick-action]').click();

  const editor = page.locator('.theme-workbench');
  await expect(editor).toBeVisible();
  await expect(editor.locator('.beta-badge')).toHaveText('beta');
  await expect(editor.getByLabel('Theme name')).toBeVisible();
  const save = editor.locator('[data-tw="save"]');
  await expect(save).toBeDisabled();

  await editor.locator('.tw-advanced summary').click();
  await editor.locator('[data-tw-setting="flat"]').check();
  await expect(save).toBeEnabled();
  await save.click();
  await expect(editor).toBeHidden();
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('muckdb.theme-slots') || '[]'));
  expect(saved[0].flat).toBe(true);
});

test('copy theme opens a visible list and advanced settings survive JSON sharing', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/?theme=custom-1`);
  await page.locator('#theme-btn').click();
  await page.locator('[data-pick-action]').click();
  const editor = page.locator('.theme-workbench');

  await editor.locator('[data-tw="copy"]').click();
  const picker = page.locator('.pick-overlay .pick-box');
  await expect(picker).toBeVisible();
  await expect(picker.locator('.pick-row').first()).toBeVisible();
  const [box, viewport] = await Promise.all([
    picker.boundingBox(),
    page.evaluate(() => ({ width: window.innerWidth, height: window.innerHeight })),
  ]);
  expect(box).not.toBeNull();
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height);
  await picker.locator('.pick-row', { hasText: 'ink' }).click();
  await expect(editor.getByLabel('Theme name')).toHaveValue('ink');

  const downloadPromise = page.waitForEvent('download');
  await editor.locator('[data-tw="export"]').click();
  const download = await downloadPromise;
  const path = await download.path();
  expect(path).not.toBeNull();
  await editor.locator('[data-tw="reset"]').click();
  await editor.locator('[data-tw-file]').setInputFiles(path!);
  await expect(editor.getByLabel('Theme name')).toHaveValue('ink');
  await editor.locator('.tw-advanced summary').click();
  await expect(editor.locator('[data-tw-setting="flat"]')).toBeChecked();
});

test('the final dashboard panel clears the fixed status bar', async ({ page }) => {
  await page.goto(`/session/${SESSION_ID}/`);
  await expect(page.locator('#panels .panel').last()).toBeVisible();

  const clearance = await page.evaluate(() => {
    const scroller = document.getElementById('panels-scroll')!;
    const last = document.querySelector('#panels .panel:last-child')!;
    scroller.scrollTop = scroller.scrollHeight;
    const statusTop = document.getElementById('statusline')!.getBoundingClientRect().top;
    return statusTop - last.getBoundingClientRect().bottom;
  });
  // Includes the dashboard's 28px bottom padding (scaled to 35px on screen).
  expect(clearance).toBeGreaterThanOrEqual(20);
});

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
