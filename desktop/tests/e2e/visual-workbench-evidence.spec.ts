import { mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { getUtilityPresentationMode } from '../../src/components/workbench/useWorkbenchLayout';

type EvidenceVariant = 'before' | 'graphite-v1' | 'graphite-v2';

const evidenceVariant = process.env.GRAPHITE_WORKBENCH_VARIANT as EvidenceVariant | undefined;
const evidenceRoot = resolve(process.cwd(), '../docs/evidence/graphite-workbench');
const viewports = [
  { width: 640, height: 448 },
  { width: 800, height: 560 },
  { width: 1024, height: 640 },
  { width: 1366, height: 768 },
  { width: 1920, height: 1080 },
] as const;

test.skip(
  evidenceVariant !== 'before' &&
    evidenceVariant !== 'graphite-v1' &&
    evidenceVariant !== 'graphite-v2',
  'Set GRAPHITE_WORKBENCH_VARIANT to before, graphite-v1, or graphite-v2 to capture approval evidence.',
);

function evidencePath(name: string): string {
  return resolve(evidenceRoot, evidenceVariant!, `${name}.png`);
}

async function capture(page: Page, name: string, locator?: Locator) {
  const path = evidencePath(name);
  mkdirSync(dirname(path), { recursive: true });
  if (locator) {
    await locator.screenshot({ path, animations: 'disabled' });
  } else {
    await page.screenshot({ path, animations: 'disabled' });
  }
}

async function openApp(page: Page, viewport: { width: number; height: number }, path = '/') {
  await page.setViewportSize(viewport);
  if (page.url().startsWith('http')) {
    await page.evaluate(() => window.localStorage.clear());
  }
  await page.goto(path);
  await expect(page.locator('.app-shell')).toBeVisible();
  if (evidenceVariant === 'before') {
    await expect(page.locator('html')).toHaveAttribute('data-palette', 'aurora');
  } else if (evidenceVariant === 'graphite-v1') {
    await page.locator('html').evaluate((root) => root.setAttribute('data-palette', 'graphite'));
  } else {
    await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  }
}

async function startDryRunWithSelection(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await expect(page.getByRole('row', { name: /Candle Run/ })).toBeVisible();
  await page.getByRole('row', { name: /Candle Run/ }).click();
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await page
    .getByRole('dialog', { name: 'Playback profile' })
    .getByRole('button', { name: 'Test playback (no input)' })
    .click();
  await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
  await expect(player.locator('.player-dry-run-label')).toBeVisible();
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await expect(page.getByRole('row', { name: /Blue Bird/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run');
  return player;
}

async function openInspector(page: Page, tab: 'Details' | 'Runtime' = 'Details') {
  const trigger = page.getByRole('button', { name: 'Open utility panel' });
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
  if (tab === 'Runtime') await inspector.getByRole('tab', { name: 'Runtime' }).click();
  else await inspector.getByRole('tab', { name: 'Details' }).click();
  return inspector;
}

async function createPlaylist(page: Page, name = 'Practice') {
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.locator('.library-add-button').click();
  const dialog = page.getByRole('dialog', { name: 'New playlist' });
  await dialog.getByLabel('Playlist name').fill(name);
  await dialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByRole('heading', { name })).toBeVisible();
  return navigator;
}

test('captures matched Graphite Workbench approval states', async ({ page }) => {
  test.setTimeout(300_000);

  for (const viewport of viewports) {
    await openApp(page, viewport, '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
    await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
    const player = await startDryRunWithSelection(page);
    const inspector = await openInspector(page, 'Details');
    const search = page.getByRole('searchbox', { name: 'Search All Songs' });
    await search.focus();
    await expect(search).toBeFocused();
    await expect(page.locator('.workbench')).toHaveAttribute(
      'data-utility-presentation',
      getUtilityPresentationMode(viewport.width, true, 'graphite'),
    );
    await capture(page, `populated-${viewport.width}x${viewport.height}`);

    const titlebarPath = evidencePath(`closeup-titlebar-${viewport.width}x${viewport.height}`);
    mkdirSync(dirname(titlebarPath), { recursive: true });
    await page.screenshot({
      path: titlebarPath,
      clip: { x: 0, y: 0, width: viewport.width, height: 48 },
      animations: 'disabled',
    });

    if (viewport.width === 640) {
      await capture(page, 'closeup-inspector-overlay-640x448', inspector);
    }
    if (viewport.width === 1366) {
      await capture(page, 'closeup-row-playing-1366x768', page.locator('.track-row.is-playing'));
      await capture(page, 'closeup-row-selected-1366x768', page.locator('.track-row.is-selected'));
      await capture(page, 'closeup-inspector-docked-1366x768', inspector);
      await capture(page, 'closeup-player-dock-1366x768', player);

      const navigator = page.getByRole('navigation', { name: 'Library' });
      await navigator.getByRole('button', { name: 'Collapse library navigator' }).click();
      await capture(page, 'navigator-collapsed-1366x768');
    }
  }

  for (const viewport of [viewports[0], viewports[3]]) {
    await openApp(page, viewport, '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
    await startDryRunWithSelection(page);
    const inspector = await openInspector(page, 'Runtime');
    await expect(
      inspector.getByText('Mode: Dry run · no physical input sent', { exact: true }),
    ).toBeVisible();
    if (viewport.width >= 1366) {
      await expect(inspector.getByText('Sender samples', { exact: true })).toBeVisible();
    }
    await capture(page, `runtime-dry-run-${viewport.width}x${viewport.height}`);
  }

  await openApp(page, viewports[1], '/?mockLibraryState=empty');
  await expect(page.locator('.empty-state strong')).toHaveText('Your catalog is empty');
  await expect(page.locator('.library-empty-playlists p')).toContainText(
    'import local sheet files',
  );
  await capture(page, 'empty-library-800x560');

  await openApp(page, viewports[1]);
  await createPlaylist(page, 'Practice');
  await expect(page.getByText('This playlist is empty')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add songs' })).toBeVisible();
  await capture(page, 'empty-playlist-800x560');

  await openApp(page, viewports[2]);
  await createPlaylist(page, 'Practice');
  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Import files…' }).click();
  await expect(page.getByRole('status')).toHaveText('Import complete: 1 song found in Practice.');
  await expect(page.getByRole('row', { name: /Local Song B/ })).toBeVisible();
  await capture(page, 'imported-playlist-1024x640');

  await openApp(page, viewports[3]);
  const navigator = await createPlaylist(page, 'Practice');
  const allSongs = navigator.getByRole('button', { name: /^All Songs/ });
  await expect(allSongs).toBeVisible({ timeout: 5_000 });
  await allSongs.click({ timeout: 5_000 });
  const songRow = page.getByRole('row', { name: /Candle Run/ });
  await expect(songRow).toBeVisible({ timeout: 5_000 });
  const actions = songRow.getByRole('button', { name: 'More actions for Candle Run' });
  await songRow.hover({ timeout: 5_000 });
  await actions.click({ timeout: 5_000 });
  const menu = page.getByRole('menu', { name: 'Actions for Candle Run' });
  await expect(menu).toBeVisible({ timeout: 5_000 });
  const menuItem = menu.getByRole('menuitem', { name: 'Add to Practice' });
  await menuItem.focus();
  await expect(menuItem).toBeFocused();
  await capture(page, 'context-menu-keyboard-focus-1366x768');

  await openApp(page, viewports[3]);
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click();
  const palette = settings.getByLabel('Color palette');
  await palette.focus();
  await expect(palette).toBeFocused();
  await capture(page, 'settings-appearance-keyboard-focus-1366x768');

  await openApp(page, viewports[3]);
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const confirmation = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(confirmation).toBeVisible();
  await expect(
    confirmation.getByRole('button', { name: 'Proceed with current settings' }),
  ).toBeFocused();
  await capture(page, 'pending-playback-confirmation-1366x768');

  await openApp(page, viewports[1], '/?mockStartFailure=target_not_found');
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true })
    .click();
  const recovery = page.getByRole('alert');
  await expect(recovery).toContainText('Sky window was not found');
  await expect(recovery.getByRole('button', { name: 'Try again' })).toBeVisible();
  await capture(page, 'playback-recovery-800x560');

  await openApp(
    page,
    viewports[2],
    '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10&mockStartResponseDelayMs=60000&mockStatusQueryFailures=1&mockDropPlaybackSnapshots=1&mockDropPlaybackStartConfirmation=1',
  );
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true })
    .click();
  const uncertain = page.getByRole('alert');
  await expect(uncertain).toContainText('Playback status is unavailable', { timeout: 20_000 });
  await expect(uncertain.getByRole('button', { name: 'Retry status' })).toBeVisible();
  await expect(uncertain.getByRole('button', { name: 'Stop playback' })).toBeVisible();
  await capture(page, 'uncertain-native-status-1024x640');

  await openApp(
    page,
    viewports[3],
    '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10&mockDiagnosticsStatus=degraded',
  );
  const physicalPlayer = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await physicalPlayer.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(physicalPlayer.getByText('Playing', { exact: true })).toBeVisible();
  const diagnostics = await openInspector(page, 'Runtime');
  const senderStatus = diagnostics.getByRole('region', { name: 'Sender-side status' });
  await expect(senderStatus).toContainText('Sender-side status: Attention');
  await expect(diagnostics.getByText('Degraded', { exact: true })).toBeVisible();
  await expect(diagnostics.getByText('Mode: Physical input', { exact: true })).toBeVisible();
  await capture(page, 'physical-playback-degraded-diagnostics-1366x768');
});
