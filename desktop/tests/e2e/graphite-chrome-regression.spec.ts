import { expect, test, type Page } from '@playwright/test';

async function openApp(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  if (page.url().startsWith('http')) await page.evaluate(() => window.localStorage.clear());
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
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
  await inspector.getByRole('tab', { name: tab }).click();
  return inspector;
}

async function readAppearance(page: Page, view: string) {
  return page.evaluate((viewName) => {
    const bounds = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    const style = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const computed = getComputedStyle(element);
      return {
        color: computed.color,
        backgroundColor: computed.backgroundColor,
        height: computed.height,
        borderTopWidth: computed.borderTopWidth,
        borderTopColor: computed.borderTopColor,
        borderRightWidth: computed.borderRightWidth,
        borderRightColor: computed.borderRightColor,
        borderBottomWidth: computed.borderBottomWidth,
        borderBottomColor: computed.borderBottomColor,
        borderLeftWidth: computed.borderLeftWidth,
        borderLeftColor: computed.borderLeftColor,
        borderRadius: computed.borderRadius,
        boxShadow: computed.boxShadow,
        fontSize: computed.fontSize,
        fontWeight: computed.fontWeight,
        outlineWidth: computed.outlineWidth,
        outlineColor: computed.outlineColor,
      };
    };
    const box = (element: Element) => {
      const { x, y, width, height, right, bottom } = element.getBoundingClientRect();
      return { x, y, width, height, right, bottom };
    };
    const workbench = document.querySelector<HTMLElement>('.workbench');
    const navigator = document.querySelector<HTMLElement>('.navigator-workbench-pane');
    const trackBrowser = document.querySelector<HTMLElement>('.track-browser-workbench-pane');
    const inspector = document.querySelector<HTMLElement>('.utility-workbench-pane');
    const playerDock = document.querySelector<HTMLElement>('.player-bar');
    const separators = Array.from(
      document.querySelectorAll<HTMLElement>('.resizable-separator:not(.is-disabled)'),
    );
    const root = getComputedStyle(document.documentElement);
    const widths = separators.map((separator) => {
      const rect = separator.getBoundingClientRect();
      const centerY = rect.y + rect.height / 2;
      const targets = [rect.x - 1.5, rect.x + rect.width / 2, rect.right + 1.5].map((x) => {
        const hit = document.elementFromPoint(x, centerY);
        return Boolean(hit && (hit === separator || separator.contains(hit)));
      });
      return { ...box(separator), acquisitionPointsHit: targets };
    });
    const navigatorBox = navigator?.getBoundingClientRect();
    const trackBox = trackBrowser?.getBoundingClientRect();
    const inspectorBox = inspector?.getBoundingClientRect();
    const workbenchBox = workbench?.getBoundingClientRect();
    const dockBox = playerDock?.getBoundingClientRect();

    return {
      view: viewName,
      viewport: { width: innerWidth, height: innerHeight, deviceScaleFactor: devicePixelRatio },
      tokens: {
        workbenchGap: root.getPropertyValue('--workbench-gap').trim(),
        titlebarHeight: root.getPropertyValue('--titlebar-height').trim(),
        chromeSurface: root.getPropertyValue('--chrome-surface').trim(),
        workbenchContent: root.getPropertyValue('--workbench-content').trim(),
        workbenchNavigation: root.getPropertyValue('--workbench-navigation').trim(),
        workbenchInspector: root.getPropertyValue('--workbench-inspector').trim(),
        paneEdge: root.getPropertyValue('--workbench-pane-edge').trim(),
        paneRadius: root.getPropertyValue('--workbench-pane-radius').trim(),
        overlayEdge: root.getPropertyValue('--workbench-overlay-edge').trim(),
        overlayRadius: root.getPropertyValue('--workbench-overlay-radius').trim(),
        dockHeight: root.getPropertyValue('--player-height').trim(),
      },
      colors: {
        shell: style('.app-shell')?.backgroundColor,
        titlebar: style('.app-titlebar')?.backgroundColor,
        navigator: style('.navigator-workbench-pane')?.backgroundColor,
        trackBrowser: style('.track-browser-workbench-pane')?.backgroundColor,
        inspector: style('.utility-workbench-pane')?.backgroundColor,
        playerDock: style('.player-bar')?.backgroundColor,
        workbenchDivider: root.getPropertyValue('--border-subtle').trim(),
      },
      elements: {
        titlebar: style('.app-titlebar'),
        search: style('.global-search'),
        navigator: style('.navigator-workbench-pane'),
        trackBrowser: style('.track-browser-workbench-pane'),
        inspector: style('.utility-workbench-pane'),
        playerDock: style('.player-bar'),
      },
      geometry: {
        workbench: workbench ? box(workbench) : null,
        workbenchGap: workbench?.dataset.workbenchGap,
        navigator: navigator ? box(navigator) : null,
        trackBrowser: trackBrowser ? box(trackBrowser) : null,
        inspector: inspector ? box(inspector) : null,
        playerDock: playerDock ? box(playerDock) : null,
        separators: widths,
        exterior: {
          left: workbenchBox && navigatorBox ? navigatorBox.x - workbenchBox.x : null,
          right: workbenchBox && inspectorBox ? workbenchBox.right - inspectorBox.right : null,
          bottom:
            workbenchBox && dockBox && trackBox
              ? dockBox.y - trackBox.bottom
              : workbenchBox && trackBox
                ? workbenchBox.bottom - trackBox.bottom
                : null,
        },
      },
      overflow: {
        documentWidth: document.documentElement.scrollWidth,
        bodyWidth: document.body.scrollWidth,
        fits: workbench?.dataset.layoutFits,
      },
    };
  }, view);
}

test('Graphite chrome preserves collision-free search and workbench geometry', async ({ page }) => {
  test.setTimeout(120_000);
  const appearances: unknown[] = [];

  await openApp(page, 1366, 768);
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
  const player = await startDryRunWithSelection(page);
  const details = await openInspector(page, 'Details');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'docked');
  await expect(details.getByRole('heading', { name: 'Song Details' })).toBeVisible();
  const search = page.getByRole('searchbox', { name: 'Search All Songs' });
  await page.mouse.move(0, 0);
  await search.evaluate((input: HTMLInputElement) => input.blur());
  await expect(search).not.toBeFocused();
  appearances.push(await readAppearance(page, 'populated-unfocused'));

  await page.keyboard.press('/');
  await expect(search).toBeFocused();
  await expect
    .poll(() => search.evaluate((input: HTMLInputElement) => input.matches(':focus-visible')))
    .toBe(true);
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
  await expect(player.locator('.player-dry-run-label')).toHaveText('Dry run');
  await page.mouse.move(0, 0);
  appearances.push(await readAppearance(page, 'populated-search-focused'));

  await openApp(page, 640, 448);
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
  const compactPlayer = await startDryRunWithSelection(page);
  const overlay = await openInspector(page, 'Details');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
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
  await expect(overlay).toBeVisible();
  const overlayBox = await overlay.boundingBox();
  expect(overlayBox).not.toBeNull();
  if (!overlayBox) throw new Error('Compact Inspector has no visible bounds');
  expect(overlayBox.x).toBeGreaterThanOrEqual(0);
  expect(overlayBox.y).toBeGreaterThanOrEqual(40);
  expect(overlayBox.x + overlayBox.width).toBeLessThanOrEqual(640);
  expect(overlayBox.y + overlayBox.height).toBeLessThanOrEqual(356);
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
    .toBeLessThanOrEqual(640);
  await page.mouse.move(0, 0);
  appearances.push(await readAppearance(page, 'compact-overlay-dry-run'));

  await openApp(page, 1366, 768);
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();
  await startDryRunWithSelection(page);
  const runtime = await openInspector(page, 'Runtime');
  await expect(
    runtime.getByText('Mode: Dry run · no physical input sent', { exact: true }),
  ).toBeVisible();
  await expect(runtime.getByText('Sender availability', { exact: true })).toBeVisible();
  await expect(runtime.getByText('Sender samples', { exact: true })).toBeVisible();
  await expect(runtime.getByText('Sender backend', { exact: true })).toBeVisible();
  await expect(runtime.locator('.diagnostics-sender-summary')).toContainText('Not measured');
  await page.mouse.move(0, 0);
  appearances.push(await readAppearance(page, 'runtime-dry-run'));
  const expected = {
    titlebar: 40,
    gap: 4,
    left: 4,
    right: 4,
    bottom: 4,
    browserWidth: 790,
    browserX: 248,
    inspectorX: 1042,
  };
  const populated = appearances[0] as {
    tokens: Record<string, string>;
    elements: Record<string, Record<string, string> | null>;
    geometry: {
      workbench: { y: number; bottom: number };
      workbenchGap?: string;
      navigator: { x: number; width: number; right: number; bottom: number };
      trackBrowser: { x: number; width: number; right: number; bottom: number };
      inspector: { x: number; width: number; right: number; bottom: number };
      playerDock: { y: number; height: number };
      separators: Array<{
        x: number;
        right: number;
        width: number;
        acquisitionPointsHit: boolean[];
      }>;
      exterior: { left: number; right: number; bottom: number };
    };
    colors: Record<string, string>;
    overflow: { documentWidth: number; bodyWidth: number; fits: string };
  };
  expect(populated.tokens.titlebarHeight).toBe(`${expected.titlebar}px`);
  expect(populated.tokens.workbenchGap).toBe(`${expected.gap}px`);
  expect(populated.geometry.workbenchGap).toBe(String(expected.gap));
  expect(populated.geometry.workbench.y).toBe(expected.titlebar);
  expect(populated.geometry.navigator).toMatchObject({
    x: expected.left,
    width: 240,
    right: expected.left + 240,
  });
  expect(populated.geometry.trackBrowser).toMatchObject({
    x: expected.browserX,
    width: expected.browserWidth,
  });
  expect(populated.geometry.inspector).toMatchObject({
    x: expected.inspectorX,
    width: 320,
    right: 1366 - expected.right,
  });
  expect(populated.geometry.separators.map(({ width }) => width)).toEqual([
    expected.gap,
    expected.gap,
  ]);
  expect(populated.geometry.separators[0].x).toBe(populated.geometry.navigator.right);
  expect(populated.geometry.separators[0].right).toBe(populated.geometry.trackBrowser.x);
  expect(populated.geometry.separators[1].x).toBe(populated.geometry.trackBrowser.right);
  expect(populated.geometry.separators[1].right).toBe(populated.geometry.inspector.x);
  expect(
    populated.geometry.separators.every(({ acquisitionPointsHit }) =>
      acquisitionPointsHit.every(Boolean),
    ),
  ).toBe(true);
  expect(populated.geometry.exterior).toEqual({
    left: expected.left,
    right: expected.right,
    bottom: expected.bottom,
  });
  expect(populated.geometry.playerDock).toMatchObject({ y: 768 - 92, height: 92 });
  expect(populated.geometry.navigator.bottom).toBe(populated.geometry.trackBrowser.bottom);
  expect(populated.geometry.trackBrowser.bottom).toBe(populated.geometry.inspector.bottom);
  expect(populated.colors).toMatchObject({
    shell: 'rgb(25, 26, 27)',
    titlebar: 'rgb(25, 26, 27)',
    navigator: 'rgb(25, 26, 27)',
    trackBrowser: 'rgb(21, 22, 24)',
    inspector: 'rgb(25, 26, 27)',
    playerDock: 'rgb(25, 26, 27)',
    workbenchDivider: '#2a2b2c',
  });
  for (const name of ['navigator', 'trackBrowser', 'inspector']) {
    expect(populated.elements[name]).toMatchObject({
      borderTopWidth: '1px',
      borderTopColor: 'rgb(49, 51, 54)',
      borderRightWidth: '1px',
      borderRightColor: 'rgb(49, 51, 54)',
      borderBottomWidth: '1px',
      borderBottomColor: 'rgb(49, 51, 54)',
      borderLeftWidth: '1px',
      borderLeftColor: 'rgb(49, 51, 54)',
      borderRadius: '6px',
    });
  }
  expect(populated.elements.titlebar?.borderBottomWidth).toBe('0px');
  expect(populated.elements.titlebar?.height).toBe(`${expected.titlebar}px`);
  expect(populated.elements.playerDock).toMatchObject({
    borderTopWidth: '1px',
    borderTopColor: 'rgb(42, 43, 44)',
  });
  expect(populated.overflow.documentWidth).toBeLessThanOrEqual(1366);
  expect(populated.overflow.bodyWidth).toBeLessThanOrEqual(1366);
  expect(populated.overflow.fits).toBe('true');

  const compact = appearances[2] as {
    elements: Record<string, Record<string, string> | null>;
    geometry: {
      workbench: { bottom: number };
      inspector: { x: number; right: number; y: number; bottom: number };
      separators: unknown[];
    };
    overflow: { documentWidth: number; fits: string };
  };
  expect(compact.elements.inspector).toMatchObject({
    borderTopWidth: '1px',
    borderTopColor: 'rgb(114, 119, 125)',
    borderRadius: '6px',
    boxShadow: 'rgba(0, 0, 0, 0.34) 0px 10px 28px 0px',
  });
  expect(compact.elements.search).toMatchObject({
    outlineWidth: '2px',
    outlineColor: 'rgb(150, 136, 198)',
  });
  expect(compact.geometry.inspector.x).toBeGreaterThanOrEqual(0);
  expect(compact.geometry.inspector.right).toBe(640 - expected.gap);
  expect(compact.geometry.inspector.y).toBe(expected.titlebar);
  expect(compact.geometry.inspector.bottom).toBe(compact.geometry.workbench.bottom - expected.gap);
  expect(compact.overflow.documentWidth).toBeLessThanOrEqual(640);
  expect(compact.overflow.fits).toBe('true');
});
