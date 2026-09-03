import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from '@playwright/test';

async function expectNoSeriousAccessibilityViolations(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.filter(
      (violation) => violation.impact === 'critical' || violation.impact === 'serious',
    ),
  ).toEqual([]);
}

async function expectWorkbenchWithinBounds(
  page: Page,
  minimumTrackWidth = 420,
  minimumUtilityWidth = 280,
) {
  const geometry = await page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, right: rect.right, bottom: rect.bottom, width: rect.width };
    };
    return {
      viewportWidth: window.innerWidth,
      workbench: box('.workbench'),
      navigator: box('.navigator-workbench-pane'),
      track: box('.track-browser-workbench-pane'),
      utility: box('.utility-workbench-pane'),
      rootMinWidth: getComputedStyle(document.documentElement).minWidth,
      bodyMinWidth: getComputedStyle(document.body).minWidth,
    };
  });

  expect(geometry.workbench).not.toBeNull();
  expect(geometry.navigator).not.toBeNull();
  expect(geometry.track).not.toBeNull();
  if (geometry.workbench && geometry.navigator && geometry.track) {
    expect(geometry.navigator.x).toBeGreaterThanOrEqual(geometry.workbench.x);
    expect(geometry.navigator.x + geometry.navigator.width).toBeLessThanOrEqual(geometry.track.x);
    expect(geometry.track.x + geometry.track.width).toBeLessThanOrEqual(
      geometry.workbench.x + geometry.workbench.width - 8,
    );
    expect(geometry.track.width).toBeGreaterThanOrEqual(minimumTrackWidth);
  }
  if (geometry.utility && geometry.workbench) {
    if (geometry.track) {
      expect(geometry.track.x + geometry.track.width).toBeLessThanOrEqual(geometry.utility.x);
    }
    expect(geometry.utility.x + geometry.utility.width).toBeLessThanOrEqual(
      geometry.workbench.x + geometry.workbench.width - 8,
    );
    expect(Math.round(geometry.viewportWidth - geometry.utility.right)).toBeGreaterThanOrEqual(8);
    expect(geometry.utility.width).toBeGreaterThanOrEqual(minimumUtilityWidth);
  }
  expect(geometry.rootMinWidth).toBe('0px');
  expect(geometry.bodyMinWidth).toBe('0px');
  await expect(page.locator('.workbench')).toHaveAttribute('data-layout-fits', 'true');
}

test('mock desktop vertical slice can search and inspect a song', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  await page.getByLabel('Search library').fill('Moonlit');
  await expect(page.getByRole('row', { name: /Moonlit Village/ })).toBeVisible();
  await page.getByRole('row', { name: /Moonlit Village/ }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByText('Low timing risk')).toBeVisible();
});

test('virtualized library pages beyond the first 200 songs', async ({ page }) => {
  await page.goto('/');
  const list = page.locator('.track-table');
  await expect(page.getByText('500 songs')).toBeVisible();
  await list.evaluate((element) => {
    element.scrollTop = 400 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  await expect(list).toHaveClass(/is-scrolling/);
  const song = page.getByRole('row', { name: /Song 401/ });
  await expect(song).toBeVisible();
  await song.click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByRole('heading', { name: 'Song 401' })).toBeVisible();
});

test('All Songs clears search while retaining the catalog count', async ({ page }) => {
  await page.goto('/');
  const navigatorItem = page.getByRole('button', { name: 'All Songs' });
  await expect(navigatorItem).toContainText('500');
  await page.getByLabel('Search library').fill('Moonlit');
  await expect(page.getByText('1 song')).toBeVisible();
  await expect(navigatorItem).toContainText('500');
  await navigatorItem.click();
  await expect(page.getByLabel('Search library')).toHaveValue('');
  await expect(page.getByText('500 songs')).toBeVisible();
  await expect(navigatorItem).toContainText('500');
});

test('Liked Songs is a real source with Player Bar save behavior', async ({ page }) => {
  await page.goto('/');
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
  await page.goto('/');
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.getByRole('button', { name: 'Create playlist' }).click();

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
  await expect(page.getByRole('heading', { name: 'Practice' })).toBeVisible();
  await expect(page.getByText('Adding songs')).toBeVisible();
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
  await expect(page.getByRole('heading', { name: 'All Songs' })).toBeVisible();
});

test('Add songs keeps local imports inside the selected playlist', async ({ page }) => {
  await page.goto('/');
  const navigator = page.getByRole('navigation', { name: 'Library' });

  await navigator.getByRole('button', { name: 'Create playlist' }).click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill('Practice');
  await createDialog.getByRole('button', { name: 'Create' }).click();
  await expect(navigator.getByRole('button', { name: 'Practice', exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Add songs' }).click();
  await page.getByRole('menuitem', { name: 'Browse All Songs…' }).click();
  await expect(page.getByRole('heading', { name: 'Practice' })).toBeVisible();
  await expect(page.getByText('Adding songs')).toBeVisible();
  await expect(page.getByPlaceholder('Search All Songs…')).toBeVisible();
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

  await page.getByRole('button', { name: 'All Songs' }).click();
  await expect(page.getByText('500 songs')).toBeVisible();
  await expect(page.getByRole('row', { name: /Local Song B/ })).toHaveCount(0);

  await page.getByRole('button', { name: 'Practice', exact: true }).click();
  await page
    .getByRole('row', { name: /Aurora Landing/ })
    .getByRole('button', {
      name: 'More actions for Aurora Landing',
    })
    .click();
  await page.getByRole('menuitem', { name: 'Remove from playlist' }).click();
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toHaveCount(0);

  await page.getByRole('button', { name: 'More actions for Practice' }).click();
  await page.getByRole('menuitem', { name: 'Delete playlist' }).click();
  const deleteDialog = page.getByRole('dialog', { name: /Delete/ });
  await expect(deleteDialog).toContainText('local files will not be deleted');
  await deleteDialog.getByRole('button', { name: 'Delete playlist' }).click();
  await expect(navigator.getByRole('button', { name: /Practice/ })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'All Songs' })).toBeVisible();
});

test('collapsed Library rail stays compact and keyboard-accessible at 800 by 560', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await navigator.getByRole('button', { name: 'Collapse library navigator' }).click();
  await expect(navigator.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  await expect(navigator.getByRole('button', { name: 'Create playlist' })).toBeVisible();
  const collapsedLabels = navigator.locator('.library-nav-item-label');
  await expect(collapsedLabels).toHaveCount(2);
  expect(
    await collapsedLabels.evaluateAll((items) =>
      items.every((item) => {
        return getComputedStyle(item).display === 'none';
      }),
    ),
  ).toBe(true);
  const pane = await page.locator('.navigator-workbench-pane').boundingBox();
  expect(pane).not.toBeNull();
  if (pane) expect(Math.round(pane.width)).toBe(56);

  await navigator.getByRole('button', { name: 'Expand library navigator' }).click();
  await expect(navigator.getByText('Your Library')).toBeVisible();
});

test('minimum viewport keeps the workbench and Player Bar bounded', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await expect(page.getByRole('grid', { name: 'Songs' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'All Songs' })).toBeVisible();
  await expect(page.getByRole('contentinfo', { name: 'Player controls' })).toBeVisible();
  const navigatorPane = await page.locator('.navigator-workbench-pane').boundingBox();
  const trackPane = await page.locator('.track-browser-workbench-pane').boundingBox();
  const playerBar = await page.getByRole('contentinfo', { name: 'Player controls' }).boundingBox();
  expect(navigatorPane).not.toBeNull();
  expect(trackPane).not.toBeNull();
  expect(playerBar).not.toBeNull();
  if (navigatorPane && trackPane && playerBar) {
    expect(Math.round(trackPane.x - (navigatorPane.x + navigatorPane.width))).toBe(8);
    expect(Math.round(playerBar.y - (navigatorPane.y + navigatorPane.height))).toBe(8);
  }
  const titlebar = await page.locator('.app-titlebar').boundingBox();
  expect(titlebar).not.toBeNull();
  if (titlebar && navigatorPane) {
    expect(Math.round(navigatorPane.y - (titlebar.y + titlebar.height))).toBe(0);
  }
  await expectWorkbenchWithinBounds(page);
  const dimensions = await page.evaluate(() => ({
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth: window.innerWidth,
    documentHeight: document.documentElement.scrollHeight,
    viewportHeight: window.innerHeight,
  }));
  expect(dimensions.documentWidth).toBeLessThanOrEqual(dimensions.viewportWidth);
  expect(dimensions.documentHeight).toBeLessThanOrEqual(dimensions.viewportHeight);
  await expectNoSeriousAccessibilityViolations(page);
});

test('minimum viewport keeps an open utility pane inside the actual client rectangle', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.locator('.utility-workbench-pane')).toBeVisible();
  await expect(page.getByRole('columnheader', { name: 'Notes' })).toBeHidden();
  await expect(page.getByRole('columnheader', { name: 'Liked' })).toBeVisible();
  await expect(page.getByRole('columnheader', { name: 'Duration' })).toBeVisible();
  const recommendationGrid = page.locator('.recommendation-grid');
  await expect(recommendationGrid).toBeVisible();
  await expect(recommendationGrid).toHaveCSS('grid-template-columns', /\S+px$/);
  await expectWorkbenchWithinBounds(page);
  const navigator = await page.locator('.navigator-workbench-pane').boundingBox();
  expect(navigator).not.toBeNull();
  if (navigator) expect(Math.round(navigator.width)).toBe(56);
});

test('browser resilience margin does not clip the workbench below the native minimum', async ({
  page,
}) => {
  await page.setViewportSize({ width: 780, height: 540 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.locator('.utility-workbench-pane')).toBeVisible();
  await expectWorkbenchWithinBounds(page, 420, 0);
});

test('desktop workbench fits the supported viewport matrix', async ({ page }) => {
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 900, height: 600 },
    { width: 1024, height: 640 },
    { width: 1200, height: 760 },
    { width: 1440, height: 900 },
    { width: 1920, height: 1080 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/');
    await expect(page.locator('.app-titlebar')).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Songs' })).toBeVisible();
    await expect(page.getByRole('contentinfo', { name: 'Player controls' })).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
      documentHeight: document.documentElement.scrollHeight,
      viewportHeight: window.innerHeight,
    }));
    expect(dimensions.documentWidth).toBeLessThanOrEqual(dimensions.viewportWidth);
    expect(dimensions.documentHeight).toBeLessThanOrEqual(dimensions.viewportHeight);

    await page.getByRole('button', { name: 'Open utility panel' }).click();
    await page.getByRole('tab', { name: 'Runtime' }).click();
    await expectWorkbenchWithinBounds(page);
    await expect(
      page.getByRole('region', { name: 'Utility: Diagnostics', exact: true }),
    ).toBeVisible();
  }
});

test('Player Bar communicates the no-selection state without actionable playback', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const primary = player.getByRole('button', { name: 'Play', exact: true });
  const labels = player.locator('.player-timeline-labels');

  await expect(primary).toBeDisabled();
  await expect(player.getByText('Select a song from your Library')).toBeVisible();
  await expect(labels).toHaveCount(1);
  await expect(labels).toHaveAttribute('aria-hidden', 'true');
  expect(await labels.textContent()).toBe('');
  await expect(player.getByRole('progressbar')).toHaveAttribute('aria-disabled', 'true');

  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await expect(
    page.getByRole('dialog', { name: 'Playback profile' }).getByRole('button', {
      name: 'Test playback (no input)',
    }),
  ).toBeDisabled();
  await page.keyboard.press('Escape');
});

test('Player transport geometry stays fixed when song timing labels appear', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const measure = async () => {
    const playBox = await player.getByRole('button', { name: 'Play', exact: true }).boundingBox();
    const timelineBox = await player.locator('.player-timeline').boundingBox();
    expect(playBox).not.toBeNull();
    expect(timelineBox).not.toBeNull();
    if (!playBox || !timelineBox) throw new Error('player geometry is unavailable');
    return {
      playCenterY: playBox.y + playBox.height / 2,
      timelineY: timelineBox.y,
    };
  };

  const idleGeometry = await measure();
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await expect(player.locator('.player-timeline-labels')).toHaveAttribute('aria-hidden', 'false');
  const selectedGeometry = await measure();

  expect(Math.abs(selectedGeometry.playCenterY - idleGeometry.playCenterY)).toBeLessThanOrEqual(1);
  expect(Math.abs(selectedGeometry.timelineY - idleGeometry.timelineY)).toBeLessThanOrEqual(1);
});

test('Titlebar search and Player primary control share the application center axis', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto('/');
  const viewportCenter = 1920 / 2;
  const searchBox = await page.getByLabel('Search library').boundingBox();
  const idleBox = await page.getByRole('button', { name: 'Play', exact: true }).boundingBox();
  expect(searchBox).not.toBeNull();
  expect(idleBox).not.toBeNull();
  if (searchBox && idleBox) {
    expect(Math.abs(searchBox.x + searchBox.width / 2 - viewportCenter)).toBeLessThanOrEqual(2);
    expect(Math.abs(idleBox.x + idleBox.width / 2 - viewportCenter)).toBeLessThanOrEqual(2);
  }

  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page.getByRole('button', { name: 'Play', exact: true }).click();
  await page
    .getByRole('group', { name: 'Playback confirmation' })
    .getByRole('button', { name: 'Proceed with current settings' })
    .click();
  const activeBox = await page.getByRole('button', { name: 'Pause' }).boundingBox();
  expect(activeBox).not.toBeNull();
  if (activeBox) {
    expect(Math.abs(activeBox.x + activeBox.width / 2 - viewportCenter)).toBeLessThanOrEqual(2);
  }
});

test('Player Bar remains compact, centered, and bounded at the minimum viewport', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const playerBox = await player.boundingBox();
  const playBox = await player.getByRole('button', { name: 'Play', exact: true }).boundingBox();
  const timelineBox = await player.locator('.player-timeline').boundingBox();
  const toolsBox = await player.locator('.player-tools').boundingBox();
  expect(playerBox).not.toBeNull();
  expect(playBox).not.toBeNull();
  expect(timelineBox).not.toBeNull();
  expect(toolsBox).not.toBeNull();
  if (playerBox && playBox && timelineBox && toolsBox) {
    expect(Math.abs(playBox.x + playBox.width / 2 - 400)).toBeLessThanOrEqual(2);
    expect(toolsBox.x + toolsBox.width).toBeLessThanOrEqual(playerBox.x + playerBox.width - 8);
    expect(timelineBox.x + timelineBox.width).toBeLessThanOrEqual(toolsBox.x);
  }
  await expect(player.getByRole('button', { name: 'Configure playback profile' })).toHaveAttribute(
    'aria-label',
    'Configure playback profile',
  );

  await page.getByRole('button', { name: 'Open utility panel' }).click();
  const openPlayBox = await player.getByRole('button', { name: 'Play', exact: true }).boundingBox();
  expect(openPlayBox).not.toBeNull();
  if (openPlayBox) {
    expect(Math.abs(openPlayBox.x + openPlayBox.width / 2 - 400)).toBeLessThanOrEqual(2);
  }
});

test('Playback Profile works through the narrow popover with focus restore', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const trigger = page.getByRole('button', { name: 'Configure playback profile' });
  await trigger.click();

  const popover = page.getByRole('dialog', { name: 'Playback profile' });
  await expect(popover).toBeVisible();
  await expect(popover.getByLabel('Hold')).toBeVisible();
  await expect(popover.getByLabel('Tempo')).toBeVisible();
  await expect(popover.getByLabel('FPS')).toBeVisible();
  await expect(popover.getByRole('button', { name: 'Test playback (no input)' })).toBeVisible();
  await expect(popover.locator('input[type="checkbox"]')).toHaveCount(0);
  await expectNoSeriousAccessibilityViolations(page);

  await page.keyboard.press('Tab');
  await expect(popover.locator(':focus')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(popover).toBeHidden();
  await expect(trigger).toBeFocused();
});

test('Library separator resizes by pointer and persists after reload', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  const separator = page.getByRole('separator', { name: 'Resize library navigator' });
  const initial = Number(await separator.getAttribute('aria-valuenow'));
  const box = await separator.boundingBox();
  expect(box).not.toBeNull();
  if (!box) return;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2);
  await page.mouse.up();
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial + 40));
  await page.reload();
  await expect(page.getByRole('separator', { name: 'Resize library navigator' })).toHaveAttribute(
    'aria-valuenow',
    String(initial + 40),
  );
});

test('selected Song Detail has no serious accessibility violations', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByRole('heading', { name: 'Aurora Landing' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
});

test('Player Bar keeps transport geometry stable through its lifecycle', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const progress = page.getByRole('progressbar', { name: /Playback progress/ });
  await expect(progress).toBeVisible();
  await expect(progress).toHaveAttribute('value', '0');
  const idlePrimary = page.getByRole('button', { name: 'Play', exact: true });
  const idleBox = await idlePrimary.boundingBox();
  expect(idleBox).not.toBeNull();

  await idlePrimary.click();
  const confirmation = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(confirmation).toBeVisible();
  await expect(
    confirmation.getByRole('button', { name: 'Proceed with current settings' }),
  ).toBeFocused();
  await expect(
    confirmation.getByRole('button', { name: 'Test playback (no input)' }),
  ).toBeVisible();
  await confirmation.getByRole('button', { name: 'Proceed with current settings' }).click();

  const activePrimary = page.getByRole('button', { name: 'Pause' });
  await expect(activePrimary).toBeVisible();
  const activeBox = await activePrimary.boundingBox();
  expect(activeBox).not.toBeNull();
  if (idleBox && activeBox) {
    expect(
      Math.abs(idleBox.x + idleBox.width / 2 - (activeBox.x + activeBox.width / 2)),
    ).toBeLessThanOrEqual(1);
  }
  await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  await page.getByRole('button', { name: 'Pause' }).click();
  await expect(page.getByRole('button', { name: 'Resume' })).toBeVisible();
  await page.getByRole('button', { name: 'Resume' }).click();
  await expect(page.getByRole('button', { name: 'Pause' })).toBeVisible();
  await page.getByRole('button', { name: 'Stop' }).click();
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
});

test('Settings modal has no serious accessibility violations and closes accessibly', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Playback defaults');
  await expect(dialog.getByRole('button', { name: 'Playback' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  const initialBox = await dialog.boundingBox();
  const settingsContent = dialog.locator('.settings-content');
  const initialContentWidth = await settingsContent.evaluate((element) => element.clientWidth);
  expect(initialBox).not.toBeNull();
  await expect(dialog.getByLabel('Hold').locator('option[value="1"]')).toHaveText('1 frame');
  for (const category of [
    'Appearance',
    'Diagnostics',
    'Updates',
    'Advanced',
    'About',
    'Playback',
  ]) {
    await dialog.getByRole('button', { name: category, exact: true }).click();
    const nextBox = await dialog.boundingBox();
    const nextContentWidth = await settingsContent.evaluate((element) => element.clientWidth);
    expect(nextBox).not.toBeNull();
    expect(nextContentWidth).toBe(initialContentWidth);
    if (initialBox && nextBox) {
      expect(Math.abs(nextBox.x - initialBox.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.y - initialBox.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.width - initialBox.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.height - initialBox.height)).toBeLessThanOrEqual(1);
    }
  }

  const expectFocusClearance = async (control: Locator) => {
    await control.focus();
    const controlBox = await control.boundingBox();
    const contentBox = await settingsContent.boundingBox();
    expect(controlBox).not.toBeNull();
    expect(contentBox).not.toBeNull();
    if (controlBox && contentBox) {
      expect(
        contentBox.x + contentBox.width - (controlBox.x + controlBox.width),
      ).toBeGreaterThanOrEqual(4);
    }
  };

  await dialog.getByRole('button', { name: 'Playback', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Hold'));
  await expectFocusClearance(dialog.getByLabel('Tempo'));
  await expectFocusClearance(dialog.getByLabel('FPS'));
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Theme'));
  await dialog.getByRole('button', { name: 'Updates', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Channel'));
  await expectFocusClearance(dialog.getByLabel('Skip version'));
  await dialog.getByRole('button', { name: 'About', exact: true }).click();
  await expect(dialog).toContainText('Native ABI');
  await expectNoSeriousAccessibilityViolations(page);

  const focusables = dialog.locator('button, select, input');
  await focusables.first().focus();
  await page.keyboard.press('Shift+Tab');
  await expect(dialog.locator(':focus')).toHaveCount(1);
  await focusables.last().focus();
  await page.keyboard.press('Tab');
  await expect(dialog.locator(':focus')).toHaveCount(1);

  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(settingsButton).toBeFocused();
});

test('wide Diagnostics integrates as a workbench pane', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await page.getByRole('tab', { name: 'Runtime' }).click();
  const panel = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(panel).toBeVisible();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const panelBox = await panel.boundingBox();
  const playerBox = await player.boundingBox();
  expect(panelBox).not.toBeNull();
  expect(playerBox).not.toBeNull();
  if (panelBox && playerBox) expect(panelBox.y + panelBox.height).toBeLessThanOrEqual(playerBox.y);
  await panel.getByRole('tab', { name: 'Timing' }).click();
  await expect(panel.getByRole('img', { name: /Maximum timing lateness/ })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
  await panel.getByRole('button', { name: 'Close utility' }).click();
  await expect(panel).toBeHidden();
  await expect(page.getByRole('button', { name: 'Open utility panel' })).toBeFocused();
});

test('Diagnostics utility separator follows pointer and keyboard direction', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await page.getByRole('tab', { name: 'Runtime' }).click();
  const separator = page.getByRole('separator', { name: 'Resize utility pane' });
  const initial = Number(await separator.getAttribute('aria-valuenow'));
  const box = await separator.boundingBox();
  expect(box).not.toBeNull();
  if (!box) return;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 20, box.y + box.height / 2);
  await page.mouse.up();
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial - 20));

  await separator.press('ArrowRight');
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial - 28));
  await separator.press('Shift+ArrowLeft');
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial + 4));
});

test('all supported themes round-trip through the settings surface', async ({ page }) => {
  await page.goto('/');
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  await page.getByRole('button', { name: 'Appearance' }).click();
  const theme = page.getByLabel('Theme');
  for (const id of ['aurora', 'minimalist', 'slate', 'cyberpunk', 'classic']) {
    await theme.selectOption(id);
    await expect(page.locator('html')).toHaveAttribute('data-theme', id);
    await expectNoSeriousAccessibilityViolations(page);
  }
  await page.keyboard.press('Escape');
  await expect(settingsButton).toBeFocused();
});

test('update indicator and typed update dialog expose safe handoff states', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const indicator = page.getByRole('button', { name: /Open update 3\.6\.0-mock/ });
  await expect(indicator).toBeVisible();
  await indicator.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Software update' });
  await expect(dialog).toContainText('Version 3.6.0-mock is available');
  await expectNoSeriousAccessibilityViolations(page);
  await dialog.getByRole('button', { name: 'Update and restart' }).click();
  await expect(dialog).toContainText('Restart handoff ready');
  await expectNoSeriousAccessibilityViolations(page);
});

test('Diagnostics drawer is bounded and accessible at the minimum viewport', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const trigger = page.getByRole('button', { name: 'Open utility panel' });
  await trigger.click();
  const drawer = page.getByRole('region', { name: 'Utility: Song Details' });
  await expect(drawer).toBeVisible();
  await expect(drawer).not.toHaveAttribute('aria-modal');
  await drawer.getByRole('tab', { name: 'Details' }).focus();
  await page.keyboard.press('ArrowRight');
  const diagnosticsDrawer = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(diagnosticsDrawer.getByRole('tab', { name: 'Performance' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
  await diagnosticsDrawer.getByRole('tab', { name: 'Timing' }).click();
  await expect(
    diagnosticsDrawer.getByRole('img', { name: /Maximum timing lateness/ }),
  ).toBeVisible();
  await expect(diagnosticsDrawer).toContainText(/No timing samples|Latest maximum lateness/);
  await diagnosticsDrawer.getByRole('tab', { name: 'Events' }).click();
  await expectNoSeriousAccessibilityViolations(page);
  await diagnosticsDrawer.getByRole('button', { name: 'Close utility' }).click();
  await expect(diagnosticsDrawer).toBeHidden();
  await expect(page.getByRole('button', { name: 'Open utility panel' })).toBeFocused();
  await trigger.click();
  await expect(page.getByRole('region', { name: 'Utility: Diagnostics' })).toBeVisible();
  await page.getByRole('button', { name: 'Close utility panel' }).click();
});

test('long sheet titles remain contained in the Track Browser, Utility, and Player Bar', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const list = page.locator('.track-table');
  await list.evaluate((element) => {
    element.scrollTop = 495 * 46;
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

test('Calibration dialog exposes safe running and terminal states', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  await page.getByRole('button', { name: 'Advanced' }).click();
  await page.getByRole('button', { name: 'Open calibration' }).click();
  const dialog = page.getByRole('dialog', { name: 'Timing calibration' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Start quick calibration' }).click();
  await expect(dialog).toContainText('Calibration complete');
  await expectNoSeriousAccessibilityViolations(page);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(dialog).toBeHidden();
});
