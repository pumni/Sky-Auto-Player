import { test, expect } from '@playwright/test';

const routes = ['/', '/faq/', '/vi/', '/vi/faq/'];

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

  test('locale switching preserves the current page', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/faq/');
    await page.locator('.locale-switch__option').filter({ hasText: 'VI' }).click();
    await expect(page).toHaveURL(/\/Sky-Auto-Player\/vi\/faq\/$/);

    await page.locator('.locale-switch__option').filter({ hasText: 'EN' }).click();
    await expect(page).toHaveURL(/\/Sky-Auto-Player\/faq\/$/);
  });

  test('mobile menu supports toggle, outside click, Escape and link activation', async ({
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
    await expect(nav).toHaveClass(/is-open/);
    await expect(nav.locator('a').first()).toBeFocused();

    await page.keyboard.press('Escape');
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(nav).not.toHaveClass(/is-open/);

    await toggle.click();
    await page.mouse.click(3, 3);
    await expect(nav).not.toHaveClass(/is-open/);

    await toggle.click();
    await nav.locator('a[href*="how-it-works"]').click();
    await expect(nav).not.toHaveClass(/is-open/);
  });
});
