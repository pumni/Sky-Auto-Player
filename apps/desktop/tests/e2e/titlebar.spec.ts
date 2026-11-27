import { expect, type Locator, type Page, test } from '@playwright/test';

const viewports = [
  { width: 640, height: 448 },
  { width: 800, height: 560 },
  { width: 1024, height: 640 },
  { width: 1366, height: 768 },
  { width: 1920, height: 1080 },
] as const;
const scaleFactors = [1, 1.25, 1.5] as const;
const longPlaylistName = 'Long playlist scope label for responsive titlebar geometry coverage';

test('titlebar icon actions expose their update, reload, and settings guidance', async ({
  page,
}) => {
  await page.goto('/?mockReleaseNotes=short');

  const update = page.getByRole('button', { name: 'Open update 4.0.0-alpha.2-mock' });
  await expect(update).toBeVisible();
  const updateDialog = page.getByRole('dialog', { name: 'Software update' });
  if (await updateDialog.isVisible()) {
    await updateDialog.getByRole('button', { name: 'Close update' }).click();
    await expect(updateDialog).toBeHidden();
  }

  await update.hover();
  await expect(
    page.getByRole('tooltip', {
      name: 'Update available: 4.0.0-alpha.2-mock. Open update details.',
    }),
  ).toHaveText('Update available: 4.0.0-alpha.2-mock. Open update details.');
  await page.mouse.move(0, 0);

  const reload = page.getByRole('button', { name: 'Reload library' });
  await reload.hover();
  await expect(page.getByRole('tooltip', { name: 'Reload library' })).toHaveText('Reload library');
  await page.mouse.move(0, 0);

  const settings = page.getByRole('button', { name: 'Open settings' });
  await settings.focus();
  await expect(page.getByRole('tooltip', { name: 'Settings · Ctrl+,' })).toHaveText(
    'Settings · Ctrl+,',
  );
});

test('Global Search keeps one clear affordance, focus, and current results through in-flight clear', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockSearchDelayMs=700');
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();

  const search = page.getByRole('searchbox', { name: 'Search Built-in Songs' });
  const globalSearch = page.locator('[data-slot="global-search"]');
  const clear = page.getByRole('button', { name: 'Clear search' });
  expect(await globalSearch.getByRole('button', { name: 'Clear search' }).count()).toBe(0);
  const nativeCancelSuppression = await search.evaluate((input) => {
    const rules = [...document.styleSheets].flatMap((sheet) => {
      try {
        return [...(sheet.cssRules ?? [])].filter(
          (rule): rule is CSSStyleRule => rule instanceof CSSStyleRule,
        );
      } catch {
        return [];
      }
    });
    const suppressionRule = rules.find(
      (rule) =>
        rule.selectorText ===
        '[data-slot="global-search"] input[type="search"]::-webkit-search-cancel-button',
    );
    return {
      scopedInput: input.closest('[data-slot="global-search"]') !== null,
      selector: suppressionRule?.selectorText ?? null,
      appearance: suppressionRule?.style.getPropertyValue('appearance') ?? null,
      display: suppressionRule?.style.getPropertyValue('display') ?? null,
    };
  });
  expect(nativeCancelSuppression).toEqual({
    scopedInput: true,
    selector: '[data-slot="global-search"] input[type="search"]::-webkit-search-cancel-button',
    appearance: 'none',
    display: 'none',
  });

  await search.fill('Blue');
  await page.waitForTimeout(150);
  await expect(clear).toBeVisible();
  await expect(globalSearch.getByRole('button', { name: 'Clear search' })).toHaveCount(1);
  await expect(search).toBeFocused();
  await search.evaluate((input) => input.blur());
  await expect(clear).toBeVisible();
  await clear.click();
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();
  await expect(clear).toHaveCount(0);
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await page.waitForTimeout(600);
  await expect(search).toHaveValue('');
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();

  await search.fill('Blue');
  await page.waitForTimeout(150);
  await page.keyboard.press('Tab');
  await expect(clear).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();
  await expect(clear).toHaveCount(0);
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  await page.waitForTimeout(600);
  await expect(search).toHaveValue('');
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
});

test('update dialog owns focus standalone and when opened from Settings', async ({ page }) => {
  await page.goto('/?mockReleaseNotes=short');
  const update = page.getByRole('button', { name: 'Open update 4.0.0-alpha.2-mock' });
  const updateDialog = page
    .getByRole('dialog')
    .filter({ has: page.getByRole('button', { name: 'Close update' }) });

  await update.click();
  await expect(updateDialog).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(1);
  await updateDialog.getByRole('button', { name: 'Close update' }).click();
  await expect(updateDialog).toBeHidden();
  await expect(update).toBeFocused();

  const settings = page.getByRole('button', { name: 'Open settings' });
  await settings.click();
  const settingsDialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(settingsDialog).toBeVisible();
  await settingsDialog.getByRole('button', { name: 'Updates', exact: true }).click();
  await settingsDialog.getByRole('button', { name: 'View update', exact: true }).click();
  await expect(updateDialog).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(1);
  await updateDialog.getByRole('button', { name: 'Close update' }).click();
  await expect(updateDialog).toBeHidden();
  await expect(settingsDialog).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(1);
  await settingsDialog.getByRole('button', { name: 'Close settings' }).click();
  await expect(settingsDialog).toBeHidden();
  await expect(settings).toBeFocused();
});

async function createLongPlaylistSource(page: Page, updateAvailable: boolean) {
  const updateFixture = updateAvailable ? '' : '&mockUpdateState=current';
  await page.goto(`/?mockPlaybackDurationMs=120000${updateFixture}`);
  const titlebar = page.locator('[data-slot="titlebar"]');
  await expect(titlebar).toHaveAttribute('data-update-available', String(updateAvailable));

  await page.locator('[data-slot="library-add-button"]').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill(longPlaylistName);
  await createDialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByRole('searchbox', { name: `Search ${longPlaylistName}` })).toBeVisible();
}

async function readTitlebarGeometry(page: Page) {
  return page.evaluate(() => {
    const selectors = {
      titlebar: '[data-slot="titlebar"]',
      leading: '[data-slot="titlebar-leading"]',
      brand: '[data-slot="titlebar-brand"]',
      search: '[data-slot="global-search"]',
      update: 'button[aria-label^="Open update "]',
      reload: 'button[aria-label="Reload library"]',
      settings: 'button[aria-label="Open settings"]',
      minimize: 'button[aria-label="Minimize window"]',
      maximize: 'button[aria-label="Maximize window"], button[aria-label="Restore window"]',
      close: 'button[aria-label="Close window"]',
      drag: '[data-slot="titlebar-drag-space"]',
      trailingDrag: '[data-slot="titlebar-drag-space-trailing"]',
      end: '[data-slot="titlebar-controls"]',
    } as const;
    const boxes: Record<
      string,
      { x: number; y: number; right: number; bottom: number; width: number; height: number } | null
    > = {};
    for (const [name, selector] of Object.entries(selectors)) {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element || getComputedStyle(element).display === 'none') {
        boxes[name] = null;
        continue;
      }
      const rect = element.getBoundingClientRect();
      boxes[name] = {
        x: rect.x,
        y: rect.y,
        right: rect.right,
        bottom: rect.bottom,
        width: rect.width,
        height: rect.height,
      };
    }
    const workbench = document.querySelector<HTMLElement>('[data-slot="workbench"]');
    const navigator = document.querySelector<HTMLElement>('[data-pane="navigator"]');
    const trackBrowser = document.querySelector<HTMLElement>('[data-pane="browser"]');
    const workbenchRect = workbench?.getBoundingClientRect();
    const navigatorRect = navigator?.getBoundingClientRect();
    const trackBrowserRect = trackBrowser?.getBoundingClientRect();
    const separators = Array.from(
      document.querySelectorAll<HTMLElement>(
        '[data-slot="resizable-separator"]:not([aria-disabled="true"])',
      ),
    ).map((separator) => {
      const rect = separator.getBoundingClientRect();
      const centerY = rect.y + rect.height / 2;
      const hit = (x: number) => {
        const target = document.elementFromPoint(x, centerY);
        return target === separator || (target !== null && separator.contains(target));
      };
      return {
        width: rect.width,
        acquisition: [hit(rect.x - 1.5), hit(rect.x + rect.width / 2), hit(rect.right + 1.5)],
      };
    });

    return {
      boxes,
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        dpr: window.devicePixelRatio,
      },
      documentWidth: document.documentElement.scrollWidth,
      updateAvailable: document
        .querySelector('[data-slot="titlebar"]')
        ?.getAttribute('data-update-available'),
      dragRegion: document
        .querySelector('[data-slot="titlebar-drag-space"]')
        ?.getAttribute('data-tauri-drag-region'),
      trailingDragRegion: document
        .querySelector('[data-slot="titlebar-drag-space-trailing"]')
        ?.getAttribute('data-tauri-drag-region'),
      searchInput: document.querySelector<HTMLInputElement>('[data-slot="global-search"] input')
        ? {
            width: document
              .querySelector<HTMLInputElement>('[data-slot="global-search"] input')!
              .getBoundingClientRect().width,
            height: document
              .querySelector<HTMLInputElement>('[data-slot="global-search"] input')!
              .getBoundingClientRect().height,
          }
        : null,
      searchFreeRegion: (() => {
        const leadingRight = boxes.leading?.right;
        const endLeft = boxes.end?.x;
        const titlebar = boxes.titlebar;
        const search = boxes.search;
        if (
          leadingRight === null ||
          leadingRight === undefined ||
          endLeft === null ||
          !titlebar ||
          !search
        ) {
          return null;
        }
        const midpoint = titlebar.x + titlebar.width / 2;
        const searchMidpoint = search.x + search.width / 2;
        return {
          left: leadingRight,
          right: endLeft,
          midpoint,
          searchMidpoint,
          centerError: Math.abs(searchMidpoint - midpoint),
        };
      })(),
      workbench: {
        fits: workbench?.dataset.layoutFits,
        gutter: workbench?.dataset.workbenchGap,
        cssGutter:
          document
            .querySelector<HTMLElement>('[data-slot="resizable-separator"]')
            ?.getBoundingClientRect().width ?? Number.NaN,
        separators,
        leftInset: workbenchRect && navigatorRect ? navigatorRect.x - workbenchRect.x : null,
        rightInset:
          workbenchRect && trackBrowserRect ? workbenchRect.right - trackBrowserRect.right : null,
        bottomInset:
          workbenchRect && trackBrowserRect ? workbenchRect.bottom - trackBrowserRect.bottom : null,
        paneGap:
          navigatorRect && trackBrowserRect ? trackBrowserRect.x - navigatorRect.right : null,
        bounds: {
          workbench: workbenchRect
            ? { x: workbenchRect.x, width: workbenchRect.width, right: workbenchRect.right }
            : null,
          navigator: navigatorRect
            ? { x: navigatorRect.x, width: navigatorRect.width, right: navigatorRect.right }
            : null,
          trackBrowser: trackBrowserRect
            ? {
                x: trackBrowserRect.x,
                width: trackBrowserRect.width,
                right: trackBrowserRect.right,
              }
            : null,
        },
      },
    };
  });
}

async function expectPointerCenter(page: Page, locator: Locator) {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  if (!box || box.width <= 0 || box.height <= 0)
    throw new Error('Titlebar hit target is not visible');

  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  const hitTarget = await locator.evaluate(
    (element, point) => {
      const hit = document.elementFromPoint(point.x, point.y);
      return hit === element || (hit !== null && element.contains(hit));
    },
    { x, y },
  );
  expect(hitTarget).toBe(true);
  await page.mouse.click(x, y);
}

async function expectCaptionHover(
  page: Page,
  locator: Locator,
  expectedBackground: string,
  expectedForeground: string,
) {
  const bounds = await locator.boundingBox();
  expect(bounds).not.toBeNull();
  if (!bounds) throw new Error('Caption control has no hit target');
  expect(bounds.width).toBeCloseTo(46, 0);
  expect(bounds.height).toBeCloseTo(40, 0);
  const dragRegion = await locator.getAttribute('data-tauri-drag-region');
  expect(dragRegion).toBe('false');
  const center = { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 };
  const centerIsHitTarget = await locator.evaluate((element, point) => {
    const hit = document.elementFromPoint(point.x, point.y);
    return hit === element || (hit !== null && element.contains(hit));
  }, center);
  expect(centerIsHitTarget).toBe(true);

  await locator.hover();
  const hover = await locator.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      background: style.backgroundColor,
      foreground: style.color,
      borderWidth: style.borderTopWidth,
      borderStyle: style.borderTopStyle,
      borderRadius: style.borderRadius,
      boxShadow: style.boxShadow,
      focusVisible: element.matches(':focus-visible'),
      outlineWidth: style.outlineWidth,
      outlineStyle: style.outlineStyle,
      outlineOffset: style.outlineOffset,
    };
  });
  expect(hover.background).toBe(expectedBackground);
  expect(hover.foreground).toBe(expectedForeground);
  expect(hover.borderWidth).toBe('0px');
  expect(hover.borderRadius).toBe('0px');
  expect(hover.boxShadow).toBe('none');
  if (hover.focusVisible) {
    expect(hover.outlineWidth).toBe('2px');
    expect(hover.outlineStyle).toBe('solid');
    expect(hover.outlineOffset).toBe('-2px');
  } else {
    expect(hover.outlineStyle).toBe('none');
  }

  await page.mouse.move(320, 300);
  const rest = await locator.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      background: style.backgroundColor,
      foreground: style.color,
      borderWidth: style.borderTopWidth,
      borderStyle: style.borderTopStyle,
      borderRadius: style.borderRadius,
      boxShadow: style.boxShadow,
    };
  });
  expect(rest.background).toBe('rgba(0, 0, 0, 0)');
  expect(rest.foreground).toBe('rgb(179, 181, 184)');
  expect(rest.borderWidth).toBe('0px');
  expect(rest.borderRadius).toBe('0px');
  expect(rest.boxShadow).toBe('none');
}

test('Graphite caption roles preserve their hover states over full safe hit targets', async ({
  page,
}) => {
  await page.emulateMedia({ forcedColors: 'none' });
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/?mockPlaybackDurationMs=120000&mockUpdateState=current');
  const minimize = page.getByRole('button', { name: 'Minimize window' });
  await expect(minimize).toHaveAccessibleName('Minimize window');
  await expectCaptionHover(page, minimize, 'rgb(48, 49, 50)', 'rgb(226, 228, 232)');

  const maximize = page.getByRole('button', { name: 'Maximize window' });
  await expect(maximize).toHaveAccessibleName('Maximize window');
  await expectCaptionHover(page, maximize, 'rgba(0, 0, 0, 0)', 'rgb(179, 181, 184)');
  await maximize.click();

  const restore = page.getByRole('button', { name: 'Restore window' });
  await expect(restore).toBeVisible();
  await expect(restore).toHaveAccessibleName('Restore window');
  await expectCaptionHover(page, restore, 'rgba(0, 0, 0, 0)', 'rgb(179, 181, 184)');
  await restore.click();
  const restoredMaximize = page.getByRole('button', { name: 'Maximize window' });
  await expect(restoredMaximize).toBeVisible();

  const close = page.getByRole('button', { name: 'Close window' });
  await expect(close).toHaveAccessibleName('Close window');
  await expectCaptionHover(page, close, 'rgb(196, 43, 54)', 'rgb(255, 255, 255)');

  await page.keyboard.press('Tab');
  await expect(close).toBeFocused();
  const keyboardFocus = await close.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      visible: element.matches(':focus-visible'),
      outlineWidth: style.outlineWidth,
      outlineOffset: style.outlineOffset,
    };
  });
  expect(keyboardFocus.visible).toBe(true);
  expect(keyboardFocus.outlineWidth).toBe('2px');
  expect(keyboardFocus.outlineOffset).toBe('-2px');
  await close.hover();
  const focusedWhileHovered = await close.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      hovered: element.matches(':hover'),
      visible: element.matches(':focus-visible'),
      outlineWidth: style.outlineWidth,
    };
  });
  expect(focusedWhileHovered).toEqual({ hovered: true, visible: true, outlineWidth: '2px' });

  await page.keyboard.press('Shift+Tab');
  await expect(restoredMaximize).toBeFocused();
  const maximizeKeyboardFocus = await restoredMaximize.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      visible: element.matches(':focus-visible'),
      background: style.backgroundColor,
      outlineWidth: style.outlineWidth,
      outlineOffset: style.outlineOffset,
    };
  });
  expect(maximizeKeyboardFocus).toEqual({
    visible: true,
    background: 'rgba(0, 0, 0, 0)',
    outlineWidth: '2px',
    outlineOffset: '-2px',
  });
  await restoredMaximize.hover();
  const maximizeFocusedWhileHovered = await restoredMaximize.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      hovered: element.matches(':hover'),
      visible: element.matches(':focus-visible'),
      background: style.backgroundColor,
      outlineWidth: style.outlineWidth,
    };
  });
  expect(maximizeFocusedWhileHovered).toEqual({
    hovered: true,
    visible: true,
    background: 'rgba(0, 0, 0, 0)',
    outlineWidth: '2px',
  });
});

async function expectNonOverlappingGeometry(
  page: Page,
  expectedDpr: number,
  updateAvailable: boolean,
) {
  await expect(page.locator('[data-slot="workbench"]')).toHaveAttribute(
    'data-workbench-width',
    String(page.viewportSize()?.width),
  );
  await expect
    .poll(() =>
      page.evaluate(() => {
        const workbench = document.querySelector<HTMLElement>('[data-slot="workbench"]');
        const trackBrowser = workbench?.querySelector<HTMLElement>('[data-pane="browser"]');
        return Boolean(
          trackBrowser && trackBrowser.getBoundingClientRect().right <= innerWidth - 4,
        );
      }),
    )
    .toBe(true);
  const geometry = await readTitlebarGeometry(page);
  expect(geometry.viewport).toEqual({
    width: geometry.viewport.width,
    height: geometry.viewport.height,
    dpr: expectedDpr,
  });
  expect(viewports.map(({ width }) => width)).toContain(geometry.viewport.width);
  expect(geometry.updateAvailable).toBe(String(updateAvailable));
  expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewport.width);
  expect(geometry.dragRegion).toBe('true');
  expect(geometry.boxes.titlebar?.height).toBe(40);
  expect(geometry.boxes.search?.height).toBeGreaterThanOrEqual(30);
  expect(geometry.boxes.search?.height).toBeLessThanOrEqual(32);
  expect(geometry.trailingDragRegion).toBe('true');
  expect(
    geometry.workbench,
    `Workbench geometry at ${geometry.viewport.width}×${geometry.viewport.height}, DPR ${expectedDpr}: ${JSON.stringify(geometry.workbench)}`,
  ).toMatchObject({
    fits: 'true',
    gutter: '4',
    cssGutter: 4,
    leftInset: 4,
    rightInset: 4,
    bottomInset: 4,
    paneGap: 4,
  });
  if (geometry.viewport.width === 640) {
    // Graphite automatically collapses the Navigator at the compact threshold while preserving expandable drag capability.
    expect(geometry.workbench.separators.length).toBeLessThanOrEqual(1);
  } else {
    expect(geometry.workbench.separators).toHaveLength(1);
    expect(geometry.workbench.separators[0]).toMatchObject({
      width: 4,
      acquisition: [true, true, true],
    });
  }

  const search = geometry.boxes.search;
  const drag = geometry.boxes.drag;
  const trailingDrag = geometry.boxes.trailingDrag;
  const leading = geometry.boxes.leading;
  const brand = geometry.boxes.brand;
  const end = geometry.boxes.end;
  expect(search).not.toBeNull();
  expect(drag).not.toBeNull();
  expect(trailingDrag).not.toBeNull();
  expect(leading).not.toBeNull();
  expect(brand).not.toBeNull();
  expect(end).not.toBeNull();
  if (!search || !drag || !trailingDrag || !leading || !brand || !end) {
    throw new Error('Titlebar search, drag region, or occupied boundary is missing');
  }
  const titlebar = geometry.boxes.titlebar;
  expect(titlebar).not.toBeNull();
  if (!titlebar) throw new Error('Titlebar bounds are missing');
  expect(
    Math.abs(search.y + search.height / 2 - (titlebar.y + titlebar.height / 2)),
  ).toBeLessThanOrEqual(1);
  expect(drag.width).toBeGreaterThanOrEqual(24);
  expect(trailingDrag.width).toBeGreaterThanOrEqual(24);
  expect(drag.height).toBeGreaterThanOrEqual(search.height);
  expect(trailingDrag.height).toBeGreaterThanOrEqual(search.height);
  expect(brand.right).toBeLessThanOrEqual(drag.x + 0.5);
  expect(drag.right).toBeLessThanOrEqual(search.x + 0.5);
  expect(search.right).toBeLessThanOrEqual(trailingDrag.x + 0.5);
  expect(trailingDrag.right).toBeLessThanOrEqual(end.x + 0.5);
  expect(geometry.searchFreeRegion).not.toBeNull();
  if (geometry.viewport.width >= 1024) {
    expect(geometry.searchFreeRegion!.centerError).toBeLessThanOrEqual(1);
  }
  expect(geometry.searchInput?.width).toBeGreaterThan(48);
  if (geometry.viewport.width <= 700) {
    const firstRightControl = geometry.boxes.update ?? geometry.boxes.reload;
    expect(search.width).toBeGreaterThanOrEqual(260);
    if (firstRightControl) {
      expect(search.right).toBeLessThanOrEqual(firstRightControl.x - 24);
    }
  }

  const occupied = Object.entries(geometry.boxes).filter(
    ([name, box]) =>
      name !== 'titlebar' &&
      name !== 'drag' &&
      name !== 'trailingDrag' &&
      name !== 'leading' &&
      name !== 'end' &&
      box !== null,
  ) as Array<[string, NonNullable<(typeof geometry.boxes)[string]>]>;
  for (const [name, box] of occupied) {
    expect(box.x, `${name} starts inside the viewport`).toBeGreaterThanOrEqual(0);
    expect(box.y, `${name} starts inside the viewport`).toBeGreaterThanOrEqual(0);
    expect(box.right, `${name} ends inside the viewport`).toBeLessThanOrEqual(
      geometry.viewport.width,
    );
    expect(box.bottom, `${name} ends inside the viewport`).toBeLessThanOrEqual(
      geometry.viewport.height,
    );
  }
  for (let left = 0; left < occupied.length; left += 1) {
    for (let right = left + 1; right < occupied.length; right += 1) {
      const [leftName, leftBox] = occupied[left]!;
      const [rightName, rightBox] = occupied[right]!;
      const intersects =
        leftBox.x < rightBox.right &&
        leftBox.right > rightBox.x &&
        leftBox.y < rightBox.bottom &&
        leftBox.bottom > rightBox.y;
      expect(intersects, `${leftName} and ${rightName} do not overlap`).toBe(false);
    }
  }
  for (const [name, box] of occupied.filter(([name]) => name !== 'search')) {
    expect(box.width, `${name} has a visible width`).toBeGreaterThanOrEqual(
      name === 'minimize' || name === 'maximize' || name === 'close'
        ? 46
        : name === 'brand'
          ? 32
          : 34,
    );
    expect(box.height, `${name} has a visible height`).toBeGreaterThanOrEqual(
      name === 'minimize' || name === 'maximize' || name === 'close'
        ? 40
        : name === 'brand'
          ? 32
          : 34,
    );
  }
}

test('titlebar stays collision-free and pointer-operable across viewport, update, label, and scale matrices', async ({
  browser,
}) => {
  test.setTimeout(240_000);
  for (const deviceScaleFactor of scaleFactors) {
    const context = await browser.newContext({
      viewport: { width: 1366, height: 768 },
      deviceScaleFactor,
    });
    try {
      for (const updateAvailable of [true, false]) {
        const page = await context.newPage();
        await createLongPlaylistSource(page, updateAvailable);
        const search = page.getByRole('searchbox', { name: `Search ${longPlaylistName}` });

        for (const viewport of viewports) {
          await page.setViewportSize(viewport);
          await expectNonOverlappingGeometry(page, deviceScaleFactor, updateAvailable);
          await expectPointerCenter(page, page.locator('[data-slot="titlebar-drag-space"]'));
          await expectPointerCenter(
            page,
            page.locator('[data-slot="titlebar-drag-space-trailing"]'),
          );

          const searchInput = page.locator('[data-slot="global-search"] input');
          await expectPointerCenter(page, searchInput);
          await expect(searchInput).toBeFocused();
          await page.evaluate(() => {
            if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
          });
          await page.keyboard.press('/');
          await expect(search).toBeFocused();
          await search.fill('Moonlit');
          await expect(search).toHaveValue('Moonlit');
          await expect(search).toBeFocused();
          await expect
            .poll(() => search.evaluate((input: HTMLInputElement) => input.selectionStart))
            .toBe('Moonlit'.length);
          await search.fill('');

          if (updateAvailable) {
            const update = page.getByRole('button', { name: /Open update / });
            await expectPointerCenter(page, update);
            const updateDialog = page.getByRole('dialog');
            await expect(updateDialog).toBeVisible();
            await updateDialog.getByRole('button', { name: 'Close update' }).click();
            await expect(updateDialog).toBeHidden();
          }

          await expectPointerCenter(page, page.getByRole('button', { name: 'Reload library' }));
          await expect(page.locator('[data-pane="browser"]')).toBeVisible();
          await expectPointerCenter(page, page.getByRole('button', { name: 'Open settings' }));
          const settings = page.getByRole('dialog', { name: 'Settings' });
          await expect(settings).toBeVisible();
          await settings.getByRole('button', { name: 'Close settings' }).click();
          await expect(settings).toBeHidden();

          await expectPointerCenter(page, page.getByRole('button', { name: 'Minimize window' }));
          const maximize = page.getByRole('button', { name: 'Maximize window' });
          await expectPointerCenter(page, maximize);
          await expect(page.getByRole('button', { name: 'Restore window' })).toBeVisible();
          await expectPointerCenter(page, page.getByRole('button', { name: 'Restore window' }));
          await expectPointerCenter(page, page.getByRole('button', { name: 'Close window' }));
          await expect
            .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
            .toBeLessThanOrEqual(viewport.width);
        }
        await page.close();
      }
    } finally {
      await context.close();
    }
  }
});
