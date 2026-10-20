import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const variant = process.env.GRAPHITE_CORRECTION_VARIANT;
const evidenceDirectory = resolve(
  process.cwd(),
  '../docs/evidence/graphite-correction',
  variant ?? 'unselected',
);

test.skip(
  variant !== 'pr-current' &&
    variant !== 'candidate' &&
    variant !== 'checkpoint-current' &&
    variant !== 'border-restored',
  'Set GRAPHITE_CORRECTION_VARIANT to capture a named Graphite correction checkpoint.',
);

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
  await expect(player.locator('.player-dry-run-label')).toBeVisible();

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

async function capture(page: Page, name: string) {
  mkdirSync(evidenceDirectory, { recursive: true });
  await page.screenshot({
    path: resolve(evidenceDirectory, `${name}.png`),
    animations: 'disabled',
  });
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
      '--selection-bg',
      '--border-subtle',
      '--border-strong',
      '--control-border',
      '--workbench-pane-edge',
      '--workbench-pane-radius',
      '--workbench-overlay-edge',
      '--workbench-overlay-radius',
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
      playerMode: '.player-track-copy .player-dry-run-label',
      playerStatus: '.player-track-copy .player-track-status',
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
        outlineColor: style.outlineColor,
      };
    };
    const readBounds = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    const rootStyle = getComputedStyle(document.documentElement);
    return {
      view: viewName,
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

test('captures the three matched Graphite visual correction states', async ({ page }) => {
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
  await search.focus();
  await expect(search).toBeFocused();
  appearances.push(await computedAppearance(page, 'populated-docked'));
  if (variant === 'border-restored') {
    const navigatorBorderHit = await page.evaluate(() => {
      const pane = document.querySelector<HTMLElement>('.navigator-workbench-pane');
      if (!pane) return false;
      const rect = pane.getBoundingClientRect();
      const target = document.elementFromPoint(rect.x + 0.5, rect.y + rect.height / 2);
      return target === pane || pane.contains(target);
    });
    expect(navigatorBorderHit).toBe(true);
  }
  await capture(page, 'populated-1366x768');

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
  await expect(compactPlayer.locator('.player-dry-run-label')).toHaveText('Dry run');
  await expect(compactInspector).toBeVisible();
  const compactInspectorBox = await compactInspector.boundingBox();
  expect(compactInspectorBox).not.toBeNull();
  if (!compactInspectorBox) throw new Error('Compact Inspector has no visible bounds');
  expect(compactInspectorBox.x).toBeGreaterThanOrEqual(0);
  expect(compactInspectorBox.y).toBeGreaterThanOrEqual(48);
  expect(compactInspectorBox.x + compactInspectorBox.width).toBeLessThanOrEqual(640);
  expect(compactInspectorBox.y + compactInspectorBox.height).toBeLessThanOrEqual(356);
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
    .toBeLessThanOrEqual(640);
  appearances.push(await computedAppearance(page, 'compact-overlay'));
  if (variant === 'border-restored') {
    const overlayBorderHit = await page.evaluate(() => {
      const pane = document.querySelector<HTMLElement>('.utility-workbench-pane.is-overlay');
      if (!pane) return false;
      const rect = pane.getBoundingClientRect();
      const target = document.elementFromPoint(rect.x + 0.5, rect.y + rect.height / 2);
      return target === pane || pane.contains(target);
    });
    expect(overlayBorderHit).toBe(true);
  }
  await capture(page, 'compact-640x448');

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
  await capture(page, 'runtime-dry-run-1366x768');

  mkdirSync(evidenceDirectory, { recursive: true });
  writeFileSync(
    resolve(evidenceDirectory, 'computed-styles.json'),
    `${JSON.stringify({ variant, appearances }, null, 2)}\n`,
    'utf8',
  );

  if (variant === 'candidate' || variant === 'border-restored') {
    const populated = appearances[0] as {
      tokens: Record<string, string>;
      elements: Record<string, Record<string, string> | null>;
      geometry: Record<string, unknown>;
    };
    const compact = appearances[1] as {
      elements: Record<string, Record<string, string> | null>;
      geometry: Record<string, unknown>;
    };
    const runtimeAppearance = appearances[2] as {
      elements: Record<string, Record<string, string> | null>;
    };
    const tokens = populated.tokens;
    expect(tokens).toMatchObject({
      '--workbench-content': '#121314',
      '--workbench-navigation': '#191a1b',
      '--workbench-inspector': '#191a1b',
      '--chrome-surface': '#191a1b',
      '--surface-raised': '#242526',
      '--control-hover': '#222324',
      '--control-pressed': '#303133',
      '--selection-bg': '#25242c',
      '--border-subtle': '#2a2b2c',
      '--border-strong': '#393b3e',
      '--control-border': '#72777d',
      '--workbench-pane-edge': '#3a3d40',
      '--workbench-pane-radius': '6px',
      '--workbench-overlay-edge': '#72777d',
      '--workbench-overlay-radius': '6px',
      '--text-primary': '#d4d4d4',
      '--text-secondary': '#a6a6a6',
      '--text-tertiary': '#929292',
      '--accent': '#8270c7',
      '--accent-hover': '#9281d4',
      '--on-accent': '#121314',
      '--focus-ring': '#b2a5e9',
    });

    expect(populated.elements.titlebar?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(populated.elements.navigator?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(populated.elements.trackBrowser?.backgroundColor).toBe('rgb(18, 19, 20)');
    expect(populated.elements.inspector?.backgroundColor).toBe('rgb(25, 26, 27)');
    for (const paneName of ['navigator', 'trackBrowser', 'inspector']) {
      const pane = populated.elements[paneName];
      expect(pane, `${paneName} is present`).not.toBeNull();
      expect(pane).toMatchObject({
        borderTopWidth: '1px',
        borderRightWidth: '1px',
        borderBottomWidth: '1px',
        borderLeftWidth: '1px',
        borderTopColor: 'rgb(58, 61, 64)',
        borderRightColor: 'rgb(58, 61, 64)',
        borderBottomColor: 'rgb(58, 61, 64)',
        borderLeftColor: 'rgb(58, 61, 64)',
        borderRadius: '6px',
        boxSizing: 'border-box',
      });
      expect(pane?.boxShadow).toBe('none');
    }
    expect(populated.elements.navigatorSectionDivider).toMatchObject({
      borderTopWidth: '1px',
      borderTopColor: 'rgb(42, 43, 44)',
    });
    expect(populated.elements.trackHeaderDivider).toMatchObject({
      borderBottomWidth: '1px',
      borderBottomColor: 'rgb(42, 43, 44)',
    });
    const populatedGeometry = populated.geometry as {
      navigator: { x: number; right: number };
      trackBrowser: { x: number; right: number };
      inspector: { x: number; right: number; width: number };
      separators: Array<{ x: number; right: number; width: number }>;
    };
    expect(populatedGeometry.separators).toHaveLength(2);
    expect(populatedGeometry.separators.map(({ width }) => width)).toEqual([8, 8]);
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
    expect(populated.elements.pageHeading).toMatchObject({ fontSize: '18px', fontWeight: '600' });
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
    expect(populated.elements.playerMode).toMatchObject({ fontSize: '11px', fontWeight: '600' });
    expect(compact.elements.search).toMatchObject({
      borderTopWidth: '1px',
      borderTopColor: 'rgb(114, 119, 125)',
      outlineWidth: '2px',
      outlineColor: 'rgb(178, 165, 233)',
    });
    expect(compact.elements.inspector).toMatchObject({
      borderTopWidth: '1px',
      borderRightWidth: '1px',
      borderBottomWidth: '1px',
      borderLeftWidth: '1px',
      borderTopColor: 'rgb(114, 119, 125)',
      borderRightColor: 'rgb(114, 119, 125)',
      borderBottomColor: 'rgb(114, 119, 125)',
      borderLeftColor: 'rgb(114, 119, 125)',
      borderRadius: '6px',
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
    expect(compactGeometry.inspector.width).toBeGreaterThanOrEqual(280);
    expect(compactGeometry.inspector.x).toBeGreaterThanOrEqual(0);
    expect(compactGeometry.inspector.y).toBeGreaterThanOrEqual(48);
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
