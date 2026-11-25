import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, type Page, test } from '@playwright/test';
import { chooseSelectOption, palettes } from './testHelpers';

const captureStage = process.env.GRAPHITE_WORKSPACE_CAPTURE;
const requestedCaptureViews = process.env.GRAPHITE_WORKSPACE_CAPTURE_VIEWS?.split(',')
  .map((view) => view.trim())
  .filter(Boolean);
const evidenceDirectory = resolve(process.cwd(), 'test-results/graphite-workspace-checkpoint');
const workbenchPreferenceKey = 'sky.ui.workbench.v5';
const playlistName = 'Evening Practice Playlist With A Long Descriptive Name';

async function openApp(
  page: Page,
  viewport: { width: number; height: number },
  navigatorPreference: 'expanded' | 'collapsed' = 'expanded',
  utilityOpen = false,
) {
  await page.setViewportSize(viewport);
  await page.addInitScript(
    ({ key, preference, openUtility }) => {
      window.localStorage.clear();
      window.localStorage.setItem(
        key,
        JSON.stringify({
          version: 5,
          navigatorPreference: preference,
          expandedNavigatorWidth: 240,
          utilityWidth: 320,
          utilityOpen: openUtility,
        }),
      );
    },
    { key: workbenchPreferenceKey, preference: navigatorPreference, openUtility: utilityOpen },
  );
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.locator('[data-app-shell]')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
}

async function startDryRunWithSelection(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const playingRow = page.getByRole('row', { name: /Candle Run/ });
  await playingRow.click();
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
  await expect(playingRow).toHaveAttribute('aria-current', 'true');
  await expect(player.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  return player;
}

async function openDetails(page: Page) {
  const utilityToggle = page.locator('button[aria-label$="utility panel"]');
  await expect(utilityToggle).toBeVisible();
  if ((await utilityToggle.getAttribute('aria-expanded')) !== 'true') await utilityToggle.click();
  const inspector = page.locator('[role="region"][aria-label^="Utility:"]');
  await expect(inspector).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator('[data-pane="inspector"]')
        .evaluate((element) => getComputedStyle(element).translate),
    )
    .toBe('none');
  await inspector.getByRole('tab', { name: 'Details' }).click();
  return inspector;
}

async function openRuntimeDiagnostics(page: Page) {
  const utilityToggle = page.locator('button[aria-label$="utility panel"]');
  await expect(utilityToggle).toBeVisible();
  if ((await utilityToggle.getAttribute('aria-expanded')) !== 'true') await utilityToggle.click();
  const inspector = page.locator('[role="region"][aria-label^="Utility:"]');
  await expect(inspector).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator('[data-pane="inspector"]')
        .evaluate((element) => getComputedStyle(element).translate),
    )
    .toBe('none');
  await inspector.getByRole('tab', { name: 'Runtime' }).click();
  await expect(page.getByRole('region', { name: 'Utility: Diagnostics' })).toBeVisible();
  return inspector;
}

async function createLongPlaylistAndOpenMenu(page: Page) {
  const createPlaylist = page.getByRole('button', { name: 'Create playlist' });
  if (await createPlaylist.isVisible()) {
    await createPlaylist.click();
  } else {
    await page.getByRole('button', { name: 'New playlist' }).click();
  }
  const dialog = page.getByRole('dialog', { name: 'New playlist' });
  await dialog.getByLabel('Playlist name').fill(playlistName);
  await dialog.getByRole('button', { name: 'Create' }).click();
  await expect(page.getByRole('heading', { name: playlistName })).toBeVisible();
  const addButton = page.getByRole('button', { name: 'Add songs' });
  await expect(addButton).toBeVisible();
  await addButton.click();
  const menu = page.getByRole('menu', { name: 'Add songs' });
  await expect(menu.getByRole('menuitem', { name: 'Browse Built-in Songs…' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Import files…' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Import folder…' })).toBeVisible();
  return menu;
}

async function capture(page: Page, name: string, movePointer = true) {
  mkdirSync(evidenceDirectory, { recursive: true });
  if (movePointer) await page.mouse.move(0, 0);
  await page.screenshot({
    path: resolve(evidenceDirectory, name),
    animations: 'disabled',
  });
}

async function captureContactSheet(
  page: Page,
  outputFile: string,
  items: { label: string; file: string }[],
) {
  const captures = items.map(({ label, file }) => ({
    label,
    data: readFileSync(resolve(evidenceDirectory, file)).toString('base64'),
  }));
  const png = await page.evaluate(async (tiles) => {
    const tileWidth = 500;
    const labelHeight = 28;
    const tileHeight = 281;
    const columns = 2;
    const rows = Math.ceil(tiles.length / columns);
    const canvas = document.createElement('canvas');
    canvas.width = tileWidth * columns;
    canvas.height = (tileHeight + labelHeight) * rows;
    const context = canvas.getContext('2d');
    if (!context) throw new Error('Could not create a canvas for the palette comparison.');
    context.fillStyle = '#0e0f11';
    context.fillRect(0, 0, canvas.width, canvas.height);
    for (const [index, tile] of tiles.entries()) {
      const image = new Image();
      image.src = `data:image/png;base64,${tile.data}`;
      await image.decode();
      const x = (index % columns) * tileWidth;
      const y = Math.floor(index / columns) * (tileHeight + labelHeight);
      context.fillStyle = '#0e0f11';
      context.fillRect(x, y, tileWidth, tileHeight + labelHeight);
      context.fillStyle = '#f2f3f5';
      context.font = '600 14px system-ui, sans-serif';
      context.fillText(tile.label, x + 10, y + 19);
      context.drawImage(image, x, y + labelHeight, tileWidth, tileHeight);
    }
    return canvas.toDataURL('image/png').slice('data:image/png;base64,'.length);
  }, captures);
  writeFileSync(resolve(evidenceDirectory, outputFile), Buffer.from(png, 'base64'));
}

async function capturePaletteContactSheet(page: Page) {
  await captureContactSheet(page, 'candidate-palette-comparison-contact-sheet.png', [
    { label: 'Light', file: 'candidate-appearance-preview-light-1366x768.png' },
    ...palettes.map((palette) => ({
      label: palette,
      file: `candidate-palette-${palette}-1366x768.png`,
    })),
  ]);
}

async function captureWorkbenchPaletteContactSheet(page: Page) {
  await captureContactSheet(page, 'candidate-workbench-palette-contact-sheet.png', [
    { label: 'Light', file: 'candidate-light-1366x768.png' },
    ...palettes.map((palette) => ({
      label: palette,
      file: `candidate-workbench-${palette}-1366x768.png`,
    })),
  ]);
}

function shouldCapture(view: string) {
  return !requestedCaptureViews?.length || requestedCaptureViews.includes(view);
}

async function readWorkspaceGeometry(page: Page) {
  return page.evaluate((preferenceKey) => {
    const rect = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    const trackTable = document.querySelector<HTMLElement>('[role="grid"]');
    const trackFrame = document.querySelector<HTMLElement>('[data-slot="track-table-frame"]');
    const scrollArea = trackTable?.closest<HTMLElement>('[data-slot="scroll-area"]');
    const verticalScrollbar = scrollArea?.querySelector<HTMLElement>(
      '[data-slot="scroll-area-scrollbar"][data-orientation="vertical"]',
    );
    const scrollbarThumb = verticalScrollbar?.querySelector<HTMLElement>(
      '[data-slot="scroll-area-thumb"]',
    );
    const browserPane = document.querySelector<HTMLElement>('[data-pane="browser"]');
    const tableHeader = document.querySelector<HTMLElement>('[role="row"][aria-rowindex="1"]');
    const headerCells = Array.from(
      document.querySelectorAll<HTMLElement>(
        '[role="row"][aria-rowindex="1"] [role="columnheader"]',
      ),
    ).filter((cell) => getComputedStyle(cell).display !== 'none');
    const rowMeasurements = (selector: string) => {
      const row = document.querySelector<HTMLElement>(selector);
      const rowRect = row?.getBoundingClientRect();
      const paneRect = browserPane?.getBoundingClientRect();
      const paneStyle = browserPane ? getComputedStyle(browserPane) : null;
      const contentLeft = paneRect
        ? paneRect.left + Number.parseFloat(paneStyle?.borderLeftWidth ?? '0')
        : null;
      const contentRight = paneRect
        ? paneRect.right - Number.parseFloat(paneStyle?.borderRightWidth ?? '0')
        : null;
      const cells = row
        ? Array.from(row.querySelectorAll<HTMLElement>('[role="gridcell"]')).filter(
            (cell) => getComputedStyle(cell).display !== 'none',
          )
        : [];
      return {
        rect: rowRect
          ? {
              x: rowRect.x,
              y: rowRect.y,
              width: rowRect.width,
              height: rowRect.height,
              right: rowRect.right,
              bottom: rowRect.bottom,
            }
          : null,
        paneEdgeInsets:
          rowRect && contentLeft !== null && contentRight !== null
            ? { left: rowRect.left - contentLeft, right: contentRight - rowRect.right }
            : null,
        rowFillInsets:
          rowRect && frameRect
            ? { left: rowRect.left - frameRect.left, right: frameRect.right - rowRect.right }
            : null,
        cells: cells.map((cell, index) => {
          const cellRect = cell.getBoundingClientRect();
          const headerRect = headerCells[index]?.getBoundingClientRect();
          return {
            name:
              headerCells[index]?.textContent?.trim() ||
              headerCells[index]?.getAttribute('aria-label') ||
              `column-${index + 1}`,
            row: { x: cellRect.x, width: cellRect.width, right: cellRect.right },
            header: headerRect
              ? { x: headerRect.x, width: headerRect.width, right: headerRect.right }
              : null,
            delta: headerRect
              ? {
                  left: cellRect.left - headerRect.left,
                  right: cellRect.right - headerRect.right,
                  width: cellRect.width - headerRect.width,
                }
              : null,
          };
        }),
      };
    };
    const tableStyle = trackTable ? getComputedStyle(trackTable) : null;
    const paneStyle = browserPane ? getComputedStyle(browserPane) : null;
    const tableRect = trackTable?.getBoundingClientRect();
    const frameRect = trackFrame?.getBoundingClientRect();
    const headerRect = tableHeader?.getBoundingClientRect();
    const titlebar = document.querySelector<HTMLElement>('[data-slot="titlebar"]');
    const titlebarBrand = document.querySelector<HTMLElement>('[data-slot="titlebar-brand"]');
    const titlebarSearch = document.querySelector<HTMLElement>('[data-slot="global-search"]');
    const titlebarEnd = document.querySelector<HTMLElement>('[data-slot="titlebar-trailing"]');
    const titlebarBrandRect = titlebarBrand?.getBoundingClientRect();
    const titlebarSearchRect = titlebarSearch?.getBoundingClientRect();
    const titlebarEndRect = titlebarEnd?.getBoundingClientRect();
    const collapseButton = document.querySelector<HTMLElement>(
      'button[aria-label="Collapse library navigator"], button[aria-label="Expand library navigator"]',
    );
    const collapseStyle = collapseButton ? getComputedStyle(collapseButton) : null;
    const inspectorPane = document.querySelector<HTMLElement>('[data-pane="inspector"]');
    const inspectorStyle = inspectorPane ? getComputedStyle(inspectorPane) : null;
    const utilityPreferenceText = localStorage.getItem(preferenceKey);
    const utilityPreference = utilityPreferenceText ? JSON.parse(utilityPreferenceText) : null;
    const rootStyle = getComputedStyle(document.documentElement);
    const freeRegionMidpoint =
      titlebarBrandRect && titlebarEndRect
        ? (titlebarBrandRect.right + titlebarEndRect.left) / 2
        : null;
    const computed = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const style = getComputedStyle(element);
      return {
        backgroundColor: style.backgroundColor,
        color: style.color,
        fontSize: style.fontSize,
        fontWeight: style.fontWeight,
        borderBottomWidth: style.borderBottomWidth,
        borderBottomColor: style.borderBottomColor,
      };
    };
    return {
      viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
      semanticTokens: {
        workbenchContent: rootStyle.getPropertyValue('--background').trim(),
        chromeSurface: rootStyle.getPropertyValue('--surface').trim(),
        paneEdge: rootStyle.getPropertyValue('--border').trim(),
        controlHover: rootStyle.getPropertyValue('--control-hover').trim(),
        controlBorder: rootStyle.getPropertyValue('--input').trim(),
        textPrimary: rootStyle.getPropertyValue('--foreground').trim(),
        textSecondary: rootStyle.getPropertyValue('--text-secondary').trim(),
        focusRing: rootStyle.getPropertyValue('--ring').trim(),
      },
      titlebar: {
        rect: rect('[data-slot="titlebar"]'),
        leading: rect('[data-slot="titlebar-leading"]'),
        brand: rect('[data-slot="titlebar-brand"]'),
        search: rect('[data-slot="global-search"]'),
        trailingActions: rect('[data-slot="titlebar-trailing"]'),
        leftDrag: rect('[data-slot="titlebar-drag-space"]'),
        rightDrag: rect('[data-slot="titlebar-drag-space-trailing"]'),
        freeRegionMidpoint,
        searchMidpoint: titlebarSearchRect
          ? titlebarSearchRect.left + titlebarSearchRect.width / 2
          : null,
        searchCenterError:
          freeRegionMidpoint !== null && titlebarSearchRect
            ? Math.abs(titlebarSearchRect.left + titlebarSearchRect.width / 2 - freeRegionMidpoint)
            : null,
        updateAvailable: titlebar?.dataset.updateAvailable ?? null,
        searchFocused:
          document.activeElement === document.querySelector('[data-slot="global-search"] input'),
      },
      collapseButton: {
        rect: rect(
          'button[aria-label="Collapse library navigator"], button[aria-label="Expand library navigator"]',
        ),
        borderColor: collapseStyle?.borderColor ?? null,
        borderWidth: collapseStyle?.borderWidth ?? null,
        backgroundColor: collapseStyle?.backgroundColor ?? null,
        color: collapseStyle?.color ?? null,
        outlineWidth: collapseStyle?.outlineWidth ?? null,
        outlineColor: collapseStyle?.outlineColor ?? null,
        outlineOffset: collapseStyle?.outlineOffset ?? null,
        boxShadow: collapseStyle?.boxShadow ?? null,
      },
      inspector: {
        presentation:
          document.querySelector<HTMLElement>('[data-slot="workbench"]')?.dataset
            .utilityPresentation ?? null,
        rect: rect('[data-pane="inspector"]'),
        widthStyle: inspectorPane?.style.width || null,
        persistedDockWidth: utilityPreference?.utilityWidth ?? null,
        borderColor: inspectorStyle?.borderColor ?? null,
        borderWidth: inspectorStyle?.borderWidth ?? null,
        borderRadius: inspectorStyle?.borderRadius ?? null,
      },
      browserContainer: rect('[data-pane="browser"]'),
      browserSection: rect('[data-slot="track-browser"]'),
      sourceHeader: rect('[data-slot="track-browser-header"]'),
      sourceLabel: rect('[data-slot="track-browser-header"] h1'),
      sourceTitle: rect('[data-slot="track-browser-header"] h1'),
      count: rect('[data-slot="track-browser-count"]'),
      tableHeader: rect('[role="row"][aria-rowindex="1"]'),
      firstSong: rect('[data-slot="track-row"]'),
      trackCanvas: {
        pane: rect('[data-pane="browser"]'),
        scrollport: tableRect
          ? { x: tableRect.x, y: tableRect.y, width: tableRect.width, right: tableRect.right }
          : null,
        frame: frameRect
          ? { x: frameRect.x, y: frameRect.y, width: frameRect.width, right: frameRect.right }
          : null,
        columnHeader: headerRect
          ? { x: headerRect.x, y: headerRect.y, width: headerRect.width, right: headerRect.right }
          : null,
        scrollportMetrics:
          trackTable && tableStyle
            ? {
                clientWidth: trackTable.clientWidth,
                offsetWidth: trackTable.offsetWidth,
                clientHeight: trackTable.clientHeight,
                offsetHeight: trackTable.offsetHeight,
                scrollHeight: trackTable.scrollHeight,
                scrollWidth: trackTable.scrollWidth,
                clientLeft: trackTable.clientLeft,
                reservedInline: trackTable.offsetWidth - trackTable.clientWidth,
                computedBorderLeft: tableStyle.borderLeftWidth,
                computedBorderRight: tableStyle.borderRightWidth,
                hasVerticalOverflow: scrollArea?.hasAttribute('data-has-overflow-y') ?? false,
              }
            : null,
        scrollbar: verticalScrollbar
          ? {
              visible: getComputedStyle(verticalScrollbar).display !== 'none',
              opacity: getComputedStyle(verticalScrollbar).opacity,
              pointerEvents: getComputedStyle(verticalScrollbar).pointerEvents,
              scrolling: verticalScrollbar.hasAttribute('data-scrolling'),
              dragging: verticalScrollbar.getAttribute('data-dragging'),
              width: verticalScrollbar.getBoundingClientRect().width,
              height: verticalScrollbar.getBoundingClientRect().height,
              thumbWidth: scrollbarThumb?.getBoundingClientRect().width ?? null,
              thumbHeight: scrollbarThumb?.getBoundingClientRect().height ?? null,
            }
          : null,
        paneBorders: paneStyle
          ? {
              left: paneStyle.borderLeftWidth,
              right: paneStyle.borderRightWidth,
              boxSizing: paneStyle.boxSizing,
            }
          : null,
        rows: {
          selected: rowMeasurements('[data-slot="track-row"][data-selected="true"]'),
          playing: rowMeasurements(
            '[data-slot="track-row"][data-playing="true"][data-selected="false"]',
          ),
          hovered: rowMeasurements('[data-slot="track-row"]:hover'),
        },
      },
      surfaces: {
        header: computed('[data-slot="track-browser-header"]'),
        sourceLabel: computed('[data-slot="track-browser-header"] h1'),
        canvas: computed('[data-pane="browser"]'),
        browserSection: computed('[data-slot="track-browser"]'),
        table: computed('[role="grid"]'),
        tableHeader: computed('[role="row"][aria-rowindex="1"]'),
      },
      documentWidth: document.documentElement.scrollWidth,
      bodyWidth: document.body.scrollWidth,
      heading:
        document.querySelector('[data-slot="track-browser"] h1')?.textContent?.trim() ?? null,
      headingTag: document.querySelector('[data-slot="track-browser"] h1')?.tagName ?? null,
      browserHeaderTablists: document.querySelectorAll(
        '[data-slot="track-browser-header"] [role="tablist"]',
      ).length,
    };
  }, workbenchPreferenceKey);
}

test.skip(
  captureStage !== 'candidate',
  'manual visual evidence capture; set GRAPHITE_WORKSPACE_CAPTURE=candidate',
);

test('captures selected Graphite workspace evidence views on request', async ({ page }) => {
  test.setTimeout(120_000);
  expect(captureStage).toBe('candidate');

  const measurements: Record<string, unknown> = {};
  await openApp(page, { width: 1366, height: 768 }, 'expanded', true);
  const widePlayer = await startDryRunWithSelection(page);
  const wideInspector = await openDetails(page);
  await expect(wideInspector.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await expect(widePlayer.locator('[role="group"][aria-label^="Playback for"] strong')).toHaveText(
    'Candle Run',
  );
  const settingsTooltipTrigger = page.getByRole('button', { name: 'Open settings' });
  await settingsTooltipTrigger.hover();
  await expect(page.getByRole('tooltip', { name: 'Settings · Ctrl+,' })).toBeVisible();
  if (shouldCapture('tooltip'))
    await capture(page, 'candidate-tooltip-settings-1366x768.png', false);
  await page.mouse.move(0, 0);
  if (shouldCapture('transport-playing'))
    await capture(page, 'candidate-transport-pause-playing-1366x768.png');
  await widePlayer.getByRole('button', { name: 'Pause', exact: true }).click();
  await expect(widePlayer.locator('[data-slot="player-track-status"]')).toHaveText('Paused');
  if (shouldCapture('transport-paused'))
    await capture(page, 'candidate-transport-resume-paused-1366x768.png');
  await widePlayer.getByRole('button', { name: 'Resume', exact: true }).click();
  await expect(widePlayer.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  await page.getByRole('row', { name: /Aurora Landing/ }).hover();
  const wideGeometry = await readWorkspaceGeometry(page);
  expect(wideGeometry.trackCanvas.scrollportMetrics?.hasVerticalOverflow).toBe(true);
  expect(wideGeometry.trackCanvas.scrollbar?.visible).toBe(true);
  expect(wideGeometry.trackCanvas.scrollbar?.width).toBe(10);
  expect(wideGeometry.trackCanvas.scrollbar?.thumbWidth).toBe(6);
  expect(wideGeometry.trackCanvas.scrollbar?.thumbHeight).toBeGreaterThan(0);
  expect(wideGeometry.trackCanvas.scrollbar?.thumbHeight).toBeLessThan(
    wideGeometry.trackCanvas.scrollbar?.height ?? 0,
  );
  expect(wideGeometry.trackCanvas.scrollbar?.opacity).toBe('0');
  expect(wideGeometry.trackCanvas.scrollbar?.pointerEvents).toBe('none');
  for (const row of Object.values(wideGeometry.trackCanvas.rows)) {
    if (!row) continue;
    for (const cell of row.cells) {
      expect(Math.abs(cell.delta?.left ?? 0)).toBeLessThanOrEqual(1);
      expect(Math.abs(cell.delta?.right ?? 0)).toBeLessThanOrEqual(1);
    }
  }
  measurements['1366x768'] = wideGeometry;
  if (shouldCapture('inspector-details'))
    await capture(page, 'candidate-inspector-details-1366x768.png');
  const tableViewport = page.getByRole('grid');
  const tableScrollbar = page
    .locator('[data-slot="scroll-area"]:has([role="grid"])')
    .locator('[data-slot="scroll-area-scrollbar"][data-orientation="vertical"]');
  await tableViewport.focus();
  await expect(tableViewport).toBeFocused();
  await expect
    .poll(() => tableScrollbar.evaluate((element) => getComputedStyle(element).opacity))
    .toBe('0');
  if (shouldCapture('scrollbar-idle'))
    await capture(page, 'candidate-scrollbar-idle-workbench.png');
  if (shouldCapture('populated')) await capture(page, 'candidate-populated-1366x768.png');

  const scrollTopBeforeWheel = await tableViewport.evaluate((element) => element.scrollTop);
  await tableViewport.hover();
  await page.mouse.wheel(0, 320);
  await expect
    .poll(() => tableViewport.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(scrollTopBeforeWheel);
  await expect
    .poll(() => tableScrollbar.evaluate((element) => element.hasAttribute('data-scrolling')))
    .toBe(true);
  await expect
    .poll(() => tableScrollbar.evaluate((element) => getComputedStyle(element).opacity))
    .toBe('1');
  await expect
    .poll(() => tableScrollbar.evaluate((element) => getComputedStyle(element).pointerEvents))
    .toBe('auto');
  if (shouldCapture('scrollbar-scrolling'))
    await capture(page, 'candidate-scrollbar-scrolling-workbench.png');

  const workbenchThumb = tableScrollbar.locator('[data-slot="scroll-area-thumb"]');
  const workbenchThumbBounds = await workbenchThumb.boundingBox();
  expect(workbenchThumbBounds).not.toBeNull();
  if (!workbenchThumbBounds) throw new Error('The Library scrollbar thumb is not measurable.');
  await page.mouse.move(
    workbenchThumbBounds.x + workbenchThumbBounds.width / 2,
    workbenchThumbBounds.y + workbenchThumbBounds.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    workbenchThumbBounds.x + workbenchThumbBounds.width / 2,
    workbenchThumbBounds.y + workbenchThumbBounds.height / 2 + 14,
  );
  await expect.poll(() => tableScrollbar.getAttribute('data-dragging')).toBe('true');
  await expect
    .poll(() => tableScrollbar.evaluate((element) => element.hasAttribute('data-scrolling')))
    .toBe(true);
  await expect
    .poll(() => tableScrollbar.evaluate((element) => element.hasAttribute('data-scrolling')))
    .toBe(false);
  await expect
    .poll(() => tableScrollbar.evaluate((element) => getComputedStyle(element).opacity))
    .toBe('1');
  await expect
    .poll(() => tableScrollbar.evaluate((element) => getComputedStyle(element).pointerEvents))
    .toBe('auto');
  if (shouldCapture('scrollbar-dragging'))
    await capture(page, 'candidate-scrollbar-dragging-workbench.png', false);
  await page.mouse.up();
  await expect.poll(() => tableScrollbar.getAttribute('data-dragging')).toBeNull();
  await expect
    .poll(() => tableScrollbar.evaluate((element) => getComputedStyle(element).opacity))
    .toBe('0');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('row', { name: /Candle Run/ }).click();
  const idlePlayer = page.getByRole('contentinfo', { name: 'Player controls' });
  await expect(idlePlayer.locator('[data-slot="player-track-status"]')).toHaveText('Selected');
  if (shouldCapture('transport-idle'))
    await capture(page, 'candidate-transport-play-idle-1366x768.png');

  await openApp(page, { width: 640, height: 448 });
  const longTitle = 'A sheet with an intentionally long title for layout verification';
  await page.getByRole('searchbox', { name: 'Search Built-in Songs' }).fill(longTitle);
  const longTitleRow = page.getByRole('row', { name: /A sheet with an intentionally long title/ });
  await expect(longTitleRow).toBeVisible();
  await longTitleRow.click();
  const longTitlePlayer = page.getByRole('contentinfo', { name: 'Player controls' });
  await longTitlePlayer.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Playback confirmation' })).toBeVisible();
  if (shouldCapture('transport-long-title'))
    await capture(page, 'candidate-transport-long-title-640x448.png');

  await openApp(page, { width: 1200, height: 760 }, 'expanded', true);
  const mediumPlayer = await startDryRunWithSelection(page);
  const mediumInspector = await openDetails(page);
  await expect(mediumInspector.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await expect(mediumPlayer.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  measurements['1200x760'] = await readWorkspaceGeometry(page);
  if (shouldCapture('medium')) await capture(page, 'candidate-populated-1200x760.png');

  await openApp(page, { width: 1920, height: 1080 }, 'expanded', true);
  const ultrawidePlayer = await startDryRunWithSelection(page);
  const ultrawideInspector = await openDetails(page);
  await expect(ultrawideInspector.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await expect(ultrawidePlayer.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  measurements['1920x1080'] = await readWorkspaceGeometry(page);
  if (shouldCapture('ultrawide')) await capture(page, 'candidate-populated-1920x1080.png');

  await openApp(page, { width: 800, height: 560 }, 'collapsed');
  await startDryRunWithSelection(page);
  await page.getByRole('row', { name: /Aurora Landing/ }).hover();
  measurements['800x560'] = await readWorkspaceGeometry(page);
  if (shouldCapture('compact')) await capture(page, 'candidate-workbench-800x560.png');

  await openApp(page, { width: 1024, height: 700 }, 'expanded', true);
  const standardPlayer = await startDryRunWithSelection(page);
  const standardInspector = await openDetails(page);
  await expect(standardInspector.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await expect(standardPlayer.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  measurements['1024x700'] = await readWorkspaceGeometry(page);
  if (shouldCapture('standard')) await capture(page, 'candidate-workbench-1024x700.png');

  await openApp(page, { width: 640, height: 448 }, 'expanded');
  const compactPlayer = await startDryRunWithSelection(page);
  const compactInspector = await openDetails(page);
  await expect(page.locator('[data-slot="workbench"]')).toHaveAttribute(
    'data-utility-presentation',
    'overlay',
  );
  await expect(page.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  await expect(compactPlayer.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  await expect(compactInspector).toBeVisible();
  await page.getByRole('row', { name: /Aurora Landing/ }).hover({ position: { x: 24, y: 23 } });
  measurements['640x448'] = await readWorkspaceGeometry(page);
  if (shouldCapture('overlay')) await capture(page, 'candidate-overlay-640x448.png');

  await openApp(page, { width: 640, height: 448 });
  await createLongPlaylistAndOpenMenu(page);
  const playlistTitle = page.getByRole('heading', { name: playlistName });
  await expect(playlistTitle).toHaveAttribute('title', playlistName);
  measurements['playlist-640x448'] = await readWorkspaceGeometry(page);
  if (shouldCapture('playlist')) await capture(page, 'candidate-playlist-add-menu-640x448.png');

  const addSongsTrigger = page.getByRole('button', { name: 'Add songs' });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu', { name: 'Add songs' })).toBeHidden();
  await addSongsTrigger.focus();
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('menuitem', { name: 'Browse Built-in Songs…' })).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('menuitem', { name: 'Import files…' })).toBeFocused();
  if (shouldCapture('menu-keyboard'))
    await capture(page, 'candidate-playlist-menu-keyboard-640x448.png');

  await openApp(page, { width: 1366, height: 768 }, 'expanded', true);
  const runtimePlayer = await startDryRunWithSelection(page);
  const runtime = await openRuntimeDiagnostics(page);
  await expect(
    runtime.getByText('Mode: Test mode (dry run) · no physical input sent', { exact: true }),
  ).toBeVisible();
  await expect(
    runtime.getByText('Test mode (dry run) · sender timing not measured', { exact: true }),
  ).toBeVisible();
  await expect(
    runtime.getByText(
      'This session sends no physical keyboard input. Start normal playback to collect physical sender measurements.',
      { exact: true },
    ),
  ).toBeVisible();
  await expect(runtime.getByText('Sender samples', { exact: true })).toHaveCount(0);
  await expect(
    runtimePlayer.locator('[role="group"][aria-label^="Playback for"] strong'),
  ).toHaveText('Candle Run');
  measurements['runtime-1366x768'] = await page.evaluate(() => {
    const inspector = document.querySelector<HTMLElement>(
      '[role="region"][aria-label^="Utility:"]',
    );
    const rect = inspector?.getBoundingClientRect();
    return {
      viewport: { width: innerWidth, height: innerHeight },
      inspector: rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null,
      dryRun: true,
      dryRunNotice:
        document.querySelector('[data-slot="diagnostics-empty-state"]')?.textContent?.trim() ??
        null,
      playbackSummary:
        document.querySelector('[data-slot="diagnostics-playback-summary"]')?.textContent?.trim() ??
        null,
      senderSummary:
        document.querySelector('[data-slot="diagnostics-sender-summary"]')?.textContent?.trim() ??
        null,
      senderSamples:
        [...document.querySelectorAll('[data-slot="diagnostics-metric"]')]
          .find((metric) => metric.textContent?.includes('Sender samples'))
          ?.textContent?.trim() ?? null,
    };
  });
  if (shouldCapture('runtime')) await capture(page, 'candidate-runtime-diagnostics-1366x768.png');
  const runtimeTabs = runtime.getByRole('tablist', { name: 'Diagnostics views' });
  await expect(runtimeTabs.getByRole('tab')).toHaveCount(3);
  await runtimeTabs.getByRole('tab', { name: 'Timing' }).click();
  await expect(runtimeTabs.getByRole('tab', { name: 'Timing' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  if (shouldCapture('runtime-timing')) await capture(page, 'candidate-runtime-timing-1366x768.png');
  await runtimeTabs.getByRole('tab', { name: 'Events' }).click();
  await expect(runtimeTabs.getByRole('tab', { name: 'Events' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  const eventRegion = page.getByRole('region', { name: 'Diagnostics events' });
  await expect(eventRegion.locator('li').first()).toBeVisible();
  const firstEventLayout = await eventRegion
    .locator('li')
    .first()
    .evaluate((row) => {
      const bounds = (element: Element | null) => {
        const rect = element?.getBoundingClientRect();
        return rect
          ? { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }
          : null;
      };
      const time = row.querySelector('time');
      const name = row.querySelector('strong');
      const detail = row.querySelector('p');
      return {
        row: bounds(row),
        time: bounds(time),
        name: bounds(name),
        detail: bounds(detail),
        timeWhitespace: time ? getComputedStyle(time).whiteSpace : null,
        nameOverflow: name ? name.scrollWidth - name.clientWidth : null,
        detailOverflow: detail ? detail.scrollWidth - detail.clientWidth : null,
      };
    });
  expect(firstEventLayout.row).not.toBeNull();
  expect(firstEventLayout.timeWhitespace).toBe('nowrap');
  expect(firstEventLayout.nameOverflow).toBeLessThanOrEqual(1);
  expect(firstEventLayout.detailOverflow).toBeLessThanOrEqual(1);
  if (firstEventLayout.row && firstEventLayout.name && firstEventLayout.detail) {
    expect(firstEventLayout.detail.left).toBeLessThanOrEqual(firstEventLayout.name.left + 1);
    expect(firstEventLayout.detail.top).toBeGreaterThanOrEqual(firstEventLayout.name.top);
    expect(firstEventLayout.detail.right).toBeLessThanOrEqual(firstEventLayout.row.right + 1);
  }
  if (shouldCapture('runtime-events')) await capture(page, 'candidate-runtime-events-1366x768.png');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings).toBeVisible();
  if (shouldCapture('settings')) await capture(page, 'candidate-settings-1366x768.png');

  await openApp(page, { width: 640, height: 448 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  const compactSettings = page.getByRole('dialog', { name: 'Settings' });
  const categoryNavigation = compactSettings.getByRole('navigation', {
    name: 'Settings categories',
  });
  const categoryNames = ['Playback', 'Appearance', 'Diagnostics', 'Updates', 'Donate', 'About'];
  for (const name of categoryNames) {
    await expect(categoryNavigation.getByRole('button', { name, exact: true })).toBeInViewport();
  }
  if (shouldCapture('settings-nav')) await capture(page, 'candidate-settings-nav-640x448.png');

  await openApp(page, { width: 640, height: 320 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  const tinySettings = page.getByRole('dialog', { name: 'Settings' });
  const tinyNavigation = tinySettings.getByRole('navigation', {
    name: 'Settings categories',
  });
  const navigationOverflow = await tinyNavigation.evaluate(
    (element) => element.scrollHeight > element.clientHeight,
  );
  expect(navigationOverflow).toBe(true);
  await tinyNavigation.hover();
  await page.mouse.wheel(0, 420);
  await expect(tinyNavigation.getByRole('button', { name: 'About', exact: true })).toBeInViewport();
  if (shouldCapture('settings-nav-tiny'))
    await capture(page, 'candidate-settings-nav-640x320-scrolled.png');

  await openApp(page, { width: 640, height: 320 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  const tinyAppearance = page.getByRole('dialog', { name: 'Settings' });
  await tinyAppearance.getByRole('button', { name: 'Appearance', exact: true }).click();
  await tinyAppearance.getByRole('combobox', { name: 'Dark palette' }).click();
  await expect(page.locator('[data-slot="select-content"]:visible')).toBeVisible();
  if (shouldCapture('settings-select'))
    await capture(page, 'candidate-settings-select-open-640x320.png');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('button', { name: 'Configure playback profile' }).click();
  const profile = page.getByRole('dialog', { name: 'Playback profile' });
  await expect(profile).toBeVisible();
  if (shouldCapture('profile')) await capture(page, 'candidate-profile-1366x768.png');

  await openApp(page, { width: 800, height: 560 });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await page.getByRole('button', { name: 'Configure playback profile' }).click();
  const compactProfile = page.getByRole('dialog', { name: 'Playback profile' });
  await compactProfile.getByText('Advanced timing', { exact: true }).click();
  await expect(compactProfile.getByLabel('FPS')).toBeVisible();
  if (shouldCapture('profile-expanded'))
    await capture(page, 'candidate-profile-expanded-800x560.png');
  await compactProfile.getByRole('combobox', { name: 'Base hold' }).click();
  await expect(page.locator('[data-slot="select-content"]:visible')).toBeVisible();
  if (shouldCapture('profile-select'))
    await capture(page, 'candidate-profile-select-open-800x560.png');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  if (shouldCapture('profile-select-light'))
    await capture(page, 'candidate-profile-select-open-light-800x560.png');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true })
    .click();
  const admission = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(admission).toBeVisible();
  if (shouldCapture('admission')) await capture(page, 'candidate-admission-1366x768.png');

  await openApp(page, { width: 640, height: 448 });
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const compactAdmissionPlayer = page.getByRole('contentinfo', { name: 'Player controls' });
  await compactAdmissionPlayer.getByRole('button', { name: 'Play', exact: true }).click();
  const compactAdmission = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(compactAdmission).toBeVisible();
  const compactTrackInfo = compactAdmissionPlayer.getByRole('group', {
    name: 'Playback for Aurora Landing',
  });
  await expect(compactTrackInfo.locator('strong')).toHaveText('Aurora Landing');
  await expect(compactTrackInfo.locator('[data-slot="status-badge"]')).toHaveText('Medium risk');
  const compactTitleFits = await compactTrackInfo.locator('strong').evaluate((title) => {
    const element = title as HTMLElement;
    return element.scrollWidth <= element.clientWidth;
  });
  expect(compactTitleFits).toBe(true);
  if (shouldCapture('compact-admission')) await capture(page, 'candidate-admission-640x448.png');

  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockReleaseNotes=short');
  const updateTrigger = page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ });
  await expect(updateTrigger).toBeVisible();
  await updateTrigger.click();
  const update = page.getByRole('dialog');
  await expect(update).toContainText('Version 4.0.0-alpha.2-mock is available');
  if (shouldCapture('update')) await capture(page, 'candidate-update-1366x768.png');

  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockLibraryState=empty');
  const emptyState = page.locator('[data-slot="library-empty-state"]');
  await expect(emptyState).toContainText('No Built-in Songs are available');
  if (shouldCapture('empty')) await capture(page, 'candidate-empty-library-1366x768.png');

  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockStartFailure=target_not_found');
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true })
    .click();
  const recovery = page.getByRole('alert');
  await expect(recovery).toContainText('Sky window was not found');
  if (shouldCapture('error')) await capture(page, 'candidate-recovery-error-1366x768.png');

  await openApp(page, { width: 1366, height: 768 });
  const appearanceSettings = async () => {
    await page.getByRole('button', { name: 'Open settings' }).click();
    const dialog = page.getByRole('dialog', { name: 'Settings' });
    await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
    return dialog;
  };
  const lightSettings = await appearanceSettings();
  await chooseSelectOption(page, lightSettings.getByRole('combobox', { name: 'Theme' }), 'light');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  if (shouldCapture('appearance-preview-light'))
    await capture(page, 'candidate-appearance-preview-light-1366x768.png');
  await lightSettings.getByRole('button', { name: 'Close settings' }).click();

  for (const palette of palettes) {
    const paletteSettings = await appearanceSettings();
    await chooseSelectOption(
      page,
      paletteSettings.getByRole('combobox', { name: 'Theme' }),
      'dark',
    );
    await chooseSelectOption(
      page,
      paletteSettings.getByRole('combobox', { name: 'Dark palette' }),
      palette,
    );
    await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
    if (shouldCapture(`palette-${palette}`))
      await capture(page, `candidate-palette-${palette}-1366x768.png`);
    await paletteSettings.getByRole('button', { name: 'Close settings' }).click();
  }
  if (shouldCapture('palette-contact-sheet')) await capturePaletteContactSheet(page);

  await openApp(page, { width: 1366, height: 768 }, 'expanded', true);
  await startDryRunWithSelection(page);
  await openRuntimeDiagnostics(page);
  const workbenchColors: Record<string, unknown> = {};
  const captureUndimmedWorkbench = async (
    label: string,
    mode: 'light' | 'dark',
    palette: string,
    file: string,
    viewKey: string,
  ) => {
    const dialog = await appearanceSettings();
    await chooseSelectOption(page, dialog.getByRole('combobox', { name: 'Theme' }), mode);
    if (mode === 'dark') {
      await chooseSelectOption(
        page,
        dialog.getByRole('combobox', { name: 'Dark palette' }),
        palette,
      );
    }
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', mode);
    await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
    await dialog.getByRole('button', { name: 'Close settings' }).click();
    const idleRow = page.locator('[data-slot="track-row"][data-state="idle"]').first();
    await idleRow.hover();
    workbenchColors[label] = await page.evaluate(() => {
      const idle = document.querySelector<HTMLElement>(
        '[data-slot="track-row"][data-state="idle"]',
      );
      const selected = document.querySelector<HTMLElement>(
        '[data-slot="track-row"][data-selected="true"]',
      );
      const playing = document.querySelector<HTMLElement>(
        '[data-slot="track-row"][data-playing="true"]',
      );
      const indicator = document.querySelector<HTMLElement>(
        '[data-variant="segmented"] [data-slot="tabs-indicator"]',
      );
      const activeTab = document.querySelector<HTMLElement>(
        '[data-variant="segmented"] [data-slot="tabs-trigger"][aria-selected="true"]',
      );
      const createBtn = document.querySelector<HTMLElement>(
        '[data-slot="library-empty-playlists-create"]',
      );
      return {
        idleHoverBg: idle ? getComputedStyle(idle).backgroundColor : null,
        selectedBg: selected ? getComputedStyle(selected).backgroundColor : null,
        playingBg: playing ? getComputedStyle(playing).backgroundColor : null,
        segmentedIndicatorBg: indicator ? getComputedStyle(indicator).backgroundColor : null,
        segmentedIndicatorBorder: indicator ? getComputedStyle(indicator).borderColor : null,
        segmentedActiveText: activeTab ? getComputedStyle(activeTab).color : null,
        createButtonBg: createBtn ? getComputedStyle(createBtn).backgroundColor : null,
        createButtonBorder: createBtn ? getComputedStyle(createBtn).borderColor : null,
      };
    });
    if (shouldCapture(viewKey)) await capture(page, file, false);
  };

  await captureUndimmedWorkbench(
    'light',
    'light',
    'graphite',
    'candidate-light-1366x768.png',
    'light',
  );
  for (const palette of palettes) {
    await captureUndimmedWorkbench(
      palette,
      'dark',
      palette,
      `candidate-workbench-${palette}-1366x768.png`,
      `workbench-${palette}`,
    );
  }
  measurements.workbenchColors = workbenchColors;
  if (shouldCapture('workbench-palette-contact-sheet'))
    await captureWorkbenchPaletteContactSheet(page);

  // Paired Wave B captures: Light admission, Dark/Light playback recovery, Dark/Light Update dialog
  if (shouldCapture('admission-light')) {
    await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'no-preference' });
    await openApp(page, { width: 640, height: 448 }, 'expanded', true);
    await page.getByRole('row', { name: /Aurora Landing/ }).click();
    await page
      .getByRole('contentinfo', { name: 'Player controls' })
      .getByRole('button', { name: 'Play', exact: true })
      .click();
    const lightAdmission = page.locator('[data-slot="playback-admission"]');
    await expect(lightAdmission).toBeVisible();
    await expect
      .poll(() => lightAdmission.evaluate((el) => getComputedStyle(el).opacity))
      .toBe('1');
    await capture(page, 'candidate-admission-light-640x448.png');
  }

  for (const mode of ['dark', 'light'] as const) {
    if (shouldCapture(`recovery-${mode}`)) {
      await page.emulateMedia({ colorScheme: mode, reducedMotion: 'no-preference' });
      await page.setViewportSize({ width: 800, height: 560 });
      await page.goto('/?mockStartFailure=target_not_found');
      await expect(page.locator('html')).toHaveAttribute('data-color-mode', mode);
      await page.getByRole('row', { name: /Blue Bird/ }).click();
      const recPlayer = page.getByRole('contentinfo', { name: 'Player controls' });
      await recPlayer.getByRole('button', { name: 'Play', exact: true }).click();
      const proceed = recPlayer.getByRole('button', { name: 'Proceed with current settings' });
      if (await proceed.isVisible().catch(() => false)) await proceed.click();
      const recovery = page.getByRole('alert');
      await expect(recovery).toContainText('Sky window was not found');
      await expect.poll(() => recovery.evaluate((el) => getComputedStyle(el).opacity)).toBe('1');
      await capture(page, `candidate-recovery-${mode}-800x560.png`);
    }
    if (shouldCapture(`update-${mode}`)) {
      await page.emulateMedia({ colorScheme: mode, reducedMotion: 'no-preference' });
      await page.setViewportSize({ width: 800, height: 560 });
      await page.goto('/?mockReleaseNotes=short');
      await expect(page.locator('html')).toHaveAttribute('data-color-mode', mode);
      await page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ }).click();
      const updateDialog = page.getByRole('dialog');
      await expect(updateDialog).toContainText('Version 4.0.0-alpha.2-mock is available');
      await capture(page, `candidate-update-${mode}-800x560.png`);
    }
  }
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'no-preference' });

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openApp(page, { width: 640, height: 448 }, 'expanded', true);
  const reducedMotionInspector = await openDetails(page);
  await expect(reducedMotionInspector).toBeVisible();
  const reducedMotionDuration = await page
    .locator('[data-pane="inspector"]')
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        transitionProperty: style.transitionProperty,
        transitionDuration: style.transitionDuration,
      };
    });
  expect(
    reducedMotionDuration.transitionProperty === 'none' ||
      reducedMotionDuration.transitionDuration
        .split(',')
        .every((duration) => Number.parseFloat(duration) === 0),
  ).toBe(true);
  measurements['640x448ReducedMotion'] = await readWorkspaceGeometry(page);
  if (shouldCapture('reduced-motion'))
    await capture(page, 'candidate-reduced-motion-overlay-640x448.png');

  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await openApp(page, { width: 1366, height: 768 });
  const forcedColorSettings = await appearanceSettings();
  const appearanceTab = forcedColorSettings.getByRole('button', {
    name: 'Appearance',
    exact: true,
  });
  await appearanceTab.focus();
  const forcedColorFocus = await appearanceTab.evaluate((element) => {
    const style = getComputedStyle(element);
    return { outlineStyle: style.outlineStyle, outlineWidth: style.outlineWidth };
  });
  expect(forcedColorFocus.outlineStyle).not.toBe('none');
  expect(Number.parseFloat(forcedColorFocus.outlineWidth)).toBeGreaterThanOrEqual(2);
  if (shouldCapture('forced-colors'))
    await capture(page, 'candidate-forced-colors-settings-1366x768.png');

  mkdirSync(evidenceDirectory, { recursive: true });
  await import('node:fs/promises').then(({ writeFile }) =>
    writeFile(
      resolve(evidenceDirectory, 'candidate-measurements.json'),
      `${JSON.stringify(measurements, null, 2)}\n`,
    ),
  );
});
