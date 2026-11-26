import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

const routes = ['/', '/faq/', '/vi/', '/vi/faq/'];
const homepageSections = ['hero', 'product', 'playback', 'quick-start', 'trust-download'];
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
    { route: '/', heading: 'Play the sheet. Not the keyboard.', cta: 'Download for Windows' },
    { route: '/vi/', heading: 'Chơi bản nhạc, không chơi bàn phím.', cta: 'Tải cho Windows' },
  ]) {
    test(`${locale.route} keeps the four-chapter narrative, anchors, and release journey`, async ({
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
      await expect(page.locator('.hero__primary')).toHaveAttribute('href', releaseUrl);
      await expect(page.locator('.hero__secondary')).toHaveAttribute('href', '#product');
      await expect(page.locator('.hero__description')).not.toContainText(
        /QPC|SendInput|foreground/i,
      );
      await expect(page.locator('.hero__meta')).toContainText('GitHub Releases');
      await expect(page.locator('.hero__risk')).toContainText(
        /Terms of Service|Điều khoản Dịch vụ/,
      );
      for (const anchor of [
        'top',
        'product',
        'playback',
        'how-it-works',
        'formats',
        'technical',
        'timing-model',
        'faq-preview',
      ]) {
        await expect(page.locator(`#${anchor}`)).toBeVisible();
      }
    });
  }

  test('public homepage anchors clear the sticky header at desktop widths', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto('/Sky-Auto-Player/');
    await page.addStyleTag({ content: 'html { scroll-behavior: auto !important; }' });

    for (const anchor of [
      'top',
      'product',
      'playback',
      'how-it-works',
      'formats',
      'technical',
      'timing-model',
      'faq-preview',
    ]) {
      await page.evaluate((id) => {
        document.getElementById(id)?.scrollIntoView({ block: 'start' });
      }, anchor);
      const offset = await page.evaluate(
        (id) => ({
          target: document.getElementById(id)!.getBoundingClientRect().top,
          header: document.querySelector('.site-header')!.getBoundingClientRect().bottom,
        }),
        anchor,
      );
      expect(offset.target, anchor).toBeGreaterThanOrEqual(offset.header - 1);
    }
  });

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

    test(`${route} keeps the 320px brand and release action visually separate`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 320, height: 844 });
      await page.goto(`/Sky-Auto-Player${route}`);
      const rightEdge = await page.evaluate(() => {
        const brandText = document.querySelector('.brand > span:last-child')!;
        const releaseAction = document.querySelector('.site-actions')!;
        return {
          brandText: brandText.getBoundingClientRect().right,
          releaseAction: releaseAction.getBoundingClientRect().left,
        };
      });
      expect(rightEdge.brandText).toBeLessThanOrEqual(rightEdge.releaseAction);
    });
  }

  test('homepage container, chapter rhythm, and shared FAQ spacing hold at responsive widths', async ({
    page,
  }) => {
    const widths = [320, 1280, 1920];
    const expected = {
      default: [48, 76.8, 96],
      chapter: [32, 64, 80],
      openingTop: [24, 38.4, 40],
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
          const box = element.getBoundingClientRect();
          return {
            top: Number.parseFloat(style.paddingTop),
            bottom: Number.parseFloat(style.paddingBottom),
            marginTop: Number.parseFloat(style.marginTop),
            width: box.width,
          };
        };
        return {
          chapter: measure('.home-chapter'),
          opening: measure('.home-opening'),
          container: measure('.home-opening'),
          footer: measure('.site-footer__layout'),
          viewport: document.documentElement.clientWidth,
        };
      });

      expect(Math.abs(spacing.chapter.marginTop - expected.chapter[index])).toBeLessThan(0.2);
      expect(spacing.chapter.top).toBe(0);
      expect(spacing.chapter.bottom).toBe(0);
      expect(Math.abs(spacing.opening.top - expected.openingTop[index])).toBeLessThan(0.2);
      expect(spacing.container.width).toBeLessThanOrEqual(1200.5);
      const leftGutter = (spacing.viewport - spacing.container.width) / 2;
      expect(leftGutter).toBeGreaterThanOrEqual(16);
      expect(Math.abs(leftGutter * 2 + spacing.container.width - spacing.viewport)).toBeLessThan(1);
      expect(Math.abs(spacing.footer.width - spacing.container.width)).toBeLessThan(1);
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

  test('opening composition puts product first at desktop and keeps the mobile action ahead of the screenshot', async ({
    page,
  }) => {
    for (const route of ['/', '/vi/']) {
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.goto(`/Sky-Auto-Player${route}`);
      const desktop = await page.evaluate(() => {
        const hero = document.querySelector('.hero')!.getBoundingClientRect();
        const image = document.querySelector('.product-showcase__image')!.getBoundingClientRect();
        const action = document.querySelector('.hero__primary')!.getBoundingClientRect();
        return {
          sideBySide: image.left > hero.left && image.left >= hero.right,
          imageBottom: image.bottom,
          actionBottom: action.bottom,
          screenshotWidth: image.width,
        };
      });
      expect(desktop.sideBySide).toBe(true);
      expect(desktop.screenshotWidth).toBeGreaterThan(600);
      expect(desktop.imageBottom).toBeLessThan(900);
      expect(desktop.actionBottom).toBeLessThan(900);

      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(`/Sky-Auto-Player${route}`);
      await expect(page.locator('main h1')).toBeVisible();
      await expect(page.locator('.hero__primary')).toBeInViewport();
      const mobile = await page.evaluate(() => {
        const hero = document.querySelector('.hero')!.getBoundingClientRect();
        const image = document.querySelector('.product-showcase__image')!.getBoundingClientRect();
        return { stacked: image.top >= hero.bottom, imageRight: image.right };
      });
      expect(mobile.stacked).toBe(true);
      expect(mobile.imageRight).toBeLessThanOrEqual(391);
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

  test('player view selector commits only the latest loaded preview and keeps links in sync', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.route('**/desktop-song-detail-browser-preview-1366x768.png', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 300));
      await route.continue();
    });
    await page.goto('/Sky-Auto-Player/');
    const group = page.getByRole('group', { name: 'Choose a player view' });
    const image = page.locator('[data-product-image]');
    const fullSize = page.locator('[data-product-full-size]');
    await expect(group).toBeVisible();
    await expect(group.getByRole('button')).toHaveCount(3);
    await expect(group.getByRole('button', { name: 'Library' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(image).toHaveAttribute('src', /desktop-library-browser-preview-1366x768\.png/);
    await expect(image).toHaveAttribute('alt', /song library/);
    await expect(fullSize).toHaveAttribute('href', /desktop-library-browser-preview-1366x768\.png/);

    await group.getByRole('button', { name: 'Song Detail' }).click();
    const settings = group.getByRole('button', { name: 'Settings' });
    await settings.click();
    await expect(settings).toHaveAttribute('aria-pressed', 'true');
    await expect(image).toHaveAttribute(
      'src',
      /desktop-settings-playback-browser-preview-1366x768\.png/,
    );
    await page.waitForTimeout(350);
    await expect(image).toHaveAttribute(
      'src',
      /desktop-settings-playback-browser-preview-1366x768\.png/,
    );

    await group.getByRole('button', { name: 'Library' }).click();
    await expect(group.getByRole('button', { name: 'Library' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await page.waitForTimeout(350);
    await expect(image).toHaveAttribute('src', /desktop-library-browser-preview-1366x768\.png/);
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Browse songs, favorites, and playlists.',
    );

    await group.getByRole('button', { name: 'Song Detail' }).click();
    await expect(group.getByRole('button', { name: 'Song Detail' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(group.getByRole('button', { name: 'Library' })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    await expect(image).toHaveAttribute('src', /desktop-song-detail-browser-preview-1366x768\.png/);
    await expect(image).toHaveAttribute('alt', /selected example song and its sheet details/);
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Review a song before you play it.',
    );
    await expect(fullSize).toHaveAttribute(
      'href',
      /desktop-song-detail-browser-preview-1366x768\.png/,
    );
    await expect
      .poll(() => image.evaluate((element) => (element as HTMLImageElement).naturalWidth))
      .toBeGreaterThan(0);
    await settings.focus();
    await page.keyboard.press('Enter');
    await expect(settings).toHaveAttribute('aria-pressed', 'true');
    await expect(image).toHaveAttribute(
      'src',
      /desktop-settings-playback-browser-preview-1366x768\.png/,
    );
    await expect(fullSize).toHaveAttribute(
      'href',
      /desktop-settings-playback-browser-preview-1366x768\.png/,
    );
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Choose playback options for your setup.',
    );
  });

  async function observePreviewLoads(page: Page) {
    await page.addInitScript(() => {
      window.Image = class extends Image {
        constructor() {
          super();
          const observe = (event: Event) => {
            document.documentElement.dataset.previewTestEvent = event.type + ':' + this.src;
          };
          this.addEventListener('load', observe);
          this.addEventListener('error', observe);
        }
      };
    });
  }

  function openingGeometry(page: Page) {
    return page.locator('.hero__primary, [data-product-image]').evaluateAll((elements) =>
      elements.map((element) => {
        const { x, y, width, height } = element.getBoundingClientRect();
        return [x + scrollX, y + scrollY, width, height];
      }),
    );
  }

  test('slow preview keeps the committed view and cancels when it is reselected', async ({
    page,
  }) => {
    await observePreviewLoads(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    let release!: () => void;
    const response = new Promise<void>((resolve) => {
      release = resolve;
    });
    try {
      await page.route('**/desktop-song-detail-browser-preview-1366x768.png', async (route) => {
        await response;
        await route.continue();
      });
      await page.goto('/Sky-Auto-Player/vi/');
      const group = page.getByRole('group', { name: 'Chọn giao diện trình phát' });
      const library = group.getByRole('button', { name: 'Thư viện' });
      const detail = group.getByRole('button', { name: 'Chi tiết bài hát' });
      const geometry = await openingGeometry(page);
      await detail.click();
      await expect(page.locator('[data-product-status]')).toHaveText('Đang tải ảnh xem trước…');
      await expect(library).toHaveAttribute('aria-pressed', 'true');
      await expect(detail).toHaveAttribute('aria-pressed', 'false');
      await expect(page.locator('[data-product-image]')).toHaveAttribute('src', /desktop-library/);
      await expect(page.locator('[data-product-full-size]')).toHaveAttribute(
        'href',
        /desktop-library/,
      );
      expect(await openingGeometry(page)).toEqual(geometry);
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await library.click();
      await expect(page.locator('[data-product-status]')).toBeEmpty();
      release();
      await expect(page.locator('html')).toHaveAttribute(
        'data-preview-test-event',
        /load:.*desktop-song-detail/,
      );
      await expect(library).toHaveAttribute('aria-pressed', 'true');
      await expect(page.locator('[data-product-status]')).toBeEmpty();
    } finally {
      release();
    }
  });

  test('a failed eager preview offers a focused retry and commits all image metadata', async ({
    page,
  }) => {
    const source = '**/desktop-library-browser-preview-1366x768.png';
    await page.setViewportSize({ width: 390, height: 844 });
    await page.route(source, (route) => route.abort());
    await page.goto('/Sky-Auto-Player/');
    await page.evaluate(() => document.fonts.ready);
    const library = page.getByRole('button', { name: 'Library' });
    const fallback = page.locator('.product-showcase__fallback');
    const status = page.locator('[data-product-status]');
    const image = page.locator('[data-product-image]');
    await expect(status).toHaveText(/preview could not load/);
    await expect(library).toHaveAttribute('aria-pressed', 'false');
    await expect(fallback.getByRole('link', { name: 'Library' })).toBeVisible();
    const geometry = await openingGeometry(page);
    await page.unroute(source);
    let release!: () => void;
    const response = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route(source, async (route) => {
      await response;
      await route.continue();
    });
    try {
      await library.click();
      await expect(status).toHaveText('Loading preview…');
      await fallback.getByRole('link', { name: 'Library' }).focus();
      release();
      await expect(library).toHaveAttribute('aria-pressed', 'true');
      await expect(library).toBeFocused();
      await expect(fallback).toBeHidden();
      await expect(status).toBeEmpty();
      await expect
        .poll(() => image.evaluate((node) => (node as HTMLImageElement).naturalWidth))
        .toBe(1366);
      await expect(image).toHaveAttribute('src', /desktop-library/);
      await expect(image).toHaveAttribute('alt', /song library/);
      await expect(page.locator('[data-product-caption]')).toHaveText(
        'Browse songs, favorites, and playlists.',
      );
      await expect(page.locator('[data-product-full-size]')).toHaveAttribute(
        'href',
        /desktop-library/,
      );
      expect(await openingGeometry(page)).toEqual(geometry);
    } finally {
      release();
    }
  });

  test('persisted pagehide cancels pending preview errors and preserves controls on return', async ({
    page,
  }) => {
    await observePreviewLoads(page);
    let release!: () => void;
    const response = new Promise<void>((resolve) => {
      release = resolve;
    });
    const source = '**/desktop-settings-playback-browser-preview-1366x768.png';
    await page.route(source, async (route) => {
      await response;
      await route.abort();
    });
    try {
      await page.goto('/Sky-Auto-Player/');
      const settings = page.getByRole('button', { name: 'Settings' });
      const status = page.locator('[data-product-status]');
      await settings.click();
      await expect(status).toHaveText('Loading preview…');
      await page.evaluate(() =>
        window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })),
      );
      release();
      await expect(page.locator('html')).toHaveAttribute(
        'data-preview-test-event',
        /error:.*desktop-settings/,
      );
      await page.evaluate(() =>
        window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })),
      );
      await expect(status).toBeEmpty();
      await expect(page.locator('.product-showcase__fallback')).toBeHidden();
      await expect(page.getByRole('button', { name: 'Library' })).toHaveAttribute(
        'aria-pressed',
        'true',
      );
      await page.unroute(source);
      await settings.click();
      await expect(settings).toHaveAttribute('aria-pressed', 'true');
      await expect(page.locator('[data-product-image]')).toHaveAttribute('src', /desktop-settings/);
    } finally {
      release();
    }
  });

  test('failed capture selection preserves the current image and accessible selection', async ({
    page,
  }) => {
    await page.route('**/desktop-settings-playback-browser-preview-1366x768.png', (route) =>
      route.abort(),
    );
    await page.goto('/Sky-Auto-Player/');
    const group = page.getByRole('group', { name: 'Choose a player view' });
    const image = page.locator('[data-product-image]');
    await page.evaluate(() => document.fonts.ready);
    const geometry = await openingGeometry(page);
    await group.getByRole('button', { name: 'Settings' }).click();
    await expect(group.getByRole('button', { name: 'Library' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(image).toHaveAttribute('src', /desktop-library-browser-preview-1366x768\.png/);
    await expect(image).toHaveAttribute('alt', /song library/);
    await expect(page.locator('[data-product-full-size]')).toHaveAttribute(
      'href',
      /desktop-library-browser-preview-1366x768\.png/,
    );
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Browse songs, favorites, and playlists.',
    );
    await expect(page.locator('[data-product-status]')).toHaveText(/preview could not load/);
    await expect(page.locator('.product-showcase__fallback')).toBeVisible();
    await expect(
      page.locator('.product-showcase__fallback').getByRole('link', { name: 'Settings' }),
    ).toHaveAttribute('href', /desktop-settings-playback-browser-preview-1366x768\.png/);

    expect(await openingGeometry(page)).toEqual(geometry);
    await group.getByRole('button', { name: 'Library' }).click();
    await expect(page.locator('[data-product-status]')).toBeEmpty();
    await expect(page.locator('.product-showcase__fallback')).toBeHidden();
    await group.getByRole('button', { name: 'Settings' }).click();
    await expect(page.locator('[data-product-status]')).toHaveText(/preview could not load/);
    await page.unroute('**/desktop-settings-playback-browser-preview-1366x768.png');
    await group.getByRole('button', { name: 'Settings' }).click();
    await expect(group.getByRole('button', { name: 'Settings' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(page.locator('[data-product-status]')).toBeEmpty();
    await expect(page.locator('.product-showcase__fallback')).toBeHidden();
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
      /bài hát mẫu đã chọn và chi tiết sheet/,
    );
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Xem lại bài hát trước khi phát.',
    );
    await expect(page.locator('[data-product-full-size]')).toHaveAttribute(
      'href',
      /desktop-song-detail-browser-preview-1366x768\.png/,
    );
  });

  test('the desktop browser preview remains contained and the site favicon contract is visible', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/Sky-Auto-Player/');
    const image = page.locator('[data-product-image]');
    await expect(image).toHaveJSProperty('complete', true);
    const metrics = await image.evaluate((element) => {
      const imageElement = element as HTMLImageElement;
      const box = imageElement.getBoundingClientRect();
      const frame = imageElement.closest('.product-showcase')!.getBoundingClientRect();
      return {
        naturalWidth: imageElement.naturalWidth,
        naturalHeight: imageElement.naturalHeight,
        insideFrame: box.left >= frame.left && box.right <= frame.right + 1,
      };
    });
    expect(metrics.naturalWidth).toBe(1366);
    expect(metrics.naturalHeight).toBe(768);
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

  test('one playback timeline synchronizes its event, timestamp, keys, and accessible progress', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const events = demo.locator('[data-demo-event]');
    const keys = demo.locator('[data-demo-key]');
    const progress = demo.locator('[data-demo-progress]');
    const elapsed = demo.locator('[data-elapsed]');
    const main = demo.locator('[data-action="play"]');
    const status = demo.locator('[data-demo-status]');

    await expect(demo.locator('[role="progressbar"]')).toHaveCount(1);
    await expect(events).toHaveCount(3);
    await expect(keys).toHaveCount(15);
    await expect(events.nth(0)).toHaveAttribute('aria-current', 'step');
    await expect(demo.locator('.playback-key.is-active')).toHaveCount(3);
    await expect(events.nth(2).locator('time')).toHaveText('1.2 s');
    await expect(progress).toHaveAttribute('aria-valuenow', '0');
    await expect(progress).not.toHaveAttribute('aria-valuetext');
    await expect(elapsed).toHaveText('0.0');

    await main.click();
    const initialStatus = await demo.locator('[data-demo-status]').textContent();
    await page.waitForTimeout(160);
    const initialProgress = Number(await progress.getAttribute('aria-valuenow'));
    expect(initialProgress).toBeGreaterThan(0);
    expect(initialProgress).toBeLessThan(700);
    await expect(demo.locator('[data-demo-status]')).toHaveText(initialStatus!);

    await expect(events.nth(1)).toHaveAttribute('aria-current', 'step', { timeout: 1200 });
    const eventState = await demo.evaluate((element) => {
      const activeEvent = element.querySelector<HTMLElement>(
        '[data-demo-event][aria-current="step"]',
      );
      const timeline = element.querySelector<HTMLElement>('[data-demo-progress]');
      const elapsedTime = element.querySelector<HTMLElement>('[data-elapsed]');
      const activeKeys = Array.from(
        element.querySelectorAll<HTMLElement>('.playback-key.is-active'),
      );
      const statusRegion = element.querySelector<HTMLElement>('[data-demo-status]');

      return {
        eventIndex: activeEvent?.dataset.demoEvent,
        progress: Number(timeline?.getAttribute('aria-valuenow')),
        elapsed: elapsedTime?.textContent?.trim(),
        keys: activeKeys.map((key) => key.dataset.demoKey),
        status: statusRegion?.textContent,
      };
    });
    expect(eventState.eventIndex).toBe('1');
    expect(eventState.keys).toEqual(['D2']);
    expect(eventState.status).toContain('Note: D2');
    const eventProgress = eventState.progress;
    expect(eventProgress).toBeGreaterThanOrEqual(700);
    expect(eventProgress).toBeLessThan(1400);
    expect(eventState.elapsed).toBe((Math.floor(eventProgress / 100) / 10).toFixed(1));

    await expect(status).toContainText('Phrase complete', {
      timeout: 2600,
    });
    await expect(demo.locator('[data-demo-event][aria-current="step"]')).toHaveCount(0);
    await expect(demo.locator('.playback-key.is-active')).toHaveCount(0);
    await expect(progress).toHaveAttribute('aria-valuenow', '2600');
    await expect(elapsed).toHaveText('2.6');
    await expect(main).toHaveText('Replay phrase');
  });

  test('timeline marker keeps a fixed width and reaches both ends as events advance', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const track = demo.locator('[data-demo-progress]');

    const start = await page.evaluate(() => {
      const track = document.querySelector<HTMLElement>('[data-demo-progress]')!;
      const marker = document.querySelector<HTMLElement>('[data-playhead]')!;
      const trackBox = track.getBoundingClientRect();
      const markerBox = marker.getBoundingClientRect();
      return { width: markerBox.width, leftGap: markerBox.left - trackBox.left };
    });
    expect(start.width).toBe(3);
    expect(Math.abs(start.leftGap)).toBeLessThanOrEqual(1);
    await expect(demo.locator('.playback-key.is-active').first()).toHaveCSS(
      'box-shadow',
      /0px -3px 0px 0px inset/,
    );
    await expect(demo.locator('[data-demo-event="0"]')).toHaveCSS(
      'border-inline-start-width',
      '2px',
    );
    const firstKey = demo.locator('[data-demo-key="A1"]');
    const keyGeometry = () =>
      firstKey.evaluate((key) => {
        const { x, y, width, height } = key.getBoundingClientRect();
        return [x + scrollX, y + scrollY, width, height];
      });
    const pressedGeometry = await keyGeometry();
    await demo.locator('[data-action="next"]').click();
    expect(await keyGeometry()).toEqual(pressedGeometry);
    await demo.locator('[data-action="previous"]').click();
    const segmentGrid = await demo.locator('.playback-timeline__segments').getAttribute('style');
    expect(await demo.locator('.playback-timeline__legend').getAttribute('style')).toBe(
      segmentGrid,
    );

    const next = demo.locator('[data-action="next"]');
    await next.click();
    await next.click();
    await next.click();
    await expect(track).toHaveAttribute('aria-valuenow', '2600');
    const end = await page.evaluate(() => {
      const track = document.querySelector<HTMLElement>('[data-demo-progress]')!;
      const marker = document.querySelector<HTMLElement>('[data-playhead]')!;
      const trackBox = track.getBoundingClientRect();
      const markerBox = marker.getBoundingClientRect();
      return { width: markerBox.width, rightGap: trackBox.right - markerBox.right };
    });
    expect(end.width).toBe(3);
    expect(Math.abs(end.rightGap)).toBeLessThanOrEqual(1);
  });

  test('manual stepping respects boundaries and pauses playback without a cancel control', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const previous = demo.locator('[data-action="previous"]');
    const next = demo.locator('[data-action="next"]');
    const main = demo.locator('[data-action="play"]');
    const progress = demo.locator('[data-demo-progress]');

    await expect(demo.locator('[data-action="cancel"]')).toHaveCount(0);
    await expect(previous).toHaveAttribute('aria-disabled', 'true');
    await previous.focus();
    await expect(previous).toBeFocused();
    await previous.evaluate((button: HTMLButtonElement) => button.click());
    await expect(demo.locator('[data-demo-event="0"]')).toHaveAttribute('aria-current', 'step');

    await next.click();
    await expect(demo.locator('[data-demo-event="1"]')).toHaveAttribute('aria-current', 'step');
    await expect(progress).toHaveAttribute('aria-valuenow', '700');
    await expect(demo.locator('.playback-key.is-active')).toHaveCount(1);
    await previous.click();
    await expect(demo.locator('[data-demo-event="0"]')).toHaveAttribute('aria-current', 'step');
    await expect(progress).toHaveAttribute('aria-valuenow', '0');

    await main.click();
    await next.click();
    await expect(demo.locator('[data-demo-event="1"]')).toHaveAttribute('aria-current', 'step');
    await expect(main).toHaveText('Continue phrase');
    await next.click();
    await expect(demo.locator('[data-demo-event="2"]')).toHaveAttribute('aria-current', 'step');
    await expect(progress).toHaveAttribute('aria-valuenow', '1400');
    await expect(demo.locator('[data-demo-key="E3"]')).toHaveClass(/is-active/);
    await next.click();
    await expect(progress).toHaveAttribute('aria-valuenow', '2600');
    await expect(demo.locator('.playback-key.is-active')).toHaveCount(0);
    await expect(next).toHaveAttribute('aria-disabled', 'true');
    await next.focus();
    await expect(next).toBeFocused();
    await previous.click();
    await expect(demo.locator('[data-demo-event="2"]')).toHaveAttribute('aria-current', 'step');
    await expect(demo.locator('.playback-key.is-active')).toHaveCount(1);
    await expect(previous).toHaveAttribute('aria-disabled', 'false');
  });

  test('pause, resume, and replay use the same timeline', async ({ page }) => {
    await page.clock.install();
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const main = demo.locator('[data-action="play"]');
    const progress = demo.locator('[data-demo-progress]');
    await main.click();
    await page.clock.runFor(720);
    const firstPauseAt = Number(await progress.getAttribute('aria-valuenow'));
    expect(firstPauseAt).toBeGreaterThanOrEqual(700);
    expect(firstPauseAt).toBeLessThan(1400);
    await expect(demo.locator('[data-demo-event="1"]')).toHaveAttribute('aria-current', 'step');
    await main.click();
    const pausedAt = await progress.getAttribute('aria-valuenow');
    await expect(main).toHaveText('Continue phrase');
    await page.clock.runFor(120);
    await expect(progress).toHaveAttribute('aria-valuenow', pausedAt!);
    await main.click();
    await page.clock.runFor(2200);
    await expect(main).toHaveText('Replay phrase', { timeout: 2600 });
    await expect(progress).toHaveAttribute('aria-valuenow', '2600');

    await main.click();
    await expect(main).toHaveText('Pause phrase');
    await page.clock.runFor(120);
    const replayProgress = Number(await progress.getAttribute('aria-valuenow'));
    expect(replayProgress).toBeGreaterThan(0);
    expect(replayProgress).toBeLessThan(700);
    await main.click();
    await expect(main).toHaveText('Continue phrase');
    const replayPausedAt = await progress.getAttribute('aria-valuenow');
    await page.clock.runFor(120);
    await expect(progress).toHaveAttribute('aria-valuenow', replayPausedAt!);
  });

  test('reduced motion preserves manual steps and moves focus when Play disappears in idle and complete states', async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const main = demo.locator('[data-action="play"]');
    const previous = demo.locator('[data-action="previous"]');
    const next = demo.locator('[data-action="next"]');
    await expect(main).toBeHidden();
    await expect(demo.locator('[data-motion-note]')).toBeVisible();
    await next.click();
    await expect(demo.locator('[data-demo-event="1"]')).toHaveAttribute('aria-current', 'step');
    await expect(demo.locator('[data-demo-progress]')).toHaveAttribute('aria-valuenow', '700');

    await page.emulateMedia({ reducedMotion: 'no-preference' });
    expect(
      await page.evaluate(() => window.matchMedia('(prefers-reduced-motion: reduce)').matches),
    ).toBe(false);
    await expect(main).toBeVisible();
    await main.focus();
    await expect(main).toBeFocused();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(main).toBeHidden();
    await expect(next).toBeFocused();

    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await expect(main).toBeVisible();
    await next.click();
    await next.click();
    await expect(demo.locator('[data-demo-progress]')).toHaveAttribute('aria-valuenow', '2600');
    await main.focus();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(main).toBeHidden();
    await expect(previous).toBeFocused();
  });

  test('visibility changes and persisted pagehide pause playback without losing controls or auto-resuming', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const demo = page.locator('[data-playback-demo]');
    const main = demo.locator('[data-action="play"]');
    const progress = demo.locator('[data-demo-progress]');
    await main.click();
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await expect(main).toHaveText('Continue phrase');
    const hiddenAt = await progress.getAttribute('aria-valuenow');
    await page.waitForTimeout(120);
    await expect(progress).toHaveAttribute('aria-valuenow', hiddenAt!);

    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
    });
    await main.click();
    await page.evaluate(() =>
      window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })),
    );
    await expect(main).toHaveText('Continue phrase');
    await main.click();
    await page.evaluate(() =>
      window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })),
    );
    await expect(main).toHaveText('Continue phrase');
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
    await expect(page.locator('.product-showcase__fallback a')).toHaveCount(3);
    await expect(page.locator('[data-product-image]')).toHaveAttribute(
      'src',
      /desktop-library-browser-preview-1366x768\.png/,
    );
    await expect(page.locator('[data-demo-controls]')).toBeHidden();
    await expect(page.locator('[data-demo-event]')).toHaveCount(3);
    await expect(page.locator('[data-demo-event="0"]')).toHaveAttribute('aria-current', 'step');
    await expect(page.locator('[data-demo-status]')).toContainText('Ready');
    await expect(page.locator('[data-demo-event="0"]')).toContainText('A1 + B2 + C3');
    await expect(page.locator('[data-demo-event="1"]')).toContainText('0.7 s');
    await expect(page.locator('[data-demo-event="2"]')).toContainText('1.2 s');
    await expect(page.locator('.playback-key')).toHaveCount(15);
    await expect(page.locator('#timing-model')).toBeVisible();
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
          '.hero__meta',
          '.hero__risk',
          '.product-showcase__caption',
          '.playback-timeline__legend li',
          '.trust-download__package',
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
      await expect(page.locator('#technical')).toContainText('Sky.Auto.Player_4.3.0_x64-setup.exe');
      await expect(page.locator('#technical')).toContainText('.exe.sig');
      await expect(page.locator('#technical')).toContainText(
        route === '/' ? 'stable Windows installer' : 'bộ cài Windows ổn định',
      );
      await expect(page.locator('#technical a[href*="account-safety"]')).toBeVisible();
      await expect(page.locator('#technical')).not.toContainText('100% safe');
    }
    await expect(page.locator('#faq-preview a')).toHaveCount(3);
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

  test('FAQ groups are visible on mobile and hash links open the matching answer in both locales', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    for (const locale of [
      {
        route: '/faq/',
        topics: ['Setup & use', 'Sheet formats', 'Safety', 'Troubleshooting'],
      },
      {
        route: '/vi/faq/',
        topics: ['Cài đặt & sử dụng', 'Định dạng sheet', 'An toàn', 'Khắc phục sự cố'],
      },
    ]) {
      await page.goto(`/Sky-Auto-Player${locale.route}`);
      await expect(page.locator('main h1')).toBeVisible();
      await expect(page.locator('.faq-toc nav')).toBeVisible();
      await expect(page.locator('.faq-toc nav a')).toHaveCount(4);
      for (const topic of locale.topics) {
        await expect(page.locator('.faq-toc nav')).toContainText(topic);
      }

      await page.goto(`/Sky-Auto-Player${locale.route}#account-safety`);
      const safetyAnswer = page.locator('#account-safety');
      await expect(safetyAnswer).toHaveAttribute('open', '');
      await page.evaluate(() => {
        window.location.hash = 'network';
      });
      const networkAnswer = page.locator('#network');
      await expect(networkAnswer).toHaveAttribute('open', '');
      await expect(networkAnswer.locator('summary')).toBeFocused();
    }
  });

  test('FAQ deep links still reach native disclosures without JavaScript', async ({ browser }) => {
    const context = await browser.newContext({
      javaScriptEnabled: false,
      viewport: { width: 390, height: 844 },
    });
    const page = await context.newPage();
    await page.goto('/Sky-Auto-Player/faq/#account-safety');
    const safetyAnswer = page.locator('#account-safety');
    await expect(safetyAnswer.locator('summary')).toBeVisible();
    await safetyAnswer.locator('summary').click();
    await expect(safetyAnswer).toHaveJSProperty('open', true);
    await expect(page.locator('.faq-toc nav a')).toHaveCount(4);
    await context.close();
  });

  test('guide table of contents follows rendered headings and stays responsive in both locales', async ({
    page,
  }) => {
    for (const route of ['/guides/windows-setup/', '/vi/guides/windows-setup/']) {
      for (const width of [390, 1440]) {
        await page.setViewportSize({ width, height: 900 });
        await page.goto(`/Sky-Auto-Player${route}`);
        const toc = page.locator('.guide-toc');
        await expect(toc).toBeVisible();
        const tocLinks = toc.locator('a');
        expect(await tocLinks.count()).toBeGreaterThanOrEqual(4);
        const linksMatchHeadings = await tocLinks.evaluateAll((links) =>
          links.every((link) => {
            const target = document.querySelector<HTMLElement>(link.getAttribute('href') || '');
            return target && /^H[23]$/.test(target.tagName) && target.id;
          }),
        );
        expect(linksMatchHeadings).toBe(true);
        await expect(page.locator('.related-guides')).toBeVisible();
        await expect(page.locator('.guide-page__cta')).toHaveCount(0);
        const widthMetrics = await page.evaluate(() => ({
          document: document.documentElement.scrollWidth,
          viewport: document.documentElement.clientWidth,
        }));
        expect(widthMetrics.document).toBeLessThanOrEqual(widthMetrics.viewport + 1);
      }
    }
  });

  test('Markdown tables stay contained and the six-step diagram reflows on mobile', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    for (const route of [
      '/guides/sheet-formats/',
      '/guides/how-it-works/',
      '/vi/guides/how-it-works/',
    ]) {
      await page.goto(`/Sky-Auto-Player${route}`);
      const localOverflow = await page.evaluate(() => {
        const element = document.querySelector<HTMLElement>('table, .arch-diagram');
        if (!element) return null;
        return {
          client: element.clientWidth,
          content: element.scrollWidth,
          document: document.documentElement.scrollWidth,
          viewport: document.documentElement.clientWidth,
        };
      });
      expect(localOverflow).not.toBeNull();
      if (route.includes('how-it-works')) {
        expect(localOverflow!.content).toBe(localOverflow!.client);
        const steps = page.locator('.arch-diagram li');
        await expect(steps).toHaveCount(6);
        const boxes = await Promise.all((await steps.all()).map((step) => step.boundingBox()));
        for (let i = 1; i < boxes.length; i++) {
          expect(boxes[i]!.y).toBeGreaterThan(boxes[i - 1]!.y + boxes[i - 1]!.height);
        }
        const vi = route.startsWith('/vi/');
        await expect(steps.locator('strong')).toHaveText([
          vi ? 'File Sheet' : 'Sheet file',
          'Parser',
          vi ? 'Timing Model' : 'Timing model',
          'Rust Dispatch',
          'Windows SendInput',
          'Game',
        ]);
        await expect(steps.first()).toContainText(
          vi ? '.txt tương thích JSON' : 'JSON-compatible .txt',
        );
        await expect(steps.last()).toContainText(vi ? 'Ranh giới tiến trình' : 'Process boundary');
      } else {
        expect(localOverflow!.content).toBeGreaterThanOrEqual(localOverflow!.client);
      }
      expect(localOverflow!.document).toBeLessThanOrEqual(localOverflow!.viewport + 1);
    }
  });

  for (const prefix of ['', '/vi']) {
    test(
      prefix + ' Guide Hub summaries match the six guide articles',
      async ({ page, context }) => {
        await page.goto('/Sky-Auto-Player' + prefix + '/guides/');
        const entries = page.locator('.guide-hub li');
        await expect(entries).toHaveCount(6);
        const articles = await entries.evaluateAll((rows) =>
          rows.map((row) => ({
            url: row.querySelector('a')!.href,
            summary: row.querySelector('p')!.textContent!.trim(),
          })),
        );
        const detail = await context.newPage();
        for (const article of articles) {
          expect(article.summary).not.toEqual('');
          await detail.goto(article.url);
          await expect(detail.locator('.guide-page__summary')).toHaveText(article.summary);
        }
        await detail.close();
      },
    );
  }

  test('forced colors preserve the playback marker, event and key cues at 320px', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 320, height: 720 });
    await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
    await page.goto('/Sky-Auto-Player/vi/');
    await page.locator('[data-action="next"]').press('Enter');
    await expect(page.locator('[data-demo-progress]')).toHaveCSS('outline-style', 'solid');
    await expect(page.locator('[data-playhead]')).toHaveCSS('width', '3px');
    await expect(page.locator('[data-playhead]')).toHaveCSS('box-shadow', /inset/);
    await expect(page.locator('.playback-timeline__segment[aria-current="step"]')).toHaveCSS(
      'forced-color-adjust',
      'none',
    );
    await expect(page.locator('[data-demo-key="D2"]')).toHaveCSS('outline-width', '2px');
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
