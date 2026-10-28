import { expect, test, type Page } from '@playwright/test';

const viewports = [
  { width: 640, height: 448 },
  { width: 800, height: 560 },
  { width: 1024, height: 640 },
  { width: 1366, height: 768 },
  { width: 1920, height: 1080 },
] as const;

async function openApp(page: Page, viewport: { width: number; height: number }, path = '/') {
  await page.setViewportSize(viewport);
  if (page.url().startsWith('http')) {
    await page.evaluate(() => window.localStorage.clear());
  }
  await page.goto(path);
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
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
  await expect(player.locator('.player-track-status')).toHaveText('Dry run · Playing');
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

test('workbench library, playback safety, keyboard, and diagnostics states remain functional', async ({
  page,
}) => {
  test.setTimeout(300_000);

  const desktopViewport = viewports[3];
  await openApp(page, desktopViewport, '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
  await startDryRunWithSelection(page);
  await openInspector(page, 'Details');
  const search = page.getByRole('searchbox', { name: 'Search All Songs' });
  await search.focus();
  await expect(search).toBeFocused();
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  const smokeNavigator = page.getByRole('navigation', { name: 'Library' });
  await smokeNavigator.getByRole('button', { name: 'Collapse library navigator' }).click();
  await expect(page.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();

  await openApp(page, desktopViewport, '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await startDryRunWithSelection(page);
  const runtimeInspector = await openInspector(page, 'Runtime');
  await expect(
    runtimeInspector.getByText('Mode: Dry run · no physical input sent', { exact: true }),
  ).toBeVisible();
  await expect(runtimeInspector.getByText('Sender samples', { exact: true })).toBeVisible();
  await expect(runtimeInspector.getByRole('tab', { name: 'Runtime' })).toBeVisible();

  await openApp(page, viewports[1], '/?mockLibraryState=empty');
  await expect(page.locator('.empty-state strong')).toHaveText('Your catalog is empty');
  await expect(page.locator('.library-empty-playlists p')).toContainText(
    'import local sheet files',
  );

  await openApp(page, viewports[1]);
  await createPlaylist(page, 'Practice');
  await expect(page.getByText('This playlist is empty')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add songs' })).toBeVisible();

  await openApp(page, viewports[2]);
  await createPlaylist(page, 'Practice');
  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Import files…' }).click();
  await expect(page.getByRole('status')).toHaveText('Import complete: 1 song found in Practice.');
  await expect(page.getByRole('row', { name: /Local Song B/ })).toBeVisible();

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
  await expect(menuItem).toBeVisible();

  await openApp(page, viewports[3]);
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click();
  const palette = settings.getByLabel('Color palette');
  await palette.focus();
  await expect(palette).toBeFocused();
  await expect(palette).toBeVisible();

  await openApp(page, viewports[3]);
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const confirmation = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(confirmation).toBeVisible();
  await expect(
    confirmation.getByRole('button', { name: 'Proceed with current settings' }),
  ).toBeFocused();

  await openApp(page, viewports[1], '/?mockStartFailure=target_not_found');
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true })
    .click();
  const recovery = page.getByRole('alert');
  await expect(recovery).toContainText('Sky window was not found');
  await expect(recovery.getByRole('button', { name: 'Try again' })).toBeVisible();

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
});
