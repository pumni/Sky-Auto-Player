import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: process.env.PLAYWRIGHT_TEST_DIR ?? './tests/e2e',
  use: {
    baseURL: process.env.PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:4173',
    ...devices['Desktop Chrome'],
  },
});
