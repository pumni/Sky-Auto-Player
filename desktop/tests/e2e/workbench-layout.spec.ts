import { expect, test, type Locator } from '@playwright/test';
import { readFileSync } from 'node:fs';
import {
  expectedUtilityPresentation,
  expectNoSeriousAccessibilityViolations,
  expectWorkbenchWithinBounds,
  readPlayerBarGeometry,
  expectFiveTransportControls,
  expectPlayerBarGeometry,
  expectStablePlayerBar,
} from './testHelpers';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

test('minimum viewport keeps the workbench and Player Bar bounded', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Built-in Songs' })).toBeVisible();
  await expect(page.getByRole('contentinfo', { name: 'Player controls' })).toBeVisible();
  const navigatorPane = await page.locator('.navigator-workbench-pane').boundingBox();
  const trackPane = await page.locator('.track-browser-workbench-pane').boundingBox();
  const playerBar = await page.getByRole('contentinfo', { name: 'Player controls' }).boundingBox();
  expect(navigatorPane).not.toBeNull();
  expect(trackPane).not.toBeNull();
  expect(playerBar).not.toBeNull();
  if (navigatorPane && trackPane && playerBar) {
    const geometry = await page.evaluate(() => ({
      cssGutter: Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue('--workbench-gap'),
      ),
      solverGutter: Number.parseFloat(
        document.querySelector<HTMLElement>('.workbench')?.dataset.workbenchGap ?? 'NaN',
      ),
    }));
    expect(geometry.solverGutter).toBe(geometry.cssGutter);
    expect(Math.round(trackPane.x - (navigatorPane.x + navigatorPane.width))).toBe(
      geometry.cssGutter,
    );
    expect(Math.round(playerBar.y - (navigatorPane.y + navigatorPane.height))).toBe(
      geometry.cssGutter,
    );
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

test('configured Tauri minimum keeps Inspector docked in the Chromium UI contract check', async ({
  page,
}) => {
  const tauriConfig = JSON.parse(
    readFileSync(new URL('../../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
  ) as {
    app: {
      windows: Array<{ label: string; minWidth?: number; minHeight?: number }>;
    };
  };
  const mainWindow = tauriConfig.app.windows.find((window) => window.label === 'main');
  if (mainWindow?.minWidth === undefined || mainWindow.minHeight === undefined) {
    throw new Error('Tauri main window minimum dimensions are not configured');
  }
  const minimumViewport = { width: mainWindow.minWidth, height: mainWindow.minHeight };
  expect(minimumViewport).toEqual({ width: 800, height: 560 });

  // setViewportSize checks CSS layout against config; it does not exercise a native window or DPI.
  await page.setViewportSize(minimumViewport);
  await page.goto('/');
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.locator('.utility-workbench-pane')).toBeVisible();
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  await expect(page.getByRole('columnheader', { name: 'Notes' })).toBeHidden();
  await expect(page.getByRole('columnheader', { name: 'Liked' })).toBeVisible();
  await expect(page.getByRole('columnheader', { name: 'Duration' })).toBeVisible();
  const recommendationGrid = page.locator('.recommendation-grid');
  await expect(recommendationGrid).toBeVisible();
  await expect(recommendationGrid).toHaveCSS('grid-template-columns', /\S+px$/);
  await expectWorkbenchWithinBounds(page);
  const navigator = await page.locator('.navigator-workbench-pane').boundingBox();
  expect(navigator).not.toBeNull();
  if (navigator) expect(Math.round(navigator.width)).toBe(48);
});

test('Chromium fallback emulation uses overlay below the docked threshold', async ({ page }) => {
  // These widths are Chromium responsive simulations below Tauri's 800×560 minimum.
  for (const viewport of [
    { width: 763, height: 540 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await page.evaluate(() => window.localStorage.clear());
    await page.goto('/');
    await page.getByRole('button', { name: 'Open utility panel' }).click();
    await expect(page.locator('.utility-workbench-pane')).toBeVisible();
    await expect(page.locator('.workbench')).toHaveAttribute(
      'data-utility-presentation',
      expectedUtilityPresentation(viewport.width),
    );
    await expectWorkbenchWithinBounds(page);
  }
});

test('Chromium checks layout at Tauri-supported desktop dimensions', async ({ page }) => {
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 900, height: 600 },
    { width: 1024, height: 640 },
    { width: 1366, height: 768 },
    { width: 1200, height: 760 },
    { width: 1440, height: 900 },
    { width: 1920, height: 1080 },
  ]) {
    await page.setViewportSize(viewport);
    await page.evaluate(() => window.localStorage.clear());
    await page.reload();
    await expect(page.locator('.app-titlebar')).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
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
    await expect(page.locator('.workbench')).toHaveAttribute(
      'data-utility-presentation',
      expectedUtilityPresentation(viewport.width),
    );
    await expect(
      page.getByRole('region', { name: 'Utility: Song Details', exact: true }),
    ).toBeVisible();
    await expectWorkbenchWithinBounds(page);
    await page.getByRole('tab', { name: 'Runtime' }).click();
    await expectWorkbenchWithinBounds(page);
    await expect(
      page.getByRole('region', { name: 'Utility: Diagnostics', exact: true }),
    ).toBeVisible();
    await page.getByRole('tab', { name: 'Details' }).click();
    await expect(
      page.getByRole('region', { name: 'Utility: Song Details', exact: true }),
    ).toBeVisible();
  }
});

test('Titlebar Search uses the available center axis and Player controls stay viewport-centered', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto('/');
  await expect(page.locator('.app-titlebar')).toBeVisible();
  const viewportCenter = 1920 / 2;
  const titlebarFreeCenter = await page.evaluate(() => {
    const leading = document.querySelector<HTMLElement>('.app-titlebar-leading');
    const trailing = document.querySelector<HTMLElement>('.app-titlebar-end');
    if (!leading || !trailing) throw new Error('Titlebar occupied boundaries are unavailable');
    return (leading.getBoundingClientRect().right + trailing.getBoundingClientRect().left) / 2;
  });
  const searchBox = await page.getByLabel('Search Built-in Songs').boundingBox();
  const idleBox = await page.getByRole('button', { name: 'Play', exact: true }).boundingBox();
  expect(searchBox).not.toBeNull();
  expect(idleBox).not.toBeNull();
  if (searchBox && idleBox) {
    expect(Math.abs(searchBox.x + searchBox.width / 2 - titlebarFreeCenter)).toBeLessThanOrEqual(2);
    expect(Math.abs(idleBox.x + idleBox.width / 2 - viewportCenter)).toBeLessThanOrEqual(2);
  }

  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page.getByRole('button', { name: 'Play', exact: true }).click();
  const confirmation = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(confirmation).toBeVisible();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await expectFiveTransportControls(player, 'Play');
  expectPlayerBarGeometry(await readPlayerBarGeometry(page));
  await confirmation.getByRole('button', { name: 'Proceed with current settings' }).click();
  const activeBox = await page.getByRole('button', { name: 'Pause' }).boundingBox();
  expect(activeBox).not.toBeNull();
  if (activeBox) {
    expect(Math.abs(activeBox.x + activeBox.width / 2 - viewportCenter)).toBeLessThanOrEqual(2);
  }
  const activeGeometry = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(activeGeometry);

  for (const label of ['Shuffle', 'Previous', 'Next', 'Stop']) {
    const button = page.getByRole('button', { name: label, exact: true });
    const style = await button.evaluate((element) => {
      const computed = getComputedStyle(element);
      return {
        borderTopWidth: computed.borderTopWidth,
        backgroundColor: computed.backgroundColor,
      };
    });
    expect(style.borderTopWidth, `${label} must remain borderless`).toBe('0px');
    expect(style.backgroundColor, `${label} must remain visually unboxed`).toBe('rgba(0, 0, 0, 0)');
  }
  const nextBox = await page.getByRole('button', { name: 'Next', exact: true }).boundingBox();
  const stopBox = await page.getByRole('button', { name: 'Stop', exact: true }).boundingBox();
  expect(nextBox).not.toBeNull();
  expect(stopBox).not.toBeNull();
  if (nextBox && stopBox) {
    const stopGap = stopBox.x - (nextBox.x + nextBox.width);
    expect(stopGap).toBeGreaterThanOrEqual(0);
    expect(stopGap).toBeLessThanOrEqual(8);
  }
});

test('Player Bar geometry covers native desktop and Chromium fallback emulation', async ({
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
    await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=1200');
    const player = page.getByRole('contentinfo', { name: 'Player controls' });
    await page.getByRole('row', { name: /Blue Bird/ }).click();
    await expectFiveTransportControls(player, 'Play');

    const idle = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(idle);
    const shuffle = player.getByRole('button', { name: 'Shuffle' });
    await expect(shuffle).toHaveAttribute('aria-pressed', 'false');
    await shuffle.click();
    await expect(shuffle).toHaveAttribute('aria-pressed', 'true');
    const activeShuffleTone = await shuffle.evaluate((element) => {
      const token =
        document.documentElement.dataset.palette === 'graphite'
          ? '--workbench-toggle-on'
          : '--accent';
      const probe = document.createElement('span');
      probe.style.color = `var(${token})`;
      document.body.append(probe);
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    });
    await expect
      .poll(async () => shuffle.evaluate((element) => getComputedStyle(element).color), {
        timeout: 2_000,
      })
      .toBe(activeShuffleTone);
    const activeShuffleColor = await shuffle.evaluate((element) => getComputedStyle(element).color);
    expect(activeShuffleColor).toBe(activeShuffleTone);
    expectStablePlayerBar(idle, await readPlayerBarGeometry(page));

    await player.getByRole('button', { name: 'Play', exact: true }).click();
    await expect(player.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
    await expectFiveTransportControls(player, 'Starting playback');
    const starting = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(starting);
    expectStablePlayerBar(idle, starting);

    await expect(player.getByText('Playing', { exact: true })).toBeVisible();
    await expectFiveTransportControls(player, 'Pause');
    const playing = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(playing);
    expectStablePlayerBar(idle, playing);

    await player.getByRole('button', { name: 'Pause' }).click();
    await expect(player.getByRole('button', { name: 'Resume' })).toBeVisible();
    await expectFiveTransportControls(player, 'Resume');
    const paused = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(paused);
    expectStablePlayerBar(idle, paused);

    await player.getByRole('button', { name: 'Resume' }).click();
    await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
    await expectFiveTransportControls(player, 'Pause');
    const resumed = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(resumed);
    expectStablePlayerBar(idle, resumed);

    const toolsBox = await player.locator('.player-tools').boundingBox();
    const timelineBox = await player.locator('.player-timeline').boundingBox();
    expect(toolsBox).not.toBeNull();
    expect(timelineBox).not.toBeNull();
    if (toolsBox && timelineBox) {
      expect(timelineBox.x + timelineBox.width).toBeLessThanOrEqual(toolsBox.x);
    }
    await expect(
      player.getByRole('button', { name: 'Configure playback profile' }),
    ).toHaveAttribute('aria-label', 'Configure playback profile');
    await player.getByRole('button', { name: 'Stop' }).click();
    await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
    await expectFiveTransportControls(player, 'Play');
    await expect(player.getByRole('button', { name: 'Stop' })).toBeDisabled();
  }
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
