import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

const routes = ['/', '/faq/', '/vi/', '/vi/faq/'];

test.describe('accessibility and responsive contracts', () => {
  for (const route of routes) {
    test(`${route} has no axe violations`, async ({ page }) => {
      await page.goto(`/Sky-Auto-Player${route}`);
      const accessibilityScanResults = await new AxeBuilder({ page }).analyze();
      expect(accessibilityScanResults.violations).toEqual([]);
    });
  }

  for (const width of [320, 360, 390, 768, 912, 1024, 1280, 1440, 1920]) {
    test(`homepage has no horizontal overflow at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/Sky-Auto-Player/');
      const metrics = await page.evaluate(() => ({
        body: document.body.scrollWidth,
        document: document.documentElement.scrollWidth,
        viewport: window.innerWidth,
      }));
      expect(metrics.body).toBeLessThanOrEqual(metrics.viewport + 1);
      expect(metrics.document).toBeLessThanOrEqual(metrics.viewport + 1);
    });
  }

  test('mobile CTA, screenshot crop and project favicon follow the UI contract', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 360, height: 900 });
    await page.goto('/Sky-Auto-Player/');

    const heroActions = page.locator('.hero__actions');
    for (const button of await heroActions.locator('.button').all()) {
      expect(await button.evaluate((element) => element.getBoundingClientRect().width)).toBe(
        await heroActions.evaluate((element) => element.getBoundingClientRect().width),
      );
    }
    await expect(page.locator('.header-download')).toBeHidden();
    await expect(page.locator('picture source[media="(max-width: 40rem)"]')).toHaveAttribute(
      'srcset',
      /picker-mobile\.webp/,
    );
    await expect(page.locator('link[rel="icon"][type="image/svg+xml"]')).toHaveAttribute(
      'href',
      /sky-auto-player-mark\.svg/,
    );
  });

  test('homepage remains usable at 200 percent zoom', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto('/Sky-Auto-Player/');
    await page.evaluate(() => {
      document.documentElement.style.zoom = '2';
    });
    await expect(page.locator('main h1')).toBeVisible();
    await expect(page.locator('.hero__actions .button').first()).toBeVisible();
  });
  test('FAQ pages expose FAQPage JSON-LD and stable fonts', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/faq/');
    const jsonLd = JSON.parse(
      (await page.locator('script[type="application/ld+json"]').textContent()) || '{}',
    );
    expect(jsonLd['@type']).toBe('FAQPage');
    expect(jsonLd.mainEntity.length).toBeGreaterThan(0);
    await expect(page.locator('body')).toHaveCSS('font-family', /Inter Variable/);
  });
});
