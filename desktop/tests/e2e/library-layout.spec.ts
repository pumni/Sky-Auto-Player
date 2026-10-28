import { expect, test, type Locator } from '@playwright/test';
import { palettes, selectPalette } from './testHelpers';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

test('shared Library and Track Browser geometry stays fixed across palettes', async ({ page }) => {
  test.setTimeout(120_000);
  const navigator = page.getByRole('navigation', { name: 'Library' });
  for (const name of ['Geometry playlist one', 'Geometry playlist two']) {
    await navigator.locator('.library-add-button').click();
    const dialog = page.getByRole('dialog', { name: 'New playlist' });
    await dialog.getByLabel('Playlist name').fill(name);
    await dialog.getByRole('button', { name: 'Create', exact: true }).click();
    await expect(navigator.getByRole('button', { name, exact: true })).toBeVisible();
  }
  await navigator.getByRole('button', { name: /^All Songs/ }).click();
  await expect(page.getByRole('heading', { name: 'All Songs' })).toBeVisible();
  for (const palette of palettes) {
    await page.setViewportSize({ width: 1366, height: 768 });
    await selectPalette(page, palette);
    const expanded = await page.evaluate(() => {
      const rect = (element: Element | null) => {
        if (!(element instanceof HTMLElement))
          throw new Error('Expected layout element is missing');
        const box = element.getBoundingClientRect();
        return { x: box.x, y: box.y, right: box.right, width: box.width, height: box.height };
      };
      const navigator = document.querySelector<HTMLElement>('.library-navigator');
      const navigatorPane = document.querySelector('.navigator-workbench-pane');
      const heading = document.querySelector<HTMLElement>('.library-navigator-heading');
      const primaryItems = document.querySelector('.library-primary-items');
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>(
          '.library-primary-items .library-nav-item, .library-nav-action-row',
        ),
      ).map((row) => {
        const item = row.matches('.library-nav-action-row')
          ? row.querySelector('.library-nav-item')
          : row;
        return {
          row: rect(row),
          icon: rect(item?.querySelector('.library-nav-item-icon') ?? null),
          label: rect(item?.querySelector('.library-nav-item-label') ?? null),
        };
      });
      if (!navigator || !heading || !primaryItems) throw new Error('Library layout is incomplete');
      const navigatorStyle = getComputedStyle(navigator);
      const headingStyle = getComputedStyle(heading);
      return {
        navigator: rect(navigatorPane),
        navigatorPadding: [
          navigatorStyle.paddingTop,
          navigatorStyle.paddingRight,
          navigatorStyle.paddingBottom,
          navigatorStyle.paddingLeft,
        ],
        headingMinHeight: headingStyle.minHeight,
        headingMarginBottom: headingStyle.marginBottom,
        primaryRowGap: getComputedStyle(primaryItems).rowGap,
        rows,
        trackHeader: rect(document.querySelector('.track-browser-header')),
        columnHeader: rect(document.querySelector('.track-table-header')),
        firstSong: rect(document.querySelector('.track-row')),
        documentWidth: document.documentElement.scrollWidth,
      };
    });
    expect(expanded.navigator.width).toBe(240);
    expect(expanded.navigatorPadding).toEqual(['12px', '8px', '12px', '8px']);
    expect(expanded.headingMinHeight).toBe('36px');
    expect(expanded.headingMarginBottom).toBe('8px');
    expect(expanded.primaryRowGap).toBe('4px');
    expect(expanded.rows).toHaveLength(5);
    for (const row of expanded.rows) {
      expect(row.row.height).toBe(42);
      expect(row.icon.width).toBe(24);
      expect(Math.abs(row.label.x - (row.icon.x + 32))).toBeLessThanOrEqual(1);
    }
    expect(expanded.trackHeader.height).toBe(40);
    expect(expanded.columnHeader.height).toBe(32);
    expect(expanded.firstSong.height).toBe(46);
    expect(expanded.firstSong.y).toBeCloseTo(expanded.columnHeader.y + 32, 0);
    expect(expanded.documentWidth).toBeLessThanOrEqual(1366);

    await page.setViewportSize({ width: 640, height: 448 });
    await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
    const compact = await page.evaluate(() => {
      const pane = document.querySelector<HTMLElement>('.navigator-workbench-pane');
      if (!pane) throw new Error('Collapsed Library pane is missing');
      const paneRect = pane.getBoundingClientRect();
      const tiles = [
        '.library-collapse-button',
        '.library-nav-item',
        '.library-add-button.is-collapsed',
      ].flatMap((selector) =>
        Array.from(document.querySelectorAll<HTMLElement>(selector)).map((element) => {
          const box = element.getBoundingClientRect();
          const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
          return {
            x: box.x,
            width: box.width,
            height: box.height,
            targeted: hit === element || (hit !== null && element.contains(hit)),
          };
        }),
      );
      return {
        paneWidth: paneRect.width,
        paneCenterX: paneRect.x + paneRect.width / 2,
        tiles,
        headerHeight: document.querySelector('.track-browser-header')?.getBoundingClientRect()
          .height,
        documentWidth: document.documentElement.scrollWidth,
      };
    });
    expect(compact.paneWidth).toBe(48);
    expect(compact.tiles.length).toBeGreaterThanOrEqual(5);
    for (const tile of compact.tiles) {
      expect(tile.width).toBe(40);
      expect(tile.height).toBe(40);
      expect(Math.abs(tile.x + tile.width / 2 - compact.paneCenterX)).toBeLessThanOrEqual(1);
      expect(tile.targeted).toBe(true);
    }
    expect(compact.headerHeight).toBe(32);
    expect(compact.documentWidth).toBeLessThanOrEqual(640);
  }
});

test('Graphite workspace header compacts by viewport and browser width over one canvas', async ({
  page,
}) => {
  for (const viewport of [
    { width: 1366, height: 768 },
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await expect(page.locator('.track-browser-header')).toBeVisible();
    await expect(page.locator('.track-row').first()).toBeVisible();
    const geometry = await page.evaluate(() => {
      const rect = (selector: string) => {
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
          background: computed.backgroundColor,
          fontSize: computed.fontSize,
          fontWeight: computed.fontWeight,
          borderBottomWidth: computed.borderBottomWidth,
          borderBottomColor: computed.borderBottomColor,
        };
      };
      const browser = document.querySelector<HTMLElement>('.track-browser-workbench-pane');
      const tableHeader = document.querySelector<HTMLElement>('.track-table-header');
      const divider = (() => {
        if (!tableHeader) return null;
        const headerStyle = getComputedStyle(tableHeader);
        const ruleStyle = getComputedStyle(tableHeader, '::after');
        return {
          height: ruleStyle.height,
          backgroundColor: ruleStyle.backgroundColor,
          left: Number.parseFloat(ruleStyle.left),
          right: Number.parseFloat(ruleStyle.right),
          width: Number.parseFloat(ruleStyle.width),
          headerWidth: tableHeader.getBoundingClientRect().width,
          paddingLeft: Number.parseFloat(headerStyle.paddingLeft),
          paddingRight: Number.parseFloat(headerStyle.paddingRight),
        };
      })();
      return {
        divider,
        viewportWidth: innerWidth,
        browserWidth: browser?.clientWidth ?? 0,
        browserScrollWidth: browser?.scrollWidth ?? 0,
        documentWidth: document.documentElement.scrollWidth,
        header: rect('.track-browser-header'),
        source: rect('.track-browser-source-label'),
        title: rect('.track-browser-header h1'),
        count: rect('.track-browser-count'),
        columnHeader: rect('.track-table-header'),
        firstSong: rect('.track-row'),
        headingTag: document.querySelector('.track-browser h1')?.tagName ?? null,
        headerTablists: document.querySelectorAll('.track-browser-header [role="tablist"]').length,
        headerStyle: style('.track-browser-header'),
        sourceStyle: style('.track-browser-source-label'),
        titleStyle: style('.track-browser-header h1'),
        countStyle: style('.track-browser-count'),
        browserStyle: style('.track-browser-workbench-pane'),
        columnStyle: style('.track-table-header'),
      };
    });

    expect(geometry.header).not.toBeNull();
    expect(geometry.source).not.toBeNull();
    expect(geometry.title).not.toBeNull();
    expect(geometry.count).not.toBeNull();
    expect(geometry.columnHeader).not.toBeNull();
    expect(geometry.firstSong).not.toBeNull();
    if (
      !geometry.header ||
      !geometry.source ||
      !geometry.title ||
      !geometry.count ||
      !geometry.columnHeader ||
      !geometry.firstSong
    ) {
      throw new Error('Graphite workspace header geometry is incomplete');
    }

    const expectedHeaderHeight = viewport.width <= 900 || geometry.browserWidth <= 520 ? 32 : 40;
    expect(geometry.header.height).toBe(expectedHeaderHeight);
    expect(geometry.columnHeader.height).toBe(32);
    expect(geometry.firstSong.height).toBe(46);
    expect(geometry.firstSong.y).toBeCloseTo(geometry.columnHeader.bottom, 0);
    expect(
      Math.abs(
        geometry.title.y + geometry.title.height / 2 - geometry.count.y - geometry.count.height / 2,
      ),
    ).toBeLessThanOrEqual(1);
    expect(geometry.headingTag).toBe('H1');
    expect(geometry.headerTablists).toBe(0);
    expect(geometry.headerStyle?.background).toBe('rgb(21, 22, 24)');
    expect(geometry.sourceStyle?.background).toBe('rgb(21, 22, 24)');
    expect(geometry.titleStyle?.fontSize).toBe(expectedHeaderHeight === 40 ? '14px' : '13px');
    expect(geometry.titleStyle?.fontWeight).toBe('600');
    expect(geometry.countStyle?.fontSize).toBe(expectedHeaderHeight === 40 ? '12px' : '11px');
    expect(geometry.browserStyle?.background).toBe('rgb(21, 22, 24)');
    expect(geometry.columnStyle?.background).toBe('rgb(21, 22, 24)');
    expect(geometry.columnStyle?.borderBottomWidth).toBe('0px');
    expect(geometry.divider).not.toBeNull();
    if (!geometry.divider) throw new Error('Graphite inset column divider is missing');
    expect(geometry.divider.height).toBe('1px');
    expect(geometry.divider.backgroundColor).toBe('rgb(42, 43, 44)');
    expect(geometry.divider.left).toBeCloseTo(geometry.divider.paddingLeft, 1);
    expect(geometry.divider.right).toBeCloseTo(geometry.divider.paddingRight, 1);
    expect(geometry.divider.left + geometry.divider.width + geometry.divider.right).toBeCloseTo(
      geometry.divider.headerWidth,
      1,
    );
    expect(geometry.browserScrollWidth).toBeLessThanOrEqual(geometry.browserWidth);
    expect(geometry.documentWidth).toBeLessThanOrEqual(viewport.width);
    await expect(page.getByRole('heading', { name: 'All Songs' })).toBeVisible();
    await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  }

  await page.setViewportSize({ width: 1366, height: 768 });
  const browserPane = page.locator('.track-browser-workbench-pane');
  const previousFlex = await browserPane.evaluate((element) => element.style.flex);
  await browserPane.evaluate((element) => {
    element.style.flex = '0 0 520px';
  });
  await expect
    .poll(() => browserPane.evaluate((element) => element.clientWidth))
    .toBeLessThanOrEqual(520);
  await expect(page.locator('.track-browser-header')).toHaveCSS('height', '32px');
  await expect(page.locator('.track-browser-header h1')).toHaveCSS('font-size', '13px');
  await browserPane.evaluate((element, flex) => {
    element.style.flex = flex;
  }, previousFlex);
});

test('collapsed Library rail stays compact and keyboard-accessible at 800 and 640 widths', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const navigator = page.getByRole('navigation', { name: 'Library' });
  const collapseButton = navigator.locator('.library-collapse-button');
  await collapseButton.click();
  await expect(collapseButton).not.toHaveAttribute('aria-pressed');
  await expect(navigator.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  await expect(navigator.getByRole('button', { name: 'Create playlist' })).toBeVisible();
  const collapsedLabels = navigator.locator('.library-nav-item-label');
  await expect(collapsedLabels).toHaveCount(3);
  expect(
    await collapsedLabels.evaluateAll((items) =>
      items.every((item) => {
        return getComputedStyle(item).display === 'none';
      }),
    ),
  ).toBe(true);
  const collapsedLayout = await page.evaluate(() => {
    const navigator = document.querySelector<HTMLElement>('.library-navigator');
    const launchers = document.querySelector<HTMLElement>('.library-collapsed-launchers');
    const footer = document.querySelector<HTMLElement>('.library-navigator-footer');
    if (!navigator || !footer) throw new Error('Collapsed Library areas are missing');
    const navigatorStyle = getComputedStyle(navigator);
    const footerStyle = getComputedStyle(footer);
    const launcherStyle = launchers ? getComputedStyle(launchers) : null;
    return {
      paddingTop: navigatorStyle.paddingTop,
      paddingBottom: navigatorStyle.paddingBottom,
      launcherCount: navigator.querySelectorAll('.library-collapsed-launchers').length,
      launcherItemCount: launchers?.querySelectorAll('.library-nav-item').length ?? 0,
      secondarySectionCount: navigator.querySelectorAll('.library-secondary-sections').length,
      launcherMarginTop: launcherStyle?.marginTop ?? null,
      launcherPaddingTop: launcherStyle?.paddingTop ?? null,
      footerPaddingTop: footerStyle.paddingTop,
    };
  });
  expect(collapsedLayout.paddingTop).toBe('8px');
  expect(collapsedLayout.paddingBottom).toBe('8px');
  expect(collapsedLayout.footerPaddingTop).toBe('8px');
  expect(collapsedLayout.secondarySectionCount).toBe(0);
  expect(collapsedLayout.launcherCount).toBe(collapsedLayout.launcherItemCount > 0 ? 1 : 0);
  if (collapsedLayout.launcherItemCount === 0) {
    expect(collapsedLayout.launcherMarginTop).toBeNull();
    expect(collapsedLayout.launcherPaddingTop).toBeNull();
  } else {
    expect(collapsedLayout.launcherMarginTop).toBe('8px');
    expect(collapsedLayout.launcherPaddingTop).toBe('6px');
  }
  await page.mouse.move(0, 0);
  const collapsedToggleStyle = await collapseButton.evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.color, background: style.backgroundColor };
  });
  expect(collapsedToggleStyle).toEqual({
    color: 'rgb(166, 166, 166)',
    background: 'rgba(0, 0, 0, 0)',
  });
  await collapseButton.hover();
  const collapsedToggleHover = await collapseButton.evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.color, background: style.backgroundColor };
  });
  expect(collapsedToggleHover).toEqual({
    color: 'rgb(212, 212, 212)',
    background: 'rgb(53, 54, 58)',
  });
  const pane = await page.locator('.navigator-workbench-pane').boundingBox();
  expect(pane).not.toBeNull();
  if (pane) {
    expect(Math.round(pane.width)).toBe(48);
    const tiles = await navigator
      .locator('.library-nav-item, .library-add-button.is-collapsed, .library-collapse-button')
      .evaluateAll((elements) =>
        elements.map((element) => {
          const rect = element.getBoundingClientRect();
          const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
          return {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            hitTargeted: hit === element || (hit !== null && element.contains(hit)),
          };
        }),
      );
    expect(tiles.length).toBeGreaterThan(0);
    for (const tile of tiles) {
      expect(tile.width).toBeCloseTo(40, 0);
      expect(tile.height).toBeCloseTo(40, 0);
      expect(Math.abs(tile.x + tile.width / 2 - (pane.x + pane.width / 2))).toBeLessThanOrEqual(1);
      expect(tile.hitTargeted).toBe(true);
    }
  }
  await page.keyboard.press('Tab');
  await collapseButton.evaluate((element) => element.focus());
  expect(await collapseButton.evaluate((element) => element.matches(':focus-visible'))).toBe(true);
  const collapseBounds = await collapseButton.boundingBox();
  const collapsedPaneBounds = await page.locator('.navigator-workbench-pane').boundingBox();
  expect(collapseBounds).not.toBeNull();
  expect(collapsedPaneBounds).not.toBeNull();
  if (collapseBounds && collapsedPaneBounds) {
    expect(collapseBounds.x - collapsedPaneBounds.x).toBeGreaterThanOrEqual(3);
    expect(
      collapsedPaneBounds.x + collapsedPaneBounds.width - collapseBounds.x - collapseBounds.width,
    ).toBeGreaterThanOrEqual(3);
  }
  expect(await collapseButton.evaluate((element) => getComputedStyle(element).outlineWidth)).toBe(
    '2px',
  );
  await expect(page.getByRole('contentinfo', { name: 'Player controls' })).toBeVisible();
  await expect(page.locator('.utility-workbench-pane')).toHaveCount(0);

  await navigator.getByRole('button', { name: 'Expand library navigator' }).click();
  await expect(navigator.getByText('Your Library')).toBeVisible();
  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  await expect(navigator.getByRole('button', { name: 'Create playlist' })).toBeVisible();
  const compactPadding = await page.locator('.library-navigator').evaluate((element) => {
    const style = getComputedStyle(element);
    return [style.paddingTop, style.paddingBottom];
  });
  expect(compactPadding).toEqual(['8px', '8px']);
  const compactTileBounds = await navigator
    .locator('.library-nav-item, .library-add-button.is-collapsed, .library-collapse-button')
    .evaluateAll((elements) =>
      elements.map((element) => {
        const rect = element.getBoundingClientRect();
        return { width: rect.width, height: rect.height };
      }),
    );
  expect(compactTileBounds.length).toBeGreaterThan(0);
  for (const tile of compactTileBounds) {
    expect(tile.width).toBeCloseTo(40, 0);
    expect(tile.height).toBeCloseTo(40, 0);
  }
});

test('Graphite Library keeps its headings, rows, actions, dividers, and compact tiles aligned', async ({
  page,
}) => {
  await page.evaluate(() => {
    window.localStorage.setItem(
      'sky.ui.workbench.v5',
      JSON.stringify({
        version: 5,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 240,
        utilityWidth: 320,
        utilityOpen: false,
      }),
    );
  });
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const navigator = page.getByRole('navigation', { name: 'Library' });
  const createPlaylist = async (name: string) => {
    await navigator.locator('.library-add-button').click();
    const dialog = page.getByRole('dialog', { name: 'New playlist' });
    await dialog.getByLabel('Playlist name').fill(name);
    await dialog.getByRole('button', { name: 'Create', exact: true }).click();
    await expect(navigator.getByRole('button', { name, exact: true })).toBeVisible();
  };
  await createPlaylist('Longest playlist title for alignment');
  await createPlaylist('Short list');

  const expanded = await page.evaluate(() => {
    const box = (element: Element | null) => {
      if (!(element instanceof HTMLElement)) throw new Error('Library geometry element is missing');
      const rect = element.getBoundingClientRect();
      return {
        x: rect.x,
        y: rect.y,
        right: rect.right,
        bottom: rect.bottom,
        width: rect.width,
        height: rect.height,
        centerX: rect.x + rect.width / 2,
      };
    };
    const pane = document.querySelector('.navigator-workbench-pane');
    const navigator = document.querySelector<HTMLElement>('.library-navigator');
    const navigatorHeading = document.querySelector<HTMLElement>('.library-navigator-heading');
    const heading = document.querySelector('.library-navigator-heading h2');
    const sectionHeading = document.querySelector('.library-section-heading');
    const headingActions = document.querySelector('.library-heading-actions');
    const divider = document.querySelector('.library-secondary-sections');
    const primaryItems = document.querySelector('.library-primary-items');
    const footer = document.querySelector('.library-navigator-footer');
    const rows = Array.from(
      document.querySelectorAll<HTMLElement>(
        '.library-primary-items .library-nav-item, .library-nav-action-row',
      ),
    ).map((row) => {
      const item = row.matches('.library-nav-action-row')
        ? row.querySelector('.library-nav-item')
        : row;
      const menu = row.querySelector('.library-more-button');
      return {
        row: box(row),
        icon: box(item?.querySelector('.library-nav-item-icon') ?? null),
        label: box(item?.querySelector('.library-nav-item-label') ?? null),
        menu: menu ? box(menu) : null,
      };
    });
    const paneBox = box(pane);
    if (!navigator || !navigatorHeading || !primaryItems) {
      throw new Error('Library navigator geometry is missing');
    }
    if (!divider) throw new Error('Library divider is missing');
    const navigatorStyle = getComputedStyle(navigator);
    const headingStyle = getComputedStyle(navigatorHeading);
    const dividerStyle = getComputedStyle(divider);
    const dividerBox = box(divider);
    return {
      pane: paneBox,
      navigatorInnerLeft:
        box(navigator).x + Number.parseFloat(getComputedStyle(navigator).paddingLeft),
      navigatorPadding: [
        navigatorStyle.paddingTop,
        navigatorStyle.paddingRight,
        navigatorStyle.paddingBottom,
        navigatorStyle.paddingLeft,
      ],
      headingStyle: {
        minHeight: headingStyle.minHeight,
        marginBottom: headingStyle.marginBottom,
        paddingTop: headingStyle.paddingTop,
        paddingRight: headingStyle.paddingRight,
        paddingBottom: headingStyle.paddingBottom,
        paddingLeft: headingStyle.paddingLeft,
      },
      dividerSpacing: {
        marginTop: dividerStyle.marginTop,
        paddingTop: dividerStyle.paddingTop,
      },
      primaryRowGap: getComputedStyle(primaryItems).rowGap,
      heading: box(heading),
      sectionHeading: box(sectionHeading),
      headingActions: box(headingActions),
      dividerInsets: [dividerBox.x - paneBox.x, paneBox.right - dividerBox.right],
      footerCenter: box(footer).centerX,
      rows,
    };
  });
  expect(expanded.navigatorPadding).toEqual(['12px', '8px', '12px', '8px']);
  expect(expanded.headingStyle).toEqual({
    minHeight: '36px',
    marginBottom: '8px',
    paddingTop: '0px',
    paddingRight: '0px',
    paddingBottom: '0px',
    paddingLeft: '8px',
  });
  expect(expanded.dividerSpacing).toEqual({ marginTop: '12px', paddingTop: '8px' });
  expect(expanded.primaryRowGap).toBe('4px');
  expect(Math.abs(expanded.heading.x - expanded.sectionHeading.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(expanded.footerCenter - expanded.pane.centerX)).toBeLessThanOrEqual(1);
  expect(Math.abs(expanded.dividerInsets[0] - expanded.dividerInsets[1])).toBeLessThanOrEqual(1);
  expect(expanded.rows).toHaveLength(5);
  for (const row of expanded.rows) {
    expect(row.icon.width).toBeCloseTo(24, 0);
    expect(Math.abs(row.icon.x - expanded.heading.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(row.icon.x - (expanded.navigatorInnerLeft + 8))).toBeLessThanOrEqual(1);
    expect(Math.abs(row.label.x - (row.icon.x + 32))).toBeLessThanOrEqual(1);
  }
  for (const row of expanded.rows) expect(row.row.height).toBeCloseTo(42, 0);
  const playlistMenus = expanded.rows.map((row) => row.menu).filter((menu) => menu !== null);
  expect(playlistMenus).toHaveLength(2);
  for (const menu of playlistMenus) {
    expect(Math.abs(menu!.right - expanded.headingActions.right)).toBeLessThanOrEqual(1);
  }

  const expectCollapsedGeometry = async () => {
    const compact = await page.evaluate(() => {
      const pane = document.querySelector<HTMLElement>('.navigator-workbench-pane');
      const heading = document.querySelector<HTMLElement>('.library-navigator-heading');
      const launchers = document.querySelector<HTMLElement>('.library-collapsed-launchers');
      const selectors = [
        '.library-collapse-button',
        '.library-nav-item',
        '.library-add-button.is-collapsed',
      ];
      if (!pane || !heading) throw new Error('Collapsed Library geometry is missing');
      const paneRect = pane.getBoundingClientRect();
      const headingStyle = getComputedStyle(heading);
      const launcherStyle = launchers ? getComputedStyle(launchers) : null;
      const tiles = selectors.flatMap((selector) =>
        Array.from(document.querySelectorAll<HTMLElement>(selector)).map((element) => {
          const rect = element.getBoundingClientRect();
          const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
          return {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            centerX: rect.x + rect.width / 2,
            targeted: hit === element || (hit !== null && element.contains(hit)),
          };
        }),
      );
      return {
        pane: { x: paneRect.x, width: paneRect.width, centerX: paneRect.x + paneRect.width / 2 },
        heading: {
          minHeight: headingStyle.minHeight,
          marginBottom: headingStyle.marginBottom,
          padding: headingStyle.padding,
        },
        launcherItemCount: launchers?.querySelectorAll('.library-nav-item').length ?? 0,
        launcherCount: pane.querySelectorAll('.library-collapsed-launchers').length,
        launcher: launcherStyle
          ? { marginTop: launcherStyle.marginTop, paddingTop: launcherStyle.paddingTop }
          : null,
        tiles,
      };
    });
    expect(compact.pane.width).toBeCloseTo(48, 0);
    expect(compact.heading).toEqual({
      minHeight: '40px',
      marginBottom: '4px',
      padding: '0px',
    });
    expect(compact.launcherItemCount).toBe(1);
    expect(compact.launcherCount).toBe(1);
    expect(compact.launcher).toEqual({ marginTop: '8px', paddingTop: '6px' });
    expect(compact.tiles.length).toBeGreaterThanOrEqual(5);
    for (const tile of compact.tiles) {
      expect(tile.width).toBeCloseTo(40, 0);
      expect(tile.height).toBeCloseTo(40, 0);
      expect(Math.abs(tile.centerX - compact.pane.centerX)).toBeLessThanOrEqual(1);
      expect(tile.targeted).toBe(true);
    }
  };

  await page.setViewportSize({ width: 800, height: 560 });
  const responsiveCollapse = navigator.locator('.library-collapse-button');
  await expect(responsiveCollapse).toBeVisible();
  if (
    !(await page
      .locator('.navigator-workbench-pane')
      .evaluate((pane) => pane.classList.contains('is-collapsed')))
  ) {
    await responsiveCollapse.click();
  }
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  await expectCollapsedGeometry();
  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  await expectCollapsedGeometry();

  await page.setViewportSize({ width: 1366, height: 768 });
  const responsiveExpand = navigator.locator('.library-collapse-button');
  await expect(responsiveExpand).toBeVisible();
  if (
    await page
      .locator('.navigator-workbench-pane')
      .evaluate((pane) => pane.classList.contains('is-collapsed'))
  ) {
    await responsiveExpand.click();
  }
  await expect(page.locator('.navigator-workbench-pane')).not.toHaveClass(/is-collapsed/);
  for (const name of ['Longest playlist title for alignment', 'Short list']) {
    await page.getByRole('button', { name: `More actions for ${name}` }).click();
    await page.getByRole('menuitem', { name: 'Delete playlist' }).click();
    const dialog = page.getByRole('dialog', { name: /Delete/ });
    await dialog.getByRole('button', { name: 'Delete playlist', exact: true }).click();
  }
  const empty = await page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) throw new Error(`${selector} is missing`);
      const rect = element.getBoundingClientRect();
      return { x: rect.x, right: rect.right, centerX: rect.x + rect.width / 2 };
    };
    return {
      playlistCount: document.querySelectorAll('.library-nav-action-row').length,
      sectionHeading: box('.library-section-heading'),
      emptyAction: box('.library-empty-playlists-create'),
      footer: box('.library-navigator-footer'),
      pane: box('.navigator-workbench-pane'),
    };
  });
  expect(empty.playlistCount).toBe(0);
  expect(Math.abs(empty.emptyAction.x - empty.sectionHeading.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(empty.footer.centerX - empty.pane.centerX)).toBeLessThanOrEqual(1);

  await page.setViewportSize({ width: 800, height: 560 });
  const emptyCollapseButton = navigator.locator('.library-collapse-button');
  if (
    !(await page
      .locator('.navigator-workbench-pane')
      .evaluate((pane) => pane.classList.contains('is-collapsed')))
  ) {
    await emptyCollapseButton.click();
  }
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  const emptyCollapsed = await navigator.evaluate((element) => ({
    playlistLauncherCount: element.querySelectorAll(
      '.library-collapsed-launchers .library-nav-item',
    ).length,
    launchers: element.querySelectorAll('.library-collapsed-launchers').length,
    expandedPlaylistSection: element.querySelectorAll('.library-secondary-sections').length,
    children: Array.from(element.children).map((child) => (child as HTMLElement).className),
  }));
  expect(emptyCollapsed).toEqual({
    playlistLauncherCount: 0,
    launchers: 0,
    expandedPlaylistSection: 0,
    children: ['library-navigator-heading', 'library-primary-items', 'library-navigator-footer'],
  });
});

test('Graphite automatic compact navigation preserves the saved expanded preference', async ({
  page,
}) => {
  const navigator = page.getByRole('navigation', { name: 'Library' });
  await page.evaluate(() => {
    window.localStorage.setItem(
      'sky.ui.workbench.v5',
      JSON.stringify({
        version: 5,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 240,
        utilityWidth: 320,
        utilityOpen: false,
      }),
    );
  });
  await page.reload();
  const preference = () =>
    page.evaluate(
      () =>
        JSON.parse(window.localStorage.getItem('sky.ui.workbench.v5') ?? 'null')
          ?.navigatorPreference,
    );

  await expect.poll(preference).toBe('expanded');
  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  await expect(navigator.getByRole('button', { name: 'Expand library navigator' })).toBeVisible();
  await expect.poll(preference).toBe('expanded');

  await navigator.getByRole('button', { name: 'Expand library navigator' }).click();
  await expect(page.locator('.navigator-workbench-pane')).not.toHaveClass(/is-collapsed/);
  await expect(page.locator('.workbench')).toHaveAttribute('data-layout-fits', 'true');
  await expect.poll(preference).toBe('expanded');

  await page.setViewportSize({ width: 800, height: 560 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '800');
  await expect(page.locator('.navigator-workbench-pane')).not.toHaveClass(/is-collapsed/);
  await expect.poll(preference).toBe('expanded');
  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-workbench-width', '640');
  await expect(page.locator('.navigator-workbench-pane')).toHaveClass(/is-collapsed/);
  await expect.poll(preference).toBe('expanded');
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
    .toBeLessThanOrEqual(640);
});
