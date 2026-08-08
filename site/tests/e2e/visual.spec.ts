import { test, expect, type Page } from '@playwright/test';

async function prepare(page: Page, route: string, width: number, height: number) {
  await page.setViewportSize({ width, height });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto(`/Sky-Auto-Player${route}`);
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
  await page.addStyleTag({
    content: `
      *,
      *::before,
      *::after {
        animation: none !important;
        transition: none !important;
        caret-color: transparent !important;
      }
    `,
  });
}

const screenshotOptions = {
  animations: 'disabled' as const,
  caret: 'hide' as const,
  maxDiffPixelRatio: 0.05,
};

test.describe('visual regression', () => {
  test.describe.configure({ mode: 'default' });

  for (const visualCase of [
    { name: 'home-en-desktop-1440.png', route: '/', width: 1440, height: 900 },
    { name: 'home-en-mobile-390.png', route: '/', width: 390, height: 844 },
    { name: 'home-vi-mobile-390.png', route: '/vi/', width: 390, height: 844 },
    { name: 'home-en-tablet-768.png', route: '/', width: 768, height: 1024 },
  ] as const) {
    test(visualCase.name, async ({ page }) => {
      await prepare(page, visualCase.route, visualCase.width, visualCase.height);
      await expect(page).toHaveScreenshot(visualCase.name, {
        fullPage: true,
        ...screenshotOptions,
      });
    });
  }

  test('header and hero desktop', async ({ page }) => {
    await prepare(page, '/', 1440, 900);
    await expect(page.locator('.hero')).toHaveScreenshot('header-hero-1440.png', {
      ...screenshotOptions,
    });
  });

  test('product peak desktop', async ({ page }) => {
    await prepare(page, '/', 1440, 900);
    await page.addStyleTag({ content: '.site-header { position: relative !important; }' });
    await expect(page.locator('.product-showcase')).toHaveScreenshot('product-peak-1440.png', {
      ...screenshotOptions,
    });
  });

  for (const utilitySection of [
    { name: 'steps-desktop-1440.png', selector: '.steps-section' },
    { name: 'formats-desktop-1440.png', selector: '.formats-section' },
  ] as const) {
    test(`${utilitySection.selector} desktop alignment`, async ({ page }) => {
      await prepare(page, '/', 1440, 900);
      await page.addStyleTag({ content: '.site-header { position: relative !important; }' });
      await expect(page.locator(utilitySection.selector)).toHaveScreenshot(utilitySection.name, {
        ...screenshotOptions,
      });
    });
  }

  test('mobile menu open', async ({ page }) => {
    await prepare(page, '/', 390, 844);
    await page.locator('.menu-toggle').click();
    await expect(page).toHaveScreenshot('mobile-menu-open-390.png', {
      ...screenshotOptions,
      fullPage: false,
    });
  });

  test('technical trust mobile', async ({ page }) => {
    await prepare(page, '/', 390, 844);
    await expect(page.locator('.technical-section')).toHaveScreenshot(
      'technical-trust-390.png',
      screenshotOptions,
    );
  });

  test('FAQ desktop', async ({ page }) => {
    await prepare(page, '/faq/', 1440, 900);
    await expect(page.locator('main')).toHaveScreenshot('faq-en-desktop-1440.png', {
      ...screenshotOptions,
    });
  });
});
