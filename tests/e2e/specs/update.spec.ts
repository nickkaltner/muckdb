import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { REPO_ROOT } from '../constants';

const cargoToml = readFileSync(join(REPO_ROOT, 'Cargo.toml'), 'utf8');
const version = cargoToml.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
if (!version) throw new Error('could not read muckdb version from Cargo.toml');

test('newer cached release turns the version indicator red with a rich tooltip', async ({ page }) => {
  await page.goto('/');
  const indicator = page.locator('#sl-state');
  await expect(indicator).toHaveClass(/update-available/);
  await expect(indicator).toHaveText(`v${version}`);
  await expect(indicator).toHaveAttribute('data-tip', /Update available/);
  await expect(indicator).toHaveAttribute('data-tip', /v99\.0\.0/);
});
