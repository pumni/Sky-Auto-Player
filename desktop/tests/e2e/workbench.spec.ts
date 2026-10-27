import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { getUtilityPresentationMode } from '../../src/components/workbench/useWorkbenchLayout';

const workbenchStorageKey = 'sky.ui.workbench.v5';

type InspectorHeaderElementGeometry = {
  x: number;
  right: number;
  y: number;
  height: number;
  width: number;
  alignSelf: string;
  display: string;
  widthStyle: string;
  minWidth: string;
  boxSizing: string;
  flex: string;
  gridTemplateColumns: string;
  paddingInline: string;
};

type InspectorHeaderGeometry = Record<string, InspectorHeaderElementGeometry | null>;

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
      gutter: Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue('--workbench-gap'),
      ),
      solverGutter: Number.parseFloat(workbench?.dataset.workbenchGap ?? 'NaN'),
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

async function readInspectorHeaderGeometry(page: Page) {
  return page.evaluate(() => {
    const selectors = [
      '.utility-workbench-pane',
      '.utility-surface',
      '.utility-tabs',
      '.utility-header',
      '.utility-header-title',
      '.utility-close-button',
      '.utility-close-button svg',
      '.utility-view-tabs',
      '.utility-view-tabs [role="tab"]:first-child',
      '.utility-view-tabs [role="tab"]:last-child',
      '.utility-content',
      '.utility-empty p',
      '.song-details-heading h3',
    ];
    return Object.fromEntries(
      selectors.map((selector) => {
        const element = document.querySelector<HTMLElement>(selector);
        if (!element) return [selector, null];
        const rect = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        return [
          selector,
          {
            x: rect.x,
            right: rect.right,
            y: rect.y,
            height: rect.height,
            width: rect.width,
            alignSelf: style.alignSelf,
            display: style.display,
            widthStyle: style.width,
            minWidth: style.minWidth,
            boxSizing: style.boxSizing,
            flex: style.flex,
            gridTemplateColumns: style.gridTemplateColumns,
            paddingInline: `${style.paddingLeft} ${style.paddingRight}`,
          },
        ];
      }),
    ) as InspectorHeaderGeometry;
  });
}

async function readInspectorTabTextAlignment(page: Page) {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll<HTMLElement>('.utility-view-tabs [role="tab"]')).map(
      (tab) => {
        const range = document.createRange();
        range.selectNodeContents(tab);
        const text = range.getBoundingClientRect();
        const button = tab.getBoundingClientRect();
        return {
          xOffset: text.x + text.width / 2 - (button.x + button.width / 2),
          yOffset: text.y + text.height / 2 - (button.y + button.height / 2),
        };
      },
    ),
  );
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

  expect(geometry.solverGutter).toBe(geometry.gutter);

  expect(geometry.navigator.right).toBeLessThanOrEqual(geometry.track.x);
  expect(geometry.track.width).toBeGreaterThanOrEqual(420);
  expect(geometry.inspector.x).toBeGreaterThanOrEqual(geometry.workbench.x);
  expect(geometry.inspector.right).toBeLessThanOrEqual(geometry.workbench.right - geometry.gutter);
  expect(geometry.inspector.bottom).toBeLessThanOrEqual(
    geometry.workbench.bottom - geometry.gutter,
  );
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
    const mode = getUtilityPresentationMode(viewport.width, true, 'graphite');
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

test('Inspector header geometry stays stable across empty, selected, cleared, and Runtime states', async ({
  page,
}) => {
  for (const viewport of [
    { width: 1366, height: 768 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/');
    await page.evaluate(() => window.localStorage.clear());
    await page.reload();
    const trigger = page.locator('.player-utility-button');
    await trigger.click();
    const inspector = page.locator('#utility-inspector');
    await expect(inspector).toBeVisible();
    await inspector.getByRole('tab', { name: 'Details' }).click();
    await expect(page.getByText('Select a song to view details.')).toBeVisible();
    const presentation = getUtilityPresentationMode(viewport.width, true, 'graphite');
    await expectInspectorGeometry(page, presentation);
    await page.mouse.move(0, 0);

    const measurements: Partial<
      Record<'empty' | 'selected' | 'cleared' | 'runtime', InspectorHeaderGeometry>
    > = {};
    const tabTextAlignment: Partial<
      Record<
        'empty' | 'selected' | 'cleared' | 'runtime',
        Awaited<ReturnType<typeof readInspectorTabTextAlignment>>
      >
    > = {};
    const record = async (state: 'empty' | 'selected' | 'cleared' | 'runtime') => {
      measurements[state] = await readInspectorHeaderGeometry(page);
      tabTextAlignment[state] = await readInspectorTabTextAlignment(page);
    };
    await record('empty');
    if (viewport.width === 1366) {
      await page.screenshot({
        path: 'test-results/final-polish-k-inspector-empty-1366x768.png',
        animations: 'disabled',
      });
    } else {
      await page.screenshot({
        path: 'test-results/frozen-spec-inspector-compact-640x448.png',
        animations: 'disabled',
      });
    }

    const selectedRow = page.getByRole('row', { name: /Blue Bird/ });
    await selectedRow.locator('.track-title-text').click();
    await expect(inspector.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
    await expectInspectorGeometry(page, presentation);
    await record('selected');
    if (viewport.width === 1366) {
      await page.screenshot({
        path: 'test-results/final-polish-k-inspector-selected-1366x768.png',
        animations: 'disabled',
      });
    }

    await page.getByRole('searchbox', { name: 'Search All Songs' }).fill('Moonlit');
    await expect(page.getByText('Select a song to view details.')).toBeVisible();
    await expectInspectorGeometry(page, presentation);
    await record('cleared');

    await inspector.getByRole('tab', { name: 'Runtime' }).click();
    await expect(page.getByRole('region', { name: 'Utility: Diagnostics' })).toBeVisible();
    await expectInspectorGeometry(page, presentation);
    await record('runtime');

    const geometryFor = (state: 'empty' | 'selected' | 'cleared' | 'runtime', selector: string) => {
      const geometry = measurements[state]?.[selector];
      if (!geometry) throw new Error(`${selector} missing in ${state} Inspector state`);
      return geometry;
    };
    const baselineHeader = geometryFor('empty', '.utility-header');
    const baselineTabs = geometryFor('empty', '.utility-view-tabs');
    const baselineTitle = geometryFor('empty', '.utility-header-title');
    const baselineClose = geometryFor('empty', '.utility-close-button');
    const baselineCloseIcon = geometryFor('empty', '.utility-close-button svg');
    const baselineSurface = geometryFor('empty', '.utility-surface');
    const baselineDetailsTab = geometryFor('empty', '.utility-view-tabs [role="tab"]:first-child');
    const baselineRuntimeTab = geometryFor('empty', '.utility-view-tabs [role="tab"]:last-child');
    const expectOriginalTabs = (state: 'empty' | 'selected' | 'cleared' | 'runtime') => {
      const tabs = geometryFor(state, '.utility-view-tabs');
      const details = geometryFor(state, '.utility-view-tabs [role="tab"]:first-child');
      const runtime = geometryFor(state, '.utility-view-tabs [role="tab"]:last-child');
      expect(
        Math.abs(details.x - tabs.x),
        `${state} first tab starts at the tablist left edge`,
      ).toBeLessThanOrEqual(1);
      expect(details.x).toBeLessThan(runtime.x);
      expect(details.flex).toBe('0 0 auto');
      expect(runtime.flex).toBe('0 0 auto');
      expect(Math.abs(details.width - baselineDetailsTab.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(runtime.width - baselineRuntimeTab.width)).toBeLessThanOrEqual(1);
      expect(runtime.right).toBeLessThanOrEqual(tabs.right);
      expect(Math.abs(details.y - runtime.y)).toBeLessThanOrEqual(1);
      const textAlignment = tabTextAlignment[state];
      expect(textAlignment).toHaveLength(2);
      for (const alignment of textAlignment ?? []) {
        expect(Math.abs(alignment.xOffset)).toBeLessThanOrEqual(1.5);
        expect(
          Math.abs(alignment.yOffset),
          `${state} tab text vertical-center offset ${alignment.yOffset}`,
        ).toBeLessThanOrEqual(1.5);
      }
    };
    await expect(inspector.getByRole('tab')).toHaveText(['Details', 'Runtime']);
    expectOriginalTabs('empty');
    for (const state of ['selected', 'cleared', 'runtime'] as const) {
      const header = geometryFor(state, '.utility-header');
      const tabs = geometryFor(state, '.utility-view-tabs');
      const headerTitle = geometryFor(state, '.utility-header-title');
      const close = geometryFor(state, '.utility-close-button');
      const closeIcon = geometryFor(state, '.utility-close-button svg');
      const surface = geometryFor(state, '.utility-surface');
      expect(Math.abs(header.x - baselineHeader.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(header.y - baselineHeader.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(header.width - baselineHeader.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(header.height - baselineHeader.height)).toBeLessThanOrEqual(1);
      expect(Math.abs(tabs.x - baselineTabs.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(tabs.y - baselineTabs.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(tabs.height - baselineTabs.height)).toBeLessThanOrEqual(1);
      expect(Math.abs(tabs.width - baselineTabs.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(headerTitle.x - baselineTitle.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(headerTitle.width - baselineTitle.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(close.x - baselineClose.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(close.y - baselineClose.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(closeIcon.x - baselineCloseIcon.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(closeIcon.y - baselineCloseIcon.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(close.width - 32)).toBeLessThanOrEqual(0.1);
      expect(Math.abs(close.height - 32)).toBeLessThanOrEqual(0.1);
      expect(Math.abs(surface.width - baselineSurface.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(close.right - (header.right - 14))).toBeLessThanOrEqual(1);
      expect(Math.abs(tabs.x - (header.x + 14))).toBeLessThanOrEqual(1);
      expect(Math.abs(tabs.right - (header.right - 14))).toBeLessThanOrEqual(1);
      expectOriginalTabs(state);
    }
    expect(
      Math.abs(
        geometryFor('empty', '.utility-empty p').y -
          geometryFor('selected', '.song-details-heading h3').y,
      ),
    ).toBeLessThanOrEqual(1);
    const widthOverflow = await page.evaluate(() =>
      [
        '.utility-surface',
        '.utility-tabs',
        '.utility-header',
        '.utility-view-tabs',
        '.utility-content',
      ].map((selector) => {
        const element = document.querySelector<HTMLElement>(selector);
        return element ? element.scrollWidth <= element.clientWidth + 1 : false;
      }),
    );
    expect(widthOverflow.every(Boolean)).toBe(true);

    const measuredSelectors = [
      '.utility-workbench-pane',
      '.utility-surface',
      '.utility-tabs',
      '.utility-header',
      '.utility-header-title',
      '.utility-close-button',
      '.utility-close-button svg',
      '.utility-view-tabs',
      '.utility-content',
    ];
    const measuredBounds = Object.fromEntries(
      Object.entries(measurements).map(([state, elements]) => [
        state,
        Object.fromEntries(
          measuredSelectors.map((selector) => {
            const element = elements?.[selector];
            return [
              selector,
              element && {
                x: element.x,
                right: element.right,
                y: element.y,
                height: element.height,
                width: element.width,
              },
            ];
          }),
        ),
      ]),
    );
    const computedStyles = Object.fromEntries(
      measuredSelectors.map((selector) => {
        const element = measurements.empty?.[selector];
        return [
          selector,
          element && {
            alignSelf: element.alignSelf,
            display: element.display,
            width: element.widthStyle,
            minWidth: element.minWidth,
            boxSizing: element.boxSizing,
            flex: element.flex,
            gridTemplateColumns: element.gridTemplateColumns,
            paddingInline: element.paddingInline,
          },
        ];
      }),
    );
    console.info(
      `INSPECTOR_METRICS ${viewport.width}x${viewport.height} ${JSON.stringify({ measuredBounds, computedStyles })}`,
    );

    if (presentation === 'overlay') {
      await expect(inspector).not.toHaveAttribute('aria-modal');
      await expect(page.getByRole('row', { name: /Moonlit Village/ })).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(page.locator('.workbench')).toHaveAttribute(
        'data-utility-presentation',
        'closed',
      );
      await expect(trigger).toBeFocused();
      await trigger.click();
      await expect(inspector).toBeVisible();
      await inspector.getByRole('button', { name: 'Close utility' }).click();
      await expect(trigger).toBeFocused();
      await expect(page.locator('.workbench')).toHaveAttribute(
        'data-utility-presentation',
        'closed',
      );
    }
  }
});

test('compact Player Dock presents one readable dry-run status without moving transport controls', async ({
  page,
}) => {
  await page.setViewportSize({ width: 640, height: 448 });
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  const title = player.locator('.player-track-copy strong');
  const status = player.locator('.player-track-status');
  await expect(title).toHaveText('Blue Bird');
  await expect(status).toHaveText('Playing');
  await expect(status).toHaveCount(1);
  await expect(player.getByText('Now Playing', { exact: true })).toHaveCount(0);

  await page.getByRole('row', { name: /Candle Run/ }).click();
  await expect(title).toHaveText('Blue Bird');
  await expect(status).toHaveText('Playing');
  await expect(page.getByRole('row', { name: /Candle Run/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await player.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible();

  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await page
    .getByRole('dialog', { name: 'Playback profile' })
    .getByRole('button', { name: 'Test playback (no input)' })
    .click();

  await expect(title).toHaveText('Blue Bird');
  await expect(status).toHaveText('Dry run · Playing');
  await expect(status).toHaveAttribute('aria-label', /No physical input is sent/);
  await expect(status).toHaveCount(1);
  await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
  await expect(player.getByRole('button', { name: 'Add to Liked Songs' })).toBeVisible();
  await expect(player.getByRole('button', { name: 'Auto Play' })).toBeVisible();
  await expect(player.getByRole('button', { name: 'Configure playback profile' })).toBeVisible();
  await expect(player.getByRole('button', { name: 'Open utility panel' })).toBeVisible();

  const geometry = await page.evaluate(() => {
    const getRect = (selector: string) => {
      const rect = document.querySelector(selector)?.getBoundingClientRect();
      return rect
        ? { x: rect.x, y: rect.y, right: rect.right, width: rect.width, height: rect.height }
        : null;
    };
    const statusElement = document.querySelector<HTMLElement>('.player-track-status');
    return {
      dock: getRect('.player-bar'),
      title: getRect('.player-track-copy strong'),
      status: getRect('.player-track-status'),
      transport: getRect('.player-transport'),
      statusMetrics: statusElement
        ? {
            display: getComputedStyle(statusElement).display,
            whiteSpace: getComputedStyle(statusElement).whiteSpace,
            clientHeight: statusElement.clientHeight,
            scrollHeight: statusElement.scrollHeight,
            clientWidth: statusElement.clientWidth,
            scrollWidth: statusElement.scrollWidth,
          }
        : null,
      primary: getRect('.player-transport .player-primary-action'),
    };
  });
  expect(geometry.dock?.height).toBe(92);
  expect(geometry.title).not.toBeNull();
  expect(geometry.status).not.toBeNull();
  expect(geometry.transport).not.toBeNull();
  expect(geometry.primary).not.toBeNull();
  if (geometry.title && geometry.status && geometry.transport && geometry.primary) {
    expect(geometry.status.y).toBeGreaterThan(geometry.title.y);
    expect(geometry.status.y + geometry.status.height).toBeLessThanOrEqual(
      geometry.dock!.y + geometry.dock!.height,
    );
    expect(geometry.status.right).toBeLessThanOrEqual(geometry.transport.x);
    expect(geometry.primary.x + geometry.primary.width / 2).toBeCloseTo(
      geometry.transport.x + geometry.transport.width / 2,
      0,
    );
  }
  expect(geometry.statusMetrics?.display).not.toBe('grid');
  expect(geometry.statusMetrics?.whiteSpace).toBe('nowrap');
  expect(geometry.statusMetrics?.scrollHeight).toBeLessThanOrEqual(
    (geometry.statusMetrics?.clientHeight ?? 0) + 1,
  );
  expect(geometry.statusMetrics?.scrollWidth).toBeLessThanOrEqual(
    (geometry.statusMetrics?.clientWidth ?? 0) + 1,
  );
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
      const expectedMode = utilityOpen
        ? getUtilityPresentationMode(viewport.width, true, 'graphite')
        : 'closed';
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

test('Graphite overlay width is responsive and preserves docked width preferences', async ({
  page,
}) => {
  for (const utilityWidth of [280, 320, 380]) {
    await page.setViewportSize({ width: 1366, height: 768 });
    const preference = {
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 240,
      utilityWidth,
      utilityOpen: true,
    };
    await page.evaluate(
      ({ key, value }) => window.localStorage.setItem(key, JSON.stringify(value)),
      { key: workbenchStorageKey, value: preference },
    );
    await page.reload();
    await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
    const dockedPane = page.locator('.utility-workbench-pane');
    await expect(dockedPane).toBeVisible();
    await expect
      .poll(() => dockedPane.evaluate((element) => element.getBoundingClientRect().width))
      .toBe(utilityWidth);
    const preferenceBeforeOverlay = await page.evaluate(
      (key) => window.localStorage.getItem(key),
      workbenchStorageKey,
    );

    await page.setViewportSize({ width: 640, height: 448 });
    await expect(page.locator('.workbench')).toHaveAttribute(
      'data-utility-presentation',
      'overlay',
    );
    await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '640');
    await expect
      .poll(() => dockedPane.evaluate((element) => element.getBoundingClientRect().width))
      .toBe(288);
    await expect(page.getByRole('region', { name: 'Utility: Song Details' })).toBeVisible();
    await expect
      .poll(() => page.evaluate((key) => window.localStorage.getItem(key), workbenchStorageKey))
      .toBe(preferenceBeforeOverlay);

    await page.setViewportSize({ width: 1366, height: 768 });
    await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
    await expect
      .poll(() => dockedPane.evaluate((element) => element.getBoundingClientRect().width))
      .toBe(utilityWidth);
    await expect
      .poll(() => page.evaluate((key) => window.localStorage.getItem(key), workbenchStorageKey))
      .toBe(preferenceBeforeOverlay);
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
