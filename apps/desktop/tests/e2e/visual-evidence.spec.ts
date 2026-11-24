import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { chooseSelectOption } from './testHelpers';

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
  await inspector.getByRole('tab', { name: 'Details' }).click();
  return inspector;
}

async function openRuntimeDiagnostics(page: Page) {
  const utilityToggle = page.locator('button[aria-label$="utility panel"]');
  await expect(utilityToggle).toBeVisible();
  if ((await utilityToggle.getAttribute('aria-expanded')) !== 'true') await utilityToggle.click();
  const inspector = page.locator('[role="region"][aria-label^="Utility:"]');
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
    const trackTable = document.querySelector<HTMLElement>('[role="grid"]');
    const trackFrame = document.querySelector<HTMLElement>('[data-slot="track-table-frame"]');
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
    const titlebar = document.querySelector<HTMLElement>('[data-slot="titlebar"]');
    const titlebarLeading = document.querySelector<HTMLElement>('[data-slot="titlebar-leading"]');
    const titlebarBrand = document.querySelector<HTMLElement>('[data-slot="titlebar-brand"]');
    const titlebarSearch = document.querySelector<HTMLElement>('[data-slot="global-search"]');
    const titlebarEnd = document.querySelector<HTMLElement>('[data-slot="titlebar-trailing"]');
    const titlebarLeadingRect = titlebarLeading?.getBoundingClientRect();
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
  await page.getByRole('row', { name: /Aurora Landing/ }).hover();
  const classicScrollbarSimulation = await page.addStyleTag({
    content: `
      [role="grid"] {
        overflow-y: scroll !important;
        scrollbar-gutter: stable !important;
        scrollbar-width: auto !important;
      }
      [role="grid"]::-webkit-scrollbar {
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

  await openApp(page, { width: 1366, height: 768 }, 'expanded', true);
  const runtimePlayer = await startDryRunWithSelection(page);
  const runtime = await openRuntimeDiagnostics(page);
  await expect(
    runtime.getByText('Mode: Dry run · no physical input sent', { exact: true }),
  ).toBeVisible();
  await expect(
    runtime.getByText('Dry run · sender timing not measured', { exact: true }),
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

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings).toBeVisible();
  if (shouldCapture('settings')) await capture(page, 'candidate-settings-1366x768.png');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('button', { name: 'Configure playback profile' }).click();
  const profile = page.getByRole('dialog', { name: 'Playback profile' });
  await expect(profile).toBeVisible();
  if (shouldCapture('profile')) await capture(page, 'candidate-profile-1366x768.png');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true })
    .click();
  const admission = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(admission).toBeVisible();
  if (shouldCapture('admission')) await capture(page, 'candidate-admission-1366x768.png');

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
  await lightSettings.getByRole('button', { name: 'Close settings' }).click();
  if (shouldCapture('light')) await capture(page, 'candidate-light-1366x768.png');

  const auroraSettings = await appearanceSettings();
  await chooseSelectOption(page, auroraSettings.getByRole('combobox', { name: 'Theme' }), 'dark');
  await chooseSelectOption(
    page,
    auroraSettings.getByRole('combobox', { name: 'Dark palette' }),
    'aurora',
  );
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'aurora');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  await auroraSettings.getByRole('button', { name: 'Close settings' }).click();
  if (shouldCapture('aurora')) await capture(page, 'candidate-aurora-1366x768.png');

  mkdirSync(evidenceDirectory, { recursive: true });
  await import('node:fs/promises').then(({ writeFile }) =>
    writeFile(
      resolve(evidenceDirectory, 'candidate-measurements.json'),
      `${JSON.stringify(measurements, null, 2)}\n`,
    ),
  );
});
