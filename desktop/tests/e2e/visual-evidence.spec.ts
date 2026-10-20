import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const captureStage = process.env.GRAPHITE_WORKSPACE_CAPTURE;
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
  await expect(player.locator('.player-dry-run-label')).toHaveText('Dry run');
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
  await expect(menu.getByRole('menuitem', { name: 'Browse All Songs…' })).toBeVisible();
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

async function readWorkspaceGeometry(page: Page) {
  return page.evaluate(() => {
    const rect = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
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
      viewport: { width: innerWidth, height: innerHeight },
      browserContainer: rect('.track-browser-workbench-pane'),
      browserSection: rect('.track-browser'),
      sourceHeader: rect('.track-browser-header'),
      sourceLabel: rect('.track-browser-source-label'),
      sourceTitle: rect('.track-browser-header h1'),
      count: rect('.track-browser-count'),
      tableHeader: rect('.track-table-header'),
      firstSong: rect('.track-row'),
      surfaces: {
        header: computed('.track-browser-header'),
        sourceLabel: computed('.track-browser-source-label'),
        canvas: computed('.track-browser-workbench-pane'),
        tableHeader: computed('.track-table-header'),
      },
      documentWidth: document.documentElement.scrollWidth,
      bodyWidth: document.body.scrollWidth,
      heading: document.querySelector('.track-browser h1')?.textContent?.trim() ?? null,
      headingTag: document.querySelector('.track-browser h1')?.tagName ?? null,
      browserHeaderTablists: document.querySelectorAll('.track-browser-header [role="tablist"]')
        .length,
    };
  });
}

test.skip(
  captureStage !== 'candidate',
  'manual visual evidence capture; set GRAPHITE_WORKSPACE_CAPTURE=candidate',
);

test('captures the Graphite G workspace views on request', async ({ page }) => {
  test.setTimeout(120_000);
  expect(captureStage).toBe('candidate');

  const measurements: Record<string, unknown> = {};
  await openApp(page, { width: 1366, height: 768 }, 'expanded', true);
  const widePlayer = await startDryRunWithSelection(page);
  const wideInspector = await openDetails(page);
  await expect(wideInspector.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await expect(widePlayer.locator('.player-track-copy strong')).toHaveText('Candle Run');
  measurements['1366x768'] = await readWorkspaceGeometry(page);
  await capture(page, 'candidate-populated-1366x768.png');

  await openApp(page, { width: 800, height: 560 }, 'collapsed');
  await startDryRunWithSelection(page);
  measurements['800x560'] = await readWorkspaceGeometry(page);
  await capture(page, 'candidate-workbench-800x560.png');

  await openApp(page, { width: 640, height: 448 }, 'expanded');
  const compactPlayer = await startDryRunWithSelection(page);
  const compactInspector = await openDetails(page);
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(page.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  await expect(compactPlayer.locator('.player-dry-run-label')).toHaveText('Dry run');
  await expect(compactInspector).toBeVisible();
  measurements['640x448'] = await readWorkspaceGeometry(page);
  await capture(page, 'candidate-overlay-640x448.png');

  await openApp(page, { width: 640, height: 448 });
  await createLongPlaylistAndOpenMenu(page);
  const playlistTitle = page.getByRole('heading', { name: playlistName });
  await expect(playlistTitle).toHaveAttribute('title', playlistName);
  measurements['playlist-640x448'] = await readWorkspaceGeometry(page);
  await capture(page, 'candidate-playlist-add-menu-640x448.png');

  mkdirSync(evidenceDirectory, { recursive: true });
  await import('node:fs/promises').then(({ writeFile }) =>
    writeFile(
      resolve(evidenceDirectory, 'candidate-measurements.json'),
      `${JSON.stringify(measurements, null, 2)}\n`,
    ),
  );
});
