import { defineConfig, devices } from '@playwright/test';

// CI workflows build first; local `npm run test:e2e` must build so preview has dist/.
const previewCommand = process.env.CI ? 'npm run preview' : 'npm run build && npm run preview';

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  reporter: 'html',
  use: {
    baseURL: 'http://localhost:4321/Sky-Auto-Player',
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: previewCommand,
    url: 'http://localhost:4321/Sky-Auto-Player/',
    reuseExistingServer: !process.env.CI,
    timeout: 120 * 1000,
  },
});
