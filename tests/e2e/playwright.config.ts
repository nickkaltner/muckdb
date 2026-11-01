import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './specs',
  timeout: 30000,
  fullyParallel: true,
  workers: 4,
  reporter: [['html', { open: 'never' }], ['list']],
  use: {
    // The seeded absolute-time timeline is formatted as `local`; pin the
    // browser clock so its display-zone assertions do not inherit GitHub's UTC.
    timezoneId: 'Australia/Brisbane',
    // Capture the first failure too: this suite deliberately has no retries.
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
