import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

const routes = ['/', '/faq/', '/vi/', '/vi/faq/'];
const homepageSections = [
  'hero',
  'benefits',
  'product',
  'quick-start',
  'playback',
  'technical-model',
  'formats',
  'trust-download',
  'faq-preview',
  'closing-cta',
];
const releaseUrl = 'https://github.com/pumni/Sky-Auto-Player/releases';

test.describe('accessibility and responsive contracts', () => {
  for (const route of routes) {
    test(`${route} has no axe violations`, async ({ page }) => {
      await page.goto(`/Sky-Auto-Player${route}`);
      const accessibilityScanResults = await new AxeBuilder({ page }).analyze();
      expect(accessibilityScanResults.violations).toEqual([]);
    });
  }

  for (const locale of [
    { route: '/', heading: 'Play the sheet. Not the keyboard.', cta: 'Get the Windows installer' },
    { route: '/vi/', heading: 'Chơi bản nhạc, không chơi bàn phím.', cta: 'Tải bộ cài Windows' },
  ]) {
    test(`${locale.route} keeps the approved narrative, section order, and release journey`, async ({
      page,
    }) => {
      await page.goto(`/Sky-Auto-Player${locale.route}`);
      await expect(page.locator('main h1')).toHaveText(locale.heading);
      await expect(page.locator('[data-home-section]').first()).toHaveAttribute(
        'data-home-section',
        homepageSections[0],
      );
      await expect
        .poll(() =>
          page
            .locator('[data-home-section]')
            .evaluateAll((sections) =>
              sections.map((section) => section.getAttribute('data-home-section')),
            ),
        )
        .toEqual(homepageSections);
      await expect(page.locator('.hero__primary')).toContainText(locale.cta);
      await expect(page.locator('.hero__primary')).toContainText(/GitHub Releases/);
      await expect(page.locator('.hero__primary')).toHaveAttribute('href', releaseUrl);
      await expect(page.locator('.hero__secondary')).toHaveAttribute('href', /#product$/);
      await expect(page.locator('.hero__description')).not.toContainText(
        /QPC|SendInput|foreground/i,
      );
    });
  }

  for (const route of ['/', '/vi/']) {
    for (const width of [320, 390, 768, 1024, 1152, 1440, 1920]) {
      test(`${route} has no horizontal overflow at ${width}px`, async ({ page }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.goto(`/Sky-Auto-Player${route}`);
        const metrics = await page.evaluate(() => ({
          body: document.body.scrollWidth,
          document: document.documentElement.scrollWidth,
          viewport: window.innerWidth,
        }));
        expect(metrics.body).toBeLessThanOrEqual(metrics.viewport + 1);
        expect(metrics.document).toBeLessThanOrEqual(metrics.viewport + 1);
      });
    }
  }

  test('semantic section spacing roles hold at narrow, fluid, and wide widths', async ({
    page,
  }) => {
    const widths = [320, 1280, 1920];
    const expected = {
      compact: [32, 51.2, 64],
      default: [48, 76.8, 96],
      feature: [64, 89.6, 128],
      transition: [40, 51.2, 64],
      product: [80, 102.4, 144],
      introTop: [20, 32, 40],
    };

    await page.setViewportSize({ width: widths[0], height: 900 });
    await page.goto('/Sky-Auto-Player/');
    for (let index = 0; index < widths.length; index += 1) {
      const width = widths[index];
      await page.setViewportSize({ width, height: 900 });
      const spacing = await page.evaluate(() => {
        const measure = (selector: string) => {
          const element = document.querySelector<HTMLElement>(selector);
          if (!element) throw new Error(`Missing spacing probe ${selector}`);
          const style = getComputedStyle(element);
          return {
            top: Number.parseFloat(style.paddingTop),
            bottom: Number.parseFloat(style.paddingBottom),
          };
        };
        return {
          compact: measure('.formats-section'),
          feature: measure('.final-cta'),
          transition: measure('.benefits'),
          product: measure('.product-showcase'),
          intro: measure('.hero'),
        };
      });

      for (const role of ['compact', 'feature', 'transition', 'product'] as const) {
        expect(
          Math.abs(spacing[role].top - expected[role][index]),
          `${role} at ${width}px`,
        ).toBeLessThan(0.2);
        expect(spacing[role].bottom).toBe(spacing[role].top);
      }
      expect(
        Math.abs(spacing.intro.top - expected.introTop[index]),
        `intro at ${width}px`,
      ).toBeLessThan(0.2);
      expect(spacing.intro.bottom).toBe(spacing.feature.bottom);
    }

    await page.goto('/Sky-Auto-Player/faq/');
    for (let index = 0; index < widths.length; index += 1) {
      await page.setViewportSize({ width: widths[index], height: 900 });
      const defaultSpacing = await page
        .locator('.faq-layout')
        .evaluate((element) => Number.parseFloat(getComputedStyle(element).marginBottom));
      expect(Math.abs(defaultSpacing - expected.default[index])).toBeLessThan(0.2);
    }
  });

  test('short landscape keeps the hero and primary action reachable without overflow', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 844, height: 390 });
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('main h1')).toBeVisible();
    await expect(page.locator('.hero__primary')).toBeVisible();
    const dimensions = await page.evaluate(() => ({
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
      heroBottom: document.querySelector('.hero')!.getBoundingClientRect().bottom,
    }));
    expect(dimensions.documentWidth).toBeLessThanOrEqual(dimensions.viewportWidth + 1);
    expect(dimensions.heroBottom).toBeGreaterThan(0);
  });

  test('player view selector updates the selected capture, caption, alt text, and full-size link', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/Sky-Auto-Player/');
    const group = page.getByRole('group', { name: 'Choose a player view' });
    const image = page.locator('[data-product-image]');
    const imageLink = page.locator('[data-product-image-link]');
    await expect(group).toBeVisible();
    await expect(group.getByRole('button')).toHaveCount(3);
    await expect(group.getByRole('button', { name: 'Library' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(image).toHaveAttribute('src', /library-real-tauri\.png/);
    await expect(image).toHaveAttribute('alt', /song library and playlists/);
    await expect(imageLink).toHaveAttribute('href', /library-real-tauri\.png/);
    await expect(image).toHaveCSS('transition-duration', '0.2s');

    await group.getByRole('button', { name: 'Song Detail' }).click();
    await expect(group.getByRole('button', { name: 'Song Detail' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(group.getByRole('button', { name: 'Library' })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    await expect(image).toHaveAttribute('src', /detail-real-tauri\.png/);
    await expect(image).toHaveAttribute('alt', /selected song and its sheet details/);
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Review a song before you play it.',
    );
    await expect(imageLink).toHaveAttribute('href', /detail-real-tauri\.png/);
    await expect
      .poll(() => image.evaluate((element) => (element as HTMLImageElement).naturalWidth))
      .toBeGreaterThan(0);
    await expect
      .poll(() => image.evaluate((element) => element.hasAttribute('data-switching')))
      .toBe(false);

    const settings = group.getByRole('button', { name: 'Settings' });
    await settings.focus();
    await page.keyboard.press('Enter');
    await expect(settings).toHaveAttribute('aria-pressed', 'true');
    await expect(image).toHaveAttribute('src', /settings-real-tauri\.png/);
    await expect(imageLink).toHaveAttribute('href', /settings-real-tauri\.png/);
    await expect
      .poll(() => image.evaluate((element) => element.hasAttribute('data-switching')))
      .toBe(false);
    await expect(page.locator('.screenshot-frame__caption-kind')).toHaveText(
      'Desktop interface capture',
    );
  });

  test('Vietnamese player view selector has localized labels, descriptions, and captions', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/vi/');
    const group = page.getByRole('group', { name: 'Chọn giao diện trình phát' });
    await expect(group.getByRole('button', { name: 'Thư viện' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await group.getByRole('button', { name: 'Chi tiết bài hát' }).click();
    await expect(page.locator('[data-product-image]')).toHaveAttribute(
      'alt',
      /bài hát đã chọn và chi tiết sheet/,
    );
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Xem lại bài hát trước khi phát.',
    );
    await expect(page.locator('.screenshot-frame__caption-kind')).toHaveText(
      'Ảnh chụp giao diện desktop',
    );
  });

  test('the desktop capture remains contained and the site favicon contract is visible', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/Sky-Auto-Player/');
    const image = page.locator('[data-product-image]');
    await expect(image).toHaveJSProperty('complete', true);
    const metrics = await image.evaluate((element) => {
      const imageElement = element as HTMLImageElement;
      const box = imageElement.getBoundingClientRect();
      const frame = imageElement.closest('.screenshot-frame')!.getBoundingClientRect();
      return {
        naturalWidth: imageElement.naturalWidth,
        naturalHeight: imageElement.naturalHeight,
        insideFrame: box.left >= frame.left && box.right <= frame.right + 1,
      };
    });
    expect(metrics.naturalWidth).toBe(1214);
    expect(metrics.naturalHeight).toBe(798);
    expect(metrics.insideFrame).toBe(true);
    await expect(page.locator('link[rel="icon"][type="image/png"][sizes="16x16"]')).toHaveAttribute(
      'href',
      /favicon-16x16\.png/,
    );
    await expect(page.locator('link[rel="icon"][type="image/png"][sizes="32x32"]')).toHaveAttribute(
      'href',
      /favicon-32x32\.png/,
    );
  });

  test('playback events, timestamp, key state, duration, and completion stay synchronized', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const events = demo.locator('[data-demo-event]');
    const keys = demo.locator('[data-demo-key]');
    const progress = demo.locator('[data-demo-progress]');
    const elapsed = demo.locator('[data-elapsed]');
    const mainControl = demo.locator('[data-action="play"]');

    await expect(events).toHaveCount(3);
    await expect(keys).toHaveCount(15);
    await expect(events.nth(0)).toHaveAttribute('aria-current', 'step');
    await expect(demo.locator('.instrument-key.is-active')).toHaveCount(3);
    await expect(events.nth(2).locator('.event-row__duration')).toHaveText('1200 ms');
    await expect(progress).toHaveAttribute('aria-valuenow', '0');
    await mainControl.click();
    await expect(demo.locator('[data-action="cancel"]')).toBeVisible();

    await expect(events.nth(1)).toHaveAttribute('aria-current', 'step', { timeout: 1200 });
    await expect(events.nth(0)).not.toHaveAttribute('aria-current', 'step');
    await expect(demo.locator('.instrument-key.is-active')).toHaveCount(1);
    await expect(demo.locator('[data-demo-key="D2"]')).toHaveClass(/is-active/);
    const synchronizedTime = await demo.evaluate((element) => {
      const time = element.querySelector<HTMLOutputElement>('[data-elapsed]')?.textContent;
      const progressElement = element.querySelector<HTMLElement>('[data-demo-progress]');
      return {
        time,
        value: progressElement?.getAttribute('aria-valuenow'),
        valueText: progressElement?.getAttribute('aria-valuetext'),
      };
    });
    expect(Number(synchronizedTime.value)).toBeGreaterThanOrEqual(700);
    expect(Number(synchronizedTime.value)).toBeLessThan(1000);
    expect(synchronizedTime.time).toBe('00:00.' + String(synchronizedTime.value).padStart(3, '0'));
    expect(synchronizedTime.valueText).toBe(`${synchronizedTime.time} / 00:02.600`);

    await expect(demo.locator('[data-demo-status]')).toHaveText(
      'Demonstration complete. No keys are active.',
      { timeout: 3200 },
    );
    await expect(demo.locator('[data-demo-event][aria-current="step"]')).toHaveCount(0);
    await expect(demo.locator('.instrument-key.is-active')).toHaveCount(0);
    await expect(progress).toHaveAttribute('aria-valuenow', '2600');
    await expect(elapsed).toHaveText('00:02.600');
    await expect(mainControl).toHaveText('Replay demonstration');
  });

  test('manual event stepping is boundary-safe, cancels playback, and reaches the complete state', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const previous = demo.locator('[data-action="previous"]');
    const next = demo.locator('[data-action="next"]');
    const main = demo.locator('[data-action="play"]');
    const progress = demo.locator('[data-demo-progress]');

    await expect(previous).toBeDisabled();
    await next.click();
    await expect(demo.locator('[data-demo-event="1"]')).toHaveAttribute('aria-current', 'step');
    await expect(progress).toHaveAttribute('aria-valuenow', '700');
    await previous.click();
    await expect(demo.locator('[data-demo-event="0"]')).toHaveAttribute('aria-current', 'step');
    await expect(progress).toHaveAttribute('aria-valuenow', '0');
    await main.click();
    await expect(demo.locator('[data-action="cancel"]')).toBeVisible();
    await next.click();
    await expect(demo.locator('[data-action="cancel"]')).toBeHidden();
    await expect(demo.locator('[data-demo-event="1"]')).toHaveAttribute('aria-current', 'step');
    await next.click();
    await expect(demo.locator('[data-demo-event="2"]')).toHaveAttribute('aria-current', 'step');
    await expect(progress).toHaveAttribute('aria-valuenow', '1400');
    await next.click();
    await expect(progress).toHaveAttribute('aria-valuenow', '2600');
    await expect(demo.locator('.instrument-key.is-active')).toHaveCount(0);
    await expect(next).toBeDisabled();
    await previous.click();
    await expect(demo.locator('[data-demo-event="2"]')).toHaveAttribute('aria-current', 'step');
    await expect(demo.locator('.instrument-key.is-active')).toHaveCount(1);
    await expect(previous).toBeEnabled();
  });

  test('pause, cancel, resume, and replay preserve one playback timeline', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const main = demo.locator('[data-action="play"]');
    const progress = demo.locator('[data-demo-progress]');

    await main.click();
    await expect
      .poll(async () => Number(await progress.getAttribute('aria-valuenow')))
      .toBeGreaterThan(0);
    await demo.locator('[data-action="cancel"]').click();
    const pausedAt = await progress.getAttribute('aria-valuenow');
    await expect(demo.locator('[data-action="cancel"]')).toBeHidden();
    await expect(main).toHaveText('Continue demonstration');
    await page.waitForTimeout(100);
    await expect(progress).toHaveAttribute('aria-valuenow', pausedAt!);
    await main.click();
    await expect(main).toHaveText('Replay demonstration', { timeout: 3500 });
    await expect(progress).toHaveAttribute('aria-valuenow', '2600');
    await main.click();
    await expect(main).toHaveText('Pause demonstration');
    await expect
      .poll(async () => Number(await progress.getAttribute('aria-valuenow')))
      .toBeLessThan(400);
    await expect(demo.locator('[data-action="cancel"]')).toBeVisible();
    await main.click();
    const replayPausedAt = await progress.getAttribute('aria-valuenow');
    await expect(main).toHaveText('Continue demonstration');
    await page.waitForTimeout(100);
    await expect(progress).toHaveAttribute('aria-valuenow', replayPausedAt!);
  });

  test('reduced motion removes animated playback but preserves manual stepping', async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    await expect(demo.locator('[data-action="play"]')).toBeHidden();
    await expect(demo.locator('[data-motion-note]')).toBeVisible();
    await demo.locator('[data-action="next"]').click();
    await expect(demo.locator('[data-demo-event="1"]')).toHaveAttribute('aria-current', 'step');
    await expect(demo.locator('[data-demo-progress]')).toHaveAttribute('aria-valuenow', '700');
    await expect(demo.locator('[data-action="previous"]')).toBeEnabled();
  });

  test('switching to reduced motion and hiding the page settles playback without auto-resume', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const main = demo.locator('[data-action="play"]');
    const progress = demo.locator('[data-demo-progress]');
    await main.click();
    await expect(demo.locator('[data-action="cancel"]')).toBeVisible();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(main).toBeHidden();
    await expect(demo.locator('[data-action="cancel"]')).toBeHidden();
    await expect(demo.locator('[data-action="next"]')).toBeFocused();
    const reducedAt = await progress.getAttribute('aria-valuenow');
    await page.waitForTimeout(100);
    await expect(progress).toHaveAttribute('aria-valuenow', reducedAt!);

    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await expect(main).toBeVisible();
    await main.click();
    await expect(demo.locator('[data-action="cancel"]')).toBeVisible();
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    const hiddenAt = await progress.getAttribute('aria-valuenow');
    await page.waitForTimeout(100);
    await expect(progress).toHaveAttribute('aria-valuenow', hiddenAt!);
    await expect(main).toHaveText('Continue demonstration');
  });

  test('static fallback keeps the product capture, event explanation, and navigation usable without JavaScript', async ({
    browser,
  }) => {
    const context = await browser.newContext({
      javaScriptEnabled: false,
      viewport: { width: 390, height: 844 },
    });
    const page = await context.newPage();
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('main h1')).toBeVisible();
    await expect(page.locator('.site-nav')).toBeVisible();
    await expect(page.locator('.header-download')).toBeVisible();
    await expect(page.locator('.menu-toggle')).toBeHidden();
    await expect(page.locator('.site-nav')).toHaveCSS('position', 'static');
    await expect(page.locator('.product-selector')).toBeHidden();
    await expect(page.locator('.product-view__fallback a')).toHaveCount(3);
    await expect(page.locator('[data-product-image]')).toHaveAttribute(
      'src',
      /library-real-tauri\.png/,
    );
    await expect(page.locator('[data-demo-controls]')).toBeHidden();
    await expect(page.locator('[data-demo-event]')).toHaveCount(3);
    await expect(page.locator('[data-demo-event="0"]')).toHaveAttribute('aria-current', 'step');
    await expect(page.locator('[data-demo-status]')).toContainText('Static example');
    await expect(page.locator('.instrument-key')).toHaveCount(15);
    await context.close();
  });

  test('header sticky behavior begins with desktop navigation', async ({ page }) => {
    await page.setViewportSize({ width: 1023, height: 900 });
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('.site-header')).toHaveCSS('position', 'relative');
    await expect(page.locator('.menu-toggle')).toBeVisible();

    await page.setViewportSize({ width: 1024, height: 900 });
    await expect(page.locator('.site-header')).toHaveCSS('position', 'sticky');
    await expect(page.locator('.menu-toggle')).toBeHidden();
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

  test('visible microcopy keeps a readable 12px floor', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/Sky-Auto-Player/');
    const sizes = await page
      .locator(
        [
          '.hero__kicker',
          '.hero__destination',
          '.screenshot-frame figcaption',
          '.event-row__start',
          '.release-details dt',
          '.final-cta__eyebrow',
        ].join(','),
      )
      .evaluateAll((elements) =>
        elements
          .filter((element) => {
            const style = getComputedStyle(element);
            return style.display !== 'none' && style.visibility !== 'hidden';
          })
          .map((element) => Number.parseFloat(getComputedStyle(element).fontSize)),
      );
    expect(sizes.length).toBeGreaterThan(0);
    expect(Math.min(...sizes)).toBeGreaterThanOrEqual(12);
  });

  test('keyboard focus stays visible and Vietnamese display text keeps its diacritics', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/vi/');
    await expect(page.locator('main h1')).toHaveText('Chơi bản nhạc, không chơi bàn phím.');
    const primaryAction = page.locator('.hero__actions .button').first();
    await primaryAction.focus();
    await expect(primaryAction).toBeFocused();
    const focusStyle = await primaryAction.evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        outlineStyle: style.outlineStyle,
        outlineWidth: Number.parseFloat(style.outlineWidth),
      };
    });
    expect(focusStyle.outlineStyle).not.toBe('none');
    expect(focusStyle.outlineWidth).toBeGreaterThanOrEqual(2);
  });

  test('download and trust section preserves the current install and safety details', async ({
    page,
  }) => {
    for (const route of ['/', '/vi/']) {
      await page.goto(`/Sky-Auto-Player${route}`);
      await expect(page.locator('#technical .trust-download__button')).toHaveAttribute(
        'href',
        releaseUrl,
      );
      await expect(page.locator('#technical')).toContainText(
        route === '/' ? 'Windows 11 x64' : 'Windows 11 64-bit',
      );
      await expect(page.locator('#technical')).toContainText('NSIS');
      await expect(page.locator('#technical')).toContainText('.exe.sig');
      await expect(page.locator('#technical a[href*="account-safety"]')).toBeVisible();
      await expect(page.locator('#technical')).not.toContainText('100% safe');
    }
    await expect(page.locator('.final-cta__actions a')).toHaveCount(2);
    await expect(page.locator('.site-footer__brand img')).toHaveAttribute('src', /favicon\.svg/);
  });

  test('published routes do not emit console or page errors', async ({ page }) => {
    const errors: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text());
    });
    page.on('pageerror', (error) => errors.push(error.message));
    for (const route of ['/', '/faq/', '/vi/', '/vi/faq/']) {
      await page.goto(`/Sky-Auto-Player${route}`);
    }
    expect(errors).toEqual([]);
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

  for (const guideRoute of [
    '/guides/how-it-works/',
    '/guides/security-boundaries/',
    '/vi/guides/how-it-works/',
    '/vi/guides/troubleshooting/',
  ]) {
    test(`${guideRoute} has no axe violations`, async ({ page }) => {
      await page.goto(`/Sky-Auto-Player${guideRoute}`);
      const results = await new AxeBuilder({ page }).exclude('pre.astro-code').analyze();
      expect(results.violations).toEqual([]);
    });
  }

  test('guide page has Article JSON-LD, breadcrumb and evidence section', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/guides/how-it-works/');
    const scripts = await page.locator('script[type="application/ld+json"]').allTextContents();
    type SchemaObj = Record<string, unknown>;
    const schemas: SchemaObj[] = scripts.flatMap((script) => {
      const parsed = JSON.parse(script) as SchemaObj | SchemaObj[];
      return Array.isArray(parsed) ? parsed : [parsed];
    });
    const article = schemas.find((schema) => schema['@type'] === 'Article');
    expect(article).toBeDefined();
    expect(article?.headline).toBeTruthy();
    const breadcrumb = schemas.find((schema) => schema['@type'] === 'BreadcrumbList');
    expect(breadcrumb).toBeDefined();
    expect((breadcrumb?.itemListElement as unknown[])?.length).toBe(3);
    await expect(page.locator('article h1')).toBeVisible();
    await expect(page.locator('.guide-page__evidence')).toBeVisible();
  });
});
