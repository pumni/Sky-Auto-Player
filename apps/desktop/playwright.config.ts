import { defineConfig, devices } from '@playwright/test';

const baseURL = process.env.PLAYWRIGHT_BASE_URL;

export default defineConfig({
  testDir: process.env.PLAYWRIGHT_TEST_DIR ?? './tests/e2e',
  use: {
    baseURL: baseURL || undefined,
    ...devices['Desktop Chrome'],
    colorScheme: 'dark',
  },
});
