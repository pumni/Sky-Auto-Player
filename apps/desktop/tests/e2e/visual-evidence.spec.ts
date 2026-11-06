import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

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
  await expect(page.locator('.app-shell')).toBeVisible();
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
  await expect(player.locator('.player-track-status')).toHaveText('Dry run · Playing');
  return player;
}

async function openDetails(page: Page) {
  const utilityToggle = page.locator('.player-utility-button');
  await expect(utilityToggle).toBeVisible();
  if ((await utilityToggle.getAttribute('aria-expanded')) !== 'true') await utilityToggle.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
  await inspector.getByRole('tab', { name: 'Details' }).click();
  return inspector;
}

async function openRuntimeDiagnostics(page: Page) {
  const utilityToggle = page.locator('.player-utility-button');
  await expect(utilityToggle).toBeVisible();
  if ((await utilityToggle.getAttribute('aria-expanded')) !== 'true') await utilityToggle.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
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

async function capture(page: Page, name: string) {
  mkdirSync(evidenceDirectory, { recursive: true });
  await page.screenshot({
    path: resolve(evidenceDirectory, name),
    animations: 'disabled',
  });
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
    const trackTable = document.querySelector<HTMLElement>('.track-table');
    const trackFrame = document.querySelector<HTMLElement>('.track-table-frame');
    const browserPane = document.querySelector<HTMLElement>('.track-browser-workbench-pane');
    const tableHeader = document.querySelector<HTMLElement>('.track-table-header');
    const headerCells = Array.from(
      document.querySelectorAll<HTMLElement>('.track-table-header [role="columnheader"]'),
    ).filter((cell) => getComputedStyle(cell).display !== 'none');
    const rowMeasurements = (selector: string) => {
      const row = document.querySelector<HTMLElement>(selector);
      const rowRect = row?.getBoundingClientRect();
      const paneRect = browserPane?.getBoundingClientRect();
      const paneStyle = browserPane ? getComputedStyle(browserPane) : null;
      const classicScrollbarReservation =
        trackTable && tableStyle
          ? trackTable.offsetWidth -
            trackTable.clientWidth -
            trackTable.clientLeft -
            Number.parseFloat(tableStyle.borderRightWidth)
          : 0;
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
        scrollbarCompensatedPaneInsets:
          rowRect && contentLeft !== null && contentRight !== null
            ? {
                left: rowRect.left - contentLeft,
                right: contentRight - rowRect.right - classicScrollbarReservation,
              }
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
    const titlebar = document.querySelector<HTMLElement>('.app-titlebar');
    const titlebarLeading = document.querySelector<HTMLElement>('.app-titlebar-leading');
    const titlebarBrand = document.querySelector<HTMLElement>('.app-titlebar-brand');
    const titlebarSearch = document.querySelector<HTMLElement>('.global-search');
    const titlebarEnd = document.querySelector<HTMLElement>('.app-titlebar-end');
    const titlebarLeadingRect = titlebarLeading?.getBoundingClientRect();
    const titlebarBrandRect = titlebarBrand?.getBoundingClientRect();
    const titlebarSearchRect = titlebarSearch?.getBoundingClientRect();
    const titlebarEndRect = titlebarEnd?.getBoundingClientRect();
    const collapseButton = document.querySelector<HTMLElement>('.library-collapse-button');
    const collapseStyle = collapseButton ? getComputedStyle(collapseButton) : null;
    const inspectorPane = document.querySelector<HTMLElement>('.utility-workbench-pane');
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
        workbenchContent: rootStyle.getPropertyValue('--workbench-content').trim(),
        chromeSurface: rootStyle.getPropertyValue('--chrome-surface').trim(),
        paneEdge: rootStyle.getPropertyValue('--workbench-pane-edge').trim(),
        controlHover: rootStyle.getPropertyValue('--control-hover').trim(),
        controlBorder: rootStyle.getPropertyValue('--control-border').trim(),
        textPrimary: rootStyle.getPropertyValue('--text-primary').trim(),
        textSecondary: rootStyle.getPropertyValue('--text-secondary').trim(),
        focusRing: rootStyle.getPropertyValue('--focus-ring').trim(),
      },
      titlebar: {
        rect: rect('.app-titlebar'),
        leading: rect('.app-titlebar-leading'),
        brand: rect('.app-titlebar-brand'),
        search: rect('.global-search'),
        trailingActions: rect('.app-titlebar-end'),
        leftDrag: rect('.titlebar-drag-space'),
        rightDrag: rect('.titlebar-drag-space-trailing'),
        freeRegionMidpoint,
        searchMidpoint: titlebarSearchRect
          ? titlebarSearchRect.left + titlebarSearchRect.width / 2
          : null,
        searchCenterError:
          freeRegionMidpoint !== null && titlebarSearchRect
            ? Math.abs(titlebarSearchRect.left + titlebarSearchRect.width / 2 - freeRegionMidpoint)
            : null,
        updateAvailable: titlebar?.dataset.updateAvailable ?? null,
        searchFocused: document.activeElement === document.querySelector('.global-search input'),
      },
      collapseButton: {
        rect: rect('.library-collapse-button'),
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
          document.querySelector<HTMLElement>('.workbench')?.dataset.utilityPresentation ?? null,
        rect: rect('.utility-workbench-pane'),
        widthStyle: inspectorPane?.style.width || null,
        persistedDockWidth: utilityPreference?.utilityWidth ?? null,
        borderColor: inspectorStyle?.borderColor ?? null,
        borderWidth: inspectorStyle?.borderWidth ?? null,
        borderRadius: inspectorStyle?.borderRadius ?? null,
      },
      browserContainer: rect('.track-browser-workbench-pane'),
      browserSection: rect('.track-browser'),
      sourceHeader: rect('.track-browser-header'),
      sourceLabel: rect('.track-browser-source-label'),
      sourceTitle: rect('.track-browser-header h1'),
      count: rect('.track-browser-count'),
      tableHeader: rect('.track-table-header'),
      firstSong: rect('.track-row'),
      trackCanvas: {
        pane: rect('.track-browser-workbench-pane'),
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
                actualClassicScrollbarReservation:
                  trackTable.offsetWidth -
                  trackTable.clientWidth -
                  trackTable.clientLeft -
                  Number.parseFloat(tableStyle.borderRightWidth),
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
          selected: rowMeasurements('.track-row.is-selected'),
          playing: rowMeasurements('.track-row.is-playing:not(.is-selected)'),
          hovered: rowMeasurements('.track-row:hover'),
        },
      },
      surfaces: {
        header: computed('.track-browser-header'),
        sourceLabel: computed('.track-browser-source-label'),
        canvas: computed('.track-browser-workbench-pane'),
        browserSection: computed('.track-browser'),
        table: computed('.track-table'),
        tableHeader: computed('.track-table-header'),
      },
      documentWidth: document.documentElement.scrollWidth,
      bodyWidth: document.body.scrollWidth,
      heading: document.querySelector('.track-browser h1')?.textContent?.trim() ?? null,
      headingTag: document.querySelector('.track-browser h1')?.tagName ?? null,
      browserHeaderTablists: document.querySelectorAll('.track-browser-header [role="tablist"]')
        .length,
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
  await expect(widePlayer.locator('.player-track-copy strong')).toHaveText('Candle Run');
  await page.getByRole('row', { name: /Aurora Landing/ }).hover();
  const classicScrollbarSimulation = await page.addStyleTag({
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
  const classicScrollbarGeometry = await readWorkspaceGeometry(page);
  expect(
    classicScrollbarGeometry.trackCanvas.scrollportMetrics?.actualClassicScrollbarReservation,
  ).toBeGreaterThan(0);
  measurements['1366x768ClassicScrollbar'] = classicScrollbarGeometry;
  await classicScrollbarSimulation.evaluate((style) => style.remove());
  measurements['1366x768'] = await readWorkspaceGeometry(page);
  if (shouldCapture('populated')) await capture(page, 'candidate-populated-1366x768.png');

  await openApp(page, { width: 800, height: 560 }, 'collapsed');
  await startDryRunWithSelection(page);
  await page.getByRole('row', { name: /Aurora Landing/ }).hover();
  measurements['800x560'] = await readWorkspaceGeometry(page);
  if (shouldCapture('compact')) await capture(page, 'candidate-workbench-800x560.png');

  await openApp(page, { width: 640, height: 448 }, 'expanded');
  const compactPlayer = await startDryRunWithSelection(page);
  const compactInspector = await openDetails(page);
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(page.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  await expect(compactPlayer.locator('.player-track-status')).toHaveText('Dry run · Playing');
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

  await openApp(page, { width: 1366, height: 768 }, 'expanded', true);
  const runtimePlayer = await startDryRunWithSelection(page);
  const runtime = await openRuntimeDiagnostics(page);
  await expect(
    runtime.getByText('Mode: Dry run · no physical input sent', { exact: true }),
  ).toBeVisible();
  await expect(runtime.getByText('Sender samples', { exact: true })).toBeVisible();
  await expect(runtimePlayer.locator('.player-track-copy strong')).toHaveText('Candle Run');
  measurements['runtime-1366x768'] = await page.evaluate(() => {
    const inspector = document.querySelector<HTMLElement>('.utility-surface');
    const rect = inspector?.getBoundingClientRect();
    return {
      viewport: { width: innerWidth, height: innerHeight },
      inspector: rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null,
      playbackSummary:
        document.querySelector('.diagnostics-playback-summary')?.textContent?.trim() ?? null,
      senderSummary:
        document.querySelector('.diagnostics-sender-summary')?.textContent?.trim() ?? null,
      senderSamples:
        [...document.querySelectorAll('.diagnostics-metric')]
          .find((metric) => metric.textContent?.includes('Sender samples'))
          ?.textContent?.trim() ?? null,
    };
  });
  if (shouldCapture('runtime')) await capture(page, 'candidate-runtime-diagnostics-1366x768.png');

  mkdirSync(evidenceDirectory, { recursive: true });
  await import('node:fs/promises').then(({ writeFile }) =>
    writeFile(
      resolve(evidenceDirectory, 'candidate-measurements.json'),
      `${JSON.stringify(measurements, null, 2)}\n`,
    ),
  );
});
