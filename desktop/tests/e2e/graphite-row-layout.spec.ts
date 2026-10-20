import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const workbenchPreferenceKey = 'sky.ui.workbench.v5';

async function openApp(
  page: Page,
  viewport: { width: number; height: number },
  navigatorPreference: 'expanded' | 'collapsed' = 'expanded',
) {
  await page.setViewportSize(viewport);
  await page.addInitScript(
    ({ key, preference }) => {
      window.localStorage.clear();
      window.localStorage.setItem(
        key,
        JSON.stringify({
          version: 5,
          navigatorPreference: preference,
          expandedNavigatorWidth: 240,
          utilityWidth: 320,
          utilityOpen: false,
        }),
      );
    },
    { key: workbenchPreferenceKey, preference: navigatorPreference },
  );
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
}

async function startDryRunWithSelection(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Candle Run/ }).click();
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await page
    .getByRole('dialog', { name: 'Playback profile' })
    .getByRole('button', { name: 'Test playback (no input)' })
    .click();
  await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await expect(page.getByRole('row', { name: /Blue Bird/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(page.getByRole('row', { name: /Candle Run/ })).toHaveAttribute(
    'aria-current',
    'true',
  );
  await expect(player.locator('.player-track-status')).toHaveText('Dry run · Playing');
  return player;
}

async function openInspector(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const trigger = player.getByRole('button', { name: 'Open utility panel' });
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
  await inspector.getByRole('tab', { name: 'Details' }).click();
  return inspector;
}

async function readGeometry(page: Page, viewportName: string) {
  return page.evaluate((name) => {
    const getRect = (element: Element | null) => {
      if (!(element instanceof HTMLElement)) return null;
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    const getStyle = (element: Element | null) => {
      if (!(element instanceof HTMLElement)) return null;
      const style = getComputedStyle(element);
      return {
        backgroundColor: style.backgroundColor,
        color: style.color,
        fontSize: style.fontSize,
        fontWeight: style.fontWeight,
        borderRadius: style.borderRadius,
        borderLeftWidth: style.borderLeftWidth,
        borderLeftColor: style.borderLeftColor,
        borderBottomWidth: style.borderBottomWidth,
        borderBottomColor: style.borderBottomColor,
        paddingInlineStart: style.paddingInlineStart,
        paddingInlineEnd: style.paddingInlineEnd,
        insetInlineStart: style.insetInlineStart,
        insetInlineEnd: style.insetInlineEnd,
      };
    };
    const rectFor = (selector: string) => getRect(document.querySelector(selector));
    const styleFor = (selector: string) => getStyle(document.querySelector(selector));
    const headerCells = Array.from(
      document.querySelectorAll<HTMLElement>('.track-table-header [role="columnheader"]'),
    ).filter((cell) => getComputedStyle(cell).display !== 'none');
    const selected = document.querySelector('.track-row.is-selected');
    const playing = document.querySelector('.track-row.is-playing');
    const hovered = document.querySelector('.track-row:hover');
    const navigator = document.querySelector('.navigator-workbench-pane');
    const tiles = Array.from(
      document.querySelectorAll<HTMLElement>(
        '.navigator-workbench-pane.is-collapsed .library-nav-item, .navigator-workbench-pane.is-collapsed .library-add-button, .navigator-workbench-pane.is-collapsed .library-collapse-button',
      ),
    );
    const preferenceText = localStorage.getItem('sky.ui.workbench.v5');
    const preference = preferenceText ? JSON.parse(preferenceText) : null;
    const documentWidth = document.documentElement.scrollWidth;
    const grid = document.querySelector<HTMLElement>('.track-table');
    const frameElement = document.querySelector<HTMLElement>('.track-table-frame');
    const headerElement = document.querySelector<HTMLElement>('.track-table-header');
    const gridStyle = grid ? getComputedStyle(grid) : null;
    const gridRect = grid?.getBoundingClientRect();
    const frameRect = frameElement?.getBoundingClientRect();
    const headerRect = headerElement?.getBoundingClientRect();
    const dividerStyle = headerElement ? getComputedStyle(headerElement, '::after') : null;
    const dividerStart = Number.parseFloat(dividerStyle?.insetInlineStart ?? '0');
    const dividerEnd = Number.parseFloat(dividerStyle?.insetInlineEnd ?? '0');
    const dividerHeight = Number.parseFloat(dividerStyle?.height ?? '0');
    const dividerBottom = Number.parseFloat(dividerStyle?.bottom ?? '0');
    return {
      view: name,
      viewport: { width: innerWidth, height: innerHeight },
      palette: document.documentElement.dataset.palette,
      savedNavigatorPreference: preference?.navigatorPreference ?? null,
      documentWidth,
      horizontalOverflow: documentWidth > innerWidth,
      trackGridHorizontalOverflow: grid ? grid.scrollWidth > grid.clientWidth + 1 : false,
      panes: {
        workbench: rectFor('.workbench'),
        navigator: getRect(navigator),
        trackBrowser: rectFor('.track-browser-workbench-pane'),
        inspector: rectFor('.utility-workbench-pane'),
        dock: rectFor('.player-bar'),
      },
      trackGrid: {
        scrollport: gridRect
          ? {
              x: gridRect.x,
              y: gridRect.y,
              width: gridRect.width,
              right: gridRect.right,
              clientWidth: grid?.clientWidth ?? null,
              offsetWidth: grid?.offsetWidth ?? null,
              clientHeight: grid?.clientHeight ?? null,
              offsetHeight: grid?.offsetHeight ?? null,
              scrollHeight: grid?.scrollHeight ?? null,
              scrollWidth: grid?.scrollWidth ?? null,
              clientLeft: grid?.clientLeft ?? null,
              clientRight:
                gridRect && grid ? gridRect.left + grid.clientLeft + grid.clientWidth : null,
              scrollbarGutter: gridStyle?.scrollbarGutter ?? null,
              classicScrollbarReservation:
                grid && gridStyle
                  ? grid.offsetWidth -
                    grid.clientWidth -
                    grid.clientLeft -
                    Number.parseFloat(gridStyle.borderRightWidth)
                  : null,
            }
          : null,
        frame: frameRect
          ? {
              x: frameRect.x,
              y: frameRect.y,
              width: frameRect.width,
              right: frameRect.right,
            }
          : null,
        header: headerRect
          ? {
              x: headerRect.x,
              y: headerRect.y,
              width: headerRect.width,
              height: headerRect.height,
              right: headerRect.right,
              bottom: headerRect.bottom,
            }
          : null,
        divider:
          headerRect && dividerStyle
            ? {
                x: headerRect.x + dividerStart,
                y: headerRect.bottom - dividerBottom - dividerHeight,
                width: headerRect.width - dividerStart - dividerEnd,
                height: dividerHeight,
                right: headerRect.right - dividerEnd,
                color: dividerStyle.backgroundColor,
                content: dividerStyle.content,
                pointerEvents: dividerStyle.pointerEvents,
                position: dividerStyle.position,
              }
            : null,
      },
      navigatorTiles: tiles.map((tile) => ({ rect: getRect(tile), style: getStyle(tile) })),
      trackBrowserContainerWidth: (() => {
        const pane = document.querySelector<HTMLElement>('.track-browser-workbench-pane');
        if (!pane) return null;
        const style = getComputedStyle(pane);
        return (
          pane.getBoundingClientRect().width -
          Number.parseFloat(style.borderLeftWidth) -
          Number.parseFloat(style.borderRightWidth)
        );
      })(),
      surfaces: {
        browser: styleFor('.track-browser-workbench-pane'),
        browserSection: styleFor('.track-browser'),
        browserHeader: styleFor('.track-browser-header'),
        sourceLabel: styleFor('.track-browser-source-label'),
        table: styleFor('.track-table'),
        columnHeader: styleFor('.track-table-header'),
        columnRule: styleFor('.track-table-header'),
      },
      rows: Object.fromEntries(
        [
          ['selected', selected],
          ['playing', playing],
          ['hovered', hovered],
        ].map(([key, row]) => {
          const headerRects = headerCells.map(getRect);
          const rowCells =
            row instanceof HTMLElement
              ? Array.from(row.querySelectorAll<HTMLElement>('[role="gridcell"]'))
                  .filter((cell) => getComputedStyle(cell).display !== 'none')
                  .map(getRect)
              : [];
          return [
            key,
            {
              rect: getRect(row),
              rowFillInsets:
                row instanceof HTMLElement && frameRect
                  ? {
                      left: row.getBoundingClientRect().left - frameRect.left,
                      right: frameRect.right - row.getBoundingClientRect().right,
                    }
                  : null,
              style: getStyle(row),
              ariaSelected: row instanceof HTMLElement ? row.getAttribute('aria-selected') : null,
              ariaCurrent: row instanceof HTMLElement ? row.getAttribute('aria-current') : null,
              cellRects: rowCells,
              headerCellRects: headerRects,
            },
          ];
        }),
      ),
      identities: {
        dockTitle: document.querySelector('.player-track-copy strong')?.textContent ?? null,
        dockStatus: document.querySelector('.player-track-status')?.textContent?.trim() ?? null,
        detailsContext: document.querySelector('.song-details-context')?.textContent ?? null,
        detailsTitle: document.querySelector('.song-details-heading h3')?.textContent ?? null,
      },
    };
  }, viewportName);
}

function expectRowGeometry(
  geometry: Awaited<ReturnType<typeof readGeometry>>,
  requireClassicScrollbar = false,
) {
  const frame = geometry.trackGrid.frame;
  const scrollport = geometry.trackGrid.scrollport;
  const header = geometry.trackGrid.header;
  const pane = geometry.panes.trackBrowser;
  const browserWidth = geometry.trackBrowserContainerWidth;
  if (!frame || !scrollport || !header || !pane || browserWidth === null) {
    throw new Error('Track Browser grid geometry is unavailable');
  }
  const compactColumns = browserWidth <= 440;
  const narrowColumns = browserWidth > 440 && browserWidth <= 520;
  const expectedInset = compactColumns ? 4 : 6;
  const expectedRowPadding = compactColumns
    ? { left: 2, right: 4, headerLeft: 8, headerRight: 4 }
    : narrowColumns
      ? { left: 4, right: 6, headerLeft: 12, headerRight: 6 }
      : { left: 10, right: 12, headerLeft: 18, headerRight: 12 };
  const divider = geometry.trackGrid.divider;
  expect(Math.abs(frame.x - (scrollport.x + scrollport.clientLeft!))).toBeLessThanOrEqual(1);
  expect(Math.abs(frame.width - scrollport.clientWidth!)).toBeLessThanOrEqual(1);
  expect(Math.abs(frame.right - scrollport.clientRight!)).toBeLessThanOrEqual(1);
  expect(Math.abs(frame.x - header.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(frame.right - header.right)).toBeLessThanOrEqual(1);
  expect(header.height).toBe(32);
  expect(geometry.surfaces.columnRule?.borderBottomWidth).toBe('0px');
  expect(divider).not.toBeNull();
  if (!divider) throw new Error('Graphite table divider pseudo-element is unavailable');
  expect(divider).toMatchObject({
    x: header.x + expectedRowPadding.headerLeft,
    y: header.bottom - 1,
    width: header.width - expectedRowPadding.headerLeft - expectedRowPadding.headerRight,
    height: 1,
    right: header.right - expectedRowPadding.headerRight,
    color: 'rgb(42, 43, 44)',
    content: '""',
    pointerEvents: 'none',
    position: 'absolute',
  });
  expect(scrollport.classicScrollbarReservation).toBeGreaterThanOrEqual(0);
  if (requireClassicScrollbar) {
    expect(scrollport.classicScrollbarReservation).toBeGreaterThan(0);
  }
  expect(geometry.trackGrid.scrollport!.scrollWidth).toBeLessThanOrEqual(
    geometry.trackGrid.scrollport!.clientWidth + 1,
  );

  for (const key of ['selected', 'playing', 'hovered'] as const) {
    const row = geometry.rows[key];
    if (!row?.rect) throw new Error(`The ${key} row has no bounding box`);
    expect(row.rowFillInsets).not.toBeNull();
    expect(Math.abs(row.rowFillInsets!.left - expectedInset)).toBeLessThanOrEqual(2);
    expect(Math.abs(row.rowFillInsets!.right)).toBeLessThanOrEqual(1);
    expect(row.style.insetInlineStart).toBe(`${expectedInset}px`);
    expect(row.style.insetInlineEnd).toBe('0px');
    expect(row.style.paddingInlineStart).toBe(`${expectedRowPadding.left}px`);
    expect(row.style.paddingInlineEnd).toBe(`${expectedRowPadding.right}px`);
    expect(geometry.surfaces.columnHeader?.paddingInlineStart).toBe(
      `${expectedRowPadding.headerLeft}px`,
    );
    expect(geometry.surfaces.columnHeader?.paddingInlineEnd).toBe(
      `${expectedRowPadding.headerRight}px`,
    );
    const paneLeftInset = row.rect.x - (pane.x + 1);
    const paneRightInset = pane.right - 1 - row.rect.right;
    expect(Math.abs(paneLeftInset - row.rowFillInsets!.left)).toBeLessThanOrEqual(1);
    expect(
      Math.abs(paneRightInset - row.rowFillInsets!.right - scrollport.classicScrollbarReservation!),
    ).toBeLessThanOrEqual(1);
    expect(row.rect.height).toBeCloseTo(46, 0);
    expect(row.style.borderRadius).toBe('4px');
    row.cellRects.forEach((cell: { x: number; width: number } | null, index: number) => {
      const header = row.headerCellRects[index];
      if (!cell || !header) throw new Error('A row or header cell has no bounding box');
      expect(
        Math.abs(cell.x - header.x),
        `${geometry.view} ${key} cell ${index} x`,
      ).toBeLessThanOrEqual(2);
      expect(
        Math.abs(cell.width - header.width),
        `${geometry.view} ${key} cell ${index} width`,
      ).toBeLessThanOrEqual(2);
      expect(
        Math.abs(cell.x + cell.width - (header.x + header.width)),
        `${geometry.view} ${key} cell ${index} right`,
      ).toBeLessThanOrEqual(2);
    });
  }
}

test('Graphite row surfaces, collapsed navigation, and dock geometry remain functional', async ({
  page,
}) => {
  test.setTimeout(60_000);
  page.setDefaultTimeout(8_000);
  page.setDefaultNavigationTimeout(8_000);
  const evidence: unknown[] = [];

  await openApp(page, { width: 1366, height: 768 });
  const player = await startDryRunWithSelection(page);
  const details = await openInspector(page);
  await expect(details.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  mkdirSync(resolve(process.cwd(), 'test-results'), { recursive: true });
  await page.screenshot({
    path: resolve(process.cwd(), 'test-results/graphite-workbench-1366x768.png'),
    animations: 'disabled',
  });
  await page.setViewportSize({ width: 789, height: 555 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '789');
  await page.keyboard.press('/');
  const nativeLikeSearch = page.getByRole('searchbox', { name: 'Search All Songs' });
  await expect(nativeLikeSearch).toBeFocused();
  const nativeLikeHoveredRow = page.getByRole('row', { name: /Aurora Landing/ });
  await nativeLikeHoveredRow.hover({ position: { x: 24, y: 23 } });
  await expect.poll(() => nativeLikeHoveredRow.evaluate((row) => row.matches(':hover'))).toBe(true);
  const nativeLikeGeometry = await readGeometry(page, 'native-like-789x555');
  expectRowGeometry(nativeLikeGeometry);
  await page.screenshot({
    path: resolve(process.cwd(), 'test-results/graphite-workbench-789x555.png'),
    animations: 'disabled',
  });
  await page.setViewportSize({ width: 1366, height: 768 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '1366');
  const scrollbarSimulation = await page.addStyleTag({
    content: `
      .track-table {
        overflow-y: scroll !important;
        scrollbar-gutter: stable !important;
        scrollbar-width: auto !important;
      }
      .track-table::-webkit-scrollbar {
        width: 16px !important;
      }
    `,
  });
  const hoveredRow = page.getByRole('row', { name: /Aurora Landing/ });
  await hoveredRow.hover();
  await expect.poll(() => hoveredRow.evaluate((row) => row.matches(':hover'))).toBe(true);
  await expect(details.locator('.song-details-context')).toHaveCount(0);
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run');
  evidence.push(await readGeometry(page, 'populated-1366x768'));
  await scrollbarSimulation.evaluate((style) => style.remove());
  const wideNativeScrollbar = await readGeometry(page, 'populated-1366x768-native-scrollbar');

  const likeButton = hoveredRow.getByRole('button', { name: /Add Aurora Landing to Liked Songs/ });
  await expect(likeButton).toBeVisible();
  const likeTarget = await likeButton.boundingBox();
  expect(likeTarget).not.toBeNull();
  if (likeTarget) {
    const hit = await page.evaluate(
      ({ x, y }) => {
        const element = document.elementFromPoint(x, y);
        return element?.closest('.track-like-button') !== null;
      },
      { x: likeTarget.x + likeTarget.width / 2, y: likeTarget.y + likeTarget.height / 2 },
    );
    expect(hit).toBe(true);
  }
  await likeButton.focus();
  await expect(likeButton).toBeFocused();
  await likeButton.click();
  await expect(hoveredRow).toHaveAttribute('aria-selected', 'false');
  const moreButton = hoveredRow.getByRole('button', { name: 'More actions for Aurora Landing' });
  await moreButton.focus();
  await expect(moreButton).toBeFocused();
  await moreButton.click();
  await expect(page.getByRole('menu', { name: 'Actions for Aurora Landing' })).toBeVisible();
  await page.keyboard.press('Escape');

  await openApp(page, { width: 800, height: 560 }, 'collapsed');
  const everydayPlayer = await startDryRunWithSelection(page);
  const everydayHovered = page.getByRole('row', { name: /Aurora Landing/ });
  await everydayHovered.hover();
  await expect.poll(() => everydayHovered.evaluate((row) => row.matches(':hover'))).toBe(true);
  await expect(page.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  await expect(everydayPlayer.locator('.player-track-status')).toHaveText('Dry run · Playing');
  evidence.push(await readGeometry(page, 'collapsed-800x560'));

  await openApp(page, { width: 640, height: 448 }, 'expanded');
  const compactPlayer = await startDryRunWithSelection(page);
  await openInspector(page);
  const compactHovered = page.getByRole('row', { name: /Aurora Landing/ });
  await compactHovered.hover({ position: { x: 24, y: 23 } });
  await expect.poll(() => compactHovered.evaluate((row) => row.matches(':hover'))).toBe(true);
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(compactPlayer.locator('.player-track-status')).toHaveText('Dry run · Playing');
  await expect(page.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  const savedPreference = await page.evaluate(
    (key) => JSON.parse(window.localStorage.getItem(key) ?? 'null')?.navigatorPreference,
    workbenchPreferenceKey,
  );
  expect(savedPreference).toBe('expanded');
  evidence.push(await readGeometry(page, 'compact-overlay-640x448'));

  const wide = evidence[0] as Awaited<ReturnType<typeof readGeometry>>;
  const collapsed = evidence[1] as Awaited<ReturnType<typeof readGeometry>>;
  const compact = evidence[2] as Awaited<ReturnType<typeof readGeometry>>;
  expectRowGeometry(wide, true);
  for (const geometry of [wideNativeScrollbar, collapsed, compact]) expectRowGeometry(geometry);
  for (const geometry of [wide, wideNativeScrollbar, collapsed, compact, nativeLikeGeometry]) {
    expect(geometry.horizontalOverflow).toBe(false);
    expect(geometry.trackGridHorizontalOverflow).toBe(false);
  }

  for (const geometry of [wide, wideNativeScrollbar, collapsed, compact]) {
    for (const surface of [
      geometry.surfaces.browser,
      geometry.surfaces.browserSection,
      geometry.surfaces.browserHeader,
      geometry.surfaces.sourceLabel,
      geometry.surfaces.table,
      geometry.surfaces.columnHeader,
    ]) {
      expect(surface?.backgroundColor).toBe('rgb(21, 22, 24)');
    }
  }
  expect(wide.surfaces.columnRule?.borderBottomWidth).toBe('0px');
  expect(wide.trackGrid.divider?.color).toBe('rgb(42, 43, 44)');
  expect(wide.rows.selected?.style.backgroundColor).toBe('rgb(37, 36, 44)');
  expect(wide.rows.hovered?.style.backgroundColor).toBe('rgb(34, 35, 36)');
  expect(wide.rows.playing?.ariaCurrent).toBe('true');
  expect(wide.rows.selected?.ariaSelected).toBe('true');
  expect(wide.identities).toMatchObject({
    dockTitle: 'Candle Run',
    detailsTitle: 'Blue Bird',
    detailsContext: null,
    dockStatus: 'Dry run · Playing',
  });

  expect(collapsed.panes.navigator?.width).toBeCloseTo(48, 0);
  for (const tile of collapsed.navigatorTiles) {
    expect(tile.rect?.width).toBeCloseTo(40, 0);
    expect(tile.rect?.height).toBeCloseTo(40, 0);
    const pane = collapsed.panes.navigator;
    if (!pane || !tile.rect) throw new Error('Collapsed navigator tile geometry is unavailable');
    expect(
      Math.abs(tile.rect.x + tile.rect.width / 2 - (pane.x + pane.width / 2)),
    ).toBeLessThanOrEqual(1);
  }
  expect(collapsed.identities.dockStatus).toBe('Dry run · Playing');
  expect(collapsed.panes.inspector).toBeNull();

  expect(compact.panes.navigator?.width).toBeCloseTo(48, 0);
  expect(compact.panes.inspector).not.toBeNull();
  expect(compact.identities.dockStatus).toBe('Dry run · Playing');
  expect(compact.savedNavigatorPreference).toBe('expanded');

  const pane = page.locator('.track-browser-workbench-pane');
  const originalFlex = await pane.evaluate((element) => element.style.flex);
  await pane.evaluate((element) => {
    element.style.flex = '0 0 438px';
  });
  await expect
    .poll(() =>
      pane.evaluate((element) => {
        const style = getComputedStyle(element);
        return (
          element.getBoundingClientRect().width -
          Number.parseFloat(style.borderLeftWidth) -
          Number.parseFloat(style.borderRightWidth)
        );
      }),
    )
    .toBeLessThanOrEqual(440);
  const narrowGeometry = await readGeometry(page, 'narrow-browser-container');
  expectRowGeometry(narrowGeometry);
  expect(narrowGeometry.horizontalOverflow).toBe(false);
  expect(narrowGeometry.trackGridHorizontalOverflow).toBe(false);
  for (const surface of [
    narrowGeometry.surfaces.browser,
    narrowGeometry.surfaces.browserSection,
    narrowGeometry.surfaces.browserHeader,
    narrowGeometry.surfaces.sourceLabel,
    narrowGeometry.surfaces.table,
    narrowGeometry.surfaces.columnHeader,
  ]) {
    expect(surface?.backgroundColor).toBe('rgb(21, 22, 24)');
  }
  evidence.push(narrowGeometry);
  await pane.evaluate((element, flex) => {
    element.style.flex = flex;
  }, originalFlex);

  await page.getByRole('button', { name: 'Expand library navigator' }).click();
  await expect(page.locator('.navigator-workbench-pane')).not.toHaveClass(/is-collapsed/);
  const manuallyExpandedNavigator = await page.locator('.navigator-workbench-pane').boundingBox();
  expect(manuallyExpandedNavigator?.width).toBeCloseTo(208, 0);
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
    .toBeLessThanOrEqual(640);
  const preferenceDuringExpansion = await page.evaluate(
    (key) => JSON.parse(window.localStorage.getItem(key) ?? 'null')?.navigatorPreference,
    workbenchPreferenceKey,
  );
  expect(preferenceDuringExpansion).toBe('expanded');

  await page.getByRole('button', { name: 'Close utility panel' }).click();
  await page.setViewportSize({ width: 800, height: 560 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '800');
  await expect(page.locator('.utility-workbench-pane')).toHaveCount(0);
  await expect(page.locator('.navigator-workbench-pane')).not.toHaveClass(/is-collapsed/);
  const restoredPreference = await page.evaluate(
    (key) => JSON.parse(window.localStorage.getItem(key) ?? 'null')?.navigatorPreference,
    workbenchPreferenceKey,
  );
  expect(restoredPreference).toBe('expanded');
  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '640');
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  const preferenceAfterReturn = await page.evaluate(
    (key) => JSON.parse(window.localStorage.getItem(key) ?? 'null')?.navigatorPreference,
    workbenchPreferenceKey,
  );
  expect(preferenceAfterReturn).toBe('expanded');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const idlePlayer = page.getByRole('contentinfo', { name: 'Player controls' });
  await expect(idlePlayer.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(idlePlayer.locator('.player-track-status')).toHaveText('Ready to play');
  await expect(idlePlayer.locator('.player-identity-label')).toHaveCount(0);
  await expect(page.getByRole('row', { name: /Blue Bird/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );

  const preferenceBefore = await page.evaluate(
    (key) => JSON.parse(window.localStorage.getItem(key) ?? 'null')?.navigatorPreference,
    workbenchPreferenceKey,
  );
  expect(preferenceBefore).toBe('expanded');
  await page.setViewportSize({ width: 800, height: 560 });
  await expect(page.locator('.navigator-workbench-pane')).not.toHaveClass(/is-collapsed/);
  const preferenceAfter = await page.evaluate(
    (key) => JSON.parse(window.localStorage.getItem(key) ?? 'null')?.navigatorPreference,
    workbenchPreferenceKey,
  );
  expect(preferenceAfter).toBe('expanded');

  await openApp(page, { width: 1366, height: 768 });
  await startDryRunWithSelection(page);
  const widePane = page.locator('.track-browser-workbench-pane');
  const originalWideFlex = await widePane.evaluate((element) => element.style.flex);
  await widePane.evaluate((element) => {
    element.style.flex = '0 0 502px';
  });
  const mediumBrowserWidth = () =>
    widePane.evaluate((element) => {
      const style = getComputedStyle(element);
      return (
        element.getBoundingClientRect().width -
        Number.parseFloat(style.borderLeftWidth) -
        Number.parseFloat(style.borderRightWidth)
      );
    });
  await expect.poll(mediumBrowserWidth).toBeGreaterThan(440);
  await expect.poll(mediumBrowserWidth).toBeLessThanOrEqual(520);
  await page.getByRole('row', { name: /Aurora Landing/ }).hover();
  const mediumGeometry = await readGeometry(page, 'medium-browser-container');
  expectRowGeometry(mediumGeometry);
  expect(mediumGeometry.horizontalOverflow).toBe(false);
  expect(mediumGeometry.trackGridHorizontalOverflow).toBe(false);
  for (const surface of [
    mediumGeometry.surfaces.browser,
    mediumGeometry.surfaces.browserSection,
    mediumGeometry.surfaces.browserHeader,
    mediumGeometry.surfaces.sourceLabel,
    mediumGeometry.surfaces.table,
    mediumGeometry.surfaces.columnHeader,
  ]) {
    expect(surface?.backgroundColor).toBe('rgb(21, 22, 24)');
  }
  const dividerReport = (geometry: Awaited<ReturnType<typeof readGeometry>>) => ({
    view: geometry.view,
    browserWidth: geometry.trackBrowserContainerWidth,
    header: geometry.trackGrid.header,
    headerPadding: {
      left: geometry.surfaces.columnHeader?.paddingInlineStart,
      right: geometry.surfaces.columnHeader?.paddingInlineEnd,
    },
    divider: geometry.trackGrid.divider,
    classicScrollbarReservation: geometry.trackGrid.scrollport?.classicScrollbarReservation,
    selectedRowFillInsets: geometry.rows.selected?.rowFillInsets,
  });
  console.log(
    `Graphite divider/scrollbar geometry: ${JSON.stringify({
      nativeLike: dividerReport(nativeLikeGeometry),
      wideClassicScrollbar: dividerReport(wide),
      medium: dividerReport(mediumGeometry),
      narrow: dividerReport(narrowGeometry),
    })}`,
  );
  await widePane.evaluate((element, flex) => {
    element.style.flex = flex;
  }, originalWideFlex);

  await page.getByRole('searchbox', { name: 'Search All Songs' }).fill('no graphite match 9472');
  await expect(page.getByText('No songs match your search', { exact: true })).toBeVisible();
  const emptyStateGeometry = await page.evaluate(() => {
    const pane = document.querySelector<HTMLElement>('.track-browser-workbench-pane');
    const browser = document.querySelector<HTMLElement>('.track-browser');
    const empty = document.querySelector<HTMLElement>('.empty-state');
    return {
      documentWidth: document.documentElement.scrollWidth,
      rowCount: document.querySelectorAll('.track-row').length,
      hasScrollport: document.querySelector('.track-table') !== null,
      paneBackground: pane ? getComputedStyle(pane).backgroundColor : null,
      browserBackground: browser ? getComputedStyle(browser).backgroundColor : null,
      emptyBackground: empty ? getComputedStyle(empty).backgroundColor : null,
    };
  });
  expect(emptyStateGeometry.documentWidth).toBeLessThanOrEqual(1366);
  expect(emptyStateGeometry.rowCount).toBe(0);
  expect(emptyStateGeometry.hasScrollport).toBe(false);
  expect(emptyStateGeometry.paneBackground).toBe('rgb(21, 22, 24)');
  expect(emptyStateGeometry.browserBackground).toBe('rgb(21, 22, 24)');
  expect(emptyStateGeometry.emptyBackground).toBe('rgba(0, 0, 0, 0)');
});
