import { expect, test, type Locator, type Page } from '@playwright/test';
import { expectNoSeriousAccessibilityViolations, startSelectedSong } from './testHelpers';

async function jumpToBuiltinPage(page: Page, target: number) {
  const trigger = page.locator('.builtin-pager-jump-trigger');
  if ((await trigger.count()) > 0) {
    await trigger.click();
    const input = page.getByRole('textbox', { name: /Page number, from/ });
    await input.fill(String(target));
    await input.press('Enter');
    await expect(trigger).toBeFocused();
  } else {
    const currentGrid = page.getByRole('grid', { name: /Built-in Songs, page \d+/ });
    const label = await currentGrid.getAttribute('aria-label');
    const currentPage = Number(label?.match(/page (\d+)/i)?.[1] ?? 1);
    const direction = target > currentPage ? 'Next page' : 'Previous page';
    for (let pageNumber = currentPage; pageNumber !== target;) {
      await page.getByRole('button', { name: direction }).click();
      pageNumber += target > currentPage ? 1 : -1;
    }
  }
  await expect(page.getByRole('grid', { name: `Built-in Songs, page ${target}` })).toBeVisible();
}

function builtinPageTriggerName(page: number, pageCount: number) {
  return `${page} / ${pageCount}, Page ${page} of ${pageCount}. Jump to page`;
}

async function readBuiltinPagerGeometry(page: Page) {
  return page.evaluate(() => {
    const required = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) throw new Error(`Missing pager geometry element: ${selector}`);
      return element;
    };
    const rect = (element: HTMLElement) => {
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
    const optionalRect = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      return element ? rect(element) : null;
    };
    return {
      previous: optionalRect('[aria-label="Previous page"]'),
      indicator: rect(required('.builtin-pager-page-slot')),
      trigger: optionalRect('.builtin-pager-jump-trigger'),
      next: optionalRect('[aria-label="Next page"]'),
      toolbar: rect(required('.builtin-pager')),
      wrap: rect(required('.builtin-pager-wrap')),
      range: rect(required('.builtin-pager-range')),
      table: rect(required('.track-table')),
      player: rect(required('.player-bar')),
    };
  });
}

async function readBrowserPanePagerLayout(page: Page) {
  return page.evaluate(() => {
    const describe = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        clientWidth: element.clientWidth,
        scrollWidth: element.scrollWidth,
        display: style.display,
        flex: style.flex,
        minWidth: style.minWidth,
        boxSizing: style.boxSizing,
        paddingInline: style.paddingInline,
        overflowX: style.overflowX,
        gridTemplateColumns: style.gridTemplateColumns,
        containerType: style.containerType,
        ariaValueMin: element.getAttribute('aria-valuemin'),
        ariaValueMax: element.getAttribute('aria-valuemax'),
        ariaValueNow: element.getAttribute('aria-valuenow'),
      };
    };
    const workbench = document.querySelector<HTMLElement>('.workbench');
    const workbenchRect = workbench?.getBoundingClientRect();
    const workbenchStyle = workbench ? getComputedStyle(workbench) : null;
    return {
      viewport: {
        innerWidth: window.innerWidth,
        innerHeight: window.innerHeight,
        visualWidth: window.visualViewport?.width,
        visualHeight: window.visualViewport?.height,
        visualScale: window.visualViewport?.scale,
      },
      workbench: {
        x: workbenchRect?.x,
        y: workbenchRect?.y,
        width: workbenchRect?.width,
        height: workbenchRect?.height,
        clientWidth: workbench?.clientWidth,
        scrollWidth: workbench?.scrollWidth,
        layoutWidth: workbench?.dataset.workbenchWidth,
        utilityPresentation: workbench?.dataset.utilityPresentation,
        layoutFits: workbench?.dataset.layoutFits,
        display: workbenchStyle?.display,
        flex: workbenchStyle?.flex,
        gridTemplateColumns: workbenchStyle?.gridTemplateColumns,
      },
      navigator: describe('.navigator-workbench-pane'),
      navigatorSeparator: describe('[aria-label="Resize library navigator"]'),
      browserPane: describe('.track-browser-workbench-pane'),
      trackBrowser: describe('.track-browser'),
      pagerWrap: describe('.builtin-pager-wrap'),
      pager: describe('.builtin-pager'),
      range: describe('.builtin-pager-range'),
      utilityPane: describe('.utility-workbench-pane'),
      utilitySeparator: describe('[aria-label="Resize utility pane"]'),
    };
  });
}

async function waitForBrowserLayoutFrames(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
}

async function waitForBrowserPaneWidth(page: Page, expectedWidth: number) {
  const browserPane = page.locator('.track-browser-workbench-pane');
  let stableLayout = { width: 0, minWidth: 0, stable: false };
  await expect
    .poll(async () => {
      stableLayout = await browserPane.evaluate(
        (element) =>
          new Promise((resolve) => {
            requestAnimationFrame(() => {
              const firstWidth = element.getBoundingClientRect().width;
              requestAnimationFrame(() => {
                const style = getComputedStyle(element);
                const secondWidth = element.getBoundingClientRect().width;
                resolve({
                  width: Math.round(secondWidth),
                  minWidth: Number.parseFloat(style.minWidth),
                  stable: Math.abs(firstWidth - secondWidth) <= 1,
                });
              });
            });
          }),
      );
      return stableLayout;
    })
    .toEqual({ width: expectedWidth, minWidth: 420, stable: true });
  return { ...(await readBrowserPanePagerLayout(page)), layoutStability: stableLayout };
}

async function constrainBrowserPaneForPagerTest(page: Page, width: number) {
  // Preserve the product's 420px min-width while exercising the container query.
  await page.locator('.track-browser-workbench-pane').evaluate((element, targetWidth) => {
    element.style.flex = `0 0 ${targetWidth}px`;
    element.style.removeProperty('min-width');
  }, width);
  return waitForBrowserPaneWidth(page, width);
}

async function readBuiltinPagerGlyphGeometry(page: Page) {
  return page.evaluate(() => {
    const required = <T extends Element>(selector: string) => {
      const element = document.querySelector<T>(selector);
      if (!element) throw new Error(`Missing pager glyph measurement element: ${selector}`);
      return element;
    };
    const bounds = (element: Element) => {
      const box = element.getBoundingClientRect();
      return {
        x: box.x,
        right: box.right,
        width: box.width,
        y: box.y,
        height: box.height,
      };
    };
    const previous = required<HTMLButtonElement>('[aria-label="Previous page"]');
    const slot = required<HTMLElement>('.builtin-pager-page-slot');
    const trigger = required<HTMLButtonElement>('.builtin-pager-jump-trigger');
    const next = required<HTMLButtonElement>('[aria-label="Next page"]');
    const previousChevron = previous.querySelector('svg path');
    const nextChevron = next.querySelector('svg path');
    if (!previousChevron || !nextChevron) throw new Error('Pager chevron paths are missing');
    const pager = required<HTMLElement>('.builtin-pager');
    const textRange = document.createRange();
    textRange.selectNodeContents(trigger);
    const text = textRange.getBoundingClientRect();
    const previousPath = previousChevron.getBoundingClientRect();
    const nextPath = nextChevron.getBoundingClientRect();
    const pagerStyle = getComputedStyle(pager);
    const triggerStyle = getComputedStyle(trigger);
    const chProbe = document.createElement('span');
    chProbe.style.cssText = 'position:absolute; visibility:hidden; width:1ch; height:0;';
    chProbe.style.font = pagerStyle.font;
    document.body.append(chProbe);
    const oneCh = chProbe.getBoundingClientRect().width;
    chProbe.remove();
    return {
      fontMetrics: {
        pagerFamily: pagerStyle.fontFamily,
        pagerSize: pagerStyle.fontSize,
        triggerFamily: triggerStyle.fontFamily,
        triggerSize: triggerStyle.fontSize,
        oneCh,
        zoom: getComputedStyle(document.documentElement).zoom,
      },
      pageSlot: bounds(slot),
      previous: bounds(previous),
      next: bounds(next),
      previousChevron: bounds(previousChevron),
      pageText: {
        x: text.x,
        right: text.right,
        width: text.width,
        y: text.y,
        height: text.height,
      },
      nextChevron: bounds(nextChevron),
      leftGlyphGap: text.left - previousPath.right,
      rightGlyphGap: nextPath.left - text.right,
      internalLeftSpace: text.left - slot.getBoundingClientRect().left,
      internalRightSpace: slot.getBoundingClientRect().right - text.right,
      outerControlGap: Number.parseFloat(pagerStyle.gap),
    };
  });
}

function expectBuiltinPagerIndicatorFitsCompactly(
  geometry: Awaited<ReturnType<typeof readBuiltinPagerGlyphGeometry>>,
  zoom = 1,
) {
  expect(geometry.fontMetrics.pagerFamily).toBe(geometry.fontMetrics.triggerFamily);
  expect(geometry.fontMetrics.pagerSize).toBe(geometry.fontMetrics.triggerSize);
  expect(geometry.pageSlot.width).toBeGreaterThanOrEqual(36 * zoom);
  expect(geometry.pageText.x).toBeGreaterThanOrEqual(geometry.pageSlot.x - zoom);
  expect(geometry.pageText.right).toBeLessThanOrEqual(geometry.pageSlot.right + zoom);
  expect(geometry.internalLeftSpace).toBeGreaterThanOrEqual(-zoom);
  expect(geometry.internalRightSpace).toBeGreaterThanOrEqual(-zoom);
  expect(geometry.internalLeftSpace).toBeLessThanOrEqual(10 * zoom);
  expect(geometry.internalRightSpace).toBeLessThanOrEqual(10 * zoom);
  expect(geometry.pageSlot.width - geometry.pageText.width).toBeLessThanOrEqual(20 * zoom);
}

async function readBuiltinPagerEditorGeometry(page: Page) {
  return page.evaluate(() => {
    const required = <T extends Element>(selector: string) => {
      const element = document.querySelector<T>(selector);
      if (!element) throw new Error(`Missing pager editor measurement element: ${selector}`);
      return element;
    };
    const bounds = (element: Element) => {
      const box = element.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height, right: box.right };
    };
    const slot = required<HTMLElement>('.builtin-pager-page-slot');
    const input = required<HTMLInputElement>('.builtin-pager-editor input');
    const pageCount = required<HTMLElement>('.builtin-pager-page-count');
    const confirm = required<HTMLButtonElement>('.builtin-pager-confirm');
    const toolbar = required<HTMLElement>('.builtin-pager');
    const slotRect = slot.getBoundingClientRect();
    const toolbarRect = toolbar.getBoundingClientRect();
    const countRect = pageCount.getBoundingClientRect();
    const confirmRect = confirm.getBoundingClientRect();
    const hit = document.elementFromPoint(
      confirmRect.x + confirmRect.width / 2,
      confirmRect.y + confirmRect.height / 2,
    );
    return {
      toolbar: bounds(toolbar),
      slot: bounds(slot),
      input: bounds(input),
      pageCount: bounds(pageCount),
      confirm: bounds(confirm),
      slotFillsToolbar:
        Math.abs(slotRect.x - toolbarRect.x) <= 1 &&
        Math.abs(slotRect.width - toolbarRect.width) <= 1,
      pageCountFits: countRect.left >= toolbarRect.left && countRect.right <= toolbarRect.right,
      receivesPointer: hit === confirm || confirm.contains(hit),
    };
  });
}

async function expectBuiltinPagerEditingControls(page: Page, pageCount: number) {
  const pager = page.getByRole('navigation', { name: 'Built-in Songs pages' });
  await expect(pager).toHaveAttribute('data-editing', 'true');
  await expect(pager.locator('button')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Previous page' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Next page' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Confirm page change' })).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.builtin-pager-popover')).toHaveCount(0);
  await expect(pager).not.toContainText(/Go to page|\bGo\b/);

  const geometry = await readBuiltinPagerEditorGeometry(page);
  expect(geometry.slotFillsToolbar).toBe(true);
  expect(geometry.slot.height).toBe(36);
  expect(geometry.input.width).toBeGreaterThanOrEqual(20);
  expect(geometry.pageCountFits).toBe(true);
  expect(geometry.confirm.width).toBeGreaterThanOrEqual(32);
  expect(geometry.confirm.height).toBeGreaterThanOrEqual(32);
  expect(geometry.receivesPointer).toBe(true);
  await expect(
    page.getByRole('textbox', { name: `Page number, from 1 to ${pageCount}` }),
  ).toBeVisible();
  return geometry;
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
    const indicator = includePager ? required('.builtin-pager-page-slot') : null;
    const pageIndicator = includePager ? required('.builtin-pager-jump-trigger') : null;
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
            if (!pageIndicator) throw new Error('Built-in page indicator is missing');
            const indicatorStyle = getComputedStyle(pageIndicator);
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
              indicatorBorderStyle: indicatorStyle.borderTopStyle,
              indicatorBorderColor: indicatorStyle.borderTopColor,
              indicatorBackgroundColor: indicatorStyle.backgroundColor,
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
  const expectBoxStable = (
    label: string,
    beforeBox: typeof before.toolbar,
    afterBox: typeof after.toolbar,
  ) => {
    for (const edge of ['x', 'y', 'width', 'height'] as const) {
      expect(
        Math.abs(afterBox[edge] - beforeBox[edge]),
        `${label} ${edge}: ${beforeBox[edge]} -> ${afterBox[edge]}`,
      ).toBeLessThanOrEqual(1);
    }
  };
  expectBoxStable('toolbar', before.toolbar, after.toolbar);
  expectBoxStable('pager wrapper', before.wrap, after.wrap);
  expectBoxStable('range', before.range, after.range);
  expectBoxStable('track table', before.table, after.table);
  expectBoxStable('player dock', before.player, after.player);
  expect(after.range.x + after.range.width).toBeLessThanOrEqual(after.toolbar.x + 1);
  expect(Math.abs(after.range.centerY - after.toolbar.centerY)).toBeLessThanOrEqual(1);

  if (
    before.previous &&
    before.trigger &&
    before.next &&
    after.previous &&
    after.trigger &&
    after.next
  ) {
    for (const control of ['previous', 'trigger', 'next'] as const) {
      expect(
        Math.abs(after[control].centerX - before[control].centerX),
        `${control} centerX: ${before[control].centerX} -> ${after[control].centerX}`,
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
    expect(after.previous.height).toBeGreaterThanOrEqual(36);
    expect(after.trigger.height).toBeGreaterThanOrEqual(36);
    expect(after.next.height).toBeGreaterThanOrEqual(36);
  }
}

function expectBuiltinPagerLayoutStableDuringZoomedFocus(
  before: Awaited<ReturnType<typeof readBuiltinPagerGeometry>>,
  after: Awaited<ReturnType<typeof readBuiltinPagerGeometry>>,
) {
  // Zoomed input focus may shift the viewport; the pager must stay fixed to its footer.
  for (const selector of ['toolbar', 'wrap', 'range', 'table'] as const) {
    const beforeBox = before[selector];
    const afterBox = after[selector];
    for (const edge of ['x', 'width', 'height'] as const) {
      expect(
        Math.abs(afterBox[edge] - beforeBox[edge]),
        `${selector} ${edge}: ${beforeBox[edge]} -> ${afterBox[edge]}`,
      ).toBeLessThanOrEqual(1);
    }
    expect(
      Math.abs(afterBox.y - after.player.y - (beforeBox.y - before.player.y)),
      `${selector} footer offset: ${beforeBox.y - before.player.y} -> ${afterBox.y - after.player.y}`,
    ).toBeLessThanOrEqual(1);
  }
  for (const edge of ['x', 'width', 'height'] as const) {
    expect(
      Math.abs(after.player[edge] - before.player[edge]),
      `player ${edge}: ${before.player[edge]} -> ${after.player[edge]}`,
    ).toBeLessThanOrEqual(1);
  }
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
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '1366');
  await waitForBrowserLayoutFrames(page);
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.getByRole('button', { name: /Built-in Songs/ }).click();
  const grid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  await expect(grid).toHaveAttribute('aria-rowcount', '101');
  await expect(page.locator('.builtin-pager-status')).toContainText('Page 1 of 5');
  await expect(page.locator('.builtin-pager-range')).toHaveText('1–100 of 500 songs');
  await expect(page.locator('.builtin-pager')).toHaveCSS('flex-wrap', 'nowrap');
  const unforcedBrowserLayout = await readBrowserPanePagerLayout(page);

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

  await expect(page.getByRole('button', { name: builtinPageTriggerName(3, 5) })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(page.getByRole('dialog')).toHaveCount(0);

  await jumpToBuiltinPage(page, 5);
  const pageFive = page.getByRole('grid', { name: 'Built-in Songs, page 5' });
  await expect(pageFive).toHaveAttribute('aria-rowcount', '101');
  await expect(page.getByRole('button', { name: builtinPageTriggerName(5, 5) })).toBeVisible();
  await pageFive.focus();
  await page.keyboard.press('End');
  await expect(page.getByRole('row', { name: /Song 500/ })).toHaveAttribute('aria-rowindex', '101');
  await expectNoSeriousAccessibilityViolations(page);

  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  const previous = page.getByRole('button', { name: 'Previous page' });
  const next = page.getByRole('button', { name: 'Next page' });
  await expect(previous).toBeEnabled();
  await expect(next).toBeDisabled();
  await previous.focus();
  await expect
    .poll(() => previous.evaluate((button) => getComputedStyle(button).outlineStyle))
    .toBe('solid');
  const pageFiveIndicator = page.getByRole('button', { name: builtinPageTriggerName(5, 5) });
  await pageFiveIndicator.focus();
  await expect
    .poll(() => pageFiveIndicator.evaluate((button) => getComputedStyle(button).outlineStyle))
    .toBe('solid');
  await page.keyboard.press('Enter');
  const pageFiveInput = page.getByRole('textbox', { name: 'Page number, from 1 to 5' });
  await expect(pageFiveInput).toBeFocused();
  const confirmPageChange = page.getByRole('button', { name: 'Confirm page change' });
  await confirmPageChange.focus();
  await expect
    .poll(() => confirmPageChange.evaluate((button) => getComputedStyle(button).outlineStyle))
    .toBe('solid');
  await pageFiveInput.focus();
  await page.keyboard.press('Escape');
  await expect(pageFiveIndicator).toBeFocused();
  for (const viewport of [
    { width: 1366, height: 768 },
    { width: 995, height: 706 },
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await expect(page.locator('.builtin-pager-jump-trigger')).toBeVisible();
    await expect(page.locator('.builtin-pager button')).toHaveCount(3);
    const geometry = await readBuiltinPagerGlyphGeometry(page);
    expectBuiltinPagerIndicatorFitsCompactly(geometry);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      viewport.width,
    );
  }

  const narrowPaneMeasurements: Array<Record<string, unknown>> = [];
  await jumpToBuiltinPage(page, 3);

  await page.setViewportSize({ width: 872, height: 560 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '872');
  const navigatorSeparator = page.getByRole('separator', { name: 'Resize library navigator' });
  await navigatorSeparator.focus();
  await page.keyboard.press('End');
  await expect(navigatorSeparator).toHaveAttribute('aria-valuenow', '340');
  const browserAt520 = await waitForBrowserPaneWidth(page, 520);
  narrowPaneMeasurements.push({ path: 'navigator keyboard resize at 872px', ...browserAt520 });

  await page.setViewportSize({ width: 800, height: 560 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '800');
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  const utilitySeparator = page.getByRole('separator', { name: 'Resize utility pane' });
  await expect(utilitySeparator).toHaveAttribute('aria-valuenow', '316');
  const browserAtMinimum = await waitForBrowserPaneWidth(page, 420);
  narrowPaneMeasurements.push({ path: 'utility docked at its maximum', ...browserAtMinimum });

  const utilitySeparatorBounds = await utilitySeparator.boundingBox();
  expect(utilitySeparatorBounds).not.toBeNull();
  const utilitySeparatorX = utilitySeparatorBounds!.x + utilitySeparatorBounds!.width / 2;
  const utilitySeparatorY = utilitySeparatorBounds!.y + utilitySeparatorBounds!.height / 2;
  await page.mouse.move(utilitySeparatorX, utilitySeparatorY);
  await page.mouse.down();
  await page.mouse.move(utilitySeparatorX + 20, utilitySeparatorY, { steps: 1 });
  await page.mouse.up();
  await expect(utilitySeparator).toHaveAttribute('aria-valuenow', '296');
  const browserAt440 = await waitForBrowserPaneWidth(page, 440);
  narrowPaneMeasurements.push({ path: 'utility pointer resize at 800px', ...browserAt440 });

  console.info(
    '[Browser pane pager layout diagnostic]',
    JSON.stringify({ unforcedBrowserLayout, narrowPaneMeasurements }),
  );

  for (const width of [520, 440]) {
    await jumpToBuiltinPage(page, 3);
    const paneLayout = width === 520 ? browserAt520 : browserAt440;
    const pagerWrap = paneLayout.pagerWrap;
    if (!pagerWrap) throw new Error('Missing pager wrapper geometry');
    expect(pagerWrap.scrollWidth).toBeLessThanOrEqual(pagerWrap.clientWidth);
    expect(paneLayout.browserPane?.minWidth).toBe('420px');

    const previous = page.getByRole('button', { name: 'Previous page' });
    const next = page.getByRole('button', { name: 'Next page' });
    const trigger = page.getByRole('button', { name: builtinPageTriggerName(3, 5) });
    await expect(previous).toBeVisible();
    await expect(next).toBeVisible();
    await expect(trigger).toBeVisible();
    await expect(previous).toBeEnabled();
    await expect(next).toBeEnabled();
    const viewGeometry = await readBuiltinPagerGeometry(page);
    const pagerControlHitTargets = await page
      .locator('.builtin-pager button')
      .evaluateAll((buttons) =>
        buttons.map((button) => {
          const rect = button.getBoundingClientRect();
          const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
          return hit === button || button.contains(hit);
        }),
      );
    expect(pagerControlHitTargets).toEqual([true, true, true]);

    await previous.focus();
    await page.keyboard.press('Tab');
    await expect(trigger).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(next).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 4' })).toBeVisible();
    expectBuiltinPagerGeometryStable(viewGeometry, await readBuiltinPagerGeometry(page));

    const pageFourTrigger = page.getByRole('button', { name: builtinPageTriggerName(4, 5) });
    await pageFourTrigger.click();
    const input = page.getByRole('textbox', { name: 'Page number, from 1 to 5' });
    const confirm = page.getByRole('button', { name: 'Confirm page change' });
    await expect(input).toBeFocused();
    await expect(confirm).toBeVisible();
    const editor = await readBuiltinPagerEditorGeometry(page);
    expectBuiltinPagerGeometryStable(viewGeometry, await readBuiltinPagerGeometry(page));
    expect(editor.input.width).toBeGreaterThanOrEqual(20);
    expect(editor.confirm.width).toBeGreaterThanOrEqual(32);
    expect(editor.confirm.height).toBeGreaterThanOrEqual(32);
    expect(editor.receivesPointer).toBe(true);
    await input.press('Tab');
    await expect(confirm).toBeFocused();
    await input.focus();
    await input.press('Escape');
    await expect(pageFourTrigger).toBeFocused();
    expectBuiltinPagerGeometryStable(viewGeometry, await readBuiltinPagerGeometry(page));
  }
});

test('F3.5 two-page Built-in pager spacing stays compact through inline editing and navigation', async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/?mockCatalogCount=106');
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.getByRole('button', { name: /Built-in Songs/ }).click();
  const previous = page.getByRole('button', { name: 'Previous page' });
  const next = page.getByRole('button', { name: 'Next page' });
  const indicator = page.getByRole('button', { name: builtinPageTriggerName(1, 2) });
  await expect(previous).toBeVisible();
  await expect(next).toBeVisible();
  await expect(indicator).toBeVisible();
  await expect(indicator).toHaveAttribute('aria-current', 'page');
  await expect(page.locator('.builtin-pager-static-indicator')).toHaveCount(0);
  await expect(page.locator('.builtin-pager button')).toHaveCount(3);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.builtin-pager')).not.toContainText(/Go to page|\bGo\b/);

  const before = await readBuiltinPagerGeometry(page);
  const beforeVisual = await readBuiltinPagerGlyphGeometry(page);
  console.info('[F3.5 compact pager spacing]', JSON.stringify(beforeVisual));
  expectBuiltinPagerIndicatorFitsCompactly(beforeVisual);
  expect(beforeVisual.leftGlyphGap).toBeLessThanOrEqual(35);
  expect(beforeVisual.rightGlyphGap).toBeLessThanOrEqual(35);
  expect(beforeVisual.previous.width).toBe(36);
  expect(beforeVisual.next.width).toBe(36);
  expect(beforeVisual.outerControlGap).toBe(4);
  await indicator.focus();
  await page.keyboard.press('Enter');
  let input = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
  await expect(input).toBeFocused();
  await input.press('Escape');
  await expect(indicator).toBeFocused();
  expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
  await page.keyboard.press('Space');
  input = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
  const confirm = page.getByRole('button', { name: 'Confirm page change' });
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('1');
  await expect(confirm).toBeVisible();
  const editorGeometry = await readBuiltinPagerEditorGeometry(page);
  expect(editorGeometry.confirm.width).toBeGreaterThanOrEqual(32);
  expect(editorGeometry.confirm.height).toBeGreaterThanOrEqual(32);
  expect(editorGeometry.input.width).toBeGreaterThanOrEqual(20);
  expect(editorGeometry.pageCountFits).toBe(true);
  expect(editorGeometry.receivesPointer).toBe(true);
  expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
  await testInfo.attach('f3.5-two-page-spacing-geometry.json', {
    body: JSON.stringify(
      { compactTwoPage: beforeVisual, pager: before, editor: editorGeometry },
      null,
      2,
    ),
    contentType: 'application/json',
  });
  await page.screenshot({
    path: testInfo.outputPath('f3.5-two-page-editor-995x706.png'),
    animations: 'disabled',
  });
  console.info('[F3.5 compact pager geometry]', JSON.stringify({ beforeVisual, editorGeometry }));

  await input.fill('2');
  await input.press('Enter');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Song 101/ })).toHaveAttribute('aria-rowindex', '2');
  await expect(page.locator('.builtin-pager-range')).toHaveText('101–106 of 106 songs');
  const pageTwoIndicator = page.getByRole('button', { name: builtinPageTriggerName(2, 2) });
  await expect(pageTwoIndicator).toBeFocused();
  expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
  const afterNavigationVisual = await readBuiltinPagerGlyphGeometry(page);
  expect(afterNavigationVisual.pageSlot.width).toBe(beforeVisual.pageSlot.width);
  expect(
    Math.abs(afterNavigationVisual.leftGlyphGap - beforeVisual.leftGlyphGap),
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(afterNavigationVisual.rightGlyphGap - beforeVisual.rightGlyphGap),
  ).toBeLessThanOrEqual(1);
  console.info('[F3.5 pager geometry after 1→2]', JSON.stringify(afterNavigationVisual));

  await pageTwoIndicator.click();
  const pageTwoInput = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
  await pageTwoInput.fill('1');
  await page.getByRole('button', { name: 'Confirm page change' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expect(page.getByRole('button', { name: builtinPageTriggerName(1, 2) })).toBeFocused();
  expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
});

test('F3.6 pager chevrons stay quiet, accessible, and grouped across navigation', async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/?mockCatalogCount=106');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();

  const previous = page.getByRole('button', { name: 'Previous page' });
  const next = page.getByRole('button', { name: 'Next page' });
  await expect(previous).toHaveAttribute('title', 'Previous page');
  await expect(next).toHaveAttribute('title', 'Next page');
  await expect(previous).toBeDisabled();
  await expect(next).toBeEnabled();

  const readControlSurface = async (control: typeof previous) =>
    control.evaluate((button) => {
      const style = getComputedStyle(button);
      return {
        width: button.getBoundingClientRect().width,
        height: button.getBoundingClientRect().height,
        background: style.backgroundColor,
        borderWidth: style.borderTopWidth,
        outlineStyle: style.outlineStyle,
        hoverColor: (() => {
          const probe = document.createElement('span');
          probe.style.backgroundColor = 'var(--control-hover)';
          document.body.append(probe);
          const color = getComputedStyle(probe).backgroundColor;
          probe.remove();
          return color;
        })(),
      };
    });

  const initialGeometry = await readBuiltinPagerGeometry(page);
  const previousRest = await readControlSurface(previous);
  const nextRest = await readControlSurface(next);
  for (const surface of [previousRest, nextRest]) {
    expect(surface.width).toBe(36);
    expect(surface.height).toBe(36);
    expect(surface.background).toBe('rgba(0, 0, 0, 0)');
    expect(surface.borderWidth).toBe('0px');
  }
  const initialVisual = await readBuiltinPagerGlyphGeometry(page);
  expectBuiltinPagerIndicatorFitsCompactly(initialVisual);

  await previous.hover();
  expect((await readControlSurface(previous)).background).toBe('rgba(0, 0, 0, 0)');
  await next.hover();
  const nextHover = await readControlSurface(next);
  expect(nextHover.background).toBe(nextHover.hoverColor);
  expect(nextHover.background).not.toBe('rgba(0, 0, 0, 0)');
  await page.mouse.move(0, 0);

  await next.focus();
  await expect
    .poll(() => next.evaluate((button) => getComputedStyle(button).outlineStyle))
    .toBe('solid');
  await page.keyboard.press('Space');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Song 101/ })).toHaveAttribute('aria-rowindex', '2');
  await expect(previous).toBeEnabled();
  await expect(next).toBeDisabled();
  expectBuiltinPagerGeometryStable(initialGeometry, await readBuiltinPagerGeometry(page));

  await next.hover();
  expect((await readControlSurface(next)).background).toBe('rgba(0, 0, 0, 0)');
  await previous.hover();
  const previousHover = await readControlSurface(previous);
  expect(previousHover.background).toBe(previousHover.hoverColor);
  await testInfo.attach('f3.6-pager-control-surfaces.json', {
    body: JSON.stringify(
      { initialGeometry, previousRest, nextRest, nextHover, previousHover },
      null,
      2,
    ),
    contentType: 'application/json',
  });
  console.info(
    '[F3.6 quiet pager controls]',
    JSON.stringify({ initialGeometry, previousRest, nextRest, nextHover, previousHover }),
  );

  await page.goto('/?mockCatalogCount=10000');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  const largeCatalogIndicator = page.getByRole('button', {
    name: builtinPageTriggerName(1, 100),
  });
  await largeCatalogIndicator.click();
  const largeCatalogEditor = await readBuiltinPagerEditorGeometry(page);
  expect(largeCatalogEditor.input.width).toBeGreaterThanOrEqual(20);
  expect(largeCatalogEditor.pageCountFits).toBe(true);
  expect(largeCatalogEditor.confirm.width).toBeGreaterThanOrEqual(32);
  expect(largeCatalogEditor.confirm.height).toBeGreaterThanOrEqual(32);
  console.info('[F3.6 100-page editor geometry]', JSON.stringify(largeCatalogEditor));
});

test('page indicator switches between compact view and full-width editing without moving its outer geometry', async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 995, height: 706 });
  const measurements: Array<Record<string, unknown>> = [];

  for (const total of [106, 500, 501, 2438, 10_000]) {
    const delay = total === 106 ? '&mockSearchDelayMs=1000' : '';
    await page.goto(`/?mockCatalogCount=${total}${delay}`);
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
    const pageCount = Math.ceil(total / 100);
    const pager = page.getByRole('navigation', { name: 'Built-in Songs pages' });
    const indicator = page.getByRole('button', { name: builtinPageTriggerName(1, pageCount) });
    const before = await readBuiltinPagerGeometry(page);
    const viewGlyphs = await readBuiltinPagerGlyphGeometry(page);
    console.info(
      '[Pager view geometry measurement]',
      JSON.stringify({ total, pageCount, before, viewGlyphs }),
    );
    await expect(pager).toHaveAttribute('data-editing', 'false');
    await expect(pager.locator('button')).toHaveCount(3);
    expectBuiltinPagerIndicatorFitsCompactly(viewGlyphs);
    expect(Math.abs(before.toolbar.width - before.indicator.width - 80)).toBeLessThanOrEqual(1);
    expect(before.previous?.width).toBe(36);
    expect(before.next?.width).toBe(36);
    if (total === 106) {
      const viewScreenshot = testInfo.outputPath('f3.7-view-mode-995x706.png');
      await page.screenshot({ path: viewScreenshot, animations: 'disabled' });
      await testInfo.attach('f3.7-view-mode-995x706.png', {
        path: viewScreenshot,
        contentType: 'image/png',
      });
    }

    if (total === 106) {
      await indicator.focus();
      await page.keyboard.press('Enter');
    } else {
      await indicator.click();
    }
    let input = page.getByRole('textbox', { name: `Page number, from 1 to ${pageCount}` });
    await expect(input).toBeFocused();
    await expect(input).toHaveValue('1');
    expect(
      await input.evaluate((element) => ({
        start: (element as HTMLInputElement).selectionStart,
        end: (element as HTMLInputElement).selectionEnd,
      })),
    ).toEqual({ start: 0, end: 1 });
    expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
    const editor = await expectBuiltinPagerEditingControls(page, pageCount);
    const duringEdit = await readBuiltinPagerGeometry(page);
    expectBuiltinPagerGeometryStable(before, duringEdit);
    if (total === 106) {
      const editScreenshot = testInfo.outputPath('f3.7-edit-mode-995x706.png');
      await page.screenshot({ path: editScreenshot, animations: 'disabled' });
      await testInfo.attach('f3.7-edit-mode-995x706.png', {
        path: editScreenshot,
        contentType: 'image/png',
      });
    }

    if (total === 106) {
      await input.press('Escape');
      await expect(indicator).toBeFocused();
      const canceled = await readBuiltinPagerGeometry(page);
      expectBuiltinPagerGeometryStable(before, canceled);

      await indicator.click();
      input = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
      await expect(input).toBeFocused();
      await input.fill('2');
      await input.press('Enter');
      await expect(pager).toHaveAttribute('aria-busy', 'true');
      const pending = await readBuiltinPagerGeometry(page);
      expectBuiltinPagerGeometryStable(before, pending);
      await expect(page.locator('.builtin-pager-range')).toHaveText('1–100 of 106 songs');
      await expect(page.getByRole('status')).toContainText('Loading page 2 of 2');
      await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
      await expect(page.getByRole('row', { name: /Song 101/ })).toHaveAttribute(
        'aria-rowindex',
        '2',
      );
      await expect(page.locator('.builtin-pager-range')).toHaveText('101–106 of 106 songs');
      const pageTwo = await readBuiltinPagerGeometry(page);
      expectBuiltinPagerGeometryStable(before, pageTwo);

      const pageTwoIndicator = page.getByRole('button', { name: builtinPageTriggerName(2, 2) });
      await pageTwoIndicator.click();
      input = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
      await expect(input).toBeFocused();
      await input.fill('1');
      await expectBuiltinPagerEditingControls(page, 2);
      await page.getByRole('button', { name: 'Confirm page change' }).click();
      await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
      const returnedToPageOne = await readBuiltinPagerGeometry(page);
      expectBuiltinPagerGeometryStable(before, returnedToPageOne);

      const pageOneIndicator = page.getByRole('button', { name: builtinPageTriggerName(1, 2) });
      await pageOneIndicator.click();
      input = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
      await expect(input).toBeFocused();
      const search = page.getByRole('searchbox', { name: 'Search Built-in Songs' });
      await search.focus();
      await expect(search).toBeFocused();
      await expect(page.getByRole('textbox', { name: /page number/i })).toHaveCount(0);
      await expect(pageOneIndicator).not.toBeFocused();
      expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
    } else {
      await input.press('Escape');
      await expect(indicator).toBeFocused();
      expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
    }

    measurements.push({ total, pageCount, view: before, viewGlyphs, edit: duringEdit, editor });
  }

  await testInfo.attach('f3.7-two-mode-pager-geometry.json', {
    body: JSON.stringify(measurements, null, 2),
    contentType: 'application/json',
  });
  console.info('[Two-mode pager geometry]', JSON.stringify(measurements));
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
    await page.mouse.move(0, 0);
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
    check(pager.indicatorBorderColor === 'rgba(0, 0, 0, 0)', 'resting trigger border is accented');
    check(
      pager.indicatorBackgroundColor === 'rgba(0, 0, 0, 0)',
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
        indicatorBorder: pager.indicatorBorderColor,
        indicatorBorderStyle: pager.indicatorBorderStyle,
        indicatorBackground: pager.indicatorBackgroundColor,
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
  const pagerGlyphMeasurements: Array<Record<string, unknown>> = [];

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
    const pageOneGlyphGeometry = await readBuiltinPagerGlyphGeometry(page);
    expectBuiltinPagerIndicatorFitsCompactly(pageOneGlyphGeometry);
    expect(pageOneGlyphGeometry.leftGlyphGap).toBeLessThanOrEqual(50);
    expect(pageOneGlyphGeometry.rightGlyphGap).toBeLessThanOrEqual(50);
    const pageOneTrigger = page.getByRole('button', { name: builtinPageTriggerName(1, 2) });
    await pageOneTrigger.click();
    const pageOneInput = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
    await expect(pageOneInput).toBeFocused();
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
    await pageOneInput.press('Escape');
    await expect(pageOneTrigger).toBeFocused();
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
    const next = page.getByRole('button', { name: 'Next page' });
    await next.focus();
    await expect(next).toBeFocused();
    await expect
      .poll(() => next.evaluate((button) => getComputedStyle(button).outlineStyle))
      .toBe('solid');
    await page.keyboard.press('Enter');
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
    const pageTwoGlyphGeometry = await readBuiltinPagerGlyphGeometry(page);
    expect(
      Math.abs(pageTwoGlyphGeometry.leftGlyphGap - pageOneGlyphGeometry.leftGlyphGap),
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(pageTwoGlyphGeometry.rightGlyphGap - pageOneGlyphGeometry.rightGlyphGap),
    ).toBeLessThanOrEqual(1);
    pagerGlyphMeasurements.push({
      viewport: viewport.label,
      pageOne: pageOneGlyphGeometry,
      pageTwo: pageTwoGlyphGeometry,
    });
    const previous = page.getByRole('button', { name: 'Previous page' });
    await previous.focus();
    await expect(previous).toBeFocused();
    await expect
      .poll(() => previous.evaluate((button) => getComputedStyle(button).outlineStyle))
      .toBe('solid');
    await assertVisualContract(`${viewport.label} page 2 focused`);
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
    const pageTwoTrigger = page.getByRole('button', { name: builtinPageTriggerName(2, 2) });
    await pageTwoTrigger.click();
    const pageTwoInput = page.getByRole('textbox', { name: 'Page number, from 1 to 2' });
    await pageTwoInput.fill('1');
    await pageTwoInput.press('Enter');
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
    expectBuiltinPagerGeometryStable(pageOneGeometry, await readBuiltinPagerGeometry(page));
  }

  await page.setViewportSize({ width: 1366, height: 768 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '1366');
  for (const width of [520, 440]) {
    await constrainBrowserPaneForPagerTest(page, width);
    await assertVisualContract(`track-browser container ${width}px`);
    const constrainedGlyphGeometry = await readBuiltinPagerGlyphGeometry(page);
    expectBuiltinPagerIndicatorFitsCompactly(constrainedGlyphGeometry);
    expect(constrainedGlyphGeometry.leftGlyphGap).toBeLessThanOrEqual(50);
    expect(constrainedGlyphGeometry.rightGlyphGap).toBeLessThanOrEqual(50);
    pagerGlyphMeasurements.push({ paneWidth: width, pageOne: constrainedGlyphGeometry });
  }
  console.info('[F3.5 compact pager viewport gaps]', JSON.stringify(pagerGlyphMeasurements));
  await page.locator('.track-browser-workbench-pane').evaluate((element) => {
    element.style.removeProperty('flex');
    element.style.removeProperty('min-width');
  });

  const zoomMeasurements: Array<Record<string, unknown>> = [];
  for (const zoom of [1.25, 1.5]) {
    await page.evaluate((scale) => {
      document.documentElement.style.zoom = String(scale);
    }, zoom);
    await page.evaluate(
      () =>
        new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
    );
    const zoomGlyphGeometry = await readBuiltinPagerGlyphGeometry(page);
    expectBuiltinPagerIndicatorFitsCompactly(zoomGlyphGeometry, zoom);
    expect(zoomGlyphGeometry.leftGlyphGap).toBeLessThanOrEqual(50 * zoom);
    expect(zoomGlyphGeometry.rightGlyphGap).toBeLessThanOrEqual(50 * zoom);
    await page.locator('.builtin-pager-jump-trigger').click();
    await expect(page.getByRole('textbox', { name: 'Page number, from 1 to 2' })).toBeFocused();
    const zoomEditorGeometry = await readBuiltinPagerEditorGeometry(page);
    expect(zoomEditorGeometry.input.width).toBeGreaterThanOrEqual(20 * zoom);
    expect(zoomEditorGeometry.confirm.width).toBeGreaterThanOrEqual(32 * zoom);
    expect(zoomEditorGeometry.confirm.height).toBeGreaterThanOrEqual(32 * zoom);
    expect(zoomEditorGeometry.pageCountFits).toBe(true);
    expect(zoomEditorGeometry.receivesPointer).toBe(true);
    zoomMeasurements.push({ zoom, pager: zoomGlyphGeometry, editor: zoomEditorGeometry });
    await page.keyboard.press('Escape');
    await page.evaluate(() => {
      document.documentElement.style.removeProperty('zoom');
    });
  }
  console.info('[F3.5 pager font zoom geometry]', JSON.stringify(zoomMeasurements));

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
  for (const count of [0, 1, 96, 100, 101, 106, 500, 501, 2438, 10_000]) {
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
    await expect(page.getByRole('button', { name: 'Next page' })).toBeEnabled();
    await expect(page.getByRole('button', { name: 'First page' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Last page' })).toHaveCount(0);
    await expect(pager.locator('.builtin-pager-static-indicator')).toHaveCount(0);
    await expect(
      page.getByRole('button', { name: builtinPageTriggerName(1, pageCount) }),
    ).toHaveAttribute('aria-current', 'page');

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
    expectBuiltinPagerIndicatorFitsCompactly(await readBuiltinPagerGlyphGeometry(page));
    await page.getByRole('button', { name: 'Next page' }).click();
    await expect(
      page.getByRole('grid', { name: `Built-in Songs, page ${transition.to}` }),
    ).toBeVisible();
    const after = await readBuiltinPagerGeometry(page);
    const afterGlyphs = await readBuiltinPagerGlyphGeometry(page);
    expectBuiltinPagerIndicatorFitsCompactly(afterGlyphs);
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
  await expect(nextProceed).toBeVisible();
  await nextProceed.click();
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
  await expect(page.getByRole('button', { name: builtinPageTriggerName(2, 5) })).toBeFocused();
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

test('F3.2 classic scrollbar geometry stays stable across Built-in page 1 and page 2', async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockCatalogCount=106');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await page.addStyleTag({
    content: `
      .track-table {
        box-sizing: border-box !important;
        border-right: 0 solid transparent !important;
      }
      .track-table[data-f32-classic-gutter='reserved'] {
        border-right-width: var(--f32-classic-gutter-extra) !important;
      }
    `,
  });

  const applyClassicScrollbarFixture = async () =>
    page.locator('.track-table').evaluate((table) => {
      const element = table as HTMLElement;
      element.dataset.f32ClassicGutter = 'free';
      const nativeReservation = element.offsetWidth - element.clientWidth;
      const stable = getComputedStyle(element).scrollbarGutter.includes('stable');
      const overflowing = element.scrollHeight > element.clientHeight;
      const targetReservation = stable || overflowing ? 16 : 0;
      const fixtureReservation = Math.max(0, targetReservation - nativeReservation);
      element.style.setProperty('--f32-classic-gutter-extra', `${fixtureReservation}px`);
      element.dataset.f32ClassicGutter = targetReservation > 0 ? 'reserved' : 'free';
    });

  const readColumns = async () =>
    page.locator('.track-table').evaluate((table) => {
      const element = table as HTMLElement;
      const style = getComputedStyle(element);
      const header = element.querySelector<HTMLElement>('.track-table-header');
      const row = element.querySelector<HTMLElement>('.track-row');
      const cells = (container: HTMLElement | null, selector: string) =>
        Array.from(container?.querySelectorAll<HTMLElement>(selector) ?? [])
          .filter((cell) => getComputedStyle(cell).display !== 'none')
          .map((cell) => {
            const rect = cell.getBoundingClientRect();
            return { x: rect.x, right: rect.right, width: rect.width };
          });
      return {
        scrollport: {
          x: element.getBoundingClientRect().x,
          width: element.getBoundingClientRect().width,
          clientWidth: element.clientWidth,
          offsetWidth: element.offsetWidth,
          clientHeight: element.clientHeight,
          scrollHeight: element.scrollHeight,
          scrollbarGutter: style.scrollbarGutter,
          classicScrollbarReservation: element.offsetWidth - element.clientWidth,
          fixtureReservation: Number.parseFloat(style.borderRightWidth) || 0,
        },
        header: {
          right: header?.getBoundingClientRect().right ?? null,
          columns: cells(header, '[role="columnheader"]'),
        },
        firstRow: { columns: cells(row, '[role="gridcell"]') },
      };
    });

  const measurements: Array<Record<string, unknown>> = [];
  for (const viewport of [
    { width: 1366, height: 768 },
    { width: 995, height: 706 },
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await expect(page.locator('.workbench')).toHaveAttribute(
      'data-workbench-width',
      String(viewport.width),
    );
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
    await applyClassicScrollbarFixture();
    const pageOne = await readColumns();
    expect(pageOne.scrollport.scrollHeight).toBeGreaterThan(pageOne.scrollport.clientHeight);
    await page.getByRole('button', { name: 'Next page' }).click();
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
    await applyClassicScrollbarFixture();
    const pageTwo = await readColumns();
    if (viewport.width > 640) {
      expect(pageTwo.scrollport.scrollHeight).toBeLessThanOrEqual(
        pageTwo.scrollport.clientHeight + 1,
      );
    } else {
      expect(pageTwo.scrollport.scrollHeight).toBeGreaterThan(pageTwo.scrollport.clientHeight);
    }

    expect(
      Math.abs(pageTwo.scrollport.clientWidth - pageOne.scrollport.clientWidth),
      `${viewport.width}px scrollport clientWidth ${pageOne.scrollport.clientWidth} -> ${pageTwo.scrollport.clientWidth}`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs((pageTwo.header.right ?? 0) - (pageOne.header.right ?? 0)),
      `${viewport.width}px header right ${pageOne.header.right} -> ${pageTwo.header.right}`,
    ).toBeLessThanOrEqual(1);
    expect(pageTwo.header.columns).toHaveLength(pageOne.header.columns.length);
    expect(pageTwo.firstRow.columns).toHaveLength(pageOne.firstRow.columns.length);
    for (const [index, cell] of pageTwo.header.columns.entries()) {
      expect(Math.abs(cell.x - pageOne.header.columns[index]!.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(cell.right - pageOne.header.columns[index]!.right)).toBeLessThanOrEqual(1);
    }
    for (const [index, cell] of pageTwo.firstRow.columns.entries()) {
      expect(Math.abs(cell.x - pageOne.firstRow.columns[index]!.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(cell.right - pageOne.firstRow.columns[index]!.right)).toBeLessThanOrEqual(1);
    }
    measurements.push({ viewport, pageOne, pageTwo });
    await page.getByRole('button', { name: 'Previous page' }).click();
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
    await applyClassicScrollbarFixture();
    expect(await readColumns()).toEqual(pageOne);
  }
  await testInfo.attach('f3.2-classic-scrollbar-page-geometry.json', {
    body: JSON.stringify(measurements, null, 2),
    contentType: 'application/json',
  });
  console.info('[F3.2 classic scrollbar geometry]', JSON.stringify(measurements));
});

test('F3.2 stable track gutter covers 106, 500, 10000 songs and a one-row search', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  const readScrollport = async () =>
    page.locator('.track-table').evaluate((table) => {
      const element = table as HTMLElement;
      const header = element.querySelector('.track-table-header')?.getBoundingClientRect();
      return {
        clientWidth: element.clientWidth,
        offsetWidth: element.offsetWidth,
        clientHeight: element.clientHeight,
        scrollHeight: element.scrollHeight,
        scrollbarGutter: getComputedStyle(element).scrollbarGutter,
        headerRight: header?.right ?? null,
      };
    });

  for (const catalogCount of [106, 500, 10_000]) {
    await page.goto(`/?mockCatalogCount=${catalogCount}`);
    await expect(page.getByText(`${catalogCount} songs`, { exact: true })).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
    const pageOne = await readScrollport();
    expect(pageOne.scrollbarGutter).toContain('stable');
    expect(pageOne.scrollHeight).toBeGreaterThan(pageOne.clientHeight);

    await page.getByRole('button', { name: 'Next page' }).click();
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
    const pageTwo = await readScrollport();
    expect(pageTwo.scrollbarGutter).toContain('stable');
    expect(pageTwo.clientWidth).toBe(pageOne.clientWidth);
    expect(pageTwo.headerRight).toBe(pageOne.headerRight);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      800,
    );

    if (catalogCount === 10_000) {
      const search = page.getByRole('searchbox', { name: 'Search Built-in Songs' });
      await search.fill('Song 10000');
      await expect(page.locator('.track-browser-count')).toHaveText('1 song');
      await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
      await expect(page.locator('.track-table')).toHaveAttribute('aria-rowcount', '2');
      const searched = await readScrollport();
      expect(searched.scrollbarGutter).toContain('stable');
      expect(searched.clientWidth).toBe(pageOne.clientWidth);
      expect(searched.scrollHeight).toBeLessThanOrEqual(searched.clientHeight + 1);
      expect(searched.headerRight).toBe(pageOne.headerRight);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        800,
      );
    }
  }
});

test('F3.3 collapsed rail directly lists playlists and scrolls only its playlist region', async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 995, height: 706 });
  const preferenceKey = 'sky.ui.workbench.v5';
  await page.addInitScript((key) => {
    window.localStorage.clear();
    window.localStorage.setItem(
      key,
      JSON.stringify({
        version: 5,
        navigatorPreference: 'collapsed',
        expandedNavigatorWidth: 240,
        utilityWidth: 320,
        utilityOpen: false,
      }),
    );
  }, preferenceKey);
  await page.goto('/?mockCatalogCount=500');
  await expect(page.locator('.app-shell')).toBeVisible();

  const names = [
    'Ánh sáng 🌅',
    'Ánh sáng của tôi',
    '🎹 Piano practice',
    'Nguyễn Ánh — Bản tình ca buổi tối 🎹🌙',
    'Evening practice',
    'Morning practice',
    'Favorites for family',
    'Quiet songs',
    ...Array.from(
      { length: 12 },
      (_, index) => 'Practice set ' + String(index + 9).padStart(2, '0'),
    ),
  ];
  const navigator = page.getByRole('navigation', { name: 'Library' });
  const navigatorPane = page.locator('.navigator-workbench-pane');
  await expect(navigatorPane).toHaveClass(/is-collapsed/);
  for (const name of names) {
    await navigator.getByRole('button', { name: 'Create playlist' }).click();
    const dialog = page.getByRole('dialog', { name: 'New playlist' });
    await dialog.getByLabel('Playlist name').fill(name);
    await dialog.getByRole('button', { name: 'Create' }).click();
    await expect(page.getByRole('heading', { name })).toBeVisible();
  }

  const scroller = navigator.locator('.library-collapsed-playlist-scroller');
  const tiles = scroller.locator('.library-collapsed-playlist-tile');
  await expect(navigator.getByRole('group', { name: 'Playlists' })).toBeVisible();
  await expect(tiles).toHaveCount(names.length);
  await expect(navigator.getByRole('button', { name: 'Playlists', exact: true })).toHaveCount(0);
  await expect(page.getByRole('menu', { name: 'Playlists' })).toHaveCount(0);
  expect(
    await tiles.evaluateAll((elements) =>
      elements.map((element) => element.getAttribute('aria-label')),
    ),
  ).toEqual(names);

  const beforeScroll = await page.evaluate(() => {
    const rect = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) throw new Error('Missing collapsed rail region: ' + selector);
      const box = element.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height };
    };
    const scroller = document.querySelector<HTMLElement>('.library-collapsed-playlist-scroller')!;
    return {
      heading: rect('.library-navigator-heading'),
      primary: rect('.library-primary-items'),
      playlists: rect('.library-collapsed-launchers'),
      footer: rect('.library-navigator-footer'),
      scrollHeight: scroller.scrollHeight,
      clientHeight: scroller.clientHeight,
      overflowY: getComputedStyle(scroller).overflowY,
      firstTile: rect('.library-collapsed-playlist-tile'),
      documentWidth: document.documentElement.scrollWidth,
      documentScrollTop: document.documentElement.scrollTop,
    };
  });
  expect(beforeScroll.scrollHeight).toBeGreaterThan(beforeScroll.clientHeight);
  expect(beforeScroll.overflowY).toBe('auto');
  expect(beforeScroll.firstTile.width).toBe(40);
  expect(beforeScroll.firstTile.height).toBe(40);
  expect(beforeScroll.documentWidth).toBeLessThanOrEqual(995);

  const lastName = names[names.length - 1]!;
  const lastTile = scroller.getByRole('button', { name: lastName });
  await lastTile.hover();
  await expect(page.getByRole('tooltip')).toHaveText(lastName);
  await page.mouse.move(0, 0);
  await expect(page.getByRole('tooltip')).toHaveCount(0);
  await navigator.getByRole('button', { name: 'Expand library navigator' }).focus();
  for (let tabIndex = 0; tabIndex < names.length + 2; tabIndex += 1) {
    await page.keyboard.press('Tab');
  }
  await expect(lastTile).toBeFocused();
  await expect(page.getByRole('tooltip')).toHaveText(lastName);
  const focusedScrollTop = await scroller.evaluate((element) => element.scrollTop);
  expect(focusedScrollTop).toBeGreaterThan(0);
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: lastName })).toBeVisible();
  await expect(navigatorPane).toHaveClass(/is-collapsed/);

  const firstName = names[0]!;
  await scroller.getByRole('button', { name: firstName }).click();
  await expect(page.getByRole('heading', { name: firstName })).toBeVisible();
  await expect(scroller.getByRole('button', { name: firstName })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(navigatorPane).toHaveClass(/is-collapsed/);
  const preference = await page.evaluate(
    (key) => JSON.parse(window.localStorage.getItem(key) ?? 'null'),
    preferenceKey,
  );
  expect(preference.navigatorPreference).toBe('collapsed');
  expect(preference.expandedNavigatorWidth).toBe(240);
  const afterSelection = await page.evaluate(() => {
    const rect = (selector: string) => {
      const box = document.querySelector<HTMLElement>(selector)!.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height };
    };
    return {
      heading: rect('.library-navigator-heading'),
      primary: rect('.library-primary-items'),
      playlists: rect('.library-collapsed-launchers'),
      footer: rect('.library-navigator-footer'),
      documentWidth: document.documentElement.scrollWidth,
    };
  });
  expect(afterSelection.heading).toEqual(beforeScroll.heading);
  expect(afterSelection.primary).toEqual(beforeScroll.primary);
  expect(afterSelection.playlists).toEqual(beforeScroll.playlists);
  expect(afterSelection.footer).toEqual(beforeScroll.footer);
  expect(afterSelection.documentWidth).toBeLessThanOrEqual(995);

  await page.setViewportSize({ width: 640, height: 448 });
  await expect(navigatorPane).toHaveClass(/is-collapsed/);
  const compactGeometry = await page.evaluate(() => {
    const scroller = document.querySelector<HTMLElement>('.library-collapsed-playlist-scroller')!;
    const pane = document.querySelector<HTMLElement>('.navigator-workbench-pane')!;
    const footer = document.querySelector<HTMLElement>('.library-navigator-footer')!;
    return {
      railWidth: pane.getBoundingClientRect().width,
      scroller: {
        width: scroller.getBoundingClientRect().width,
        height: scroller.getBoundingClientRect().height,
        scrollHeight: scroller.scrollHeight,
        clientHeight: scroller.clientHeight,
        scrollTop: scroller.scrollTop,
      },
      footerY: footer.getBoundingClientRect().y,
      documentWidth: document.documentElement.scrollWidth,
    };
  });
  expect(compactGeometry.railWidth).toBe(48);
  expect(compactGeometry.scroller.scrollHeight).toBeGreaterThan(
    compactGeometry.scroller.clientHeight,
  );
  expect(compactGeometry.documentWidth).toBeLessThanOrEqual(640);
  await testInfo.attach('f3.3-collapsed-playlist-rail-geometry.json', {
    body: JSON.stringify({ beforeScroll, afterSelection, compactGeometry }, null, 2),
    contentType: 'application/json',
  });
  console.info(
    '[F3.3 collapsed playlist rail geometry]',
    JSON.stringify({ beforeScroll, afterSelection, compactGeometry }),
  );
  await expectNoSeriousAccessibilityViolations(page);
});

test('F3.4 page indicator opens an inline editor at every multi-page count', async ({
  page,
}, testInfo) => {
  const measurements: Array<Record<string, unknown>> = [];
  for (const total of [106, 500]) {
    await page.setViewportSize({ width: 995, height: 706 });
    await page.goto('/?mockCatalogCount=' + total);
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
    const pageCount = Math.ceil(total / 100);
    const pager = page.getByRole('navigation', { name: 'Built-in Songs pages' });
    const indicator = page.getByRole('button', { name: builtinPageTriggerName(1, pageCount) });
    await expect(indicator).toHaveAttribute('aria-current', 'page');
    await expect(pager.locator('button')).toHaveCount(3);
    await expect(page.locator('.builtin-pager-static-indicator')).toHaveCount(0);
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(pager).not.toContainText(/Go to page|\bGo\b/);
    const before = await readBuiltinPagerGeometry(page);
    await indicator.click();
    const input = page.getByRole('textbox', { name: `Page number, from 1 to ${pageCount}` });
    await expect(input).toBeFocused();
    await expect(input).toHaveValue('1');
    const editing = await readBuiltinPagerGeometry(page);
    expectBuiltinPagerGeometryStable(before, editing);
    await input.fill(String(pageCount));
    if (total === 106) {
      await input.press('Enter');
    } else {
      await page.getByRole('button', { name: 'Confirm page change' }).click();
    }
    await expect(
      page.getByRole('grid', { name: `Built-in Songs, page ${pageCount}` }),
    ).toBeVisible();
    const afterGlyphs = await readBuiltinPagerGlyphGeometry(page);
    expectBuiltinPagerIndicatorFitsCompactly(afterGlyphs);
    const lastRow = page.getByRole('row', { name: new RegExp(`Song ${total}`) });
    await page.getByRole('grid', { name: `Built-in Songs, page ${pageCount}` }).focus();
    await page.keyboard.press('End');
    await expect(lastRow).toBeVisible();
    await expect(page.locator('.builtin-pager-range')).toHaveText(
      `${(pageCount - 1) * 100 + 1}–${total} of ${total} songs`,
    );
    expectBuiltinPagerGeometryStable(before, await readBuiltinPagerGeometry(page));
    measurements.push({
      total,
      pageCount,
      before,
      editing,
      afterGlyphs,
      after: await readBuiltinPagerGeometry(page),
    });
  }

  await page.setViewportSize({ width: 995, height: 706 });
  await page.goto('/?mockCatalogCount=501');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  const sixPagePager = page.getByRole('navigation', { name: 'Built-in Songs pages' });
  const sixPageTrigger = page.getByRole('button', {
    name: '1 / 6, Page 1 of 6. Jump to page',
  });
  await expect(sixPagePager.locator('button')).toHaveCount(3);
  const beforeEdit = await readBuiltinPagerGeometry(page);
  expectBuiltinPagerIndicatorFitsCompactly(await readBuiltinPagerGlyphGeometry(page));
  await sixPageTrigger.click();
  const input = page.getByRole('textbox', { name: 'Page number, from 1 to 6' });
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('1');
  await expect(sixPagePager).not.toContainText(/Go to page|\bGo\b/);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.builtin-pager-popover')).toHaveCount(0);
  expectBuiltinPagerGeometryStable(beforeEdit, await readBuiltinPagerGeometry(page));
  await input.fill('999');
  await input.press('Enter');
  await expect(page.getByRole('alert')).toHaveText('Enter a whole page number from 1 to 6.');
  await expect(input).toHaveAttribute('aria-invalid', 'true');
  await expect(input).toBeFocused();
  expectBuiltinPagerGeometryStable(beforeEdit, await readBuiltinPagerGeometry(page));
  const errorGeometry = await page.locator('.builtin-pager-error').boundingBox();
  const slotGeometry = await page.locator('.builtin-pager-page-slot').boundingBox();
  expect(errorGeometry).not.toBeNull();
  expect(slotGeometry).not.toBeNull();
  await input.fill('6');
  await input.press('Enter');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 6' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Song 501/ })).toBeVisible();
  const sixPageTriggerAfter = page.getByRole('button', {
    name: '6 / 6, Page 6 of 6. Jump to page',
  });
  await expect(sixPageTriggerAfter).toBeFocused();
  expectBuiltinPagerGeometryStable(beforeEdit, await readBuiltinPagerGeometry(page));
  measurements.push({
    total: 501,
    pageCount: 6,
    beforeEdit,
    error: { errorGeometry, slotGeometry },
    afterCommit: await readBuiltinPagerGeometry(page),
  });
  await expectNoSeriousAccessibilityViolations(page);

  await page.goto('/?mockCatalogCount=2438');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  const manyPageTrigger = page.getByRole('button', {
    name: '1 / 25, Page 1 of 25. Jump to page',
  });
  const beforeTwentyFive = await readBuiltinPagerGeometry(page);
  expectBuiltinPagerIndicatorFitsCompactly(await readBuiltinPagerGlyphGeometry(page));
  await manyPageTrigger.click();
  const manyPageInput = page.getByRole('textbox', { name: 'Page number, from 1 to 25' });
  const editorTwentyFive = await readBuiltinPagerEditorGeometry(page);
  expect(editorTwentyFive.input.width).toBeGreaterThanOrEqual(20);
  expect(editorTwentyFive.confirm.width).toBeGreaterThanOrEqual(32);
  expect(editorTwentyFive.confirm.height).toBeGreaterThanOrEqual(32);
  expect(editorTwentyFive.pageCountFits).toBe(true);
  expect(editorTwentyFive.receivesPointer).toBe(true);
  expectBuiltinPagerGeometryStable(beforeTwentyFive, await readBuiltinPagerGeometry(page));
  await manyPageInput.fill('25');
  await page.getByRole('button', { name: 'Confirm page change' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 25' })).toBeVisible();
  const twentyFivePageGlyphs = await readBuiltinPagerGlyphGeometry(page);
  expectBuiltinPagerIndicatorFitsCompactly(twentyFivePageGlyphs);
  expectBuiltinPagerGeometryStable(beforeTwentyFive, await readBuiltinPagerGeometry(page));
  await expect(page.locator('.builtin-pager-range')).toHaveText('2401–2438 of 2438 songs');
  await expect(
    page.getByRole('button', { name: '25 / 25, Page 25 of 25. Jump to page' }),
  ).toBeFocused();

  await page.goto('/?mockCatalogCount=10000');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  const hundredPageTrigger = page.getByRole('button', {
    name: '1 / 100, Page 1 of 100. Jump to page',
  });
  const beforeHundred = await readBuiltinPagerGeometry(page);
  await hundredPageTrigger.click();
  const hundredPageInput = page.getByRole('textbox', { name: 'Page number, from 1 to 100' });
  await expect(hundredPageInput).toBeFocused();
  await hundredPageInput.fill('100');
  const editingHundred = await readBuiltinPagerGeometry(page);
  const editorHundredGeometry = await readBuiltinPagerEditorGeometry(page);
  expect(editorHundredGeometry.slot.width).toBeGreaterThan(88);
  expect(editorHundredGeometry.input.width).toBeGreaterThanOrEqual(20);
  expect(editorHundredGeometry.confirm.width).toBeGreaterThanOrEqual(32);
  expect(editorHundredGeometry.confirm.height).toBeGreaterThanOrEqual(32);
  expect(editorHundredGeometry.pageCountFits).toBe(true);
  expect(editorHundredGeometry.receivesPointer).toBe(true);
  expectBuiltinPagerGeometryStable(beforeHundred, editingHundred);
  await hundredPageInput.press('Enter');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 100' })).toBeVisible();
  await expect(page.locator('.builtin-pager-range')).toHaveText('9901–10000 of 10000 songs');
  await expect(
    page.getByRole('button', { name: '100 / 100, Page 100 of 100. Jump to page' }),
  ).toBeFocused();
  const hundredPageGlyphs = await readBuiltinPagerGlyphGeometry(page);
  expectBuiltinPagerIndicatorFitsCompactly(hundredPageGlyphs);
  expectBuiltinPagerGeometryStable(beforeHundred, await readBuiltinPagerGeometry(page));
  const hundredPageZoomMeasurements: Array<Record<string, unknown>> = [];
  for (const zoom of [1.25, 1.5]) {
    await page.evaluate((scale) => {
      document.documentElement.style.zoom = String(scale);
    }, zoom);
    await waitForBrowserLayoutFrames(page);
    const view = await readBuiltinPagerGeometry(page);
    const glyphs = await readBuiltinPagerGlyphGeometry(page);
    expectBuiltinPagerIndicatorFitsCompactly(glyphs, zoom);
    const zoomViewPosition = await page.evaluate(() => {
      const workbench = document.querySelector<HTMLElement>('.workbench');
      const player = document.querySelector<HTMLElement>('.player-bar');
      return {
        scrollY: window.scrollY,
        visualOffsetTop: window.visualViewport?.offsetTop,
        rootZoom: getComputedStyle(document.documentElement).zoom,
        appShellScrollTop: document.querySelector<HTMLElement>('.app-shell')?.scrollTop,
        workbenchY: workbench?.getBoundingClientRect().y,
        playerY: player?.getBoundingClientRect().y,
      };
    });

    const trigger = page.getByRole('button', {
      name: '100 / 100, Page 100 of 100. Jump to page',
    });
    await trigger.focus();
    await page.keyboard.press('Enter');
    const input = page.getByRole('textbox', { name: 'Page number, from 1 to 100' });
    await expect(input).toBeFocused();
    const editing = await readBuiltinPagerGeometry(page);
    const zoomEditPosition = await page.evaluate(() => {
      const workbench = document.querySelector<HTMLElement>('.workbench');
      const player = document.querySelector<HTMLElement>('.player-bar');
      return {
        scrollY: window.scrollY,
        visualOffsetTop: window.visualViewport?.offsetTop,
        rootZoom: getComputedStyle(document.documentElement).zoom,
        appShellScrollTop: document.querySelector<HTMLElement>('.app-shell')?.scrollTop,
        workbenchY: workbench?.getBoundingClientRect().y,
        playerY: player?.getBoundingClientRect().y,
      };
    });
    console.info(
      '[F3.4 hundred-page zoom geometry]',
      JSON.stringify({
        zoom,
        zoomViewPosition,
        zoomEditPosition,
        indicator: {
          slotWidth: view.indicator.width,
          textWidth: glyphs.pageText.width,
          leftGap: glyphs.internalLeftSpace,
          rightGap: glyphs.internalRightSpace,
        },
        viewFooterOffset: view.toolbar.y - view.player.y,
        editFooterOffset: editing.toolbar.y - editing.player.y,
        editToolbar: editing.toolbar,
      }),
    );
    expectBuiltinPagerLayoutStableDuringZoomedFocus(view, editing);
    const editor = await readBuiltinPagerEditorGeometry(page);
    expect(editor.input.width).toBeGreaterThanOrEqual(20 * zoom);
    expect(editor.confirm.width).toBeGreaterThanOrEqual(32 * zoom);
    expect(editor.confirm.height).toBeGreaterThanOrEqual(32 * zoom);
    expect(editor.pageCountFits).toBe(true);
    expect(editor.receivesPointer).toBe(true);
    await input.press('Tab');
    await expect(page.getByRole('button', { name: 'Confirm page change' })).toBeFocused();
    await input.focus();
    await input.press('Escape');
    await expect(trigger).toBeFocused();
    expectBuiltinPagerLayoutStableDuringZoomedFocus(view, await readBuiltinPagerGeometry(page));
    hundredPageZoomMeasurements.push({ zoom, glyphs, editor });
    await page.evaluate(() => document.documentElement.style.removeProperty('zoom'));
    await waitForBrowserLayoutFrames(page);
  }
  console.info('[F3.4 hundred-page zoom geometry]', JSON.stringify(hundredPageZoomMeasurements));
  measurements.push({
    total: 10_000,
    before: beforeHundred,
    editing: editingHundred,
    editor: editorHundredGeometry,
    afterGlyphs: hundredPageGlyphs,
    zoom: hundredPageZoomMeasurements,
    after: await readBuiltinPagerGeometry(page),
  });

  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockCatalogCount=10000');
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '1366');
  for (const width of [520, 440]) {
    await constrainBrowserPaneForPagerTest(page, width);
    const trigger = page.getByRole('button', {
      name: '1 / 100, Page 1 of 100. Jump to page',
    });
    const before = await readBuiltinPagerGeometry(page);
    await trigger.click();
    const field = page.getByRole('textbox', { name: 'Page number, from 1 to 100' });
    await expect(field).toBeFocused();
    const during = await readBuiltinPagerGeometry(page);
    expectBuiltinPagerGeometryStable(before, during);
    const confirm = page.getByRole('button', { name: 'Confirm page change' });
    const confirmGeometry = await confirm.boundingBox();
    expect(confirmGeometry).not.toBeNull();
    expect(confirmGeometry!.width).toBeGreaterThanOrEqual(32);
    expect(confirmGeometry!.height).toBeGreaterThanOrEqual(32);
    expect(
      await confirm.evaluate((button) => {
        const rect = button.getBoundingClientRect();
        const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
        return hit === button || button.contains(hit);
      }),
    ).toBe(true);
    const geometry = await page.locator('.builtin-pager-wrap').evaluate((element) => ({
      width: element.clientWidth,
      scrollWidth: element.scrollWidth,
      pagerWidth: element.querySelector('.builtin-pager')?.getBoundingClientRect().width ?? 0,
    }));
    expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.width);
    measurements.push({ paneWidth: width, before, during, geometry });
    await field.press('Escape');
    await expect(trigger).toBeFocused();
  }
  await page.locator('.track-browser-workbench-pane').evaluate((element) => {
    element.style.removeProperty('flex');
    element.style.removeProperty('min-width');
  });
  await testInfo.attach('f3.4-inline-page-editor-geometry.json', {
    body: JSON.stringify(measurements, null, 2),
    contentType: 'application/json',
  });
  console.info('[F3.4 inline pager geometry]', JSON.stringify(measurements));
});
