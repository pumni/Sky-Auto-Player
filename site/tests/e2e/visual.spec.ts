import { test, expect } from '@playwright/test';

const visualCases = [
  { name: 'home-en-desktop-1440.png', route: '/', width: 1440, height: 900 },
  { name: 'home-en-mobile-390.png', route: '/', width: 390, height: 844 },
  { name: 'home-vi-mobile-390.png', route: '/vi/', width: 390, height: 844 },
] as const;

test.describe('visual QA baselines', () => {
  test.skip(
    !!process.env.CI,
    'Snapshots are generated locally on Windows, skip on CI Linux due to OS rendering differences',
  );

  for (const visualCase of visualCases) {
    test(visualCase.name, async ({ page }) => {
      await page.setViewportSize({ width: visualCase.width, height: visualCase.height });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(`/Sky-Auto-Player${visualCase.route}`);

      await expect(page).toHaveScreenshot(visualCase.name, {
        animations: 'disabled',
        caret: 'hide',
        fullPage: true,
        maxDiffPixelRatio: 0.01,
      });
    });
  }
});
