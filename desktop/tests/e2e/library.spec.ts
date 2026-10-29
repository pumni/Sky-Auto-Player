import { expect, test, type Locator, type Page } from '@playwright/test';
import { expectNoSeriousAccessibilityViolations, startSelectedSong } from './testHelpers';

async function jumpToBuiltinPage(page: Page, target: number) {
  await page.locator('.builtin-pager-jump-trigger').click();
  const input = page.getByRole('textbox', { name: 'Go to page' });
  await input.fill(String(target));
  await input.press('Enter');
  await expect(page.getByRole('grid', { name: `Built-in Songs, page ${target}` })).toBeVisible();
  await expect(page.locator('.builtin-pager-jump-trigger')).toBeFocused();
}

function builtinPageTriggerName(page: number, pageCount: number) {
  return `${page} / ${pageCount}, Page ${page} of ${pageCount}. Go to page`;
}

async function readBuiltinPagerGeometry(page: Page) {
  return page.evaluate(() => {
    const rect = (element: Element | null) => {
      if (!(element instanceof HTMLElement)) throw new Error('Pager element is missing');
      const box = element.getBoundingClientRect();
      return {
        x: box.x,
        y: box.y,
        width: box.width,
        height: box.height,
        centerX: box.x + box.width / 2,
        centerY: box.y + box.height / 2,
      };
    };
    return {
      previous: rect(document.querySelector('[aria-label="Previous page"]')),
      indicator: rect(document.querySelector('.builtin-pager-jump-trigger')),
      next: rect(document.querySelector('[aria-label="Next page"]')),
      toolbar: rect(document.querySelector('.builtin-pager')),
      range: rect(document.querySelector('.builtin-pager-range')),
    };
  });
}

async function readTrackBrowserVisualGeometry(page: Page, includePager = true) {
  return page.evaluate((includePager) => {
    const required = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) throw new Error(`Missing visual regression element: ${selector}`);
      return element;
    };
    const rect = (element: HTMLElement) => {
      const box = element.getBoundingClientRect();
      return {
        x: box.x,
        y: box.y,
        width: box.width,
        height: box.height,
        right: box.right,
        bottom: box.bottom,
        centerX: box.x + box.width / 2,
        centerY: box.y + box.height / 2,
      };
    };
    const baseline = (element: HTMLElement) => {
      const marker = document.createElement('span');
      marker.style.cssText =
        'display:inline-block;width:0;height:0;padding:0;border:0;margin:0;vertical-align:baseline;font-size:0;line-height:0;';
      element.append(marker);
      const y = marker.getBoundingClientRect().y;
      marker.remove();
      return y;
    };

    const header = required('.track-browser-header');
    const heading = required('.track-browser-heading');
    const source = required('.track-browser-source-label');
    const title = required('.track-browser-header h1');
    const count = required('.track-browser-count');
    const wrap = includePager ? required('.builtin-pager-wrap') : null;
    const pager = includePager ? required('.builtin-pager') : null;
    const previous = includePager ? required('[aria-label="Previous page"]') : null;
    const indicator = includePager ? required('.builtin-pager-jump-trigger') : null;
    const next = includePager ? required('[aria-label="Next page"]') : null;
    const player = includePager ? required('.player-bar') : null;
    const titleStyle = getComputedStyle(title);
    const countStyle = getComputedStyle(count);
    const countSeparatorStyle = getComputedStyle(count, '::before');
    const titleTextBaseline = baseline(title);
    const countTextBaseline = baseline(count);

    const pagerMetrics =
      wrap && pager && previous && indicator && next && player
        ? (() => {
            const wrapStyle = getComputedStyle(wrap);
            const triggerStyle = getComputedStyle(indicator);
            const previousStyle = getComputedStyle(previous);
            return {
              wrap: rect(wrap),
              toolbar: rect(pager),
              previous: rect(previous),
              indicator: rect(indicator),
              next: rect(next),
              player: rect(player),
              topClearance:
                previous.getBoundingClientRect().top -
                wrap.getBoundingClientRect().top -
                Number.parseFloat(wrapStyle.borderTopWidth),
              bottomClearance:
                wrap.getBoundingClientRect().bottom -
                Number.parseFloat(wrapStyle.borderBottomWidth) -
                previous.getBoundingClientRect().bottom,
              triggerBorderColor: triggerStyle.borderTopColor,
              previousBorderColor: previousStyle.borderTopColor,
              triggerBackgroundColor: triggerStyle.backgroundColor,
              previousBackgroundColor: previousStyle.backgroundColor,
              clientWidth: wrap.clientWidth,
              scrollWidth: wrap.scrollWidth,
            };
          })()
        : null;

    return {
      viewport: { width: window.innerWidth, height: window.innerHeight },
      header: rect(header),
      heading: rect(heading),
      source: rect(source),
      title: rect(title),
      count: rect(count),
      titleTextBaseline,
      countTextBaseline,
      titleFontSize: Number.parseFloat(titleStyle.fontSize),
      titleLineHeight: Number.parseFloat(titleStyle.lineHeight),
      titleOverflow: titleStyle.overflow,
      titleTextOverflow: titleStyle.textOverflow,
      titleWhiteSpace: titleStyle.whiteSpace,
      separator: {
        content: countSeparatorStyle.content,
        fontSize: countSeparatorStyle.fontSize,
        lineHeight: countSeparatorStyle.lineHeight,
        verticalAlign: countSeparatorStyle.verticalAlign,
      },
      pager: pagerMetrics,
      documentWidth: document.documentElement.scrollWidth,
    };
  }, includePager);
}

function expectBuiltinPagerGeometryStable(
  before: Awaited<ReturnType<typeof readBuiltinPagerGeometry>>,
  after: Awaited<ReturnType<typeof readBuiltinPagerGeometry>>,
) {
  for (const control of ['previous', 'indicator', 'next'] as const) {
    expect(
      Math.abs(after[control].centerX - before[control].centerX),
      `${control} centerX: ${before[control].centerX} -> ${after[control].centerX}; ` +
        `toolbar x ${before.toolbar.x} -> ${after.toolbar.x}, ` +
        `range ${JSON.stringify(before.range)} -> ${JSON.stringify(after.range)}`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(after[control].centerY - before[control].centerY),
      `${control} centerY: ${before[control].centerY} -> ${after[control].centerY}`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(after[control].width - before[control].width),
      `${control} width: ${before[control].width} -> ${after[control].width}`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(after[control].height - before[control].height),
      `${control} height: ${before[control].height} -> ${after[control].height}`,
    ).toBeLessThanOrEqual(1);
  }
  expect(
    Math.abs(after.toolbar.y - before.toolbar.y),
    `toolbar y: ${before.toolbar.y} -> ${after.toolbar.y}`,
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(after.toolbar.height - before.toolbar.height),
    `toolbar height: ${before.toolbar.height} -> ${after.toolbar.height}`,
  ).toBeLessThanOrEqual(1);
  expect(after.range.x + after.range.width).toBeLessThanOrEqual(after.toolbar.x + 1);
  expect(Math.abs(after.range.centerY - after.toolbar.centerY)).toBeLessThanOrEqual(1);
  expect(after.previous.height).toBeGreaterThanOrEqual(32);
  expect(after.indicator.height).toBeGreaterThanOrEqual(32);
  expect(after.next.height).toBeGreaterThanOrEqual(32);
}

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

test('mock desktop vertical slice can search and inspect a song', async ({ page }) => {
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expect(navigator.getByRole('button', { name: /Built-in Songs/ })).toBeEnabled();
  await expect(navigator.getByRole('button', { name: /Liked Songs/ })).toBeEnabled();
  await expect(navigator.getByText('Playlists')).toBeVisible();
  await expect(navigator.getByRole('button', { name: /All Songs/ })).toHaveCount(0);
  await expect(navigator.getByRole('button', { name: /My Songs/ })).toHaveCount(0);
  const search = page.getByLabel('Search Built-in Songs');
  await search.focus();
  await search.fill('Moonlit');
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toHaveCount(0);
  const moonlitRow = page.getByRole('row', { name: /Moonlit Village/ });
  await expect(moonlitRow).toBeVisible();
  await expect(search).toBeFocused();
  await moonlitRow.locator('.track-title-text').click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeFocused();
  await expect(moonlitRow).toHaveAttribute('aria-selected', 'true');
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(moonlitRow).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByText('Low timing risk')).toBeVisible();
});

test('startup can recover built-in navigation while the catalog search is delayed', async ({
  page,
}) => {
  await page.goto('/?mockSearchDelayMs=1000&mockNavigationFailures=1');
  const navigator = page.getByRole('navigation', { name: 'Library' });

  await expect(page.getByRole('alert')).toContainText('Couldn’t check Built-in Songs');
  await page.getByRole('button', { name: 'Retry library navigation' }).click();
  const builtin = navigator.getByRole('button', { name: /Built-in Songs/ });
  await expect(builtin).toBeEnabled();
  await expect(builtin).toContainText('500');
  await builtin.click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
});

test('Built-in Songs pages keep local row semantics and adapt to the browser container', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.getByRole('button', { name: /Built-in Songs/ }).click();
  const grid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  await expect(grid).toHaveAttribute('aria-rowcount', '101');
  await expect(page.locator('.builtin-pager-status')).toContainText('Page 1 of 5');
  await expect(page.locator('.builtin-pager-range')).toHaveText('1–100 of 500 songs');
  await expect(page.locator('.builtin-pager')).toHaveCSS('flex-wrap', 'nowrap');

  await jumpToBuiltinPage(page, 3);
  const pageThree = page.getByRole('grid', { name: 'Built-in Songs, page 3' });
  await expect(pageThree).toHaveAttribute('aria-rowcount', '101');
  await pageThree.focus();
  await page.keyboard.press('Home');
  await expect(page.getByRole('row', { name: /Song 201/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await page.keyboard.press('End');
  await expect(page.getByRole('row', { name: /Song 300/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );

  const pageThreeIndicator = page.getByRole('button', { name: builtinPageTriggerName(3, 5) });
  await pageThreeIndicator.focus();
  await page.keyboard.press('Enter');
  await expect(pageThreeIndicator).toHaveAttribute('aria-expanded', 'true');
  const jump = page.getByRole('textbox', { name: 'Go to page' });
  await expect(jump).toBeFocused();
  for (const invalidPage of ['0', '-1', '1.5', '6', '9007199254740992']) {
    await jump.fill(invalidPage);
    await jump.press('Enter');
    await expect(page.getByRole('alert')).toContainText('Enter a whole page number from 1 to 5');
  }
  await jump.press('Escape');
  await expect(page.getByRole('textbox', { name: 'Go to page' })).toHaveCount(0);
  await expect(pageThreeIndicator).toBeFocused();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 3' })).toBeVisible();

  await pageThreeIndicator.focus();
  await page.keyboard.press('Space');
  await page.getByRole('textbox', { name: 'Go to page' }).fill('5');
  await page.getByRole('button', { name: 'Go', exact: true }).click();
  const pageFive = page.getByRole('grid', { name: 'Built-in Songs, page 5' });
  await expect(pageFive).toHaveAttribute('aria-rowcount', '101');
  await expect(page.getByRole('button', { name: builtinPageTriggerName(5, 5) })).toBeFocused();
  await pageFive.focus();
  await page.keyboard.press('End');
  await expect(page.getByRole('row', { name: /Song 500/ })).toHaveAttribute('aria-rowindex', '101');
  await expectNoSeriousAccessibilityViolations(page);

  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await expect(page.getByRole('button', { name: builtinPageTriggerName(5, 5) })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect
    .poll(() =>
      page
        .getByRole('button', { name: builtinPageTriggerName(5, 5) })
        .evaluate((button) => getComputedStyle(button).outlineStyle),
    )
    .toBe('solid');
  await page.setViewportSize({ width: 800, height: 560 });
  await expect(page.locator('.builtin-pager-range')).toBeVisible();
  await expect(page.getByRole('button', { name: builtinPageTriggerName(5, 5) })).toBeVisible();
  await expect(page.getByRole('button', { name: 'First page' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Last page' })).toHaveCount(0);
  await expect(page.locator('.builtin-pager button')).toHaveCount(3);
  const geometry = await page.evaluate(() => ({
    browserWidth:
      document.querySelector<HTMLElement>('.track-browser-workbench-pane')?.clientWidth ?? 0,
    browserScrollWidth:
      document.querySelector<HTMLElement>('.track-browser-workbench-pane')?.scrollWidth ?? 0,
    documentWidth: document.documentElement.scrollWidth,
  }));
  expect(geometry.browserScrollWidth).toBeLessThanOrEqual(geometry.browserWidth);
  expect(geometry.documentWidth).toBeLessThanOrEqual(800);

  const expectCompactJumpToFit = async () => {
    const indicator = page.getByRole('button', { name: builtinPageTriggerName(5, 5) });
    await indicator.focus();
    await page.keyboard.press('Enter');
    await expect(indicator).toHaveAttribute('aria-expanded', 'true');
    const jump = page.getByRole('textbox', { name: 'Go to page' });
    await expect(jump).toBeVisible();
    const jumpGeometry = await jump.evaluate((input) => {
      const pane = document.querySelector<HTMLElement>('.track-browser-workbench-pane');
      const player = document.querySelector<HTMLElement>('.player-bar');
      const popover = input.closest<HTMLElement>('.builtin-pager-popover');
      const toolbar = document.querySelector<HTMLElement>('.builtin-pager');
      if (!pane || !player || !popover || !toolbar) return null;
      const paneRect = pane.getBoundingClientRect();
      const playerRect = player.getBoundingClientRect();
      const popoverRect = popover.getBoundingClientRect();
      const toolbarRect = toolbar.getBoundingClientRect();
      return {
        paneLeft: paneRect.left,
        paneRight: paneRect.right,
        paneTop: paneRect.top,
        paneBottom: paneRect.bottom,
        popoverLeft: popoverRect.left,
        popoverRight: popoverRect.right,
        popoverTop: popoverRect.top,
        popoverBottom: popoverRect.bottom,
        popoverWidth: popoverRect.width,
        popoverScrollWidth: popover.scrollWidth,
        toolbarTop: toolbarRect.top,
        playerTop: playerRect.top,
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight,
      };
    });
    expect(jumpGeometry).not.toBeNull();
    if (jumpGeometry) {
      expect(jumpGeometry.popoverLeft).toBeGreaterThanOrEqual(0);
      expect(jumpGeometry.popoverRight).toBeLessThanOrEqual(jumpGeometry.viewportWidth);
      expect(jumpGeometry.popoverTop).toBeGreaterThanOrEqual(0);
      expect(jumpGeometry.popoverBottom).toBeLessThanOrEqual(jumpGeometry.viewportHeight);
      expect(jumpGeometry.popoverLeft).toBeGreaterThanOrEqual(jumpGeometry.paneLeft);
      expect(jumpGeometry.popoverRight).toBeLessThanOrEqual(jumpGeometry.paneRight);
      expect(jumpGeometry.popoverTop).toBeLessThan(jumpGeometry.toolbarTop);
      expect(jumpGeometry.popoverBottom).toBeLessThanOrEqual(jumpGeometry.toolbarTop);
      expect(jumpGeometry.popoverBottom).toBeLessThanOrEqual(jumpGeometry.playerTop);
      expect(jumpGeometry.popoverScrollWidth).toBeLessThanOrEqual(jumpGeometry.popoverWidth);
    }
    await jump.press('Escape');
    await expect(indicator).toBeFocused();
  };

  await page.setViewportSize({ width: 1366, height: 768 });
  await expectCompactJumpToFit();
  await page.setViewportSize({ width: 800, height: 560 });
  await expectCompactJumpToFit();

  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.builtin-pager-range')).toBeVisible();
  await expect(page.locator('.builtin-pager')).toBeVisible();
  await expectCompactJumpToFit();

  await page.setViewportSize({ width: 1366, height: 768 });
  const browserPane = page.locator('.track-browser-workbench-pane');
  await browserPane.evaluate((element) => {
    element.style.flex = '0 0 520px';
    element.style.minWidth = '0';
  });
  await expect(page.locator('.builtin-pager-range')).toBeVisible();
  await expect(page.getByRole('button', { name: builtinPageTriggerName(5, 5) })).toBeVisible();
  const pagerGeometry = await page.locator('.builtin-pager').evaluate((element) => ({
    width: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(pagerGeometry.scrollWidth).toBeLessThanOrEqual(pagerGeometry.width);
  await expectCompactJumpToFit();
  await browserPane.evaluate((element) => {
    element.style.flex = '0 0 440px';
  });
  const narrowPagerGeometry = await page.locator('.builtin-pager-wrap').evaluate((element) => ({
    width: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(narrowPagerGeometry.scrollWidth).toBeLessThanOrEqual(narrowPagerGeometry.width);
  await expectCompactJumpToFit();
  await browserPane.evaluate((element) => {
    element.style.removeProperty('flex');
    element.style.removeProperty('min-width');
  });
});

test('Built-in pager controls keep their geometry across navigation and jump popover changes', async ({
  page,
}) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.getByRole('button', { name: /Built-in Songs/ }).click();
  const previous = page.getByRole('button', { name: 'Previous page' });
  const next = page.getByRole('button', { name: 'Next page' });
  const indicator = page.locator('.builtin-pager-jump-trigger');
  await expect(previous).toBeVisible();
  await expect(next).toBeVisible();
  await expect(indicator).toBeVisible();

  const pageOne = await readBuiltinPagerGeometry(page);
  await indicator.click();
  await expect(page.getByRole('textbox', { name: 'Go to page' })).toBeVisible();
  const popupOpen = await readBuiltinPagerGeometry(page);
  expectBuiltinPagerGeometryStable(pageOne, popupOpen);
  await page.getByRole('textbox', { name: 'Go to page' }).press('Escape');
  await expect(indicator).toBeFocused();
  expectBuiltinPagerGeometryStable(pageOne, await readBuiltinPagerGeometry(page));

  await next.click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  expectBuiltinPagerGeometryStable(pageOne, await readBuiltinPagerGeometry(page));
  await page.getByRole('button', { name: 'Previous page' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  expectBuiltinPagerGeometryStable(pageOne, await readBuiltinPagerGeometry(page));
});

test('F3.1 heading typography and pager alignment stay clear at desktop and compact widths', async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/?mockCatalogCount=106');
  await expect(page.getByRole('heading', { name: 'Built-in Songs' })).toBeVisible();
  await expect(page.locator('.track-browser-count')).toHaveText('106 songs');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  const visualMeasurements: Array<{
    label: string;
    metrics: Awaited<ReturnType<typeof readTrackBrowserVisualGeometry>>;
  }> = [];

  const assertVisualContract = async (label: string) => {
    const metrics = await readTrackBrowserVisualGeometry(page);
    visualMeasurements.push({ label, metrics });
    const pager = metrics.pager;
    if (!pager) throw new Error(`${label}: pager geometry is required`);
    const baselineDelta = Math.abs(metrics.titleTextBaseline - metrics.countTextBaseline);
    const pagerClearanceDelta = Math.abs(pager.topClearance - pager.bottomClearance);
    const failures: string[] = [];
    const check = (condition: boolean, reason: string) => {
      if (!condition) failures.push(reason);
    };

    check(metrics.titleLineHeight >= metrics.titleFontSize * 1.39, 'short title line box');
    check(baselineDelta <= 0.5, 'title/count separator baseline mismatch');
    check(
      metrics.titleOverflow === 'hidden' &&
        metrics.titleTextOverflow === 'ellipsis' &&
        metrics.titleWhiteSpace === 'nowrap',
      'title truncation contract changed',
    );
    check(metrics.source.y >= metrics.header.y, 'label crosses header top');
    check(metrics.source.bottom <= metrics.header.bottom, 'label crosses header bottom');
    check(metrics.title.y >= metrics.source.y - 1, 'title line box crosses label top');
    check(metrics.title.bottom <= metrics.source.bottom + 1, 'title line box crosses label bottom');
    check(metrics.heading.right <= metrics.header.right + 1, 'heading crosses header right');
    check(metrics.title.right <= metrics.count.x + 1, 'title intrudes into count');
    check(metrics.separator.content === '"·"', 'separator dot missing');
    check(
      metrics.separator.verticalAlign === 'baseline',
      'separator uses a non-baseline alignment',
    );
    check(
      pager.triggerBorderColor === pager.previousBorderColor,
      'resting trigger border is accented',
    );
    check(
      pager.triggerBackgroundColor === pager.previousBackgroundColor,
      'resting trigger surface is accented',
    );
    check(pagerClearanceDelta <= 1, 'pager top/bottom clearance is asymmetric');
    check(
      Math.min(pager.previous.height, pager.indicator.height, pager.next.height) >= 32,
      'pager target is shorter than 32px',
    );
    check(pager.scrollWidth <= pager.clientWidth, 'pager overflows horizontally');
    check(metrics.documentWidth <= metrics.viewport.width, 'document overflows horizontally');
    check(pager.wrap.bottom <= pager.player.y + 1, 'pager overlaps the player dock');

    const facts = {
      viewport: metrics.viewport,
      title: {
        fontSize: metrics.titleFontSize,
        lineHeight: metrics.titleLineHeight,
        baselineDelta,
        overflow: metrics.titleOverflow,
        textOverflow: metrics.titleTextOverflow,
      },
      count: { textBaseline: metrics.countTextBaseline, separator: metrics.separator },
      pager: {
        topClearance: pager.topClearance,
        bottomClearance: pager.bottomClearance,
        clearanceDelta: pagerClearanceDelta,
        triggerBorder: pager.triggerBorderColor,
        previousBorder: pager.previousBorderColor,
        triggerBackground: pager.triggerBackgroundColor,
        previousBackground: pager.previousBackgroundColor,
        height: pager.toolbar.height,
      },
    };
    expect.soft(failures, `${label}: ${JSON.stringify(facts)}`).toEqual([]);
  };

  const viewportCases = [
    { width: 995, height: 706, label: 'reference 995x706' },
    { width: 1366, height: 768, label: 'wide 1366x768' },
    { width: 800, height: 560, label: 'compact 800x560' },
    { width: 640, height: 448, label: 'compact 640x448' },
  ];

  for (const viewport of viewportCases) {
    await page.setViewportSize(viewport);
    await expect(page.locator('.workbench')).toHaveAttribute(
      'data-workbench-width',
      String(viewport.width),
    );
    const navigatorPane = page.locator('.navigator-workbench-pane');
    if (viewport.width <= 640) {
      await expect(navigatorPane).toHaveClass(/is-collapsed/);
    } else {
      await expect(navigatorPane).not.toHaveClass(/is-collapsed/);
    }
    await page.evaluate(
      () =>
        new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
    );
    await assertVisualContract(viewport.label);
    const pageOneGeometry = await readBuiltinPagerGeometry(page);
    await page.getByRole('button', { name: 'Next page' }).click();
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
    await page.getByRole('button', { name: 'Previous page' }).focus();
    await page.keyboard.press('Tab');
    const indicator = page.getByRole('button', { name: builtinPageTriggerName(2, 2) });
    await expect(indicator).toBeFocused();
    await expect
      .poll(() => indicator.evaluate((button) => getComputedStyle(button).outlineStyle))
      .toBe('solid');
    await assertVisualContract(`${viewport.label} page 2 focused`);
    await indicator.click();
    await expect(page.getByRole('textbox', { name: 'Go to page' })).toBeVisible();
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
    await page.getByRole('textbox', { name: 'Go to page' }).press('Escape');
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
    await page.getByRole('button', { name: 'Previous page' }).click();
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
  }

  await page.setViewportSize({ width: 1366, height: 768 });
  const browserPane = page.locator('.track-browser-workbench-pane');
  for (const width of [520, 440]) {
    await browserPane.evaluate((element, constrainedWidth) => {
      element.style.flex = `0 0 ${constrainedWidth}px`;
      element.style.minWidth = '0';
    }, width);
    await assertVisualContract(`track-browser container ${width}px`);
  }
  await browserPane.evaluate((element) => {
    element.style.removeProperty('flex');
    element.style.removeProperty('min-width');
  });

  await page.setViewportSize({ width: 640, height: 448 });
  const mixedPlaylistTitle =
    'Bản tình ca của Nguyễn Ánh — The Long Evening Song Collection for Practice';
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill(mixedPlaylistTitle);
  await createDialog.getByRole('button', { name: 'Create' }).click();
  const mixedHeading = page.getByRole('heading', { name: mixedPlaylistTitle });
  await expect(mixedHeading).toBeVisible();
  await expect(mixedHeading).toHaveAttribute('title', mixedPlaylistTitle);
  const playlistMetrics = await readTrackBrowserVisualGeometry(page, false);
  visualMeasurements.push({
    label: 'mixed Vietnamese playlist at 640x448',
    metrics: playlistMetrics,
  });
  expect(playlistMetrics.titleLineHeight).toBeGreaterThanOrEqual(
    playlistMetrics.titleFontSize * 1.39,
  );
  expect(
    Math.abs(playlistMetrics.titleTextBaseline - playlistMetrics.countTextBaseline),
  ).toBeLessThanOrEqual(1);
  expect(playlistMetrics.title.bottom).toBeLessThanOrEqual(playlistMetrics.source.bottom + 1);
  expect(playlistMetrics.title.right).toBeLessThanOrEqual(playlistMetrics.count.x + 1);

  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/?mockCatalogCount=106');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath('f3.1-after-995x706.png'),
    animations: 'disabled',
  });
  visualMeasurements.push({
    label: 'after screenshot 995x706',
    metrics: await readTrackBrowserVisualGeometry(page),
  });
  await import('node:fs/promises').then(({ writeFile }) =>
    writeFile(
      testInfo.outputPath('f3.1-geometry-report.json'),
      `${JSON.stringify(visualMeasurements, null, 2)}\n`,
    ),
  );
});

test('Built-in pager handles the approved result-count boundaries', async ({ page }) => {
  for (const count of [0, 1, 96, 100, 101, 106, 500, 2438, 10_000]) {
    await page.goto(`/?mockCatalogCount=${count}`);
    if (count === 0) {
      await expect(page.getByText('The built-in song catalog is empty.')).toBeVisible();
      await expect(page.locator('.builtin-pager')).toHaveCount(0);
      await expect(page.getByText(/Page 1 of 0/)).toHaveCount(0);
      continue;
    }

    const firstLast = Math.min(100, count);
    await expect(page.locator('.builtin-pager-range')).toHaveText(
      `1–${firstLast} of ${count} songs`,
    );
    if (count <= 100) {
      await expect(page.locator('.builtin-pager')).toHaveCount(0);
      await expect(page.getByRole('button', { name: /Go to page/ })).toHaveCount(0);
      continue;
    }

    const pageCount = Math.ceil(count / 100);
    const pager = page.getByRole('navigation', { name: 'Built-in Songs pages' });
    await expect(pager.locator('button')).toHaveCount(3);
    await expect(page.getByRole('button', { name: 'Previous page' })).toBeDisabled();
    await expect(
      page.getByRole('button', { name: builtinPageTriggerName(1, pageCount) }),
    ).toHaveAttribute('aria-current', 'page');
    await expect(page.getByRole('button', { name: 'Next page' })).toBeEnabled();
    await expect(page.getByRole('button', { name: 'First page' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Last page' })).toHaveCount(0);

    if (count === 106) {
      await page.getByRole('button', { name: 'Next page' }).click();
      await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toHaveAttribute(
        'aria-rowcount',
        '7',
      );
      await expect(page.getByRole('row', { name: /Song 101/ })).toHaveAttribute(
        'aria-rowindex',
        '2',
      );
      await expect(page.locator('.builtin-pager-range')).toHaveText('101–106 of 106 songs');
    }
  }
});

test('Built-in pager control centers stay fixed across 9→10 and 99→100', async ({ page }) => {
  for (const transition of [
    { total: 2438, from: 9, to: 10 },
    { total: 10_000, from: 99, to: 100 },
  ]) {
    await page.goto(`/?mockCatalogCount=${transition.total}`);
    await jumpToBuiltinPage(page, transition.from);
    const before = await readBuiltinPagerGeometry(page);
    await page.getByRole('button', { name: 'Next page' }).click();
    await expect(
      page.getByRole('grid', { name: `Built-in Songs, page ${transition.to}` }),
    ).toBeVisible();
    const after = await readBuiltinPagerGeometry(page);
    expectBuiltinPagerGeometryStable(before, after);
    await expect(page.locator('.builtin-pager-jump-trigger')).toHaveAccessibleName(
      builtinPageTriggerName(transition.to, Math.ceil(transition.total / 100)),
    );
  }
});

test('Show now playing scrolls to a deep built-in row across and within pages', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.getByRole('button', { name: /Built-in Songs/ }).click();
  await jumpToBuiltinPage(page, 2);

  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const table = page.locator('.track-table');
  const song175 = page.getByRole('row', { name: /Song 175/ });
  await table.evaluate((element) => {
    element.scrollTop = 74 * 46;
  });
  await expect(song175).toBeInViewport();
  await song175.locator('.track-title-text').click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 175');

  const song180 = page.getByRole('row', { name: /Song 180/ });
  await table.evaluate((element) => {
    element.scrollTop = 79 * 46;
  });
  await expect(song180).toBeInViewport();
  await song180.locator('.track-title-text').click();
  await expect(song180).toHaveAttribute('aria-selected', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 175');

  await jumpToBuiltinPage(page, 1);
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expect.poll(() => table.evaluate((element) => element.scrollTop)).toBe(0);

  await page.getByRole('button', { name: 'Show now playing, row 175' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  await expect(song175).toBeInViewport();
  await expect(song180).toHaveAttribute('aria-selected', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 175');

  await table.evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect.poll(() => table.evaluate((element) => element.scrollTop)).toBe(0);
  await page.getByRole('button', { name: 'Show now playing, row 175' }).click();
  await expect(song175).toBeInViewport();
  await expect(song180).toHaveAttribute('aria-selected', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 175');

  await jumpToBuiltinPage(page, 1);
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expect.poll(() => table.evaluate((element) => element.scrollTop)).toBe(0);
});

test('Built-in playback crosses page boundaries without replacing selection', async ({ page }) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await page
    .getByRole('navigation', { name: 'Library' })
    .getByRole('button', { name: /Built-in Songs/ })
    .click();

  const pageOne = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  const song100 = page.getByRole('row', { name: /Song 100/ });
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const table = page.locator('.track-table');
  await expect(pageOne).toBeVisible();
  await table.evaluate((element) => {
    element.scrollTop = 99 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  await expect(song100).toBeInViewport();
  await song100.locator('.track-title-text').click();
  await expect(song100).toHaveAttribute('aria-selected', 'true');
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const initialProceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await initialProceed.isVisible().catch(() => false)) await initialProceed.click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 100');

  await player.getByRole('button', { name: 'Next' }).click();
  const nextProceed = page.getByRole('button', { name: 'Proceed with current settings' });
  if (await nextProceed.isVisible().catch(() => false)) await nextProceed.click();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 101');
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(pageOne).toBeVisible();
  await expect(song100).toHaveAttribute('aria-selected', 'true');
  await expect(song100).not.toHaveAttribute('aria-current', 'true');

  await page.getByRole('button', { name: 'Show now playing, row 101' }).click();
  const pageTwo = page.getByRole('grid', { name: 'Built-in Songs, page 2' });
  const song101 = page.getByRole('row', { name: /Song 101/ });
  await expect(pageTwo).toBeVisible();
  await expect(song101).toBeInViewport();
  await expect(song101).toHaveAttribute('aria-current', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 101');

  await page.getByRole('button', { name: 'Previous page' }).click();
  await expect(pageOne).toBeVisible();
  await table.evaluate((element) => {
    element.scrollTop = 99 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  await expect(song100).toBeInViewport();
  await expect(song100).toHaveAttribute('aria-selected', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 101');
  await page.getByRole('button', { name: 'Show now playing, row 101' }).click();
  await expect(pageTwo).toBeVisible();
  await expect(song101).toBeInViewport();
  await expect(song101).toHaveAttribute('aria-current', 'true');
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 101');
});

test('rapid A-B-A search keeps the newest query results visible', async ({ page }) => {
  await page.goto('/?mockSearchDelayMs=1000');
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  const search = page.getByLabel('Search Built-in Songs');

  await search.fill('Aurora');
  await expect(page.locator('[role="status"][aria-busy="true"]')).toBeVisible();
  await search.fill('Moonlit');
  await page.waitForTimeout(150);
  await search.fill('Aurora');

  const auroraRow = page.getByRole('row', { name: /Aurora Landing/ });
  await expect(auroraRow).toBeVisible({ timeout: 5_000 });
  await expect(page.getByRole('row', { name: /Moonlit Village/ })).toHaveCount(0);
  await expect(search).toHaveValue('Aurora');
  await expect(search).toBeFocused();
});

test('search clears browser selection while Now Playing remains on the active song', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const playingRow = page.getByRole('row', { name: /Blue Bird/ });
  await playingRow.locator('.track-title-text').click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');

  const search = page.getByLabel('Search Built-in Songs');
  await search.fill('Moonlit');
  const browsedRow = page.getByRole('row', { name: /Moonlit Village/ });
  await expect(browsedRow).toBeVisible();
  await expect(playingRow).toHaveCount(0);
  await expect(search).toBeFocused();
  await expect(page.getByText('Select a song to view details.')).toBeVisible();
  await expect(browsedRow).toHaveAttribute('aria-selected', 'false');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();

  await browsedRow.locator('.track-title-text').click();
  await expect(browsedRow).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('heading', { name: 'Moonlit Village' })).toBeVisible();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
});

test('row selection returns focus to the grid and Like and menu actions stay isolated', async ({
  page,
}) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill('Practice');
  await createDialog.getByRole('button', { name: 'Create' }).click();
  await expect(createDialog).toBeHidden();
  await navigator.getByRole('button', { name: 'Built-in Songs' }).click();

  const row = page.getByRole('row', { name: /Aurora Landing/ });
  const grid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  await row.locator('.track-title-text').click();
  await expect(row).toHaveAttribute('aria-selected', 'true');
  await expect(grid).toBeFocused();
  await page.keyboard.press('ArrowDown');
  const nextRow = page.getByRole('row', { name: /Blue Bird/ });
  await expect(nextRow).toHaveAttribute('aria-selected', 'true');
  await expect(grid).toBeFocused();

  const unselectedRow = page.getByRole('row', { name: /Candle Run/ });
  const actionsButton = unselectedRow.getByRole('button', { name: 'More actions for Candle Run' });
  await actionsButton.click();
  const menu = page.getByRole('menu', { name: 'Actions for Candle Run' });
  await menu.getByRole('menuitem', { name: 'Add to Practice' }).click();
  await expect(unselectedRow).toHaveAttribute('aria-selected', 'false');
  await expect(nextRow).toHaveAttribute('aria-selected', 'true');
  await expect(actionsButton).toBeFocused();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');

  await unselectedRow.getByRole('button', { name: 'Add Candle Run to Liked Songs' }).click();
  await expect(unselectedRow).toHaveAttribute('aria-selected', 'false');
  await expect(nextRow).toHaveAttribute('aria-selected', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(
    unselectedRow.getByRole('button', { name: 'Remove Candle Run from Liked Songs' }),
  ).toHaveAttribute('aria-pressed', 'true');
});

test('playlist onboarding exposes create and import actions in expanded and collapsed layouts', async ({
  page,
}) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });

  for (const [width, height] of [
    [640, 448],
    [800, 560],
    [1366, 768],
  ] as const) {
    await page.setViewportSize({ width, height });
    const expandedToggle = navigator.getByRole('button', {
      name: 'Collapse library navigator',
    });
    const expandToggle = navigator.getByRole('button', {
      name: 'Expand library navigator',
    });
    const emptyGuidance = navigator.locator('.library-empty-playlists');
    if (width === 640) {
      await expect(expandToggle).toBeVisible();
      await expect(page.locator('.track-browser-guidance')).toHaveCount(0);
    } else {
      await expect(expandedToggle).toBeVisible();
      await expect(emptyGuidance.locator('p')).toHaveCount(0);
      await expect(emptyGuidance.getByRole('button', { name: 'Create playlist' })).toBeVisible();
      await expandedToggle.click();
      await expect(expandToggle).toBeVisible();
    }
    const collapsedCreate = navigator.locator('.library-navigator-footer .library-add-button');
    await expect(collapsedCreate).toBeVisible();
    await collapsedCreate.click();
    const createDialog = page.getByRole('dialog', { name: 'New playlist' });
    await expect(createDialog).toBeVisible();
    await createDialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(createDialog).toBeHidden();
    await expect(collapsedCreate).toBeFocused();
    if (width > 640) await expect(expandToggle).toBeVisible();

    if (width > 640) {
      await expandToggle.click();
    }
    const geometry = await page.evaluate(() => ({
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
      documentHeight: document.documentElement.scrollHeight,
      viewportHeight: window.innerHeight,
    }));
    expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewportWidth);
    expect(geometry.documentHeight).toBeLessThanOrEqual(geometry.viewportHeight);
  }
});

test('empty catalog keeps playlist onboarding while a populated catalog stays uncluttered', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  await expect(page.locator('.track-browser-guidance')).toHaveCount(0);
  await expect(page.locator('.library-empty-playlists p')).toHaveCount(0);

  await page.goto('/?mockLibraryState=empty');
  await expect(page.locator('.empty-state strong')).toHaveText('No Built-in Songs are available');
  await expect(page.locator('.track-browser-guidance')).toContainText(
    'Create a playlist to import local sheets.',
  );
  await expect(page.locator('.library-empty-playlists p')).toContainText(
    'import local sheet files',
  );
  await expectNoSeriousAccessibilityViolations(page);
});

test('populated library keeps playlist actions available without repeated onboarding text', async ({
  page,
}) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });

  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.track-browser-guidance')).toHaveCount(0);
  await page.setViewportSize({ width: 1366, height: 768 });
  await expect(navigator.locator('.library-empty-playlists p')).toHaveCount(0);
  await expect(
    navigator.locator('.library-empty-playlists').getByRole('button', { name: 'Create playlist' }),
  ).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  await page.setViewportSize({ width: 640, height: 448 });
  await navigator.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill('Practice');
  await createDialog.getByRole('button', { name: 'Create' }).click();
  await expect(createDialog).toBeHidden();
  await expect(navigator.locator('.library-add-button')).toBeFocused();
  await expect(page.getByRole('heading', { name: 'Practice' })).toBeVisible();
  await expect(page.getByText('This playlist is empty')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add songs' })).toBeVisible();
  await page.setViewportSize({ width: 1366, height: 768 });
  await expectNoSeriousAccessibilityViolations(page);

  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Browse Built-in Songs…' }).click();
  await expect(page.getByRole('heading', { name: 'Add songs to Practice' })).toBeVisible();
  const addSearch = page.getByRole('searchbox', {
    name: 'Search Built-in Songs to add to Practice',
  });
  await addSearch.fill('Moonlit');
  const catalogRow = page.getByRole('row', { name: /Moonlit Village/ });
  await expect(catalogRow).toBeVisible();
  await expect(addSearch).toBeFocused();
  await page.getByRole('button', { name: 'Back to playlist' }).click();
  await expect(page.getByText('This playlist is empty')).toBeVisible();

  await navigator.getByRole('button', { name: 'Built-in Songs' }).click();
  const player = await startSelectedSong(page);
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();

  await navigator.getByRole('button', { name: 'Practice', exact: true }).click();
  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Import files…' }).click();
  await expect(page.getByRole('status')).toHaveText('Import complete: 1 song found in Practice.');
  await expect(page.getByRole('row', { name: /Local Song B/ })).toBeVisible();
  await expect(page.locator('.track-browser-count')).toHaveText('1 song');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.getByRole('contentinfo', { name: 'Player controls' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Local Song B/ })).toBeVisible();
});

test('compact playlist source keeps Add, Import, Back, long-name access, and hit targets', async ({
  page,
}) => {
  const playlistName = 'Evening Practice Playlist With A Long Descriptive Name';
  await page.setViewportSize({ width: 640, height: 448 });
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill(playlistName);
  await createDialog.getByRole('button', { name: 'Create' }).click();

  const heading = page.getByRole('heading', { name: playlistName });
  const addButton = page.getByRole('button', { name: 'Add songs' });
  await expect(heading).toBeVisible();
  await expect(heading).toHaveAttribute('title', playlistName);
  await expect(addButton).toBeVisible();
  await expect(addButton).toBeEnabled();
  const headerLayout = await page.evaluate(() => {
    const header = document.querySelector<HTMLElement>('.track-browser-header');
    const label = document.querySelector<HTMLElement>('.track-browser-source-label');
    const count = document.querySelector<HTMLElement>('.track-browser-count');
    const add = document.querySelector<HTMLElement>('.playlist-add-songs-button');
    if (!header || !label || !count || !add) return null;
    const box = (element: HTMLElement) => {
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    return { header: box(header), label: box(label), count: box(count), add: box(add) };
  });
  expect(headerLayout).not.toBeNull();
  if (headerLayout) {
    expect(headerLayout.header.height).toBe(32);
    expect(headerLayout.label.right).toBeLessThanOrEqual(headerLayout.add.x);
    expect(headerLayout.count.right).toBeLessThanOrEqual(headerLayout.add.x);
    expect(headerLayout.label.y).toBeGreaterThanOrEqual(headerLayout.header.y);
    expect(headerLayout.label.bottom).toBeLessThanOrEqual(headerLayout.header.bottom);
  }

  await addButton.focus();
  await page.keyboard.press('Enter');
  const menu = page.getByRole('menu', { name: 'Add songs' });
  const firstMenuItem = menu.getByRole('menuitem', { name: 'Browse Built-in Songs…' });
  await expect(firstMenuItem).toBeVisible();
  await expect(firstMenuItem).toBeFocused();
  await expectNoSeriousAccessibilityViolations(page);
  const menuBounds = await menu.boundingBox();
  expect(menuBounds).not.toBeNull();
  if (menuBounds) {
    expect(menuBounds.x).toBeGreaterThanOrEqual(0);
    expect(menuBounds.y).toBeGreaterThanOrEqual(0);
    expect(menuBounds.x + menuBounds.width).toBeLessThanOrEqual(640);
    expect(menuBounds.y + menuBounds.height).toBeLessThanOrEqual(448);
  }
  for (const name of ['Browse Built-in Songs…', 'Import files…', 'Import folder…']) {
    const item = menu.getByRole('menuitem', { name });
    await expect(item).toBeVisible();
    const box = await item.boundingBox();
    expect(box).not.toBeNull();
    if (box) {
      const receivesPointer = await page.evaluate(
        ({ x, y }) => document.elementFromPoint(x, y)?.closest('[role="menuitem"]') !== null,
        { x: box.x + box.width / 2, y: box.y + box.height / 2 },
      );
      expect(receivesPointer).toBe(true);
    }
  }

  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(addButton).toBeFocused();
  await expectNoSeriousAccessibilityViolations(page);

  await addButton.click();
  await menu.getByRole('menuitem', { name: 'Import files…' }).click();
  await expect(page.getByRole('status')).toHaveText(
    'Import complete: 1 song found in Evening Practice Playlist With A Long Descriptive Name.',
  );
  await expect(page.getByRole('row', { name: /Local Song B/ })).toBeVisible();

  await addButton.click();
  await page.getByRole('menuitem', { name: 'Browse Built-in Songs…' }).click();
  const addHeading = page.getByRole('heading', {
    name: `Add songs to ${playlistName}`,
  });
  await expect(addHeading).toBeVisible();
  await expect(addHeading).toHaveAttribute('title', `Add songs to ${playlistName}`);
  const backButton = page.getByRole('button', { name: 'Back to playlist' });
  await expect(backButton).toBeVisible();
  await expect(page.getByText('500 songs in Built-in Songs', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('searchbox', { name: `Search Built-in Songs to add to ${playlistName}` }),
  ).toBeVisible();
  const addModeHeader = await page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    return {
      header: box('.track-browser-header'),
      back: box('.track-browser-back-button'),
      source: box('.track-browser-source-label'),
      count: box('.track-browser-count'),
    };
  });
  expect(addModeHeader.header?.height).toBe(32);
  expect(addModeHeader.back).not.toBeNull();
  expect(addModeHeader.source).not.toBeNull();
  expect(addModeHeader.count).not.toBeNull();
  if (addModeHeader.back && addModeHeader.source && addModeHeader.count && addModeHeader.header) {
    expect(addModeHeader.back.right).toBeLessThanOrEqual(addModeHeader.source.x);
    expect(addModeHeader.source.right).toBeLessThanOrEqual(addModeHeader.count.x);
    expect(addModeHeader.count.right).toBeLessThanOrEqual(addModeHeader.header.right);
    expect(addModeHeader.back.y).toBeGreaterThanOrEqual(addModeHeader.header.y);
    expect(addModeHeader.back.bottom).toBeLessThanOrEqual(addModeHeader.header.bottom);
  }
  const backHitTargeted = await backButton.evaluate((button) => {
    const rect = button.getBoundingClientRect();
    const target = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
    return target === button || button.contains(target);
  });
  expect(backHitTargeted).toBe(true);
  await backButton.click();
  await expect(heading).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
});

test('playlist Built-in picker pages through song 101 and adds it to the same playlist', async ({
  page,
}) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill('Practice');
  await createDialog.getByRole('button', { name: 'Create' }).click();
  await expect(page.getByRole('heading', { name: 'Practice' })).toBeVisible();

  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Browse Built-in Songs…' }).click();
  await expect(page.getByRole('heading', { name: 'Add songs to Practice' })).toBeVisible();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await jumpToBuiltinPage(page, 2);

  const song101 = page.getByRole('row', { name: /Song 101/ });
  const pageTwo = page.getByRole('grid', { name: 'Built-in Songs, page 2' });
  await expect(pageTwo).toBeVisible();
  await expect(pageTwo).toHaveAttribute('aria-rowcount', '101');
  await expect(page.locator('.builtin-pager-jump-trigger')).toBeFocused();
  await expect(song101).toBeVisible();
  await expect(song101).toHaveAttribute('aria-rowindex', '2');
  await expectNoSeriousAccessibilityViolations(page);
  await song101.getByRole('button', { name: 'More actions for Song 101' }).click();
  await page
    .getByRole('menu', { name: 'Actions for Song 101' })
    .getByRole('menuitem', { name: 'Add to Practice' })
    .click();

  await expect(page.getByRole('heading', { name: 'Add songs to Practice' })).toBeVisible();
  await page.getByRole('button', { name: 'Back to playlist' }).click();
  await expect(page.getByRole('heading', { name: 'Practice' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Song 101/ })).toBeVisible();
  await expect(page.locator('.track-browser-count')).toHaveText('1 song');
});

test('idle Graphite selection stays distinct from native Now Playing', async ({ page }) => {
  const selectedRow = page.getByRole('row', { name: /Blue Bird/ });
  const playingRow = page.getByRole('row', { name: /Candle Run/ });
  const player = page.getByRole('contentinfo', { name: 'Player controls' });

  await selectedRow.click();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await expect(playingRow).not.toHaveAttribute('aria-current', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(player.locator('.player-track-status')).toHaveText('Ready to play');
  await expect(player.locator('.player-track-status')).toHaveCount(1);

  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await expect(page.locator('.song-details-context')).toHaveCount(0);
});

test('selected, hovered, and focused rows reveal actions without showing them on every row', async ({
  page,
}) => {
  const firstRow = page.getByRole('row', { name: /Aurora Landing/ });
  const firstMenu = firstRow.getByRole('button', { name: 'More actions for Aurora Landing' });
  const firstLike = firstRow.getByRole('button', { name: 'Add Aurora Landing to Liked Songs' });
  const otherRow = page.getByRole('row', { name: /Blue Bird/ });
  const otherMenu = otherRow.getByRole('button', { name: 'More actions for Blue Bird' });
  const otherLike = otherRow.getByRole('button', { name: 'Add Blue Bird to Liked Songs' });

  await expect
    .poll(() => firstMenu.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('0');
  await expect
    .poll(() => firstLike.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('0');

  await firstRow.click();
  await expect(firstRow).toHaveAttribute('aria-selected', 'true');
  await expect
    .poll(() => firstMenu.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('1');
  await expect
    .poll(() => firstLike.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('1');

  await otherRow.hover();
  await expect
    .poll(() => otherMenu.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('1');
  await expect
    .poll(() => otherLike.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('1');
  const likeBox = await otherLike.boundingBox();
  expect(likeBox).not.toBeNull();
  if (likeBox) {
    await expect
      .poll(() =>
        page.evaluate(
          ({ x, y }) => document.elementFromPoint(x, y)?.closest('.track-like-button') !== null,
          { x: likeBox.x + likeBox.width / 2, y: likeBox.y + likeBox.height / 2 },
        ),
      )
      .toBe(true);
  }
  await otherLike.click();
  const toggledLike = otherRow.locator('.track-like-button');
  await expect(toggledLike).toHaveAttribute('aria-pressed', 'true');
  await expect(otherRow).toHaveAttribute('aria-selected', 'false');
  await expect(firstRow).toHaveAttribute('aria-selected', 'true');
  await page.getByLabel('Search Built-in Songs').focus();
  await page.getByLabel('Search Built-in Songs').hover();
  await expect
    .poll(() => otherMenu.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('0');
  await expect
    .poll(() => toggledLike.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('1');

  await otherMenu.focus();
  await expect
    .poll(() => otherMenu.evaluate((button) => getComputedStyle(button).opacity))
    .toBe('1');
  await otherMenu.press('Enter');
  const menu = page.getByRole('menu', { name: 'Actions for Blue Bird' });
  await expect(menu).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);
  await expect(otherMenu).toBeFocused();
  await expect(otherRow).toHaveAttribute('aria-selected', 'false');
  await expect(firstRow).toHaveAttribute('aria-selected', 'true');
});

test('Built-in Songs pages beyond the first 200 songs', async ({ page }) => {
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await jumpToBuiltinPage(page, 5);
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 5' })).toBeVisible();
  await expect(page.locator('.builtin-pager-status')).toHaveText(
    'Showing 401–500 of 500 songs. Page 5 of 5.',
  );
  const song = page.getByRole('row', { name: /Song 401/ });
  await expect(song).toBeVisible();
  await song.click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByRole('heading', { name: 'Song 401' })).toBeVisible();
});

test('virtualized grid navigation keeps its active descendant mounted across pages', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000');
  const grid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  const list = page.locator('.track-table');

  await grid.focus();
  await page.keyboard.press('End');
  const lastRow = page.getByRole('row', { name: /Song 100/ });
  await expect(lastRow).toBeVisible();
  await expect(lastRow).toHaveAttribute('aria-selected', 'true');
  const lastRowId = await lastRow.getAttribute('id');
  expect(lastRowId).toBeTruthy();
  await expect(grid).toHaveAttribute('aria-activedescendant', lastRowId ?? '');
  await expect
    .poll(() =>
      grid.evaluate((element) => {
        const id = element.getAttribute('aria-activedescendant');
        const activeRow = id ? document.getElementById(id) : null;
        return Boolean(activeRow && element.contains(activeRow) && activeRow.isConnected);
      }),
    )
    .toBe(true);

  await list.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event('scroll'));
  });
  await expect(lastRow).toHaveCount(0);
  await expect.poll(() => grid.getAttribute('aria-activedescendant')).toBeNull();

  await grid.focus();
  await page.keyboard.press('End');
  await expect(lastRow).toHaveAttribute('aria-selected', 'true');
  await expect(grid).toHaveAttribute('aria-activedescendant', lastRowId ?? '');
  await page.keyboard.press('Home');
  const firstRow = page.getByRole('row', { name: /Aurora Landing/ });
  await expect(firstRow).toHaveAttribute('aria-selected', 'true');
  const firstRowId = await firstRow.getAttribute('id');
  expect(firstRowId).toBeTruthy();
  await expect(grid).toHaveAttribute('aria-activedescendant', firstRowId ?? '');

  await page.getByRole('button', { name: 'Next page' }).click();
  const secondPageGrid = page.getByRole('grid', { name: 'Built-in Songs, page 2' });
  await expect(secondPageGrid).toBeVisible();
  await expect(lastRow).toHaveCount(0);
  await expect.poll(() => secondPageGrid.getAttribute('aria-activedescendant')).toBeNull();
  await secondPageGrid.focus();
  await page.keyboard.press('Home');
  const firstRowOnPageTwo = page.getByRole('row', { name: /Song 101/ });
  await expect(firstRowOnPageTwo).toHaveAttribute('aria-selected', 'true');
  const firstRowOnPageTwoId = await firstRowOnPageTwo.getAttribute('id');
  expect(firstRowOnPageTwoId).toBeTruthy();
  await expect(secondPageGrid).toHaveAttribute('aria-activedescendant', firstRowOnPageTwoId ?? '');
  await expect
    .poll(() =>
      secondPageGrid.evaluate((element, id) => {
        const activeRow = id ? document.getElementById(id) : null;
        return Boolean(activeRow && element.contains(activeRow) && activeRow.isConnected);
      }, firstRowOnPageTwoId),
    )
    .toBe(true);
  const search = page.getByLabel('Search Built-in Songs');
  await search.fill('Moonlit');
  await expect(page.getByRole('row', { name: /Moonlit Village/ })).toBeVisible();
  await expect(firstRow).toHaveCount(0);
  await expect.poll(() => grid.getAttribute('aria-activedescendant')).toBeNull();
  await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBe(0);
});

test('Built-in Songs clears search while retaining the catalog count', async ({ page }) => {
  const navigatorItem = page.getByRole('button', { name: 'Built-in Songs' });
  await expect(navigatorItem).toContainText('500');
  await page.getByLabel('Search Built-in Songs').fill('Moonlit');
  await expect(page.getByText('1 song', { exact: true })).toBeVisible();
  await expect(navigatorItem).toContainText('500');
  await navigatorItem.click();
  await expect(page.getByLabel('Search Built-in Songs')).toHaveValue('');
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await expect(navigatorItem).toContainText('500');
});

test('keeps slash local to the New Playlist name field', async ({ page }) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.locator('.library-add-button').click();

  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  const nameInput = createDialog.getByLabel('Playlist name');
  await nameInput.focus();
  await nameInput.pressSequentially('A/B');
  await expect(nameInput).toHaveValue('A/B');
  await expect(nameInput).toBeFocused();

  await createDialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(createDialog).toBeHidden();

  const search = page.getByLabel('Search Built-in Songs');
  await page.keyboard.press('/');
  await expect(search).toBeFocused();
});

test('Liked Songs is a real source with Player Bar save behavior', async ({ page }) => {
  const song = page.getByRole('row', { name: /Aurora Landing/ });
  await song.click();
  const likeButton = page.getByRole('button', { name: 'Add to Liked Songs' });
  await likeButton.click();
  await expect(page.getByRole('button', { name: 'Remove from Liked Songs' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  const likedNavItem = page.locator('.library-navigator').getByRole('button', {
    name: 'Liked Songs',
  });
  await expect(likedNavItem).toContainText('1');

  await likedNavItem.click();
  await expect(page.getByRole('heading', { name: 'Liked Songs' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
});

test('Playlists support create, rename, delete, and membership actions', async ({ page }) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.locator('.library-add-button').click();

  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill('Practice');
  await createDialog.getByRole('button', { name: 'Create' }).click();
  await expect(createDialog).toBeHidden();
  await expect(navigator.getByRole('button', { name: 'Practice', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Practice' })).toBeVisible();
  await expect(page.getByText('0 songs')).toBeVisible();

  await page.getByRole('button', { name: 'More actions for Practice' }).click();
  await expect(page.getByRole('menuitem', { name: 'Add songs' })).toBeVisible();
  await page.getByRole('menuitem', { name: 'Add songs' }).click();
  await expect(page.getByRole('heading', { name: 'Add songs to Practice' })).toBeVisible();
  await expect(page.getByText(/in Built-in Songs/)).toBeVisible();
  await expect(navigator.locator('.library-nav-action-row.is-active')).toContainText('Practice');
  await page.getByRole('button', { name: 'Back to playlist' }).click();

  await page.getByRole('button', { name: 'More actions for Practice' }).click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();
  const renameDialog = page.getByRole('dialog', { name: 'Rename Practice' });
  await renameDialog.getByLabel('Playlist name').fill('Morning Practice');
  await renameDialog.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByRole('heading', { name: 'Morning Practice' })).toBeVisible();

  await page.getByRole('button', { name: 'More actions for Morning Practice' }).click();
  await page.getByRole('menuitem', { name: 'Delete playlist' }).click();
  const deleteDialog = page.getByRole('dialog', { name: /Delete/ });
  await expect(deleteDialog).toContainText('Songs and local files will not be deleted');
  await deleteDialog.getByRole('button', { name: 'Delete playlist' }).click();
  await expect(
    navigator.getByRole('button', { name: 'Morning Practice', exact: true }),
  ).toBeHidden();
  await expect(page.getByRole('heading', { name: 'Built-in Songs' })).toBeVisible();
});

test('Add songs keeps local imports inside the selected playlist', async ({ page }) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });

  await navigator.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill('Practice');
  await createDialog.getByRole('button', { name: 'Create' }).click();
  await expect(navigator.getByRole('button', { name: 'Practice', exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Browse Built-in Songs…' }).click();
  await expect(page.getByRole('heading', { name: 'Add songs to Practice' })).toBeVisible();
  await expect(page.getByText(/in Built-in Songs/)).toBeVisible();
  await expect(page.getByPlaceholder('Search Built-in Songs…')).toBeVisible();
  await expect(navigator.getByRole('button', { name: 'Practice', exact: true })).toBeVisible();
  const existingSong = page.getByRole('row', { name: /Aurora Landing/ });
  await existingSong.getByRole('button', { name: 'More actions for Aurora Landing' }).click();
  await page.getByRole('menuitem', { name: 'Add to Practice' }).click();
  await expect(navigator.getByRole('button', { name: 'Practice', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Back to playlist' }).click();
  await navigator.getByRole('button', { name: 'Practice', exact: true }).click();
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  await expect(page.getByText('1 song')).toBeVisible();

  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Import files…' }).click();
  await expect(page.getByRole('heading', { name: 'Practice' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Local Song B/ })).toBeVisible();
  await expect(navigator.getByRole('button', { name: 'Practice', exact: true })).toBeVisible();
  await expect(navigator.getByText('Local')).toHaveCount(0);

  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Import folder…' }).click();
  await expect(navigator.getByRole('button', { name: 'Practice', exact: true })).toBeVisible();

  await navigator.locator('.library-add-button').click();
  const secondPlaylistDialog = page.getByRole('dialog', { name: 'New playlist' });
  await secondPlaylistDialog.getByLabel('Playlist name').fill('Weekend');
  await secondPlaylistDialog.getByRole('button', { name: 'Create' }).click();
  await expect(page.getByRole('heading', { name: 'Weekend' })).toBeVisible();
  await navigator.getByRole('button', { name: 'Practice', exact: true }).click();
  const importedSong = page.getByRole('row', { name: /Local Song B/ });
  await importedSong.getByRole('button', { name: 'More actions for Local Song B' }).click();
  await page.getByRole('menuitem', { name: 'Add to Weekend' }).click();
  await navigator.getByRole('button', { name: 'Weekend', exact: true }).click();
  await expect(page.getByRole('row', { name: /Local Song B/ })).toBeVisible();
  await expect(page.getByText('1 song')).toBeVisible();

  await page.getByRole('button', { name: 'Built-in Songs' }).click();
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await expect(page.getByRole('row', { name: /Local Song B/ })).toHaveCount(0);

  await page.getByRole('button', { name: 'Practice', exact: true }).click();
  const selectedPlaylistRow = page.getByRole('row', { name: /Aurora Landing/ });
  const songsGrid = page.getByRole('grid', { name: 'Songs' });
  await selectedPlaylistRow.click();
  await expect(selectedPlaylistRow).toHaveAttribute('aria-selected', 'true');
  await selectedPlaylistRow
    .getByRole('button', {
      name: 'More actions for Aurora Landing',
    })
    .click();
  await page.getByRole('menuitem', { name: 'Remove from playlist' }).click();
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toHaveCount(0);
  await expect(songsGrid).toBeFocused();
  await expect(songsGrid).not.toHaveAttribute('aria-activedescendant');
  await expect(songsGrid.locator('[aria-selected="true"]')).toHaveCount(0);
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await expect(player.getByText('No song selected')).toBeVisible();
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeDisabled();

  await page.getByRole('button', { name: 'More actions for Practice' }).click();
  await page.getByRole('menuitem', { name: 'Delete playlist' }).click();
  const deleteDialog = page.getByRole('dialog', { name: /Delete/ });
  await expect(deleteDialog).toContainText('local files will not be deleted');
  await deleteDialog.getByRole('button', { name: 'Delete playlist' }).click();
  await expect(navigator.getByRole('button', { name: /Practice/ })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Built-in Songs' })).toBeVisible();
});

test('long sheet titles remain contained in the Track Browser, Utility, and Player Bar', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  for (let pageNumber = 2; pageNumber <= 5; pageNumber += 1) {
    await page.getByRole('button', { name: 'Next page' }).click();
    await expect(
      page.getByRole('grid', { name: `Built-in Songs, page ${pageNumber}` }),
    ).toBeVisible();
  }
  const list = page.locator('.track-table');
  await list.evaluate((element) => {
    element.scrollTop = 95 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  const longTitle = /A sheet with an intentionally long title/;
  await expect(page.getByRole('row', { name: longTitle })).toBeVisible();
  await page.getByRole('row', { name: longTitle }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByRole('heading', { name: longTitle })).toBeVisible();
  await expect(
    page.getByText(/This intentionally long timing-risk explanation verifies wrapping/),
  ).toBeVisible();
  const overflow = await page.evaluate(() => ({
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth: window.innerWidth,
    titleOverflowHandled: [
      ...document.querySelectorAll('.track-cell-title, .player-track-copy strong'),
    ].every((element) => {
      if (element.scrollWidth <= element.clientWidth) return true;
      const style = getComputedStyle(element);
      return style.overflow === 'hidden' && style.textOverflow === 'ellipsis';
    }),
    utilityReasonOverflow: [...document.querySelectorAll('.song-details-view')].some(
      (element) => element.scrollWidth > element.clientWidth,
    ),
  }));
  expect(overflow.documentWidth).toBeLessThanOrEqual(overflow.viewportWidth);
  expect(overflow.titleOverflowHandled).toBe(true);
  expect(overflow.utilityReasonOverflow).toBe(false);
});
