import { expect, test, type Page } from '@playwright/test';

type TrackHeaderDividerRule = {
  height: string;
  backgroundColor: string;
  left: number;
  right: number;
  width: number;
  headerWidth: number;
  paddingLeft: number;
  paddingRight: number;
};

async function openApp(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  if (page.url().startsWith('http')) await page.evaluate(() => window.localStorage.clear());
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
}

async function startDryRunWithSelection(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const playingRow = page.getByRole('row', { name: /Candle Run/ });
  await expect(playingRow).toBeVisible();
  await playingRow.click();
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await page
    .getByRole('dialog', { name: 'Playback profile' })
    .getByRole('button', { name: 'Test playback (no input)' })
    .click();
  await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
  await expect(player.locator('.player-track-status')).toHaveText('Dry run · Playing');

  const selectedRow = page.getByRole('row', { name: /Blue Bird/ });
  await selectedRow.click();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await expect(playingRow).toHaveClass(/is-playing/);
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run');
  return player;
}

async function openInspector(page: Page, tab: 'Details' | 'Runtime') {
  const trigger = page.getByRole('button', { name: 'Open utility panel' });
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
  if (tab === 'Runtime') await inspector.getByRole('tab', { name: 'Runtime' }).click();
  else await inspector.getByRole('tab', { name: 'Details' }).click();
  return inspector;
}

async function computedAppearance(page: Page, view: string) {
  return page.evaluate((viewName) => {
    const tokenNames = [
      '--workbench-content',
      '--workbench-navigation',
      '--workbench-inspector',
      '--chrome-surface',
      '--surface-raised',
      '--control-hover',
      '--control-pressed',
      '--workbench-control-rest',
      '--workbench-control-hover',
      '--workbench-control-pressed',
      '--workbench-control-disabled',
      '--workbench-control-danger-hover',
      '--selection-bg',
      '--border-subtle',
      '--border-strong',
      '--control-border',
      '--workbench-pane-edge',
      '--workbench-pane-radius',
      '--workbench-overlay-edge',
      '--workbench-overlay-radius',
      '--workbench-radius-xs',
      '--workbench-radius-sm',
      '--workbench-radius-md',
      '--workbench-radius-lg',
      '--workbench-radius-xl',
      '--workbench-radius-2xl',
      '--radius-control',
      '--radius-panel',
      '--radius-dialog',
      '--text-primary',
      '--text-secondary',
      '--text-tertiary',
      '--accent',
      '--accent-hover',
      '--on-accent',
      '--focus-ring',
    ];
    const selectors: Record<string, string> = {
      titlebar: '.app-titlebar',
      search: '.global-search',
      searchShortcut: '.global-search kbd',
      libraryRow: '.library-nav-item:not(.is-active)',
      trackRow: '.track-row',
      playerTool: '.player-auto-play-button',
      navigator: '.navigator-workbench-pane',
      navigatorHeading: '.library-navigator h2',
      navigatorItem: '.library-nav-item:not(.is-active) .library-nav-item-label',
      navigatorActiveItem: '.library-nav-item.is-active .library-nav-item-label',
      playlistHeading: '.library-section-heading',
      trackBrowser: '.track-browser-workbench-pane',
      pageHeading: '.track-browser-header h1',
      resultCount: '.track-browser-count',
      columnHeading: '.track-table-header',
      selectedRow: '.track-row.is-selected',
      selectedTitle: '.track-row.is-selected .track-title-text',
      playingRow: '.track-row.is-playing:not(.is-selected)',
      playingTitle: '.track-row.is-playing:not(.is-selected) .track-title-text',
      inspector: '.utility-workbench-pane',
      inspectorHeading: '.utility-header h2',
      inspectorTab: '.utility-view-tabs [role="tab"]',
      selectedSongHeading: '.song-details-heading h3',
      runtimeStatus: '.diagnostics-playback-summary strong',
      senderStatus: '.diagnostics-sender-summary strong',
      runtimeMetricLabel: '.diagnostics-metric span',
      runtimeMetricValue: '.diagnostics-metric strong',
      navigatorSectionDivider: '.library-secondary-sections',
      trackHeaderDivider: '.track-table-header',
      playerDock: '.player-bar',
      playerTrack: '.player-track-copy strong',
      playerStatus: '.player-track-copy .player-track-status',
      libraryCollapseButton: '.library-collapse-button',
    };
    const readStyle = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const style = getComputedStyle(element);
      return {
        color: style.color,
        backgroundColor: style.backgroundColor,
        fontSize: style.fontSize,
        fontWeight: style.fontWeight,
        letterSpacing: style.letterSpacing,
        height: style.height,
        borderBottomWidth: style.borderBottomWidth,
        borderTopWidth: style.borderTopWidth,
        borderRightWidth: style.borderRightWidth,
        borderLeftWidth: style.borderLeftWidth,
        borderTopColor: style.borderTopColor,
        borderRightColor: style.borderRightColor,
        borderBottomColor: style.borderBottomColor,
        borderLeftColor: style.borderLeftColor,
        borderRadius: style.borderRadius,
        boxShadow: style.boxShadow,
        boxSizing: style.boxSizing,
        outlineWidth: style.outlineWidth,
        outlineStyle: style.outlineStyle,
        outlineColor: style.outlineColor,
        outlineOffset: style.outlineOffset,
      };
    };
    const readBounds = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    const rootStyle = getComputedStyle(document.documentElement);
    const trackHeader = document.querySelector<HTMLElement>('.track-table-header');
    const trackHeaderRule = (() => {
      if (!trackHeader) return null;
      const headerStyle = getComputedStyle(trackHeader);
      const ruleStyle = getComputedStyle(trackHeader, '::after');
      return {
        height: ruleStyle.height,
        backgroundColor: ruleStyle.backgroundColor,
        left: Number.parseFloat(ruleStyle.left),
        right: Number.parseFloat(ruleStyle.right),
        width: Number.parseFloat(ruleStyle.width),
        headerWidth: trackHeader.getBoundingClientRect().width,
        paddingLeft: Number.parseFloat(headerStyle.paddingLeft),
        paddingRight: Number.parseFloat(headerStyle.paddingRight),
      };
    })();
    return {
      view: viewName,
      trackHeaderRule,
      viewport: { width: innerWidth, height: innerHeight },
      tokens: Object.fromEntries(
        tokenNames.map((name) => [name, rootStyle.getPropertyValue(name).trim()]),
      ),
      geometry: {
        workbench: readBounds('.workbench'),
        navigator: readBounds('.navigator-workbench-pane'),
        trackBrowser: readBounds('.track-browser-workbench-pane'),
        inspector: readBounds('.utility-workbench-pane'),
        separators: Array.from(document.querySelectorAll<HTMLElement>('.resizable-separator')).map(
          (element) => {
            const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
            return { x, y, width, height, right, bottom };
          },
        ),
      },
      elements: Object.fromEntries(
        Object.entries(selectors).map(([name, selector]) => [name, readStyle(selector)]),
      ),
    };
  }, view);
}

test('Graphite pane surfaces, compact overlay, and runtime hierarchy remain stable', async ({
  page,
}) => {
  test.setTimeout(120_000);
  const appearances: unknown[] = [];

  await openApp(page, 1366, 768);
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Library' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Collapse library navigator' })).toBeVisible();
  await startDryRunWithSelection(page);
  const details = await openInspector(page, 'Details');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  await expect(details.getByRole('heading', { name: 'Song Details' })).toBeVisible();
  const search = page.getByRole('searchbox', { name: 'Search All Songs' });
  const collapseButton = page.getByRole('button', { name: 'Collapse library navigator' });
  await page.mouse.move(0, 0);
  const idleCollapseStyle = await collapseButton.evaluate((button) => {
    const style = getComputedStyle(button);
    return {
      borderColor: style.borderColor,
      backgroundColor: style.backgroundColor,
      color: style.color,
      boxShadow: style.boxShadow,
    };
  });
  expect(idleCollapseStyle).toEqual({
    borderColor: 'rgba(0, 0, 0, 0)',
    backgroundColor: 'rgba(0, 0, 0, 0)',
    color: 'rgb(166, 166, 166)',
    boxShadow: 'none',
  });
  await collapseButton.hover();
  const hoverCollapseStyle = await collapseButton.evaluate((button) => {
    const style = getComputedStyle(button);
    return {
      borderColor: style.borderColor,
      backgroundColor: style.backgroundColor,
      color: style.color,
      boxShadow: style.boxShadow,
    };
  });
  expect(hoverCollapseStyle).toEqual({
    borderColor: 'rgba(0, 0, 0, 0)',
    backgroundColor: 'rgb(53, 54, 58)',
    color: 'rgb(212, 212, 212)',
    boxShadow: 'none',
  });
  await page.mouse.move(0, 0);
  await page.locator('.library-add-button').focus();
  await page.keyboard.press('Tab');
  await expect(collapseButton).toBeFocused();
  const focusCollapseStyle = await collapseButton.evaluate((button) => {
    const style = getComputedStyle(button);
    return {
      outlineWidth: style.outlineWidth,
      outlineStyle: style.outlineStyle,
      outlineColor: style.outlineColor,
      outlineOffset: style.outlineOffset,
      boxShadow: style.boxShadow,
    };
  });
  expect(focusCollapseStyle).toEqual({
    outlineWidth: '2px',
    outlineStyle: 'solid',
    outlineColor: 'rgb(178, 165, 233)',
    outlineOffset: '1px',
    boxShadow: 'none',
  });
  await page.keyboard.press('Enter');
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  await page.keyboard.press('Enter');
  await expect(page.locator('.navigator-workbench-pane')).not.toHaveClass(/is-collapsed/);
  await search.focus();
  await expect(search).toBeFocused();
  appearances.push(await computedAppearance(page, 'populated-docked'));
  const navigatorBorderHit = await page.evaluate(() => {
    const pane = document.querySelector<HTMLElement>('.navigator-workbench-pane');
    if (!pane) return false;
    const rect = pane.getBoundingClientRect();
    const target = document.elementFromPoint(rect.x + 0.5, rect.y + rect.height / 2);
    return target === pane || pane.contains(target);
  });
  expect(navigatorBorderHit).toBe(true);

  await openApp(page, 640, 448);
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
  const compactPlayer = await startDryRunWithSelection(page);
  const compactInspector = await openInspector(page, 'Details');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  const update = page.getByRole('button', { name: /Open update/ });
  await expect(update).toBeVisible();
  const compactSearch = page.getByRole('searchbox', { name: 'Search All Songs' });
  await compactSearch.evaluate((input: HTMLInputElement) => input.blur());
  await page.keyboard.press('/');
  await expect(compactSearch).toBeFocused();
  await expect
    .poll(() =>
      compactSearch.evaluate((input: HTMLInputElement) => input.matches(':focus-visible')),
    )
    .toBe(true);
  await expect(compactPlayer.locator('.player-track-status')).toHaveText('Dry run · Playing');
  await expect(compactInspector).toBeVisible();
  const compactInspectorBox = await compactInspector.boundingBox();
  expect(compactInspectorBox).not.toBeNull();
  if (!compactInspectorBox) throw new Error('Compact Inspector has no visible bounds');
  expect(compactInspectorBox.x).toBeGreaterThanOrEqual(0);
  expect(compactInspectorBox.y).toBeGreaterThanOrEqual(40);
  expect(compactInspectorBox.x + compactInspectorBox.width).toBeLessThanOrEqual(640);
  expect(compactInspectorBox.y + compactInspectorBox.height).toBeLessThanOrEqual(356);
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
    .toBeLessThanOrEqual(640);
  appearances.push(await computedAppearance(page, 'compact-overlay'));
  const overlayBorderHit = await page.evaluate(() => {
    const pane = document.querySelector<HTMLElement>('.utility-workbench-pane.is-overlay');
    if (!pane) return false;
    const rect = pane.getBoundingClientRect();
    const target = document.elementFromPoint(rect.x + 0.5, rect.y + rect.height / 2);
    return target === pane || pane.contains(target);
  });
  expect(overlayBorderHit).toBe(true);

  await openApp(page, 1366, 768);
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await startDryRunWithSelection(page);
  const runtime = await openInspector(page, 'Runtime');
  await expect(
    runtime.getByText('Mode: Dry run · no physical input sent', { exact: true }),
  ).toBeVisible();
  await expect(runtime.getByText('Sender availability', { exact: true })).toBeVisible();
  await expect(runtime.getByText('Sender samples', { exact: true })).toBeVisible();
  await expect(runtime.getByText('Sender backend', { exact: true })).toBeVisible();
  await expect(runtime.locator('.diagnostics-sender-summary')).toContainText('Not measured');
  appearances.push(await computedAppearance(page, 'runtime-dry-run'));
  {
    const populated = appearances[0] as {
      tokens: Record<string, string>;
      elements: Record<string, Record<string, string> | null>;
      geometry: Record<string, unknown>;
      trackHeaderRule: TrackHeaderDividerRule | null;
    };
    const compact = appearances[1] as {
      elements: Record<string, Record<string, string> | null>;
      geometry: Record<string, unknown>;
      trackHeaderRule: TrackHeaderDividerRule | null;
    };
    const runtimeAppearance = appearances[2] as {
      elements: Record<string, Record<string, string> | null>;
    };
    const tokens = populated.tokens;
    expect(tokens).toMatchObject({
      '--workbench-content': '#151618',
      '--workbench-navigation': '#191a1b',
      '--workbench-inspector': '#191a1b',
      '--chrome-surface': '#191a1b',
      '--surface-raised': '#242526',
      '--control-hover': '#222324',
      '--control-pressed': '#404246',
      '--workbench-control-rest': 'transparent',
      '--workbench-control-hover': '#35363a',
      '--workbench-control-pressed': '#404246',
      '--workbench-control-disabled': '#8b8c90',
      '--workbench-control-danger-hover': '#452d32',
      '--selection-bg': '#25242c',
      '--border-subtle': '#2a2b2c',
      '--border-strong': '#393b3e',
      '--control-border': '#72777d',
      '--workbench-pane-edge': '#313336',
      '--workbench-pane-radius': '10px',
      '--workbench-overlay-edge': '#72777d',
      '--workbench-overlay-radius': '12px',
      '--workbench-radius-xs': '4px',
      '--workbench-radius-sm': '6px',
      '--workbench-radius-md': '8px',
      '--workbench-radius-lg': '10px',
      '--workbench-radius-xl': '12px',
      '--workbench-radius-2xl': '14px',
      '--radius-control': '8px',
      '--radius-panel': '10px',
      '--radius-dialog': '14px',
      '--text-primary': '#d4d4d4',
      '--text-secondary': '#a6a6a6',
      '--text-tertiary': '#929292',
      '--accent': '#b2a5e9',
      '--accent-hover': '#c0b4f1',
      '--on-accent': '#ffffff',
      '--focus-ring': '#b2a5e9',
    });

    expect(populated.elements.titlebar?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(populated.elements.navigator?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(populated.elements.trackBrowser?.backgroundColor).toBe('rgb(21, 22, 24)');
    expect(populated.elements.inspector?.backgroundColor).toBe('rgb(25, 26, 27)');
    for (const paneName of ['navigator', 'trackBrowser', 'inspector']) {
      const pane = populated.elements[paneName];
      expect(pane, `${paneName} is present`).not.toBeNull();
      expect(pane).toMatchObject({
        borderTopWidth: '1px',
        borderRightWidth: '1px',
        borderBottomWidth: '1px',
        borderLeftWidth: '1px',
        borderTopColor: 'rgb(49, 51, 54)',
        borderRightColor: 'rgb(49, 51, 54)',
        borderBottomColor: 'rgb(49, 51, 54)',
        borderLeftColor: 'rgb(49, 51, 54)',
        borderRadius: '10px',
        boxSizing: 'border-box',
      });
      expect(pane?.boxShadow).toBe('none');
    }
    expect(populated.elements.libraryRow?.borderRadius).toBe('6px');
    expect(populated.elements.trackRow?.borderRadius).toBe('6px');
    expect(populated.elements.search?.borderRadius).toBe('8px');
    expect(populated.elements.searchShortcut?.borderRadius).toBe('4px');
    expect(populated.elements.playerTool?.borderRadius).toBe('8px');
    expect(populated.elements.inspectorTab?.borderRadius).toBe('0px');
    expect(populated.elements.navigatorSectionDivider).toMatchObject({
      borderTopWidth: '1px',
      borderTopColor: 'rgb(42, 43, 44)',
    });
    // The Graphite divider is inset within the header, not an edge-to-edge border.
    for (const appearance of [populated, compact]) {
      expect(appearance.elements.trackHeaderDivider?.borderBottomWidth).toBe('0px');
      const rule = appearance.trackHeaderRule;
      expect(rule).not.toBeNull();
      if (!rule) throw new Error('Graphite column-header divider is missing');
      expect(rule.height).toBe('1px');
      expect(rule.backgroundColor).toBe('rgb(42, 43, 44)');
      expect(rule.left).toBeCloseTo(rule.paddingLeft, 1);
      expect(rule.right).toBeCloseTo(rule.paddingRight, 1);
      expect(rule.left + rule.width + rule.right).toBeCloseTo(rule.headerWidth, 1);
    }
    const populatedGeometry = populated.geometry as {
      navigator: { x: number; right: number };
      trackBrowser: { x: number; right: number };
      inspector: { x: number; right: number; width: number };
      separators: Array<{ x: number; right: number; width: number }>;
    };
    expect(populatedGeometry.separators).toHaveLength(2);
    expect(populatedGeometry.separators.map(({ width }) => width)).toEqual([4, 4]);
    expect(populatedGeometry.separators[0].x).toBeCloseTo(populatedGeometry.navigator.right, 1);
    expect(populatedGeometry.separators[0].right).toBeCloseTo(populatedGeometry.trackBrowser.x, 1);
    expect(populatedGeometry.separators[1].x).toBeCloseTo(populatedGeometry.trackBrowser.right, 1);
    expect(populatedGeometry.separators[1].right).toBeCloseTo(populatedGeometry.inspector.x, 1);
    expect(populatedGeometry.inspector.x).toBeGreaterThanOrEqual(0);
    expect(populatedGeometry.inspector.width).toBeGreaterThanOrEqual(280);
    expect(populatedGeometry.inspector.right).toBeLessThanOrEqual(1366);
    expect(populated.elements.playerDock?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(populated.elements.selectedRow?.backgroundColor).toBe('rgb(37, 36, 44)');
    expect(populated.elements.playingRow?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(populated.elements.navigatorHeading).toMatchObject({
      fontSize: '13px',
      fontWeight: '600',
    });
    expect(populated.elements.navigatorItem).toMatchObject({ fontSize: '13px', fontWeight: '500' });
    expect(populated.elements.playlistHeading).toMatchObject({
      fontSize: '11px',
      fontWeight: '600',
    });
    expect(populated.elements.pageHeading).toMatchObject({ fontSize: '14px', fontWeight: '600' });
    expect(populated.elements.resultCount?.fontSize).toBe('12px');
    expect(populated.elements.columnHeading).toMatchObject({ fontSize: '11px', fontWeight: '600' });
    expect(populated.elements.selectedRow).toMatchObject({ fontSize: '13px', fontWeight: '600' });
    expect(populated.elements.selectedRow?.height).toBe('46px');
    expect(populated.elements.selectedTitle).toMatchObject({ fontSize: '13px', fontWeight: '600' });
    expect(populated.elements.playingTitle).toMatchObject({
      color: 'rgb(166, 166, 166)',
      fontSize: '13px',
      fontWeight: '500',
    });
    expect(populated.elements.inspectorHeading).toMatchObject({
      fontSize: '14px',
      fontWeight: '600',
    });
    expect(populated.elements.inspectorTab).toMatchObject({ fontSize: '12px', fontWeight: '500' });
    expect(populated.elements.selectedSongHeading).toMatchObject({
      fontSize: '14px',
      fontWeight: '600',
    });
    expect(populated.elements.playerTrack).toMatchObject({ fontSize: '14px', fontWeight: '600' });
    expect(populated.elements.playerDock?.height).toBe('92px');
    expect(populated.elements.playerStatus).toMatchObject({ fontSize: '12px', fontWeight: '600' });
    expect(compact.elements.search).toMatchObject({
      borderTopWidth: '1px',
      borderTopColor: 'rgb(61, 63, 67)',
      outlineWidth: '2px',
      outlineColor: 'rgb(150, 136, 198)',
    });
    expect(compact.elements.pageHeading).toMatchObject({ fontSize: '13px', fontWeight: '600' });
    expect(compact.elements.resultCount?.fontSize).toBe('11px');
    expect(compact.elements.inspector).toMatchObject({
      borderTopWidth: '1px',
      borderRightWidth: '1px',
      borderBottomWidth: '1px',
      borderLeftWidth: '1px',
      borderTopColor: 'rgb(114, 119, 125)',
      borderRightColor: 'rgb(114, 119, 125)',
      borderBottomColor: 'rgb(114, 119, 125)',
      borderLeftColor: 'rgb(114, 119, 125)',
      borderRadius: '12px',
      boxSizing: 'border-box',
    });
    const compactGeometry = compact.geometry as {
      workbench: { bottom: number };
      inspector: {
        x: number;
        y: number;
        width: number;
        height: number;
        right: number;
        bottom: number;
      };
    };
    expect(compactGeometry.inspector.width).toBe(288);
    expect(compactGeometry.inspector.x).toBeGreaterThanOrEqual(0);
    expect(compactGeometry.inspector.y).toBeGreaterThanOrEqual(40);
    expect(compactGeometry.inspector.right).toBeLessThanOrEqual(640);
    expect(compactGeometry.inspector.bottom).toBeLessThanOrEqual(compactGeometry.workbench.bottom);
    expect(runtimeAppearance.elements.runtimeStatus).toMatchObject({
      fontSize: '13px',
      fontWeight: '600',
    });
    expect(runtimeAppearance.elements.senderStatus).toMatchObject({
      fontSize: '12px',
      fontWeight: '600',
    });
    expect(runtimeAppearance.elements.runtimeMetricLabel?.fontSize).toBe('12px');
    expect(runtimeAppearance.elements.runtimeMetricValue).toMatchObject({
      fontSize: '12px',
      fontWeight: '600',
    });
  }
});
