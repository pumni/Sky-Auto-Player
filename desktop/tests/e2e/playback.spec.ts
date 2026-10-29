import { expect, test, type Locator } from '@playwright/test';
import {
  expectNoSeriousAccessibilityViolations,
  readPlayerBarGeometry,
  expectFiveTransportControls,
  readRightDockGeometry,
  expectPlayerBarGeometry,
  expectStablePlayerBar,
  expectRecoveryBannerWithinViewport,
  startSelectedSong,
} from './testHelpers';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

test('Player Bar communicates the no-selection state without actionable playback', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const primary = player.getByRole('button', { name: 'Play', exact: true });
  const labels = player.locator('.player-timeline-labels');

  await expectFiveTransportControls(player, 'Play');
  await expect(player.getByRole('button', { name: 'Shuffle' })).toBeEnabled();
  await expect(player.getByRole('button', { name: 'Previous' })).toBeDisabled();
  await expect(player.getByRole('button', { name: 'Next' })).toBeDisabled();
  await expect(player.getByRole('button', { name: 'Stop' })).toBeDisabled();
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

test('five transport controls keep geometry at desktop and Chromium fallback sizes', async ({
  page,
}) => {
  const supportedDesktopViewports = [
    { width: 1200, height: 760 },
    { width: 800, height: 560 },
  ];
  const chromiumResponsiveFallbackViewports = [{ width: 640, height: 448 }];
  for (const viewport of [...supportedDesktopViewports, ...chromiumResponsiveFallbackViewports]) {
    await page.setViewportSize(viewport);
    await page.goto('/');
    const player = page.getByRole('contentinfo', { name: 'Player controls' });
    await expectFiveTransportControls(player, 'Play');
    await expect(player.getByRole('button', { name: 'Stop' })).toBeDisabled();

    const beforeSelection = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(beforeSelection);
    const beforeTools = await readRightDockGeometry(page);
    expect(beforeTools).not.toBeNull();
    if (beforeTools) {
      expect(beforeTools.autoPlay.x).toBeLessThan(beforeTools.profile.x);
      expect(beforeTools.profile.x).toBeLessThan(beforeTools.utility.x);
      expect(
        Math.max(
          beforeTools.autoPlay.centerY,
          beforeTools.profile.centerY,
          beforeTools.utility.centerY,
        ) -
          Math.min(
            beforeTools.autoPlay.centerY,
            beforeTools.profile.centerY,
            beforeTools.utility.centerY,
          ),
      ).toBeLessThanOrEqual(1);
    }

    await page.getByRole('row', { name: /Blue Bird/ }).click();
    await expectFiveTransportControls(player, 'Play');
    await expect(player.getByRole('button', { name: 'Stop' })).toBeDisabled();
    const afterSelection = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(afterSelection);
    expectStablePlayerBar(beforeSelection, afterSelection);
    expect(afterSelection).not.toBeNull();
    if (afterSelection) {
      expect(afterSelection.visibleControls.Shuffle).toBeDefined();
      expect(afterSelection.visibleControls.Previous).toBeDefined();
      expect(afterSelection.visibleControls.Primary).toBeDefined();
      expect(afterSelection.visibleControls.Next).toBeDefined();
      expect(afterSelection.visibleControls.Stop).toBeDefined();
    }
    if (beforeTools && afterSelection) {
      expect(beforeTools.autoPlay.x).toBeGreaterThanOrEqual(
        afterSelection.controls.x + afterSelection.controls.width - 1,
      );
    }
  }
});

test('dock and transport toggles show palette-specific active state without layout movement', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 760 });
  await page.goto('/');
  const player = await startSelectedSong(page);

  const measureStyle = (locator: Locator) =>
    locator.evaluate((element) => {
      const style = getComputedStyle(element);
      const box = element.getBoundingClientRect();
      const probe = document.createElement('span');
      probe.style.color = 'var(--text-primary)';
      document.body.append(probe);
      const primary = getComputedStyle(probe).color;
      probe.style.color = 'var(--accent-ink)';
      const accentInk = getComputedStyle(probe).color;
      probe.style.color = 'var(--accent-ink-hover)';
      const accentInkHover = getComputedStyle(probe).color;
      probe.style.color = 'var(--text-tertiary)';
      const tertiary = getComputedStyle(probe).color;
      probe.style.color = 'var(--text-secondary)';
      const secondary = getComputedStyle(probe).color;
      probe.style.color = 'var(--workbench-control-disabled)';
      const disabled = getComputedStyle(probe).color;
      probe.style.color = 'var(--workbench-toggle-on)';
      const toggleOn = getComputedStyle(probe).color;
      probe.style.color = 'var(--workbench-toggle-off)';
      const toggleOff = getComputedStyle(probe).color;
      probe.style.backgroundColor = 'var(--workbench-control-hover)';
      const controlHover = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return {
        color: style.color,
        background: style.backgroundColor,
        borderTop: style.borderTopWidth,
        x: box.x,
        y: box.y,
        width: box.width,
        height: box.height,
        primary,
        accentInk,
        accentInkHover,
        tertiary,
        secondary,
        disabled,
        toggleOn,
        toggleOff,
        controlHover,
        palette: document.documentElement.dataset.palette,
      };
    });

  for (const name of ['Shuffle', 'Previous', 'Next']) {
    const button = player.getByRole('button', { name, exact: true });
    const before = await measureStyle(button);
    await button.hover();
    await expect
      .poll(async () => (await measureStyle(button)).color, { timeout: 2_000 })
      .toBe(before.primary);
    const after = await measureStyle(button);
    expect(after.color).toBe(after.primary);
    expect(after.background).toBe(after.controlHover);
    expect(after.borderTop).toBe('0px');
    expect(after.x).toBe(before.x);
    expect(after.y).toBe(before.y);
    expect(after.width).toBe(before.width);
    expect(after.height).toBe(before.height);
  }

  const stop = player.getByRole('button', { name: 'Stop' });
  await stop.click();
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  await page.mouse.move(0, 0);
  await expect
    .poll(async () => (await measureStyle(stop)).color, { timeout: 2_000 })
    .toBe((await measureStyle(stop)).disabled);
  expect(await stop.isDisabled()).toBe(true);
  const stopBefore = await measureStyle(stop);
  const stopBox = await stop.boundingBox();
  if (!stopBox) throw new Error('disabled Stop geometry is unavailable');
  await page.mouse.move(stopBox.x + stopBox.width / 2, stopBox.y + stopBox.height / 2);
  const stopAfter = await measureStyle(stop);
  expect(stopAfter.color).toBe(stopBefore.color);
  expect(stopAfter.color).toBe(stopAfter.disabled);
  expect(stopAfter.background).toBe('rgba(0, 0, 0, 0)');
  expect(stopAfter.borderTop).toBe('0px');
  expect(stopAfter.x).toBe(stopBefore.x);
  expect(stopAfter.y).toBe(stopBefore.y);

  const shuffle = player.getByRole('button', { name: 'Shuffle', exact: true });
  const inactiveShuffle = await measureStyle(shuffle);
  await expect(shuffle).toHaveAttribute('aria-pressed', 'false');
  await shuffle.click();
  await expect(shuffle).toHaveAttribute('aria-pressed', 'true');
  const activeShuffleTone = (style: Awaited<ReturnType<typeof measureStyle>>) =>
    style.palette === 'graphite' ? style.toggleOn : style.accentInk;
  await expect
    .poll(async () => (await measureStyle(shuffle)).color, { timeout: 2_000 })
    .toBe(activeShuffleTone(await measureStyle(shuffle)));
  await page.mouse.move(0, 0);
  const activeShuffleBefore = await measureStyle(shuffle);
  expect(activeShuffleBefore.color).toBe(activeShuffleTone(activeShuffleBefore));
  expect(activeShuffleBefore.color).not.toBe(inactiveShuffle.color);
  expect(activeShuffleBefore.background).toBe('rgba(0, 0, 0, 0)');
  expect(activeShuffleBefore.x).toBe(inactiveShuffle.x);
  expect(activeShuffleBefore.y).toBe(inactiveShuffle.y);
  await shuffle.hover();
  await expect
    .poll(async () => (await measureStyle(shuffle)).color, { timeout: 2_000 })
    .toBe(activeShuffleTone(await measureStyle(shuffle)));
  const activeShuffleAfter = await measureStyle(shuffle);
  expect(activeShuffleAfter.color).toBe(activeShuffleTone(activeShuffleAfter));
  expect(activeShuffleAfter.background).toBe(activeShuffleAfter.controlHover);

  await shuffle.focus();
  await expect(shuffle).toBeFocused();
  await shuffle.press('Space');
  await expect(shuffle).toHaveAttribute('aria-pressed', 'false');
  await shuffle.press('Space');
  await expect(shuffle).toHaveAttribute('aria-pressed', 'true');
  await expect
    .poll(async () => (await measureStyle(shuffle)).color, { timeout: 2_000 })
    .toBe(activeShuffleTone(await measureStyle(shuffle)));

  const autoPlay = player.getByRole('button', { name: 'Auto Play' });
  const autoPlayBefore = await measureStyle(autoPlay);
  const activeAutoPlayTone = (style: Awaited<ReturnType<typeof measureStyle>>) =>
    style.palette === 'graphite' ? style.toggleOn : style.accentInk;
  const hoveredAutoPlayTone = (style: Awaited<ReturnType<typeof measureStyle>>) =>
    style.palette === 'graphite' ? style.toggleOn : style.accentInkHover;
  expect(autoPlayBefore.color).toBe(activeAutoPlayTone(autoPlayBefore));
  expect(autoPlayBefore.background).toBe('rgba(0, 0, 0, 0)');
  await autoPlay.hover();
  await expect
    .poll(async () => (await measureStyle(autoPlay)).color, { timeout: 2_000 })
    .toBe(hoveredAutoPlayTone(autoPlayBefore));
  const autoPlayHover = await measureStyle(autoPlay);
  expect(autoPlayHover.color).toBe(hoveredAutoPlayTone(autoPlayHover));
  expect(autoPlayHover.background).toBe(autoPlayHover.controlHover);
  expect(autoPlayHover.borderTop).toBe('0px');
  expect(autoPlayHover.x).toBe(autoPlayBefore.x);
  expect(autoPlayHover.y).toBe(autoPlayBefore.y);

  await autoPlay.click();
  await expect(autoPlay).toHaveAttribute('aria-pressed', 'false');
  await page.mouse.move(0, 0);
  const inactiveAutoPlayTone = (style: Awaited<ReturnType<typeof measureStyle>>) =>
    style.palette === 'graphite' ? style.toggleOff : style.tertiary;
  await expect
    .poll(async () => (await measureStyle(autoPlay)).color, { timeout: 2_000 })
    .toBe(inactiveAutoPlayTone(await measureStyle(autoPlay)));
  const autoPlayOff = await measureStyle(autoPlay);
  expect(autoPlayOff.color).toBe(inactiveAutoPlayTone(autoPlayOff));
  await autoPlay.hover();
  await expect
    .poll(async () => (await measureStyle(autoPlay)).color, { timeout: 2_000 })
    .toBe(autoPlayOff.primary);
  const autoPlayOffHover = await measureStyle(autoPlay);
  expect(autoPlayOffHover.color).toBe(autoPlayOffHover.primary);
  expect(autoPlayOffHover.background).toBe(autoPlayOffHover.controlHover);

  await page.mouse.move(0, 0);
  await autoPlay.focus();
  await autoPlay.press('Space');
  await expect(autoPlay).toBeFocused();
  await expect(autoPlay).toHaveAttribute('aria-pressed', 'true');
  await expect
    .poll(async () => (await measureStyle(autoPlay)).color, { timeout: 2_000 })
    .toBe(activeAutoPlayTone(await measureStyle(autoPlay)));
  await autoPlay.press('Space');
  await expect(autoPlay).toHaveAttribute('aria-pressed', 'false');

  await page.mouse.move(0, 0);
  await shuffle.evaluate((element: HTMLButtonElement) => element.blur());
  await autoPlay.evaluate((element: HTMLButtonElement) => element.blur());
  for (const palette of ['aurora', 'minimalist', 'slate', 'cyberpunk', 'classic']) {
    await page.locator('html').evaluate((root, selected) => {
      root.setAttribute('data-palette', selected);
    }, palette);
    await expect
      .poll(async () => {
        const style = await measureStyle(shuffle);
        return style.color === style.accentInk;
      })
      .toBe(true);
    await expect
      .poll(async () => {
        const style = await measureStyle(autoPlay);
        return style.color === style.tertiary;
      })
      .toBe(true);
  }
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

test('natural finish retires the old session and starts the next context song', async ({
  page,
}) => {
  // Keep the successor transition observable without extending the assertion timeout.
  await page.goto('/?mockPlaybackDurationMs=2500,120000&mockStartDelayMs=100');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.locator('.player-track-status')).toHaveText('Playing');
  const transition = player.getByRole('status');
  await expect(async () => {
    const statusText = await transition.textContent().catch(() => null);
    const trackTitle = await player
      .locator('.player-track-copy strong')
      .textContent()
      .catch(() => null);
    expect(statusText === 'Starting next song' || trackTitle === 'Candle Run').toBe(true);
  }).toPass({ timeout: 12_000 });
  const playingRow = page.getByRole('row', { name: /Blue Bird/ });
  await expect(playingRow).not.toHaveAttribute('aria-current', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run', {
    timeout: 12_000,
  });
  await expect(player.getByText('Playing', { exact: true })).toBeVisible({ timeout: 12_000 });
  await expect(page.getByRole('row', { name: /Candle Run/ })).toHaveAttribute(
    'aria-current',
    'true',
    { timeout: 12_000 },
  );
  await expect(page.getByRole('alert', { name: 'Playback error' })).toHaveCount(0);
});

test('Auto Play advances while Built-in browsing keeps selection and page separate', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 760 });
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const list = page.locator('.track-table');
  await page.getByRole('button', { name: 'Next page' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  await page.getByRole('button', { name: 'Next page' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 3' })).toBeVisible();
  await list.evaluate((element) => {
    element.scrollTop = 48 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  const selectedRow = page.getByRole('row', { name: /Song 249/ });
  await expect(selectedRow).toBeVisible();
  await selectedRow.click();
  const selectedGeometry = await selectedRow.boundingBox();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await expect(selectedRow).toHaveAttribute('aria-current', 'true');
  const playingGeometry = await selectedRow.boundingBox();
  expect(selectedGeometry).not.toBeNull();
  expect(playingGeometry).not.toBeNull();
  if (selectedGeometry && playingGeometry) {
    expect(Math.abs(playingGeometry.height - selectedGeometry.height)).toBeLessThanOrEqual(0.5);
  }

  await list.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event('scroll'));
  });
  await player.getByRole('button', { name: 'Next' }).click();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 250');

  const nowPlayingRow = page.getByRole('row', { name: /Song 250/ });
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 3' })).toBeVisible();
  await expect(nowPlayingRow).toHaveCount(0);
  await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBe(0);
  await list.evaluate((element) => {
    element.scrollTop = 48 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  await expect(nowPlayingRow).toHaveAttribute('aria-current', 'true');
  await expect(nowPlayingRow).toHaveAttribute('aria-selected', 'false');
  await expect(nowPlayingRow).toBeInViewport();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
});

test('natural Auto Play preserves the Built-in page and selection while advancing playback', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 760 });
  await page.goto('/?mockPlaybackDurationMs=850,120000&mockStartDelayMs=10');
  const list = page.locator('.track-table');
  await page.getByRole('button', { name: 'Next page' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  await page.getByRole('button', { name: 'Next page' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 3' })).toBeVisible();
  await list.evaluate((element) => {
    element.scrollTop = 48 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  const selectedRow = page.getByRole('row', { name: /Song 249/ });
  await expect(selectedRow).toBeVisible();
  await selectedRow.click();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(selectedRow).toHaveAttribute('aria-current', 'true');

  await list.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event('scroll'));
  });
  await expect(player.getByRole('status')).toHaveText('Starting next song', { timeout: 5_000 });
  await expect(player.locator('.player-track-copy strong')).toHaveText('Song 250', {
    timeout: 5_000,
  });

  const nowPlayingRow = page.getByRole('row', { name: /Song 250/ });
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 3' })).toBeVisible();
  await expect(nowPlayingRow).toHaveCount(0);
  await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBe(0);
  await list.evaluate((element) => {
    element.scrollTop = 48 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  await expect(nowPlayingRow).toHaveAttribute('aria-current', 'true');
  await expect(nowPlayingRow).toHaveAttribute('aria-selected', 'false');
  await expect(nowPlayingRow).toBeInViewport();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
});

test('Now Playing does not pull a browsed search context back to the playback context', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();

  await page.getByLabel('Search Built-in Songs').fill('Song');
  const list = page.locator('.track-table');
  await expect(page.locator('.track-browser-count')).toHaveText('488 songs');
  await list.evaluate((element) => {
    element.scrollTop = 2800;
    element.dispatchEvent(new Event('scroll'));
  });
  const browsedScrollTop = await list.evaluate((element) => element.scrollTop);
  await player.getByRole('button', { name: 'Next' }).click();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run');
  await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBe(browsedScrollTop);
});

test('natural Auto Play continues its frozen context after browsing a search', async ({ page }) => {
  await page.goto('/?mockPlaybackDurationMs=2000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();

  const search = page.getByLabel('Search Built-in Songs');
  await search.fill('Moonlit');
  await expect(page.getByRole('row', { name: /Moonlit Village/ })).toBeVisible();

  await expect(player.getByRole('status')).toHaveText('Starting next song', { timeout: 10_000 });
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run', {
    timeout: 10_000,
  });
  await expect(search).toHaveValue('Moonlit');
  await expect(page.getByRole('row', { name: /Moonlit Village/ })).toBeVisible();
});

test('natural Auto Play keeps a newly browsed Library source during its handoff', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=3000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  const transition = player.getByRole('status');
  await expect(transition).toHaveText('Starting next song', { timeout: 10_000 });
  const handoffStartedAt = await page.evaluate(() => performance.now());

  const likedSongs = page.locator('.library-navigator').getByRole('button', {
    name: 'Liked Songs',
  });
  await likedSongs.click();
  await expect(page.getByRole('heading', { name: 'Liked Songs' })).toBeVisible();
  const browseElapsedMs = await page.evaluate(
    (startedAt) => performance.now() - startedAt,
    handoffStartedAt,
  );
  expect(browseElapsedMs).toBeLessThan(450);

  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run', {
    timeout: 10_000,
  });
  await expect(page.getByRole('heading', { name: 'Liked Songs' })).toBeVisible();
});

test('browsing and liking another Library row keeps Player Bar identity on Now Playing', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.locator('.player-track-status')).toHaveText('Playing');

  const selectedRow = page.getByRole('row', { name: /Candle Run/ });
  await selectedRow.click();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await player.getByRole('button', { name: 'Add to Liked Songs' }).click();
  await expect(player.getByRole('button', { name: 'Remove from Liked Songs' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Candle Run/ })).toBeVisible();

  await player.getByRole('button', { name: 'Open utility panel' }).click();
  const details = page.getByRole('tabpanel', { name: 'Details' });
  await expect(details.getByRole('heading', { name: 'Candle Run' })).toBeVisible();
  await expect(details.locator('.song-details-context')).toHaveCount(0);

  await player.getByRole('button', { name: 'Pause' }).click();
  await expect(player.getByRole('button', { name: 'Resume' })).toBeVisible();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  const stop = player.getByRole('button', { name: 'Stop', exact: true });
  await stop.click();
  await expect(stop).toBeDisabled();
  await expect(player.getByText('Last played', { exact: true })).toBeVisible();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await expect(details.getByRole('heading', { name: 'Candle Run' })).toBeVisible();
});

test('dry-run playback is labeled in the dock and explains that no physical input is sent', async ({
  page,
}) => {
  const viewports = [
    { width: 640, height: 448 },
    { width: 800, height: 560 },
    { width: 1366, height: 768 },
  ];
  await page.setViewportSize(viewports[0]!);
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Candle Run/ }).click();
  await player.getByRole('button', { name: 'Configure playback profile' }).click();

  const profile = page.getByRole('dialog', { name: 'Playback profile' });
  await expect(
    profile.getByText(
      'Tests the current profile with dry-run playback; no physical keyboard input is sent.',
    ),
  ).toBeVisible();
  await profile.getByRole('button', { name: 'Test playback (no input)' }).click();

  const playbackStatus = player.locator('.player-track-status');
  await expect(playbackStatus).toHaveText('Dry run · Playing');
  await expect(playbackStatus).toHaveAttribute('aria-label', /No physical input is sent/);
  await expect(playbackStatus).toHaveCount(1);
  const playbackStatusLayout = await playbackStatus.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    const style = window.getComputedStyle(element);
    return {
      rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      display: style.display,
      minWidth: style.minWidth,
      gridColumn: style.gridColumn,
      palette: document.documentElement.dataset.palette,
    };
  });
  if (!(await playbackStatus.isVisible())) {
    console.error(`Player Dock playback status layout: ${JSON.stringify(playbackStatusLayout)}`);
  }
  await expect(
    playbackStatus,
    `Player Dock playback status layout: ${JSON.stringify(playbackStatusLayout)}`,
  ).toBeVisible();

  for (const viewport of viewports) {
    await page.setViewportSize(viewport);
    await expect(playbackStatus).toHaveText('Dry run · Playing');
    expectPlayerBarGeometry(await readPlayerBarGeometry(page));
    const titleBox = await player.locator('.player-track-copy strong').boundingBox();
    const trackInfoBox = await player.locator('.player-track-info').boundingBox();
    const transportBox = await player.locator('.player-transport').boundingBox();
    expect(titleBox).not.toBeNull();
    expect(trackInfoBox).not.toBeNull();
    expect(transportBox).not.toBeNull();
    if (titleBox && trackInfoBox && transportBox) {
      expect(titleBox.x).toBeGreaterThanOrEqual(trackInfoBox.x);
      expect(titleBox.x + titleBox.width).toBeLessThanOrEqual(trackInfoBox.x + trackInfoBox.width);
      expect(trackInfoBox.x + trackInfoBox.width).toBeLessThanOrEqual(transportBox.x);
    }
    if (viewport.width <= 800) {
      const statusBox = await playbackStatus.boundingBox();
      expect(statusBox).not.toBeNull();
      if (statusBox && titleBox) expect(statusBox.y).toBeGreaterThan(titleBox.y);
    }
  }

  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run');
  await expect(page.getByRole('row', { name: /Blue Bird/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  const utilityToggle = player.getByRole('button', { name: /utility panel/ });
  if ((await utilityToggle.getAttribute('aria-expanded')) !== 'true') {
    await utilityToggle.click();
  }
  const details = page.getByRole('tabpanel', { name: 'Details' });
  await expect(details.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await expect(details.locator('.song-details-context')).toHaveCount(0);

  await expectNoSeriousAccessibilityViolations(page);

  await page.getByRole('tab', { name: 'Runtime' }).click();
  const diagnostics = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(diagnostics.getByText('Playback status: Playing', { exact: true })).toBeVisible();
  await expect(diagnostics.getByText('Now Playing: Candle Run', { exact: true })).toBeVisible();
  await expect(
    diagnostics.getByText('Mode: Dry run · no physical input sent', { exact: true }),
  ).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  const playerBox = await player.boundingBox();
  expect(playerBox).not.toBeNull();
  if (playerBox) {
    expect(playerBox.x).toBeGreaterThanOrEqual(0);
    expect(playerBox.x + playerBox.width).toBeLessThanOrEqual(viewports.at(-1)!.width + 1);
  }
  await player.getByRole('button', { name: 'Stop' }).click();
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
});

test('physical playback omits the dry-run indicator at responsive viewport sizes', async ({
  page,
}) => {
  for (const viewport of [
    { width: 640, height: 448 },
    { width: 800, height: 560 },
    { width: 1366, height: 768 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
    const player = await startSelectedSong(page);
    await expect(player.locator('.player-track-status')).toHaveText('Playing');
    expectPlayerBarGeometry(await readPlayerBarGeometry(page));
  }
});

test('Next waits for retirement and starts the next context song', async ({ page }) => {
  await page.goto('/?mockPlaybackDurationMs=5_000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();

  await player.getByRole('button', { name: 'Next' }).click();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run');
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(page.getByRole('alert', { name: 'Playback error' })).toHaveCount(0);
});

test('Shuffle changes Next traversal without repeating visited songs', async ({ page }) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const shuffle = player.getByRole('button', { name: 'Shuffle' });
  await shuffle.click();
  await expect(shuffle).toHaveAttribute('aria-pressed', 'true');
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();

  const visited = ['Blue Bird'];
  for (let count = 0; count < 4; count += 1) {
    const currentTitle = await player.locator('.player-track-copy strong').textContent();
    await player.getByRole('button', { name: 'Next' }).click();
    const title = player.locator('.player-track-copy strong');
    const proceed = page.getByRole('button', { name: 'Proceed with current settings' });
    const playing = player.getByText('Playing', { exact: true });
    let confirmationAccepted = false;
    await expect(async () => {
      if (!confirmationAccepted && (await proceed.isVisible())) {
        await proceed.click();
        confirmationAccepted = true;
      }
      await expect(title).not.toHaveText(currentTitle ?? '');
      await expect(playing).toBeVisible();
    }).toPass({ timeout: 10_000 });
    const titleText = (await title.textContent())?.trim();
    expect(titleText).toBeTruthy();
    if (!titleText) throw new Error('shuffled Now Playing title is missing');
    visited.push(titleText);
  }
  expect(new Set(visited).size).toBe(visited.length);
  expect(visited[1]).not.toBe('Candle Run');
});

test('Auto Play Off stays on the finished song, and a live On toggle advances it', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=700&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const autoPlay = player.getByRole('button', { name: 'Auto Play' });
  await expect(autoPlay).toHaveAttribute('aria-pressed', 'true');
  await autoPlay.click();
  await expect(autoPlay).toHaveAttribute('aria-pressed', 'false');
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  const profileDialog = page.getByRole('dialog', { name: 'Playback profile' });
  await expect(profileDialog.getByRole('switch', { name: 'Auto Play' })).toHaveCount(0);
  await page.keyboard.press('Escape');

  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible({
    timeout: 5_000,
  });
  await expect(player.locator('.player-track-copy strong')).toHaveText('Blue Bird');

  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await autoPlay.click();
  await expect(autoPlay).toHaveAttribute('aria-pressed', 'true');
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run', {
    timeout: 5_000,
  });
});

test('Auto Play dock toggle mirrors Settings and stays outside the compact profile', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 760 });
  await page.goto('/');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const dockAutoPlay = player.getByRole('button', { name: 'Auto Play' });
  await expect(dockAutoPlay).toHaveAttribute('aria-pressed', 'true');
  await dockAutoPlay.click();
  await expect(dockAutoPlay).toHaveAttribute('aria-pressed', 'false');

  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  const profileDialog = page.getByRole('dialog', { name: 'Playback profile' });
  await expect(profileDialog.getByRole('switch', { name: 'Auto Play' })).toHaveCount(0);
  const profileBox = await profileDialog.boundingBox();
  expect(profileBox).not.toBeNull();
  if (profileBox) expect(profileBox.width).toBeLessThanOrEqual(320);
  await page.keyboard.press('Escape');

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings.getByRole('switch', { name: 'Auto Play' })).toHaveAttribute(
    'aria-checked',
    'false',
  );
  await settings.getByRole('switch', { name: 'Auto Play' }).click();
  await expect(settings.getByRole('switch', { name: 'Auto Play' })).toHaveAttribute(
    'aria-checked',
    'true',
  );
  await expect(dockAutoPlay).toHaveAttribute('aria-pressed', 'true');
});

test('rapid Play and risk confirmation clicks create only one active session', async ({ page }) => {
  await page.goto('/?mockPlaybackDurationMs=5_000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).dblclick();
  const proceed = page.getByRole('button', { name: 'Proceed with current settings' });
  await expect(proceed).toBeVisible();
  await proceed.dblclick();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(player.getByRole('button', { name: 'Stop' })).toHaveCount(1);
  await expect(page.getByRole('alert', { name: 'Playback error' })).toHaveCount(0);
});

test('starting never exposes Pause and transport fits the supported narrow viewport', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/?mockPlaybackDurationMs=5_000&mockStartDelayMs=600');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(player.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
  await expect(player.getByRole('button', { name: 'Pause' })).toHaveCount(0);
  await expect(player.getByRole('button', { name: 'Stop' })).toBeEnabled();

  const dimensions = await page.evaluate(() => ({
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth: window.innerWidth,
  }));
  expect(dimensions.documentWidth).toBeLessThanOrEqual(dimensions.viewportWidth);
  await player.getByRole('button', { name: 'Stop' }).click();
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
});

test('target startup failures remain actionable at desktop and Chromium fallback sizes', async ({
  page,
}) => {
  const supportedDesktopViewports = [
    { width: 1200, height: 760 },
    { width: 1280, height: 720 },
    { width: 800, height: 560 },
  ];
  const chromiumResponsiveFallbackViewports = [{ width: 640, height: 448 }];
  for (const viewport of [...supportedDesktopViewports, ...chromiumResponsiveFallbackViewports]) {
    await page.setViewportSize(viewport);
    await page.goto('/?mockStartFailure=target_not_found');
    const player = await startSelectedSong(page);
    await expectRecoveryBannerWithinViewport(page);
    await expect(page.getByRole('alert')).toContainText('Sky window was not found');
    await expect(page.getByRole('alert')).toContainText(
      'Open Sky and make sure its window is visible',
    );
    await expect(player.getByRole('button', { name: 'Try again' })).toBeEnabled();
    await expect(player.getByRole('button', { name: 'Stop playback' })).toHaveCount(0);
    expectPlayerBarGeometry(await readPlayerBarGeometry(page));
  }
});

test('Player Bar keeps transport geometry stable through its lifecycle', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/?mockPlaybackDurationMs=120000');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const progress = page.getByRole('progressbar', { name: /Playback progress/ });
  await expect(progress).toBeVisible();
  await expect(progress).toHaveAttribute('value', '0');
  const idlePrimary = page.getByRole('button', { name: 'Play', exact: true });
  const idleBox = await idlePrimary.boundingBox();
  expect(idleBox).not.toBeNull();
  const idleGeometry = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(idleGeometry);

  await idlePrimary.click();
  const confirmation = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(confirmation).toBeVisible();
  const confirmationGeometry = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(confirmationGeometry);
  expectStablePlayerBar(idleGeometry, confirmationGeometry);
  await expect(player.getByRole('button', { name: 'Previous' })).toBeDisabled();
  await expect(player.getByRole('button', { name: 'Next' })).toBeDisabled();
  await expect(
    confirmation.getByRole('button', { name: 'Cancel playback confirmation' }),
  ).toBeEnabled();
  const transportStatus = player.locator('.player-transport-status');
  await expect(transportStatus).toHaveText('Awaiting confirmation');
  const playerBox = await player.boundingBox();
  const statusBox = await transportStatus.boundingBox();
  expect(playerBox).not.toBeNull();
  expect(statusBox).not.toBeNull();
  if (playerBox && statusBox) {
    expect(statusBox.y).toBeGreaterThanOrEqual(playerBox.y);
    expect(statusBox.y + statusBox.height).toBeLessThanOrEqual(playerBox.y + playerBox.height);
  }
  await expect(
    confirmation.getByRole('button', { name: 'Proceed with current settings' }),
  ).toBeFocused();
  await expect(
    confirmation.getByRole('button', { name: 'Test playback (no input)' }),
  ).toBeVisible();

  await confirmation.getByRole('button', { name: 'Cancel playback confirmation' }).click();
  await expect(page.getByRole('group', { name: 'Playback confirmation' })).toHaveCount(0);
  await idlePrimary.waitFor({ state: 'visible' });
  await idlePrimary.click();
  await expect(confirmation).toBeVisible();
  await confirmation.getByRole('button', { name: 'Proceed with current settings' }).click();

  const activePrimary = page.getByRole('button', { name: 'Pause' });
  await expect(activePrimary).toBeVisible();
  await expect(player.getByRole('button', { name: 'Previous' })).toBeDisabled();
  const activeBox = await activePrimary.boundingBox();
  expect(activeBox).not.toBeNull();
  if (idleBox && activeBox) {
    expect(
      Math.abs(idleBox.x + idleBox.width / 2 - (activeBox.x + activeBox.width / 2)),
    ).toBeLessThanOrEqual(1);
  }
  const activeGeometry = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(activeGeometry);
  expectStablePlayerBar(idleGeometry, activeGeometry);
  await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  await page.getByRole('button', { name: 'Pause' }).click();
  await expect(page.getByRole('button', { name: 'Resume' })).toBeVisible();
  const pausedGeometry = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(pausedGeometry);
  expectStablePlayerBar(idleGeometry, pausedGeometry);
  await page.getByRole('button', { name: 'Resume' }).click();
  await expect(page.getByRole('button', { name: 'Pause' })).toBeVisible();
  const resumedGeometry = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(resumedGeometry);
  expectStablePlayerBar(idleGeometry, resumedGeometry);
  await page.getByRole('button', { name: 'Stop' }).click();
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
});
