import { test, expect } from '@playwright/test';
import type { Page } from '@playwright/test';

const origin = process.env.PLAYWRIGHT_BASE_URL;
if (!origin) {
  throw new Error(
    'PLAYWRIGHT_BASE_URL is required. Run tests through "bun run test:e2e" or provide an explicit PLAYWRIGHT_BASE_URL.',
  );
}

async function contrastRatio(page: Page, selector: string) {
  return page.locator(selector).evaluate((element: HTMLElement) => {
    const style = window.getComputedStyle(element);
    const context = document.createElement('canvas').getContext('2d', {
      willReadFrequently: true,
    });
    if (!context) throw new Error('Canvas 2D is unavailable for computed-color measurement');

    const channels = (color: string) => {
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = color;
      context.fillRect(0, 0, 1, 1);
      return Array.from(context.getImageData(0, 0, 1, 1).data)
        .slice(0, 3)
        .map((channel) => channel / 255);
    };
    const luminance = (color: string) =>
      channels(color)
        .map((channel) =>
          channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4,
        )
        .reduce((sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index], 0);
    const foreground = luminance(style.color);
    const background = luminance(style.backgroundColor);
    const lighter = Math.max(foreground, background);
    const darker = Math.min(foreground, background);
    return (lighter + 0.05) / (darker + 0.05);
  });
}

test.describe('computed contrast in the localized global shell', () => {
  for (const route of ['/', '/vi/'] as const) {
    test(route + ' selected language and release action have readable text', async ({ page }) => {
      await page.goto(origin + route);
      await expect(page.locator('.home-language')).toBeVisible();
      expect(await contrastRatio(page, '.home-language')).toBeGreaterThanOrEqual(4.5);
      await expect(page.locator('.header-download')).toHaveCount(0);
      await page.goto(origin + route + 'faq/');
      const activeLanguage = page.locator(".locale-switch__option[aria-current='page']");
      const releases = page.locator('.header-download');

      await expect(activeLanguage).toBeVisible();
      await expect(releases).toBeVisible();
      expect(
        await contrastRatio(page, ".locale-switch__option[aria-current='page']"),
      ).toBeGreaterThanOrEqual(4.5);
      expect(await contrastRatio(page, '.header-download')).toBeGreaterThanOrEqual(4.5);
    });
  }
});
