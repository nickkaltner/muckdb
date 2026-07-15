import { defineConfig, devices } from '@playwright/test';
import { BASE_URL } from './constants';

export default defineConfig({
  testDir: './specs',
  globalSetup: require.resolve('./global-setup'),
  globalTeardown: require.resolve('./global-teardown'),
  timeout: 30000,
  fullyParallel: false,
  workers: 1,
  reporter: [['html', { open: 'never' }], ['list']],
  use: {
    baseURL: BASE_URL,
    // The seeded absolute-time timeline is formatted as `local`; pin the
    // browser clock so its display-zone assertions do not inherit GitHub's UTC.
    timezoneId: 'Australia/Brisbane',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
