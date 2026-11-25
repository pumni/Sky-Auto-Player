import { test, expect } from '@playwright/test';

const routes = [
  '/',
  '/faq/',
  '/vi/',
  '/vi/faq/',
  '/guides/',
  '/vi/guides/',
  '/guides/how-it-works/',
  '/guides/sheet-formats/',
  '/guides/windows-setup/',
  '/guides/timing-engine/',
  '/guides/security-boundaries/',
  '/guides/troubleshooting/',
  '/vi/guides/how-it-works/',
  '/vi/guides/sheet-formats/',
  '/vi/guides/windows-setup/',
  '/vi/guides/timing-engine/',
  '/vi/guides/security-boundaries/',
  '/vi/guides/troubleshooting/',
];

const guideRoutes = [
  { en: '/guides/how-it-works/', vi: '/vi/guides/how-it-works/' },
  { en: '/guides/security-boundaries/', vi: '/vi/guides/security-boundaries/' },
];

test.describe('navigation and route contracts', () => {
  for (const route of routes) {
    test(`${route} renders with stable canonical metadata`, async ({ page }) => {
      const response = await page.goto(`/Sky-Auto-Player${route}`);
      expect(response?.status()).toBe(200);
      await expect(page.locator('main h1')).toBeVisible();

      const canonical = await page.locator('link[rel="canonical"]').getAttribute('href');
      expect(canonical).toBe(`https://pumni.github.io/Sky-Auto-Player${route}`);

      const alternates = await page
        .locator('link[rel="alternate"]')
        .evaluateAll((links) => links.map((link) => link.getAttribute('href')));
      expect(alternates.every((href) => href && !href.includes('//Sky-Auto-Player/'))).toBe(true);
      expect(alternates).toContain(
        `https://pumni.github.io/Sky-Auto-Player${route.startsWith('/vi') ? route.slice(3) || '/' : `/vi${route}`}`,
      );
    });
  }

  for (const pair of guideRoutes) {
    for (const [locale, route] of Object.entries(pair)) {
      const lang = locale as 'en' | 'vi';
      test(`${route} guide page has breadcrumb and H1`, async ({ page }) => {
        const response = await page.goto(`/Sky-Auto-Player${route}`);
        expect(response?.status()).toBe(200);
        await expect(page.locator('article h1')).toBeVisible();
        // Breadcrumb must have 3 items: Home / Guides / [title]
        const breadcrumb = page.locator('.breadcrumb__list li:not(.breadcrumb__item--separator)');
        await expect(breadcrumb).toHaveCount(3);
        // Canonical must match route
        const canonical = await page.locator('link[rel="canonical"]').getAttribute('href');
        expect(canonical).toBe(`https://pumni.github.io/Sky-Auto-Player${route}`);
        // hreflang must include the opposite locale
        const alternates = await page
          .locator('link[rel="alternate"]')
          .evaluateAll((links) => links.map((link) => link.getAttribute('href')));
        const opposite = lang === 'en' ? pair.vi : pair.en;
        expect(alternates).toContain(`https://pumni.github.io/Sky-Auto-Player${opposite}`);
      });
    }
  }

  test('locale switching preserves the current page (FAQ)', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/faq/');
    await page.locator('.locale-switch__option').filter({ hasText: 'VI' }).click();
    await expect(page).toHaveURL(/\/Sky-Auto-Player\/vi\/faq\/$/);

    await page.locator('.locale-switch__option').filter({ hasText: 'EN' }).click();
    await expect(page).toHaveURL(/\/Sky-Auto-Player\/faq\/$/);
  });

  test('mobile menu supports toggle, outside click, Escape, natural tab order and link activation', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/Sky-Auto-Player/');
    const skipLink = page.locator('.skip-link');
    await expect(skipLink).toHaveCSS('opacity', '0');
    await skipLink.focus();
    await expect(skipLink).toBeFocused();
    await expect(skipLink).toHaveCSS('opacity', '1');

    const toggle = page.locator('.menu-toggle');
    const nav = page.locator('.site-nav');

    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(nav).not.toBeVisible();

    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    await expect(nav).toHaveAttribute('data-state', 'open');
    await expect(nav.locator('a').first()).toBeFocused();
    await page.keyboard.press('Shift+Tab');
    await expect(page.locator('.brand')).toBeFocused();
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');

    await page.keyboard.press('Escape');
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(nav).toHaveAttribute('aria-hidden', 'true');
    await expect(nav).toHaveAttribute('data-state', 'closed', { timeout: 1000 });

    await toggle.click();
    await page.mouse.click(3, 3);
    await expect(nav).toHaveAttribute('aria-hidden', 'true');
    await expect(nav).toHaveAttribute('data-state', 'closed', { timeout: 1000 });

    await toggle.click();
    await nav.locator('a[href*="guides"]').first().click();
    await expect(nav).toHaveAttribute('aria-hidden', 'true');
    await expect(nav).toHaveAttribute('data-state', 'closed', { timeout: 1000 });
  });

  test('mobile and tablet headers keep the release action visible outside the menu', async ({
    page,
  }) => {
    for (const width of [390, 768]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/Sky-Auto-Player/');
      const releases = page.locator('.header-download');
      await expect(releases).toBeVisible();
      await expect(releases).toHaveAttribute(
        'href',
        'https://github.com/pumni/Sky-Auto-Player/releases',
      );
      await expect(releases).toHaveAccessibleName(/GitHub Releases/);
      await expect(page.locator('.menu-toggle')).toBeVisible();
    }
  });

  test('locale switching preserves the guide article route in both languages', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/guides/how-it-works/');
    await page.locator('.locale-switch__option[lang="vi"]').click();
    await expect
      .poll(() => new URL(page.url()).pathname)
      .toBe('/Sky-Auto-Player/vi/guides/how-it-works/');

    await page.locator('.locale-switch__option[lang="en"]').click();
    await expect
      .poll(() => new URL(page.url()).pathname)
      .toBe('/Sky-Auto-Player/guides/how-it-works/');
  });
});
