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

  test('mobile CTA, real desktop screenshot and project favicon follow the UI contract', async ({
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
    await page.locator('.menu-toggle').click();
    await expect(page.locator('.nav-download')).toBeVisible();
    await expect(page.locator('.nav-download')).toHaveAttribute(
      'href',
      'https://github.com/pumni/Sky-Auto-Player/releases/latest',
    );
    await expect(page.locator('picture source[media="(max-width: 40rem)"]')).toHaveAttribute(
      'srcset',
      /minimum-real-tauri\.png/,
    );
    await expect(page.locator('picture source[media="(max-width: 40rem)"]')).toHaveAttribute(
      'width',
      '920',
    );
    await expect(page.locator('picture source[media="(max-width: 40rem)"]')).toHaveAttribute(
      'height',
      '620',
    );
    await expect(page.locator('.screenshot-frame__caption-meta')).toHaveText(
      'PNG · REAL TAURI WINDOW',
    );
    await expect(page.locator('.screenshot-frame img')).toHaveAttribute(
      'alt',
      'Sky Auto Player desktop Library',
    );
    const screenshot = page.locator('.screenshot-frame img');
    await expect(screenshot).toHaveJSProperty('complete', true);
    const imageMetrics = await screenshot.evaluate((element) => {
      const image = element as HTMLImageElement;
      const box = image.getBoundingClientRect();
      const frame = image.closest('.screenshot-frame')!.getBoundingClientRect();
      return {
        naturalWidth: image.naturalWidth,
        naturalHeight: image.naturalHeight,
        renderedWidth: box.width,
        renderedHeight: box.height,
        insideFrame: box.left >= frame.left && box.right <= frame.right + 1,
      };
    });
    expect(imageMetrics.naturalWidth).toBeGreaterThan(0);
    expect(imageMetrics.naturalHeight).toBeGreaterThan(0);
    expect(imageMetrics.renderedWidth).toBeGreaterThan(200);
    expect(imageMetrics.renderedHeight).toBeGreaterThan(100);
    expect(imageMetrics.insideFrame).toBe(true);
    const pixelMetrics = await screenshot.evaluate((element) => {
      const image = element as HTMLImageElement;
      const canvas = document.createElement('canvas');
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('Canvas 2D context unavailable');
      context.drawImage(image, 0, 0);
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
      let nonBlack = 0;
      const colors = new Set<string>();
      for (let index = 0; index < pixels.length; index += 4) {
        const red = pixels[index];
        const green = pixels[index + 1];
        const blue = pixels[index + 2];
        if (red + green + blue > 24) nonBlack += 1;
        if (colors.size < 128) colors.add(`${red},${green},${blue}`);
      }
      return { nonBlack, colors: colors.size };
    });
    expect(pixelMetrics.nonBlack).toBeGreaterThan(1000);
    expect(pixelMetrics.colors).toBeGreaterThan(32);
    const renderedEvidence = await page.locator('.screenshot-frame').screenshot();
    expect(renderedEvidence.byteLength).toBeGreaterThan(10_000);
    await expect(page.locator('link[rel="icon"][type="image/svg+xml"]')).toHaveAttribute(
      'href',
      /sky-auto-player-mark\.svg/,
    );
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
          '.ui-kicker',
          '.hero__annotation',
          '.proof-strip__index',
          '.screenshot-frame figcaption',
          '.ledger-row__state',
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

  test('timing stage keeps a complete static state with reduced motion', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('.timing-console')).not.toHaveClass(/is-playing/);
    await expect(page.locator('.timing-event--active')).toHaveAttribute('aria-current', 'step');
    await page.locator('[data-timing-replay]').click();
    await expect(page.locator('.timing-console')).not.toHaveClass(/is-playing/);
    await expect(page.locator('.timing-event[data-event-index="3"]')).toHaveAttribute(
      'aria-current',
      'step',
    );
  });

  test('timing illustration replays once and keeps event-to-key state legible', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const console = page.locator('[data-timing-console]');
    const replay = page.locator('[data-timing-replay]');
    await expect(replay).toHaveAccessibleName('Replay illustration');
    await replay.click();
    await expect(console).toHaveClass(/is-playing/);
    await expect(console.locator('.timing-event[aria-current="step"]')).toHaveCount(0);
    await expect(console.locator('.timing-event[data-event-index="1"]')).toHaveAttribute(
      'aria-current',
      'step',
      { timeout: 1500 },
    );
    await expect(console).not.toHaveClass(/is-playing/, { timeout: 4000 });
    await expect(console.locator('.timing-event[data-event-index="3"]')).toHaveAttribute(
      'aria-current',
      'step',
    );
  });

  test('mobile timing console shows only the relevant key excerpt', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('.instrument-key:visible')).toHaveCount(5);
    await expect(page.locator('.timing-console__events > li')).toHaveCount(3);
  });

  test('proof transition and causal score keep list semantics without nested panels', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('.proof-strip__capabilities > li')).toHaveCount(4);
    await expect(page.locator('.causal-steps > li')).toHaveCount(3);
    await expect(page.locator('.causal-diagram')).not.toHaveClass(/ui-instrument/);
  });

  test('desktop product screenshot carries more visual weight than the timing console', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/Sky-Auto-Player/');
    const screenshotWidth = await page
      .locator('.screenshot-frame img')
      .evaluate((element) => element.getBoundingClientRect().width);
    const consoleWidth = await page
      .locator('.timing-console')
      .evaluate((element) => element.getBoundingClientRect().width);
    expect(screenshotWidth).toBeGreaterThan(consoleWidth);
    expect(screenshotWidth).toBeGreaterThan(850);
  });

  test('desktop hero copy and timing console keep separate layout bounds', async ({ page }) => {
    for (const width of [1280, 1440, 1920]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/Sky-Auto-Player/');
      const headerBox = await page.locator('.site-header').boundingBox();
      const heroBox = await page.locator('.hero').boundingBox();
      const copyBox = await page.locator('.hero__copy').boundingBox();
      const consoleBox = await page.locator('.timing-console').boundingBox();

      expect(headerBox).not.toBeNull();
      expect(heroBox).not.toBeNull();
      expect(copyBox).not.toBeNull();
      expect(consoleBox).not.toBeNull();
      expect(heroBox!.y - (headerBox!.y + headerBox!.height)).toBeLessThanOrEqual(48);
      expect(
        consoleBox!.y - (headerBox!.y + headerBox!.height),
        `timing console offset below header at ${width}px`,
      ).toBeLessThanOrEqual(80);
      expect(copyBox!.x + copyBox!.width).toBeLessThan(consoleBox!.x);
      expect(consoleBox!.height, `timing console height at ${width}px`).toBeLessThan(600);
    }
  });

  test('comparison and how-it-works preserve table and ordered-list semantics', async ({
    page,
  }) => {
    await page.goto('/Sky-Auto-Player/');
    const table = page.locator('.comparison-table');
    await expect(table.locator('caption')).toHaveCount(1);
    await expect(table.locator('thead th[scope="col"]')).toHaveCount(2);
    await expect(table.locator('tbody tr')).toHaveCount(6);
    await expect(page.locator('ol.steps > li')).toHaveCount(3);
  });

  test('step and format labels do not collide with their copy', async ({ page }) => {
    for (const width of [768, 1024, 1280, 1440, 1920]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/Sky-Auto-Player/');

      const stepCollisions = await page.locator('.step').evaluateAll((steps) =>
        steps.map((step) => {
          const label = step.querySelector('.step__number')!.getBoundingClientRect();
          const copy = step.querySelector('.step__copy')!.getBoundingClientRect();
          const overlapsHorizontally = label.left < copy.right && label.right > copy.left;
          const overlapsVertically = label.top < copy.bottom && label.bottom > copy.top;
          return overlapsHorizontally && overlapsVertically;
        }),
      );
      expect(stepCollisions, `step collision state at ${width}px`).not.toContain(true);

      const formatCollisions = await page.locator('.format-row').evaluateAll((rows) =>
        rows.map((row) => {
          const extension = row.querySelector('.format-row__extension')!;
          const extensionBox = extension.getBoundingClientRect();
          const copyBox = row.querySelector('.format-row__copy')!.getBoundingClientRect();
          const tagsBox = row.querySelector('.format-row__tags')!.getBoundingClientRect();
          const intersects = (first: DOMRect, second: DOMRect) =>
            first.left < second.right &&
            first.right > second.left &&
            first.top < second.bottom &&
            first.bottom > second.top;
          return {
            extensionCopy: intersects(extensionBox, copyBox),
            copyTags: intersects(copyBox, tagsBox),
            extensionOverflow: extension.scrollWidth > extension.clientWidth + 1,
          };
        }),
      );
      expect(formatCollisions, `format collision state at ${width}px`).not.toContainEqual(
        expect.objectContaining({
          extensionCopy: true,
        }),
      );
      expect(formatCollisions, `format tag collision state at ${width}px`).not.toContainEqual(
        expect.objectContaining({
          copyTags: true,
        }),
      );
      expect(formatCollisions, `format label overflow state at ${width}px`).not.toContainEqual(
        expect.objectContaining({
          extensionOverflow: true,
        }),
      );
    }
  });

  test('technical boundary exposes explicit states and a visible risk link', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('.ledger-row__state--yes')).toHaveCount(1);
    await expect(page.locator('.ledger-row__state--no')).toHaveCount(3);
    await expect(page.locator('.notice a')).toHaveAttribute('href', /faq\/#account-safety$/);
    await expect(page.locator('.technical-section')).not.toContainText('100% safe');
  });

  test('final measure and footer retain clear actions and brand closure', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('.final-cta__measure')).toContainText('M.12 / READY');
    await expect(page.locator('.final-cta__actions a')).toHaveCount(2);
    await expect(page.locator('.site-footer__brand img')).toHaveAttribute(
      'src',
      /sky-auto-player-mark\.svg/,
    );
  });

  test('core content and navigation remain available without JavaScript', async ({ browser }) => {
    const context = await browser.newContext({ javaScriptEnabled: false });
    const page = await context.newPage();
    await page.goto('/Sky-Auto-Player/');
    await expect(page.locator('main h1')).toBeVisible();
    await expect(page.locator('.timing-event[aria-current="step"]')).toBeVisible();
    await expect(page.locator('.site-nav')).toBeVisible();
    await expect(page.locator('.menu-toggle')).toBeHidden();
    await context.close();
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
      const results = await new AxeBuilder({ page })
        // Shiki's github-dark theme uses #6a737d comments on #24292e bg (3.04:1, below 4.5:1).
        // This is a known third-party theme limitation — exclude code blocks from color-contrast.
        .exclude('pre.astro-code')
        .analyze();
      expect(results.violations).toEqual([]);
    });
  }

  test('guide page has Article JSON-LD, breadcrumb and evidence section', async ({ page }) => {
    await page.goto('/Sky-Auto-Player/guides/how-it-works/');
    // BaseLayout emits structuredData array as a single <script> tag containing a JSON array.
    // Parse each script tag and flatten any arrays to get a flat list of schema objects.
    const scripts = await page.locator('script[type="application/ld+json"]').allTextContents();
    type SchemaObj = Record<string, unknown>;
    const schemas: SchemaObj[] = scripts.flatMap((s) => {
      const parsed = JSON.parse(s) as SchemaObj | SchemaObj[];
      return Array.isArray(parsed) ? parsed : [parsed];
    });
    const article = schemas.find((s) => s['@type'] === 'Article');
    expect(article).toBeDefined();
    expect(article?.headline).toBeTruthy();
    // BreadcrumbList
    const breadcrumb = schemas.find((s) => s['@type'] === 'BreadcrumbList');
    expect(breadcrumb).toBeDefined();
    expect((breadcrumb?.itemListElement as unknown[])?.length).toBe(3);
    // H1 inside article element
    await expect(page.locator('article h1')).toBeVisible();
    // Evidence section
    await expect(page.locator('.guide-page__evidence')).toBeVisible();
  });
});
