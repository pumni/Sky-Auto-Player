import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const workbenchStorageKey = 'sky.ui.workbench.v5';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => window.localStorage.clear());
  await page.reload();
});

async function expectNoSeriousAccessibilityViolations(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.filter(
      (violation) => violation.impact === 'critical' || violation.impact === 'serious',
    ),
  ).toEqual([]);
}

async function readWorkbenchGeometry(page: Page) {
  return page.evaluate(() => {
    const rectangle = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const box = element.getBoundingClientRect();
      return {
        x: box.x,
        y: box.y,
        right: box.right,
        bottom: box.bottom,
        width: box.width,
        height: box.height,
      };
    };
    const workbench = document.querySelector<HTMLElement>('.workbench');
    return {
      mode: workbench?.dataset.utilityPresentation ?? 'missing',
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        dpr: window.devicePixelRatio,
      },
      workbench: rectangle('.workbench'),
      navigator: rectangle('.navigator-workbench-pane'),
      track: rectangle('.track-browser-workbench-pane'),
      inspector: rectangle('.utility-workbench-pane'),
      player: rectangle('.player-bar'),
      document: {
        width: document.documentElement.scrollWidth,
        height: document.documentElement.scrollHeight,
      },
    };
  });
}

async function expectInspectorGeometry(page: Page, mode: 'docked' | 'overlay') {
  const geometry = await readWorkbenchGeometry(page);
  expect(geometry.mode).toBe(mode);
  expect(geometry.workbench).not.toBeNull();
  expect(geometry.navigator).not.toBeNull();
  expect(geometry.track).not.toBeNull();
  expect(geometry.inspector).not.toBeNull();
  expect(geometry.player).not.toBeNull();
  if (
    !geometry.workbench ||
    !geometry.navigator ||
    !geometry.track ||
    !geometry.inspector ||
    !geometry.player
  ) {
    throw new Error('Workbench geometry is incomplete');
  }

  expect(geometry.navigator.right).toBeLessThanOrEqual(geometry.track.x);
  expect(geometry.track.width).toBeGreaterThanOrEqual(420);
  expect(geometry.inspector.x).toBeGreaterThanOrEqual(geometry.workbench.x);
  expect(geometry.inspector.right).toBeLessThanOrEqual(geometry.workbench.right - 8);
  expect(geometry.inspector.bottom).toBeLessThanOrEqual(geometry.player.y);
  expect(geometry.document.width).toBeLessThanOrEqual(geometry.viewport.width);
  expect(geometry.document.height).toBeLessThanOrEqual(geometry.viewport.height);
  await expect(page.locator('.workbench')).toHaveAttribute('data-layout-fits', 'true');

  if (mode === 'docked') {
    expect(geometry.track.right).toBeLessThanOrEqual(geometry.inspector.x);
    expect(geometry.inspector.width).toBeGreaterThanOrEqual(280);
    await expect(page.getByRole('separator', { name: 'Resize utility pane' })).toBeVisible();
    await expect(page.locator('.utility-workbench-pane')).not.toHaveClass(/is-overlay/);
  } else {
    expect(geometry.inspector.width).toBeGreaterThanOrEqual(280);
    expect(geometry.inspector.width).toBeLessThanOrEqual(400);
    expect(geometry.inspector.x).toBeLessThan(geometry.track.right);
    await expect(page.getByRole('separator', { name: 'Resize utility pane' })).toHaveCount(0);
    await expect(page.locator('.utility-workbench-pane')).toHaveClass(/is-overlay/);
  }
  return geometry;
}

async function startMockPlayback(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page
    .getByRole('row', { name: /Blue Bird/ })
    .locator('.track-title-text')
    .click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  return player;
}

test('Inspector presentation preserves the workbench across the logical viewport matrix', async ({
  page,
}) => {
  for (const viewport of [
    { width: 1920, height: 1080 },
    { width: 1366, height: 768 },
    { width: 1024, height: 640 },
    { width: 900, height: 600 },
    { width: 800, height: 560 },
    { width: 780, height: 540 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await page.evaluate(() => window.localStorage.clear());
    await page.reload();

    const trigger = page.locator('.player-utility-button');
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    const mode = viewport.width < 788 ? 'overlay' : 'docked';
    await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', mode);
    const details = page.getByRole('region', { name: 'Utility: Song Details' });
    await expect(details).toBeVisible();
    await expectInspectorGeometry(page, mode);

    if ([640, 800, 1366].includes(viewport.width)) {
      await expectNoSeriousAccessibilityViolations(page);
    }

    await page.getByRole('tab', { name: 'Runtime' }).click();
    const diagnostics = page.getByRole('region', { name: 'Utility: Diagnostics' });
    await expect(diagnostics.locator('.diagnostics-view')).toBeVisible();
    await expectInspectorGeometry(page, mode);
    await page.getByRole('tab', { name: 'Details' }).click();
    await expect(details).toBeVisible();
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'closed');
  }
});

test('compact Inspector supports focus restoration, resize continuity, and outside row selection', async ({
  page,
}) => {
  await page.setViewportSize({ width: 640, height: 448 });
  await page.goto('/');
  const trigger = page.locator('.player-utility-button');
  const details = page.getByRole('region', { name: 'Utility: Song Details' });
  const grid = page.getByRole('grid', { name: 'Songs' });

  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  await expect(details.getByRole('tab', { name: 'Details' })).toBeFocused();
  await expect(details).not.toHaveAttribute('aria-modal');
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('region', { name: 'Utility: Diagnostics' })).toBeVisible();
  await page.keyboard.press('ArrowLeft');
  await expect(details).toBeVisible();
  const detailsTab = details.getByRole('tab', { name: 'Details' });
  await expect(detailsTab).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(details.locator('.song-details-view')).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(detailsTab).toBeFocused();

  const auroraTitle = page
    .getByRole('row', { name: /Aurora Landing/ })
    .locator('.track-title-text');
  const auroraTitleBox = await auroraTitle.boundingBox();
  const inspectorBox = await page.locator('.utility-workbench-pane').boundingBox();
  expect(auroraTitleBox).not.toBeNull();
  expect(inspectorBox).not.toBeNull();
  if (auroraTitleBox && inspectorBox) {
    expect(auroraTitleBox.x + auroraTitleBox.width / 2).toBeLessThan(inspectorBox.x);
  }
  await auroraTitle.click();
  await expect(page.getByRole('heading', { name: 'Aurora Landing' })).toBeVisible();
  await expect(details).toBeVisible();
  await expect(grid).toBeFocused();

  await page.setViewportSize({ width: 800, height: 560 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  await expect(grid).toBeFocused();
  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(grid).toBeFocused();

  await page.keyboard.press('Escape');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'closed');
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(trigger).toBeFocused();

  await trigger.click();
  await expect(details.getByRole('tab', { name: 'Details' })).toBeFocused();
  await details.getByRole('button', { name: 'Close utility' }).click();
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(trigger).toBeFocused();
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'closed');

  await trigger.click();
  await expect(details).toBeVisible();
  await trigger.click();
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'closed');
  await expect(trigger).toBeFocused();
});

test('active Runtime tab and focus survive docked-to-overlay viewport transitions', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const trigger = page.locator('.player-utility-button');
  await trigger.click();
  await page.getByRole('tab', { name: 'Runtime' }).click();
  const diagnostics = page.getByRole('region', { name: 'Utility: Diagnostics' });
  const runtimeTab = diagnostics.getByRole('tab', { name: 'Runtime' });
  await expect(diagnostics).toBeVisible();
  await expect(runtimeTab).toBeFocused();

  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(diagnostics).toBeVisible();
  await expect(runtimeTab).toBeFocused();
  await expectInspectorGeometry(page, 'overlay');

  await page.setViewportSize({ width: 1366, height: 768 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  await expect(diagnostics).toBeVisible();
  await expect(runtimeTab).toBeFocused();
  await expectInspectorGeometry(page, 'docked');

  await trigger.click();
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'closed');
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await trigger.click();
  const reopenedDiagnostics = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(reopenedDiagnostics).toBeVisible();
  await expect(reopenedDiagnostics.getByRole('tab', { name: 'Runtime' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(trigger).toBeFocused();

  await trigger.click();
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'closed');
  await expect(trigger).toBeFocused();
});

test('resizing never rewrites persisted widths or the user open preference', async ({ page }) => {
  const viewports = [
    { width: 640, height: 448 },
    { width: 800, height: 560 },
    { width: 1366, height: 768 },
    { width: 780, height: 540 },
  ];

  for (const utilityOpen of [false, true]) {
    const preferences = {
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 312,
      utilityWidth: 372,
      utilityOpen,
    };
    await page.evaluate(
      ({ key, value }) => window.localStorage.setItem(key, JSON.stringify(value)),
      { key: workbenchStorageKey, value: preferences },
    );
    await page.reload();
    const persistedBeforeResize = await page.evaluate(
      (key) => window.localStorage.getItem(key),
      workbenchStorageKey,
    );

    for (const viewport of viewports) {
      await page.setViewportSize(viewport);
      const expectedMode = utilityOpen ? (viewport.width < 788 ? 'overlay' : 'docked') : 'closed';
      await expect(page.locator('.workbench')).toHaveAttribute(
        'data-utility-presentation',
        expectedMode,
      );
      if (utilityOpen) {
        await expect(page.getByRole('region', { name: 'Utility: Song Details' })).toBeVisible();
      } else {
        await expect(page.locator('.utility-workbench-pane')).toHaveCount(0);
      }
      await expect
        .poll(() => page.evaluate((key) => window.localStorage.getItem(key), workbenchStorageKey))
        .toBe(persistedBeforeResize);
    }
  }
});

test('long Song Details content scrolls independently in the compact overlay', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 448 });
  await page.goto('/');
  const trackTable = page.locator('.track-table');
  await trackTable.evaluate((element) => {
    element.scrollTop = 495 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  const longTitle = /A sheet with an intentionally long title/;
  const longRow = page.getByRole('row', { name: longTitle });
  await expect(longRow).toBeVisible();
  await longRow.locator('.track-title-text').click();
  await page.locator('.player-utility-button').click();

  const details = page.getByRole('region', { name: 'Utility: Song Details' });
  await expect(details.getByRole('heading', { name: longTitle })).toBeVisible();
  await expect(
    details.getByText(/This intentionally long timing-risk explanation verifies wrapping/),
  ).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
  const detailsContent = details.locator('.song-details-view');
  const contentSize = await detailsContent.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
    trackScrollTop: document.querySelector<HTMLElement>('.track-table')?.scrollTop ?? 0,
  }));
  expect(contentSize.scrollWidth).toBeLessThanOrEqual(contentSize.clientWidth);
  expect(contentSize.scrollHeight).toBeGreaterThan(contentSize.clientHeight);
  await detailsContent.focus();
  await expect(detailsContent).toBeFocused();
  await page.keyboard.press('PageDown');
  await expect
    .poll(() => detailsContent.evaluate((element) => element.scrollTop), { timeout: 1_000 })
    .toBeGreaterThan(0);
  expect(await trackTable.evaluate((element) => element.scrollTop)).toBe(
    contentSize.trackScrollTop,
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(640);
  await expectInspectorGeometry(page, 'overlay');
});

test('compact Runtime retains live metrics, event history, trace export, and independent scrolling', async ({
  page,
}) => {
  await page.setViewportSize({ width: 640, height: 448 });
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const player = await startMockPlayback(page);
  const pause = player.getByRole('button', { name: 'Pause' });
  const resume = player.getByRole('button', { name: 'Resume' });
  for (let transition = 0; transition < 10; transition += 1) {
    await pause.click();
    await expect(resume).toBeVisible();
    await resume.click();
    await expect(pause).toBeVisible();
  }
  const playerBeforeInspector = await player.boundingBox();
  const trigger = page.locator('.player-utility-button');
  await trigger.click();
  const details = page.getByRole('region', { name: 'Utility: Song Details' });
  await expect(details.getByRole('tab', { name: 'Details' })).toBeFocused();
  await page.keyboard.press('ArrowRight');

  const diagnostics = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(diagnostics.getByRole('tab', { name: 'Performance' })).toBeVisible();
  const senderSamples = diagnostics.getByText('Sender samples', { exact: true }).locator('..');
  await expect
    .poll(async () => Number(await senderSamples.locator('strong').textContent()))
    .toBeGreaterThan(3);
  await expect(senderSamples).toContainText(/[1-9]\d*/);
  await expect(diagnostics.getByText('Completion p95', { exact: true })).toBeVisible();

  const runtimeView = diagnostics.locator('.diagnostics-view');
  const trackScrollTop = await page
    .locator('.track-table')
    .evaluate((element) => element.scrollTop);
  const performanceSize = await runtimeView.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }));
  expect(performanceSize.scrollHeight).toBeGreaterThan(performanceSize.clientHeight);
  await runtimeView.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  expect(await runtimeView.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  expect(await page.locator('.track-table').evaluate((element) => element.scrollTop)).toBe(
    trackScrollTop,
  );

  await runtimeView.evaluate((element) => {
    element.scrollTop = 0;
  });
  await diagnostics.getByRole('tab', { name: 'Performance' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(diagnostics.getByRole('img', { name: /SendInput pre-call lateness/ })).toBeVisible();
  await page.keyboard.press('ArrowRight');
  const eventPanel = diagnostics.locator('.diagnostics-scroll');
  await expect(
    eventPanel
      .locator('.diagnostics-lines strong')
      .filter({ hasText: 'playback.state_changed' })
      .first(),
  ).toBeVisible();
  const eventPanelSize = await eventPanel.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }));
  expect(eventPanelSize.scrollHeight).toBeGreaterThan(eventPanelSize.clientHeight);
  await eventPanel.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  expect(await eventPanel.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);

  await runtimeView.evaluate((element) => {
    element.scrollTop = 0;
  });
  const [traceDownload] = await Promise.all([
    page.waitForEvent('download'),
    diagnostics.getByRole('button', { name: 'Export last sender trace' }).click(),
  ]);
  expect(traceDownload.suggestedFilename()).toBe('sky-sender-trace.json');

  await expectNoSeriousAccessibilityViolations(page);
  const playerAfterInspector = await player.boundingBox();
  expect(playerAfterInspector).toEqual(playerBeforeInspector);
  await expectInspectorGeometry(page, 'overlay');

  await page.keyboard.press('Escape');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'closed');
  await expect(trigger).toBeFocused();
});

test('640 by 448 CSS logical pixels remain usable at 125 percent device scale', async ({
  browser,
  page,
}) => {
  const baseUrl = new URL('/', page.url()).toString();
  const context = await browser.newContext({
    viewport: { width: 640, height: 448 },
    deviceScaleFactor: 1.25,
  });
  try {
    const scaledPage = await context.newPage();
    await scaledPage.goto(baseUrl);
    await scaledPage.locator('.player-utility-button').click();
    const geometry = await expectInspectorGeometry(scaledPage, 'overlay');
    expect(geometry.viewport).toEqual({ width: 640, height: 448, dpr: 1.25 });
    await expect(scaledPage.getByRole('region', { name: 'Utility: Song Details' })).toBeVisible();
    await expectNoSeriousAccessibilityViolations(scaledPage);
  } finally {
    await context.close();
  }
});
