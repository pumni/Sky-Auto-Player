import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

const routes = ['/', '/faq/', '/vi/', '/vi/faq/'];
const homepageSections = ['hero', 'product', 'playback', 'quick-start', 'trust-download'];
const releaseUrl = 'https://github.com/pumni/Sky-Auto-Player/releases/latest';

test.describe('accessibility and responsive contracts', () => {
  for (const route of routes) {
    test(`${route} has no axe violations`, async ({ page }) => {
      await page.goto(`/Sky-Auto-Player${route}`);
      const accessibilityScanResults = await new AxeBuilder({ page }).analyze();
      expect(accessibilityScanResults.violations).toEqual([]);
    });
  }

  for (const locale of [
    { route: '/', heading: 'Your music. Into Sky.', cta: 'Download for Windows' },
    { route: '/vi/', heading: 'Âm nhạc của bạn. Trong Sky.', cta: 'Tải cho Windows' },
  ]) {
    test(`${locale.route} keeps the five-chapter narrative, anchors, and release journey`, async ({
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
      await expect(
        page.locator('.site-header .button--primary, main .button--primary'),
      ).toHaveCount(1);
      await expect(page.locator('.header-download')).toHaveCount(0);
      await expect(page.locator('.home-language')).toBeVisible();
      await expect(page.locator('.trust-download__button')).toHaveAttribute('href', releaseUrl);
      await expect(page.locator('#formats code')).toHaveText(['.json', '.skysheet', '.txt']);
      await expect(page.locator('#formats p')).toContainText('JSON');
      await expect(page.locator('.quick-start__setup a')).toHaveAttribute(
        'href',
        /guides\/windows-setup\/$/,
      );
      await expect(page.locator('#faq-preview a')).toHaveCount(2);
      await expect(page.locator('#faq-preview a').nth(0)).toHaveAttribute('href', /faq\/$/);
      await expect(page.locator('#faq-preview a').nth(1)).toHaveAttribute(
        'href',
        /guides\/troubleshooting\/$/,
      );
      await expect(page.locator('.hero__secondary')).toHaveAttribute('href', '#how-it-works');
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

  for (const width of [390, 1024, 1440]) {
    test(`${width}px fragment links use one header clearance and clamp at the document end`, async ({
      page,
      browser,
    }) => {
      const viewport = { width, height: width === 390 ? 844 : width === 1024 ? 768 : 900 };
      const desktop = width >= 1024;
      const clearance = desktop ? { min: 85, max: 105 } : { min: 16, max: 32 };
      const home = '/Sky-Auto-Player/';
      const readAnchor = async (targetPage: typeof page, id: string) => {
        await targetPage.evaluate(() => document.fonts.ready);
        await expect
          .poll(async () =>
            targetPage.locator('#' + id).evaluate((target, { min, max }) => {
              const top = target.getBoundingClientRect().top;
              const maximum = document.documentElement.scrollHeight - innerHeight;
              const documentTop = scrollY + top;
              return (
                (top >= min && top <= max) ||
                (documentTop <= max && scrollY <= 1) ||
                (documentTop - max >= maximum && maximum - scrollY <= 1)
              );
            }, clearance),
          )
          .toBe(true);
      };

      await page.setViewportSize(viewport);
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(home);
      await page.locator('.hero__secondary').press('Enter');
      await expect(page).toHaveURL(/#how-it-works$/);
      await readAnchor(page, 'how-it-works');
      if (!desktop) await page.locator('.menu-toggle').click();
      await page.locator('.site-nav a[href$="#product"]').press('Enter');
      await expect(page.locator('#product')).toBeFocused();
      await readAnchor(page, 'product');
      if (!desktop) await page.locator('.menu-toggle').click();
      await page.locator('.site-nav a[href$="#how-it-works"]').press('Enter');
      await expect(page.locator('#how-it-works')).toBeFocused();
      await readAnchor(page, 'how-it-works');

      for (const id of [
        'top',
        'product',
        'playback',
        'how-it-works',
        'formats',
        'technical',
        'timing-model',
        'faq-preview',
      ]) {
        await page.goto(home + '#' + id);
        await readAnchor(page, id);
      }
      expect(
        await page.evaluate(() => document.documentElement.scrollHeight - innerHeight - scrollY),
      ).toBeLessThanOrEqual(1);

      await page.goto(home + 'faq/#account-safety');
      await expect(page.locator('#account-safety')).toHaveAttribute('open', '');
      await readAnchor(page, 'account-safety');
      await page.goto(home + 'guides/windows-setup/#adding-songs');
      await readAnchor(page, 'adding-songs');

      const context = await browser.newContext({
        viewport,
        javaScriptEnabled: false,
        reducedMotion: 'reduce',
      });
      try {
        const staticPage = await context.newPage();
        await staticPage.goto(home + '#product');
        await readAnchor(staticPage, 'product');
        await staticPage.goto(home + 'guides/windows-setup/#adding-songs');
        await readAnchor(staticPage, 'adding-songs');
      } finally {
        await context.close();
      }
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

    test(`${route} keeps the 320px brand and language action visually separate`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 320, height: 844 });
      await page.goto(`/Sky-Auto-Player${route}`);
      const rightEdge = await page.evaluate(() => {
        const brandText = document.querySelector('.brand > span:last-child')!;
        const languageAction = document.querySelector('.site-actions')!;
        return {
          brandText: brandText.getBoundingClientRect().right,
          languageAction: languageAction.getBoundingClientRect().left,
        };
      });
      expect(rightEdge.brandText).toBeLessThanOrEqual(rightEdge.languageAction);
    });
  }

  test('Home band spacing, shared alignment, and FAQ spacing hold at responsive widths', async ({
    page,
  }) => {
    const widths = [320, 390, 768, 1024, 1440, 1920];
    const faqSpacing = [48, 48, 48, 61.44, 86.4, 96];
    await page.goto('/Sky-Auto-Player/');
    for (const width of widths) {
      await page.setViewportSize({ width, height: 900 });
      const spacing = await page.evaluate(() => {
        const bounds = (selector: string) =>
          document.querySelector(selector)!.getBoundingClientRect();
        const contentGaps = [
          bounds('.product-showcase__features').top -
            Math.max(bounds('#top').bottom, bounds('#product').bottom),
          bounds('#playback').top - bounds('.product-showcase__features').bottom,
          bounds('.quick-start__layout').top - bounds('#playback').bottom,
          bounds('.trust-download__layout').top - bounds('.quick-start__resources').bottom,
        ];
        const footer = document.querySelector('.site-footer__layout')!.getBoundingClientRect();
        const rails = [
          '.home-intro__layout',
          '.playback-demo__layout',
          '.quick-start__layout',
          '.trust-download__layout',
        ].map((selector) => {
          const rect = document.querySelector(selector)!.getBoundingClientRect();
          return { left: rect.left, right: rect.right, width: rect.width };
        });
        return {
          gaps: contentGaps,
          footerLeft: footer.left,
          footerWidth: footer.width,
          rails,
          viewport: document.documentElement.clientWidth,
        };
      });
      expect(spacing.gaps).toHaveLength(4);
      const [opening, firstBand, secondBand, closing] = spacing.gaps;
      const band = Math.min(116, Math.max(64, width * 0.08));
      const closingBand = Math.min(64, Math.max(40, width * 0.04));
      expect(opening).toBeGreaterThanOrEqual(27);
      expect(opening).toBeLessThanOrEqual(77);
      expect(Math.abs(firstBand)).toBeLessThanOrEqual(2);
      expect(secondBand).toBeGreaterThanOrEqual(band - 1);
      expect(secondBand).toBeLessThanOrEqual(band + 2);
      expect(closing).toBeGreaterThanOrEqual(closingBand - 1);
      expect(closing).toBeLessThanOrEqual(closingBand + 2);
      expect(spacing.footerLeft).toBeGreaterThanOrEqual(16);
      expect(spacing.footerWidth).toBeLessThanOrEqual(1280.5);
      const rail = spacing.rails[0];
      expect(rail.left).toBeGreaterThanOrEqual(16);
      expect(rail.right).toBeLessThanOrEqual(spacing.viewport - 16);
      for (const aligned of spacing.rails.slice(1)) {
        expect(Math.abs(aligned.left - rail.left)).toBeLessThanOrEqual(1);
        expect(Math.abs(aligned.right - rail.right)).toBeLessThanOrEqual(1);
        expect(Math.abs(aligned.width - rail.width)).toBeLessThanOrEqual(1);
      }
      if (width === 1440 || width === 1920) expect(rail.width).toBeCloseTo(1280, 0);
    }

    await page.goto('/Sky-Auto-Player/faq/');
    for (let index = 0; index < widths.length; index += 1) {
      await page.setViewportSize({ width: widths[index], height: 900 });
      const defaultSpacing = await page
        .locator('.faq-layout')
        .evaluate((element) => Number.parseFloat(getComputedStyle(element).marginBottom));
      expect(Math.abs(defaultSpacing - faqSpacing[index])).toBeLessThan(0.2);
    }
  });

  test('Home composition uses one rail and the approved responsive grouping in both locales', async ({
    page,
  }) => {
    const viewports = [
      { width: 1920, height: 1080 },
      { width: 1440, height: 900 },
      { width: 1280, height: 800 },
      { width: 1024, height: 768 },
      { width: 768, height: 1024 },
      { width: 390, height: 844 },
      { width: 320, height: 768 },
      { width: 844, height: 390 },
    ];

    for (const route of ['/', '/vi/']) {
      for (const viewport of viewports) {
        await page.setViewportSize(viewport);
        await page.goto(`/Sky-Auto-Player${route}`);
        await page.evaluate(() => document.fonts.ready);
        const geometry = await page.evaluate(() => {
          const bounds = (selector: string) => {
            const rect = document.querySelector(selector)!.getBoundingClientRect();
            return {
              left: rect.left,
              right: rect.right,
              top: rect.top,
              bottom: rect.bottom,
              width: rect.width,
            };
          };
          const stepHeadings = [...document.querySelectorAll('.quick-start__step h3')].map(
            (element) => {
              const rect = element.getBoundingClientRect();
              return { left: rect.left, top: rect.top };
            },
          );
          const trustHeadings = [...document.querySelectorAll('.trust-download__details h3')].map(
            (element) => element.getBoundingClientRect().top,
          );
          const ctaButton = bounds('.trust-download__button');
          const ctaNote = bounds('.trust-download__destination');
          return {
            documentWidth: document.documentElement.scrollWidth,
            viewportWidth: document.documentElement.clientWidth,
            rail: [
              bounds('.home-intro__layout'),
              bounds('.playback-demo__layout'),
              bounds('.quick-start__layout'),
              bounds('.trust-download__layout'),
            ],
            productImage: bounds('[data-product-image]'),
            hero: bounds('#top'),
            product: bounds('#product'),
            productSelector: bounds('.product-selector'),
            setupGuide: bounds('.quick-start__setup'),
            formats: bounds('.quick-start__formats'),
            resourceLinkSpacing: [
              ...document.querySelectorAll<HTMLAnchorElement>('.quick-start__resource-link'),
            ].map((link) => {
              const content = link.parentElement!;
              const body =
                content.querySelector<HTMLElement>('.quick-start__resource-body') ??
                content.querySelector<HTMLElement>('p')!;
              return {
                linkTop: link.getBoundingClientRect().top,
                lastBodyBottom: body.getBoundingClientRect().bottom,
              };
            }),
            playbackHeader: bounds('.playback-demo__header'),
            playbackStage: bounds('.playback-demo__stage'),
            playbackSupport: bounds('.playback-demo__support'),
            playbackTimeline: bounds('.playback-timeline'),
            playbackKeys: bounds('.playback-keyboard'),
            playbackDOMOrder: [
              ...document.querySelectorAll(
                '#playback .playback-demo__header, #playback .playback-demo__stage, #playback .playback-demo__support',
              ),
            ].map((element) => element.className),
            quickStartSteps: [...document.querySelectorAll('.quick-start__step')].map((element) => {
              const rect = element.getBoundingClientRect();
              return { left: rect.left, top: rect.top, width: rect.width };
            }),
            stepHeadings,
            trustHeadings,
            ctaCopy: bounds('.trust-download__cta-copy'),
            ctaAction: bounds('.trust-download__cta-action'),
            ctaButton,
            ctaNote,
            featureSecond: bounds('.product-showcase__features > div:nth-child(2)'),
            trustSecond: bounds('.trust-download__details > div:nth-child(2)'),
            supportLinks: bounds('.trust-download__support-links'),
            supportLinksDirection: getComputedStyle(
              document.querySelector('.trust-download__support-links')!,
            ).flexDirection,
            supportLinkItems: [
              ...document.querySelectorAll('.trust-download__support-links > li'),
            ].map((element) => {
              const rect = element.getBoundingClientRect();
              return { left: rect.left, top: rect.top, bottom: rect.bottom };
            }),
            navigation: bounds('.site-nav'),
            headerRail: bounds('.site-header__inner'),
          };
        });

        expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewportWidth + 1);
        const [product, playback, quickStart, trust] = geometry.rail;
        for (const aligned of [playback, quickStart, trust]) {
          expect(
            Math.abs(aligned.left - product.left),
            `${route} ${viewport.width}px rail left`,
          ).toBeLessThanOrEqual(1);
          expect(
            Math.abs(aligned.right - product.right),
            `${route} ${viewport.width}px rail right`,
          ).toBeLessThanOrEqual(1);
        }
        expect(geometry.playbackDOMOrder).toEqual([
          'playback-demo__header',
          'playback-demo__stage',
          'playback-demo__support',
        ]);
        expect(geometry.playbackKeys.top).toBeGreaterThan(geometry.playbackTimeline.bottom);
        expect(geometry.productSelector.bottom).toBeLessThanOrEqual(geometry.productImage.top);
        if (viewport.width >= 1440) {
          expect(product.width).toBeCloseTo(1280, 0);
        }
        if (viewport.width >= 1024) {
          expect(geometry.product.left).toBeGreaterThan(geometry.hero.right);
          const navCenter = (geometry.navigation.left + geometry.navigation.right) / 2;
          const headerCenter = (geometry.headerRail.left + geometry.headerRail.right) / 2;
          expect(Math.abs(navCenter - headerCenter)).toBeLessThanOrEqual(1);
          expect(geometry.playbackStage.left).toBeGreaterThan(geometry.playbackHeader.right);
          expect(geometry.playbackSupport.top).toBeGreaterThanOrEqual(
            geometry.playbackHeader.bottom,
          );
        } else {
          expect(geometry.product.top).toBeGreaterThan(geometry.hero.bottom);
          expect(geometry.playbackHeader.bottom).toBeLessThanOrEqual(geometry.playbackStage.top);
          expect(geometry.playbackSupport.top).toBeGreaterThanOrEqual(
            geometry.playbackStage.bottom + 20,
          );
        }

        if (viewport.width >= 768) {
          expect(geometry.quickStartSteps[0].left).toBeLessThan(geometry.quickStartSteps[1].left);
          expect(geometry.quickStartSteps[1].left).toBeLessThan(geometry.quickStartSteps[2].left);
          const headingTop = geometry.stepHeadings[0].top;
          for (const heading of geometry.stepHeadings.slice(1)) {
            expect(Math.abs(heading.top - headingTop)).toBeLessThanOrEqual(1);
          }
          const trustHeadingTop = geometry.trustHeadings[0];
          for (const headingTopValue of geometry.trustHeadings.slice(1)) {
            expect(Math.abs(headingTopValue - trustHeadingTop)).toBeLessThanOrEqual(1);
          }
          expect(geometry.ctaAction.left).toBeGreaterThan(geometry.ctaCopy.left);
          expect(geometry.formats.left).toBeGreaterThan(geometry.setupGuide.right);
          for (const aligned of [
            geometry.featureSecond,
            geometry.trustSecond,
            geometry.supportLinks,
            geometry.ctaAction,
          ]) {
            expect(Math.abs(aligned.left - geometry.formats.left)).toBeLessThanOrEqual(1);
          }
          expect(geometry.supportLinksDirection).toBe('row');
        } else {
          expect(geometry.supportLinksDirection).toBe('column');
          for (let index = 1; index < geometry.supportLinkItems.length; index += 1) {
            expect(geometry.supportLinkItems[index].top).toBeGreaterThan(
              geometry.supportLinkItems[index - 1].bottom,
            );
          }
          expect(geometry.formats.top).toBeGreaterThan(geometry.setupGuide.bottom);
          expect(geometry.quickStartSteps[0].top).toBeLessThan(geometry.quickStartSteps[1].top);
          expect(geometry.quickStartSteps[1].top).toBeLessThan(geometry.quickStartSteps[2].top);
          expect(geometry.quickStartSteps[0].left).toBeCloseTo(geometry.quickStartSteps[1].left, 0);
          expect(geometry.stepHeadings[0].left).toBeGreaterThan(
            geometry.quickStartSteps[0].left + 24,
          );
        }
        for (const resource of geometry.resourceLinkSpacing) {
          expect(resource.linkTop - resource.lastBodyBottom).toBeGreaterThanOrEqual(7.5);
          expect(resource.linkTop - resource.lastBodyBottom).toBeLessThanOrEqual(8.5);
        }
        expect(geometry.ctaButton.bottom).toBeLessThanOrEqual(geometry.ctaNote.top);
        expect(geometry.ctaNote.top - geometry.ctaButton.bottom).toBeLessThanOrEqual(12);
      }
    }
  });

  test('both locales expose a meaningful portion of the real preview on the opening fold', async ({
    page,
  }) => {
    const viewports = [
      { width: 1440, height: 900, minVisible: 180 },
      { width: 1024, height: 768, minVisible: 120 },
      { width: 390, height: 844, minVisible: 100 },
    ];
    for (const route of ['/', '/vi/']) {
      for (const { width, height, minVisible } of viewports) {
        await page.setViewportSize({ width, height });
        await page.goto(`/Sky-Auto-Player${route}`);
        await page.evaluate(() => document.fonts.ready);
        const image = page.locator('[data-product-image]');
        await expect
          .poll(() => image.evaluate((img: HTMLImageElement) => img.naturalWidth))
          .toBe(1366);
        expect(await image.evaluate((img: HTMLImageElement) => img.naturalHeight)).toBe(768);
        const geometry = await page.evaluate(() => {
          const rect = (selector: string) =>
            document.querySelector(selector)!.getBoundingClientRect();
          const hero = rect('#top');
          const risk = rect('.hero__risk');
          const action = rect('.hero__primary');
          const secondary = rect('.hero__secondary');
          const preview = rect('[data-product-image]');
          const selector = rect('.product-selector');
          const product = rect('#product');
          return {
            centerOffset: (hero.left + hero.right - preview.left - preview.right) / 2,
            previewWidth: preview.width,
            previewHeight: preview.height,
            visible: Math.max(0, Math.min(preview.bottom, innerHeight) - Math.max(preview.top, 0)),
            openingOrder:
              action.bottom <= risk.top &&
              secondary.bottom <= risk.top &&
              risk.bottom <= hero.bottom &&
              (hero.bottom <= product.top || hero.right <= product.left) &&
              selector.bottom <= preview.top,
            productOnscreen:
              preview.left >= 0 && preview.right <= document.documentElement.clientWidth,
            heroOnscreen: action.top >= 0 && secondary.top >= 0 && risk.bottom <= innerHeight,
            buttons: [...document.querySelectorAll('[data-product-view]')].map(
              (button) => button.getBoundingClientRect().height,
            ),
          };
        });
        expect(geometry.openingOrder).toBe(true);
        expect(geometry.heroOnscreen).toBe(true);
        expect(geometry.productOnscreen).toBe(true);
        if (width < 1024) expect(geometry.centerOffset).toBeCloseTo(0, 1);
        else expect(geometry.centerOffset).toBeLessThan(0);
        expect(geometry.previewWidth / geometry.previewHeight).toBeCloseTo(1366 / 768, 2);
        expect(geometry.visible).toBeGreaterThanOrEqual(minVisible);
        if (width === 1440) {
          expect(geometry.previewWidth).toBeGreaterThanOrEqual(640);
          expect(geometry.previewWidth).toBeLessThanOrEqual(800);
        }
        expect(geometry.buttons).toHaveLength(3);
        for (const button of geometry.buttons) expect(button).toBeGreaterThanOrEqual(44);
      }
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
    const library = group.getByRole('button', { name: 'Library' });
    await expect(library).toHaveAttribute('aria-pressed', 'true');
    await expect(library).toHaveCSS('border-radius', '0px');
    await expect(library).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await expect(library).toHaveCSS('border-bottom-style', 'solid');
    await expect(library).toHaveCSS('border-bottom-width', '2px');
    await expect(library).toHaveCSS('min-height', '44px');
    expect(await group.evaluate((element) => getComputedStyle(element).boxShadow)).not.toBe('none');
    await expect(image).toHaveAttribute('src', /desktop-library-browser-preview-1366x768\.png/);
    await expect(image).toHaveAttribute('alt', /song library/);
    await expect(fullSize).toHaveAttribute('href', /desktop-library-browser-preview-1366x768\.png/);

    await group.getByRole('button', { name: 'Song details' }).click();
    const settings = group.getByRole('button', { name: 'Playback settings' });
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
      'Search bundled songs, save favorites and organize playlists.',
    );

    await group.getByRole('button', { name: 'Song details' }).click();
    await expect(group.getByRole('button', { name: 'Song details' })).toHaveAttribute(
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
      'Review sheet compatibility before playback. Blue Bird is selected in this preview.',
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
      'Adjust playback defaults and key-hold duration.',
    );
    await expect(settings).toHaveCSS('border-bottom-style', 'solid');
    await expect(settings).toHaveCSS('border-bottom-width', '2px');
    const focusStyle = await settings.evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        outlineStyle: style.outlineStyle,
        outlineWidth: Number.parseFloat(style.outlineWidth),
      };
    });
    expect(focusStyle.outlineStyle).not.toBe('none');
    expect(focusStyle.outlineWidth).toBeGreaterThanOrEqual(2);
    await page.emulateMedia({ forcedColors: 'active' });
    const forcedColors = await settings.evaluate((element) => ({
      active: matchMedia('(forced-colors: active)').matches,
      indicator: getComputedStyle(element).borderBottomColor,
      baseline: getComputedStyle(element.parentElement!).boxShadow,
    }));
    expect(forcedColors.active).toBe(true);
    expect(forcedColors.indicator).not.toBe('rgba(0, 0, 0, 0)');
    expect(forcedColors.baseline).toBe('none');
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
      const detail = group.getByRole('button', { name: 'Chi tiết bài' });
      const geometry = await openingGeometry(page);
      await detail.click();
      await expect(page.locator('[data-product-status]')).toHaveText('Đang tải ảnh xem trước…');
      await expect(detail).toHaveAttribute('data-pending', '');
      await expect(detail).toHaveCSS('border-bottom-style', 'dashed');
      await expect(detail).toHaveCSS('border-bottom-width', '2px');
      await expect(detail).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
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
        'Search bundled songs, save favorites and organize playlists.',
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
      const settings = page.getByRole('button', { name: 'Playback settings' });
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
    await group.getByRole('button', { name: 'Playback settings' }).click();
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
      'Search bundled songs, save favorites and organize playlists.',
    );
    await expect(page.locator('[data-product-status]')).toHaveText(/preview could not load/);
    await expect(page.locator('.product-showcase__fallback')).toBeVisible();
    await expect(
      page.locator('.product-showcase__fallback').getByRole('link', { name: 'Playback settings' }),
    ).toHaveAttribute('href', /desktop-settings-playback-browser-preview-1366x768\.png/);

    expect(await openingGeometry(page)).toEqual(geometry);
    await group.getByRole('button', { name: 'Library' }).click();
    await expect(page.locator('[data-product-status]')).toBeEmpty();
    await expect(page.locator('.product-showcase__fallback')).toBeHidden();
    await group.getByRole('button', { name: 'Playback settings' }).click();
    await expect(page.locator('[data-product-status]')).toHaveText(/preview could not load/);
    await page.unroute('**/desktop-settings-playback-browser-preview-1366x768.png');
    await group.getByRole('button', { name: 'Playback settings' }).click();
    await expect(group.getByRole('button', { name: 'Playback settings' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(page.locator('[data-product-status]')).toBeEmpty();
    await expect(page.locator('.product-showcase__fallback')).toBeHidden();
  });

  test('Vietnamese player view selector has compact localized names and selected captions', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/vi/');
    const group = page.getByRole('group', { name: 'Chọn giao diện trình phát' });
    await expect(group.getByRole('button', { name: 'Thư viện' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    const details = group.getByRole('button', { name: 'Chi tiết bài' });
    await expect(details).toHaveAccessibleName('Chi tiết bài');
    await details.click();
    await expect(page.locator('[data-product-image]')).toHaveAttribute(
      'alt',
      /bài hát mẫu đã chọn và chi tiết sheet/,
    );
    await expect(page.locator('[data-product-caption]')).toHaveText(
      'Kiểm tra khả năng tương thích trước khi phát. Ảnh minh họa đang chọn Blue Bird.',
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
    const trackWidths = await demo
      .locator('[data-demo-segment]')
      .evaluateAll((segments) => segments.map((segment) => segment.getBoundingClientRect().width));
    const legendWidths = await demo
      .locator('[data-demo-event]')
      .evaluateAll((events) => events.map((event) => event.getBoundingClientRect().width));
    for (const widths of [trackWidths, legendWidths]) {
      expect(widths).toHaveLength(3);
      // Chromium rounds fractional CSS grid tracks to 1/64px. Equal-duration
      // events can therefore differ by one layout quantum at the narrower rail.
      expect(Math.abs(widths[0] - widths[1])).toBeLessThanOrEqual(1 / 64);
      expect(widths[2] / widths[0]).toBeCloseTo(1200 / 700, 2);
    }

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
    await expect(page.locator('.header-download')).toHaveCount(0);
    await expect(page.locator('.home-language')).toBeVisible();
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
          '.trust-download__destination',
          '.product-showcase__provenance',
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
    expect(sizes).toHaveLength(8);
    expect(Math.min(...sizes)).toBeGreaterThanOrEqual(12);
  });

  test('keyboard focus stays visible and Vietnamese display text keeps its diacritics', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/vi/');
    await expect(page.locator('main h1')).toHaveText('Âm nhạc của bạn. Trong Sky.');
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

  test('both locales explain imports, compatibility and the desktop dry run once', async ({
    page,
  }) => {
    for (const route of ['/', '/vi/']) {
      await page.goto('/Sky-Auto-Player' + route);
      const features = page.locator('.home-intro .product-showcase__features');
      await expect(features.getByRole('heading', { level: 3 })).toHaveText(
        route === '/'
          ? ['Bring your own sheets', 'Check compatibility before playing']
          : ['Dùng bản nhạc của riêng bạn', 'Kiểm tra trước khi phát'],
      );
      await expect(features).toContainText(
        route === '/' ? 'original files stay where they are' : 'Các tệp gốc vẫn ở nguyên vị trí',
      );
      await expect(page.locator('#playback').getByText(/Test playback \(no input\)/)).toHaveCount(
        1,
      );
      await expect(page.locator('#timing-model')).toContainText(
        route === '/' ? 'No audio' : 'không có âm thanh',
      );
      await expect(page.locator('#how-it-works ol li')).toHaveCount(3);
      await expect(page.locator('#how-it-works ol')).toContainText('15');
      await expect(
        page.locator('#how-it-works .quick-start__setup .quick-start__resource-link'),
      ).toHaveAttribute('href', /windows-setup\/$/);
    }
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
        route === '/'
          ? 'Installs for the current Windows user'
          : 'Cài đặt cho tài khoản Windows hiện tại',
      );
      await expect(page.locator('#technical')).toContainText('_x64-setup.exe');
      await expect(page.locator('#technical')).toContainText('.exe.sig');
      await expect(page.locator('#technical')).toContainText(
        route === '/' ? 'stable Windows installer' : 'bộ cài Windows ổn định',
      );
      const trust = page.locator('#technical');
      await expect(trust.getByRole('heading', { level: 3 })).toHaveText(
        route === '/'
          ? ['Choose the right installer', 'Account safety']
          : ['Chọn đúng bộ cài', 'An toàn tài khoản'],
      );
      await expect(trust).toContainText(/SmartScreen/);
      const installation = trust.locator('.trust-download__details > div').nth(0);
      const safety = trust.locator('.trust-download__details > div').nth(1);
      await expect(installation.locator('.trust-download__detail-list > li')).toHaveCount(3);
      await expect(installation.locator('.trust-download__detail-list')).toContainText(
        route === '/'
          ? 'Choose the stable Windows installer ending in _x64-setup.exe'
          : 'Chọn bộ cài Windows ổn định có tên kết thúc bằng _x64-setup.exe',
      );
      await expect(installation.locator('.trust-download__detail-list')).toContainText('.exe.sig');
      await expect(installation.locator('.trust-download__warning')).toContainText(
        route === '/' ? 'Unsigned installer' : 'Bộ cài không có chữ ký Authenticode',
      );
      await expect(safety.locator('.trust-download__detail-list > li')).toHaveCount(2);
      await expect(safety.locator('.trust-download__detail-list')).toContainText(
        'Windows SendInput',
      );
      await expect(safety.locator('.trust-download__detail-list')).toContainText(
        route === '/'
          ? 'Does not inspect game memory or change game files'
          : 'Không đọc bộ nhớ hoặc sửa tệp game',
      );
      await expect(safety.locator('.trust-download__warning')).toHaveText(
        route === '/'
          ? 'This cannot guarantee account safety.'
          : 'Cách này không bảo đảm an toàn tài khoản.',
      );
      await expect(trust.locator('a[href*="account-safety"]')).toBeVisible();
      await expect(page.locator('#technical')).not.toContainText('100% safe');
    }
    await expect(page.locator('#faq-preview a')).toHaveCount(2);
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

  test('narrow Vietnamese legend gives event labels equal space while the track stays duration-weighted', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 320, height: 768 });
    await page.goto('/Sky-Auto-Player/vi/');
    const pixels = (selector: string) =>
      page
        .locator(selector)
        .evaluateAll((nodes) => nodes.map((node) => node.getBoundingClientRect().width));
    const legend = await pixels('[data-demo-event]');
    expect(legend).toHaveLength(3);
    expect(legend[0]).toBeCloseTo(legend[1], 1);
    expect(legend[1]).toBeCloseTo(legend[2], 1);
    const track = await pixels('[data-demo-segment]');
    expect(track).toHaveLength(3);
    expect(track[2] / track[0]).toBeCloseTo(1200 / 700, 2);
    await expect(page.locator('[data-demo-event][aria-current="step"]')).toContainText(
      'A1 + B2 + C3',
    );
  });

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
