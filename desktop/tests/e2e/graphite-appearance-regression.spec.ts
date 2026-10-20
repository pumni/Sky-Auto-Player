import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

async function openApp(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  if (page.url().startsWith('http')) await page.evaluate(() => window.localStorage.clear());
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
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

async function openInspector(page: Page, tab: 'Details' | 'Runtime' = 'Details') {
  const trigger = page.getByRole('button', { name: 'Open utility panel' });
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
  await inspector.getByRole('tab', { name: tab }).click();
  return inspector;
}

async function readAppearance(page: Page, view: string) {
  return page.evaluate(async (viewName) => {
    const element = <T extends HTMLElement>(selector: string) =>
      document.querySelector<T>(selector);
    const rect = (node: Element) => {
      const { x, y, width, height, right, bottom } = node.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    const style = (selector: string, pseudo?: string) => {
      const node = element(selector);
      if (!node) return null;
      const computed = getComputedStyle(node, pseudo);
      return {
        color: computed.color,
        backgroundColor: computed.backgroundColor,
        height: computed.height,
        borderWidth: computed.borderTopWidth,
        borderColor: computed.borderTopColor,
        borderRadius: computed.borderRadius,
        outlineStyle: computed.outlineStyle,
        outlineWidth: computed.outlineWidth,
        outlineColor: computed.outlineColor,
        outlineOffset: computed.outlineOffset,
        boxShadow: computed.boxShadow,
        fontSize: computed.fontSize,
        fontWeight: computed.fontWeight,
      };
    };
    const luminance = (color: string) => {
      const channels = color
        .match(/[\d.]+/g)
        ?.slice(0, 3)
        .map(Number);
      if (!channels || channels.length !== 3) return null;
      const values = channels.map((channel) => {
        const normalized = channel / 255;
        return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * values[0] + 0.7152 * values[1] + 0.0722 * values[2];
    };
    const contrast = (first: string, second: string) => {
      const a = luminance(first);
      const b = luminance(second);
      if (a === null || b === null) return null;
      return Number(((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)).toFixed(2));
    };
    const root = getComputedStyle(document.documentElement);
    const search = element('.global-search');
    const searchInput = element<HTMLInputElement>('.global-search input');
    const browser = element('.track-browser-workbench-pane');
    const navigator = element('.navigator-workbench-pane');
    const inspector = element('.utility-workbench-pane');
    const workbench = element('.workbench');
    const titlebar = element('.app-titlebar');
    const player = element('.player-bar');
    const updateButton = element('.update-indicator');
    const searchRect = search?.getBoundingClientRect();
    const inputRect = searchInput?.getBoundingClientRect();
    const searchCenterHit = searchRect
      ? document.elementFromPoint(
          searchRect.x + searchRect.width / 2,
          searchRect.y + searchRect.height / 2,
        )
      : null;
    const inputHit = inputRect
      ? document.elementFromPoint(
          inputRect.x + inputRect.width / 2,
          inputRect.y + inputRect.height / 2,
        )
      : null;
    const searchFill = style('.global-search')?.backgroundColor ?? '';
    const placeholder = style('.global-search input', '::placeholder')?.color ?? '';
    const searchEdge = style('.global-search')?.borderColor ?? '';
    const focusRing = style('.global-search')?.outlineColor ?? '';
    const paneColors = [navigator, browser, inspector].map((pane) =>
      pane ? getComputedStyle(pane).borderTopColor : null,
    );

    return {
      view: viewName,
      viewport: { width: innerWidth, height: innerHeight, deviceScaleFactor: devicePixelRatio },
      palette: document.documentElement.dataset.palette,
      tokens: {
        workbenchContent: root.getPropertyValue('--workbench-content').trim(),
        workbenchNavigation: root.getPropertyValue('--workbench-navigation').trim(),
        workbenchInspector: root.getPropertyValue('--workbench-inspector').trim(),
        paneEdge: root.getPropertyValue('--workbench-pane-edge').trim(),
        controlBoundary: root.getPropertyValue('--workbench-control-boundary').trim(),
        searchSurface: root.getPropertyValue('--workbench-search-surface').trim(),
        searchIdleEdge: root.getPropertyValue('--workbench-search-idle-edge').trim(),
        searchFocus: root.getPropertyValue('--workbench-search-focus').trim(),
        divider: root.getPropertyValue('--workbench-divider').trim(),
      },
      computed: {
        titlebar: style('.app-titlebar'),
        search: style('.global-search'),
        placeholder: style('.global-search input', '::placeholder'),
        input: style('.global-search input'),
        browser: style('.track-browser-workbench-pane'),
        navigator: style('.navigator-workbench-pane'),
        inspector: style('.utility-workbench-pane'),
        playerDock: style('.player-bar'),
        updateButton: updateButton ? style('.update-indicator') : null,
      },
      geometry: {
        search: search ? rect(search) : null,
        input: searchInput ? rect(searchInput) : null,
        browser: browser ? rect(browser) : null,
        navigator: navigator ? rect(navigator) : null,
        inspector: inspector ? rect(inspector) : null,
        workbench: workbench ? rect(workbench) : null,
        titlebar: titlebar ? rect(titlebar) : null,
        playerDock: player ? rect(player) : null,
        cssWorkbenchGap: root.getPropertyValue('--workbench-gap').trim(),
        solverWorkbenchGap: workbench?.dataset.workbenchGap ?? null,
        separators: Array.from(
          document.querySelectorAll<HTMLElement>('.resizable-separator:not(.is-disabled)'),
        ).map(rect),
        inputHitTargeted: Boolean(
          inputHit && (inputHit === searchInput || searchInput?.contains(inputHit)),
        ),
        searchHitTargeted: Boolean(searchCenterHit && search?.contains(searchCenterHit)),
        updateHitTargeted: updateButton
          ? Boolean(
              document
                .elementFromPoint(
                  updateButton.getBoundingClientRect().x +
                    updateButton.getBoundingClientRect().width / 2,
                  updateButton.getBoundingClientRect().y +
                    updateButton.getBoundingClientRect().height / 2,
                )
                ?.closest('.update-indicator'),
            )
          : false,
      },
      accessibility: {
        searchRole:
          searchInput?.getAttribute('role') ??
          (searchInput?.type === 'search' ? 'searchbox' : 'textbox'),
        searchAccessibleName:
          searchInput?.getAttribute('aria-label') ?? searchInput?.getAttribute('placeholder'),
        searchFocused: document.activeElement === searchInput,
        searchFocusVisible: searchInput?.matches(':focus-visible') ?? false,
        searchTargetSize: searchRect
          ? { width: searchRect.width, height: searchRect.height }
          : null,
        textContrastPlaceholderOnSearchFill: contrast(placeholder, searchFill),
        nonTextContrastSearchEdgeOnFill: contrast(searchEdge, searchFill),
        nonTextContrastFocusRingOnFill: contrast(focusRing, searchFill),
        paneBorderColors: paneColors,
        paneEdgesAreDecorative: paneColors.every((color) => color !== null),
      },
      overflow: {
        documentWidth: document.documentElement.scrollWidth,
        bodyWidth: document.body.scrollWidth,
        fits: document.querySelector<HTMLElement>('.workbench')?.dataset.layoutFits,
      },
    };
  }, view);
}

async function expectAccessibility(
  page: Page,
  appearance: Awaited<ReturnType<typeof readAppearance>>,
) {
  expect(appearance.accessibility.searchRole).toBe('searchbox');
  expect(appearance.accessibility.searchAccessibleName).toBe('Search All Songs');
  expect(appearance.geometry.inputHitTargeted).toBe(true);
  expect(appearance.geometry.searchHitTargeted).toBe(true);
  expect(appearance.geometry.updateHitTargeted).toBe(true);
  expect(appearance.accessibility.searchTargetSize?.width).toBeGreaterThanOrEqual(24);
  expect(appearance.accessibility.searchTargetSize?.height).toBeGreaterThanOrEqual(24);
  expect(appearance.accessibility.textContrastPlaceholderOnSearchFill).toBeGreaterThanOrEqual(4.5);
  expect(appearance.accessibility.paneEdgesAreDecorative).toBe(true);
  const audit = await new AxeBuilder({ page }).analyze();
  expect(
    audit.violations.filter(({ impact }) => impact === 'critical' || impact === 'serious'),
  ).toEqual([]);
  return audit.violations.map(({ id, impact, description }) => ({ id, impact, description }));
}

test('Graphite appearance preserves semantic colors, hit targets, contrast, and focus', async ({
  page,
}) => {
  test.setTimeout(120_000);
  const appearances: Awaited<ReturnType<typeof readAppearance>>[] = [];

  await openApp(page, 1366, 768);
  await startDryRunWithSelection(page);
  await openInspector(page, 'Details');
  const search = page.getByRole('searchbox', { name: 'Search All Songs' });
  await page.mouse.move(0, 0);
  await search.evaluate((input: HTMLInputElement) => input.blur());
  await expect(search).not.toBeFocused();
  let appearance = await readAppearance(page, 'workbench-search-unfocused-1366x768');
  appearances.push(appearance);
  await expectAccessibility(page, appearance);
  await search.click();
  await expect(search).toBeFocused();
  await search.evaluate((input: HTMLInputElement) => input.blur());
  await page.keyboard.press('/');
  await expect(search).toBeFocused();
  await expect
    .poll(() => search.evaluate((input: HTMLInputElement) => input.matches(':focus-visible')))
    .toBe(true);
  await page.mouse.move(0, 0);
  appearance = await readAppearance(page, 'workbench-search-focused-1366x768');
  appearances.push(appearance);
  await expectAccessibility(page, appearance);
  await openApp(page, 800, 560);
  await startDryRunWithSelection(page);
  await openInspector(page, 'Details');
  const everydaySearch = page.getByRole('searchbox', { name: 'Search All Songs' });
  await everydaySearch.evaluate((input: HTMLInputElement) => input.blur());
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  appearance = await readAppearance(page, 'everyday-workbench-800x560');
  appearances.push(appearance);
  await expectAccessibility(page, appearance);
  await openApp(page, 640, 448);
  const compactPlayer = await startDryRunWithSelection(page);
  const overlay = await openInspector(page, 'Details');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(compactPlayer.locator('.player-track-status')).toHaveText('Dry run · Playing');
  await expect(overlay).toBeVisible();
  const compactSearch = page.getByRole('searchbox', { name: 'Search All Songs' });
  await compactSearch.evaluate((input: HTMLInputElement) => input.blur());
  appearance = await readAppearance(page, 'compact-overlay-640x448');
  appearances.push(appearance);
  await expectAccessibility(page, appearance);
  const expected = {
    workbenchContent: '#151618',
    browser: 'rgb(21, 22, 24)',
    paneEdge: 'rgb(49, 51, 54)',
    searchSurface: 'rgb(32, 33, 35)',
    searchSurfaceToken: '#202123',
    searchEdge: 'rgb(61, 63, 67)',
    searchEdgeToken: '#3d3f43',
    focusRing: 'rgb(150, 136, 198)',
    focusRingToken: '#9688c6',
  };
  expect(appearances).toHaveLength(4);
  for (const [index, item] of appearances.entries()) {
    expect(item.palette).toBe('graphite');
    expect(item.tokens.workbenchContent).toBe(expected.workbenchContent);
    expect(item.tokens.controlBoundary).toBe('#72777d');
    expect(item.tokens.divider).toBe('#2a2b2c');
    expect(item.tokens.searchSurface).toBe(expected.searchSurfaceToken);
    expect(item.tokens.searchIdleEdge).toBe(expected.searchEdgeToken);
    expect(item.tokens.searchFocus).toBe(expected.focusRingToken);
    expect(item.geometry.cssWorkbenchGap).toBe('4px');
    expect(item.geometry.solverWorkbenchGap).toBe('4');
    expect(item.computed.browser?.backgroundColor).toBe(expected.browser);
    expect(item.computed.browser?.borderColor).toBe(expected.paneEdge);
    expect(item.computed.navigator?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(item.computed.inspector?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(item.computed.titlebar?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(item.computed.titlebar?.height).toBe('40px');
    expect(item.computed.playerDock?.backgroundColor).toBe('rgb(25, 26, 27)');
    expect(item.computed.navigator?.borderColor).toBe(expected.paneEdge);
    expect(item.computed.navigator?.borderRadius).toBe('6px');
    expect(item.computed.browser?.borderRadius).toBe('6px');
    expect(item.computed.inspector?.borderColor).toBe(
      index === 3 ? 'rgb(114, 119, 125)' : expected.paneEdge,
    );
    expect(item.computed.inspector?.borderRadius).toBe('6px');
    if (index === 3) {
      expect(item.computed.inspector?.boxShadow).toContain('0px 10px 28px');
    }
    expect(item.computed.search?.backgroundColor).toBe(expected.searchSurface);
    expect(item.computed.search?.borderColor).toBe(expected.searchEdge);
    expect(item.computed.search?.borderWidth).toBe('1px');
    expect(item.computed.updateButton?.borderColor).toBe('rgb(114, 119, 125)');
    expect(item.computed.search?.fontSize).toBe('13px');
    expect(item.computed.input?.fontSize).toBe('13px');
    expect(item.accessibility.textContrastPlaceholderOnSearchFill).toBeGreaterThanOrEqual(4.5);
    expect(item.accessibility.nonTextContrastFocusRingOnFill).toBeGreaterThanOrEqual(3);
    expect(item.overflow.documentWidth).toBeLessThanOrEqual(item.viewport.width);
    expect(item.overflow.bodyWidth).toBeLessThanOrEqual(item.viewport.width);
    if (index === 1) {
      expect(item.accessibility.searchFocused).toBe(true);
      expect(item.accessibility.searchFocusVisible).toBe(true);
      expect(item.computed.search).toMatchObject({
        outlineStyle: 'solid',
        outlineWidth: '2px',
        outlineColor: expected.focusRing,
        outlineOffset: '1px',
        boxShadow: 'none',
      });
    } else {
      expect(item.computed.search?.outlineStyle).toBe('none');
    }
  }
});
