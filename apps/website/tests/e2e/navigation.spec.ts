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

  for (const locale of ['/', '/vi/']) {
    test(`${locale} desktop hash navigation stays accessible before route navigation`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 1280, height: 900 });
      await page.goto(`/Sky-Auto-Player${locale}`);
      const nav = page.locator('.site-nav');

      await nav.locator('a[href*="#how-it-works"]').click();
      await expect(page).toHaveURL(/#how-it-works$/);
      await expect(page.locator('#how-it-works')).toBeFocused();
      await expect(nav).toBeVisible();
      await expect(nav).not.toHaveAttribute('aria-hidden', 'true');
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(false);

      await nav.locator('a[href$="/guides/"]').click();
      await expect(page).toHaveURL(/\/guides\/$/);
      await expect(page.locator('.site-nav')).toBeVisible();
      expect(
        await page.locator('.site-nav').evaluate((element) => (element as HTMLElement).inert),
      ).toBe(false);
    });
  }

  for (const locale of ['/', '/vi/']) {
    test(`${locale} mobile hash navigation leaves focus on the destination`, async ({ page }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(`/Sky-Auto-Player${locale}`);
      const toggle = page.locator('.menu-toggle');
      const nav = page.locator('.site-nav');

      await toggle.click();
      await expect(toggle).toBeFocused();
      await nav.locator('a[href*="#how-it-works"]').click();

      await expect(page).toHaveURL(/#how-it-works$/);
      await expect(page.locator('#how-it-works')).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      await expect(nav).toHaveAttribute('aria-hidden', 'true');
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(true);
      await expect(toggle).not.toBeFocused();
    });
  }

  for (const locale of ['/', '/vi/']) {
    test(`${locale} mobile disclosure keeps natural keyboard traversal and Escape restores the toggle`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(`/Sky-Auto-Player${locale}`);
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
      await expect(nav).toBeVisible();
      await expect(toggle).toBeFocused();
      await page.keyboard.press('Tab');
      await expect(nav.locator('a').first()).toBeFocused();
      await page.keyboard.press('Tab');
      await expect(nav.locator('a').nth(1)).toBeFocused();
      await page.keyboard.press('Shift+Tab');
      await expect(nav.locator('a').first()).toBeFocused();
      await page.keyboard.press('Shift+Tab');
      await expect(toggle).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      await page.keyboard.press('Shift+Tab');
      await expect(page.locator('.brand')).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');

      await toggle.click();
      await page.keyboard.press('Escape');
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      await expect(nav).toHaveAttribute('aria-hidden', 'true');
      await expect(nav).not.toBeVisible();
      await expect(toggle).toBeFocused();
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(true);
    });
  }

  for (const locale of ['/', '/vi/']) {
    test(`${locale} mobile outside pointer preserves focusable targets and restores focus for nonfocusable targets`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(`/Sky-Auto-Player${locale}`);
      const toggle = page.locator('.menu-toggle');
      const nav = page.locator('.site-nav');

      await toggle.click();
      await page.locator('.home-language').evaluate((element) => {
        element.addEventListener('click', (event) => event.preventDefault(), { once: true });
      });
      await page.locator('.home-language').click();
      await expect(nav).toHaveAttribute('aria-hidden', 'true');
      await expect(page.locator('.home-language')).toBeFocused();
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(true);

      await toggle.click();
      const outsideTarget = page.locator('.site-footer__identity p');
      await outsideTarget.scrollIntoViewIfNeeded();
      const outsideBox = await outsideTarget.boundingBox();
      expect(outsideBox).not.toBeNull();
      await page.mouse.move(
        outsideBox!.x + outsideBox!.width / 2,
        outsideBox!.y + outsideBox!.height / 2,
      );
      await page.mouse.down();
      await page.mouse.up();
      await expect(nav).toHaveAttribute('aria-hidden', 'true');
      await expect(toggle).toBeFocused();
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(true);
    });
  }

  for (const locale of ['/', '/vi/']) {
    test(`${locale} modified navigation clicks keep the disclosure open`, async ({ page }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(`/Sky-Auto-Player${locale}`);
      const toggle = page.locator('.menu-toggle');
      const nav = page.locator('.site-nav');
      await toggle.click();

      const modifiedClick = await page.evaluate(() => {
        const link = document.querySelector<HTMLAnchorElement>('.site-nav a[href*="github.com"]');
        if (!link) throw new Error('GitHub navigation link was not found');
        document.addEventListener('click', (event) => event.preventDefault(), { once: true });
        const event = new MouseEvent('click', {
          bubbles: true,
          cancelable: true,
          button: 0,
          ctrlKey: true,
        });
        const uncanceled = link.dispatchEvent(event);
        return { ctrlKey: event.ctrlKey, defaultPrevented: event.defaultPrevented, uncanceled };
      });
      expect(modifiedClick).toEqual({ ctrlKey: true, defaultPrevented: true, uncanceled: false });
      await expect(page).toHaveURL(new RegExp(`/Sky-Auto-Player${locale.replaceAll('/', '\\/')}$`));
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(false);
    });
  }

  for (const locale of ['/', '/vi/']) {
    test(`${locale} mobile rapid toggles, reduced motion changes and history restoration remain coherent`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(`/Sky-Auto-Player${locale}`);
      const toggle = page.locator('.menu-toggle');
      const nav = page.locator('.site-nav');

      await toggle.click();
      await expect(toggle).toBeFocused();
      await page.emulateMedia({ reducedMotion: 'no-preference' });
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      await expect(nav).toBeVisible();
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(false);
      await toggle.click();
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      await expect(nav).toBeVisible();
      await expect(toggle).toBeFocused();
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(false);

      await nav.locator('a[href$="/guides/"]').click();
      await expect(page).toHaveURL(/\/guides\/$/);
      await page.goBack();
      await expect(page).toHaveURL(new RegExp(`/Sky-Auto-Player${locale.replaceAll('/', '\\/')}$`));
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      await expect(nav).toHaveAttribute('aria-hidden', 'true');
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(true);
      await page.goForward();
      await expect(page).toHaveURL(/\/guides\/$/);
      await expect(page.locator('.menu-toggle')).toHaveAttribute('aria-expanded', 'false');
      expect(
        await page.locator('.site-nav').evaluate((element) => (element as HTMLElement).inert),
      ).toBe(true);
    });
  }

  for (const locale of ['/', '/vi/']) {
    test(`${locale} resize transfers focus to visible navigation and preserves a focused link`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(`/Sky-Auto-Player${locale}`);
      const toggle = page.locator('.menu-toggle');
      const nav = page.locator('.site-nav');

      await page.emulateMedia({ reducedMotion: 'reduce' });
      await toggle.focus();
      await page.setViewportSize({ width: 1280, height: 900 });
      await expect(toggle).not.toBeVisible();
      await expect(nav.locator('a[href]').first()).toBeFocused();
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(false);
      await expect(nav).not.toHaveAttribute('aria-hidden', 'true');

      const focusedLink = nav.locator('a[href]').nth(1);
      await focusedLink.focus();
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(toggle).toBeVisible();
      await expect(focusedLink).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(false);

      await page.setViewportSize({ width: 1280, height: 900 });
      await expect(focusedLink).toBeFocused();
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(false);
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(focusedLink).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');

      await page.emulateMedia({ reducedMotion: 'no-preference' });
      // Cross the breakpoint repeatedly without waiting for transitions between resizes.
      for (const width of [1280, 390, 1280, 390, 1280, 390]) {
        await page.setViewportSize({ width, height: 844 });
      }
      await expect(focusedLink).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');

      const unrelated = page.locator('.hero__primary');
      await unrelated.focus();
      await page.setViewportSize({ width: 1280, height: 900 });
      await expect(toggle).toBeHidden();
      await expect(unrelated).toBeFocused();
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(toggle).toBeVisible();
      await expect(unrelated).toBeFocused();
      await expect(toggle).toHaveAttribute('aria-expanded', 'false');
      expect(await nav.evaluate((element) => (element as HTMLElement).inert)).toBe(true);
    });
  }

  test('Home language switch stays outside the menu and documentation retains the release action', async ({ page }) => {
    for (const width of [320, 390, 768]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/Sky-Auto-Player/');
      await expect(page.locator('.header-download')).toHaveCount(0);
      await expect(page.locator('.home-language')).toBeVisible();
      await expect(page.locator('.home-language')).toHaveAttribute('href', '/Sky-Auto-Player/vi/');
      await expect(page.locator('.menu-toggle')).toBeVisible();
      await page.locator('.home-language').click();
      await expect(page).toHaveURL(/\/vi\/$/);
      await expect(page.locator('.home-language')).toHaveAttribute('href', '/Sky-Auto-Player/');
      await page.locator('.home-language').click();
      await expect(page).toHaveURL(/\/Sky-Auto-Player\/$/);
      await page.goto('/Sky-Auto-Player/faq/');
      await expect(page.locator('.header-download')).toBeVisible();
      await expect(page.locator('.header-download')).toHaveAttribute('href', 'https://github.com/pumni/Sky-Auto-Player/releases/latest');
      await expect(page.locator('.header-download')).toHaveAccessibleName(/GitHub Releases/);
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
