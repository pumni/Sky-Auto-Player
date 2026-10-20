import { expect, test, type Locator, type Page } from '@playwright/test';

const viewports = [
  { width: 640, height: 448 },
  { width: 800, height: 560 },
  { width: 1024, height: 640 },
  { width: 1366, height: 768 },
  { width: 1920, height: 1080 },
] as const;
const scaleFactors = [1, 1.25, 1.5] as const;
const longPlaylistName = 'Long playlist scope label for responsive titlebar geometry coverage';

async function createLongPlaylistSource(page: Page, updateAvailable: boolean) {
  const updateFixture = updateAvailable ? '' : '&mockUpdateState=current';
  await page.goto(`/?mockPlaybackDurationMs=120000${updateFixture}`);
  const titlebar = page.locator('.app-titlebar');
  await expect(titlebar).toHaveAttribute('data-update-available', String(updateAvailable));

  await page.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill(longPlaylistName);
  await createDialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByRole('searchbox', { name: `Search ${longPlaylistName}` })).toBeVisible();
}

async function readTitlebarGeometry(page: Page) {
  return page.evaluate(() => {
    const selectors = {
      titlebar: '.app-titlebar',
      search: '.global-search',
      update: '.update-indicator',
      reload: 'button[aria-label="Reload library"]',
      settings: 'button[aria-label="Open settings"]',
      minimize: 'button[aria-label="Minimize window"]',
      maximize: 'button[aria-label="Maximize window"], button[aria-label="Restore window"]',
      close: 'button[aria-label="Close window"]',
      drag: '.titlebar-drag-space',
      trailingDrag: '.titlebar-drag-space-trailing',
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
    const workbench = document.querySelector<HTMLElement>('.workbench');
    const navigator = document.querySelector<HTMLElement>('.navigator-workbench-pane');
    const trackBrowser = document.querySelector<HTMLElement>('.track-browser-workbench-pane');
    const workbenchRect = workbench?.getBoundingClientRect();
    const navigatorRect = navigator?.getBoundingClientRect();
    const trackBrowserRect = trackBrowser?.getBoundingClientRect();
    const separators = Array.from(
      document.querySelectorAll<HTMLElement>('.resizable-separator:not(.is-disabled)'),
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
        .querySelector('.app-titlebar')
        ?.getAttribute('data-update-available'),
      dragRegion: document
        .querySelector('.titlebar-drag-space')
        ?.getAttribute('data-tauri-drag-region'),
      trailingDragRegion: document
        .querySelector('.titlebar-drag-space-trailing')
        ?.getAttribute('data-tauri-drag-region'),
      searchInput: document.querySelector<HTMLInputElement>('.global-search input')
        ? {
            width: document
              .querySelector<HTMLInputElement>('.global-search input')!
              .getBoundingClientRect().width,
            height: document
              .querySelector<HTMLInputElement>('.global-search input')!
              .getBoundingClientRect().height,
          }
        : null,
      workbench: {
        fits: workbench?.dataset.layoutFits,
        gutter: workbench?.dataset.workbenchGap,
        cssGutter: getComputedStyle(document.documentElement)
          .getPropertyValue('--workbench-gap')
          .trim(),
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

async function expectNonOverlappingGeometry(
  page: Page,
  expectedDpr: number,
  updateAvailable: boolean,
) {
  await expect(page.locator('.workbench')).toHaveAttribute(
    'data-workbench-width',
    String(page.viewportSize()?.width),
  );
  await expect
    .poll(() =>
      page.evaluate(() => {
        const workbench = document.querySelector<HTMLElement>('.workbench');
        const trackBrowser = workbench?.querySelector<HTMLElement>('.track-browser-workbench-pane');
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
  expect(geometry.boxes.search?.height).toBeGreaterThanOrEqual(34);
  expect(
    geometry.workbench,
    `Workbench geometry at ${geometry.viewport.width}×${geometry.viewport.height}, DPR ${expectedDpr}: ${JSON.stringify(geometry.workbench)}`,
  ).toMatchObject({
    fits: 'true',
    gutter: '4',
    cssGutter: '4px',
    leftInset: 4,
    rightInset: 4,
    bottomInset: 4,
    paneGap: 4,
  });
  if (geometry.viewport.width === 640) {
    // Graphite automatically collapses and disables Navigator resizing at the compact threshold.
    expect(geometry.workbench.separators).toHaveLength(0);
  } else {
    expect(geometry.workbench.separators).toHaveLength(1);
    expect(geometry.workbench.separators[0]).toMatchObject({
      width: 4,
      acquisition: [true, true, true],
    });
  }

  const search = geometry.boxes.search;
  const drag = geometry.boxes.drag;
  expect(search).not.toBeNull();
  expect(drag).not.toBeNull();
  if (!search || !drag) throw new Error('Titlebar search or drag region is missing');
  expect(drag.width).toBeGreaterThanOrEqual(48);
  expect(drag.height).toBeGreaterThanOrEqual(geometry.boxes.search?.height ?? 0);
  expect(geometry.searchInput?.width).toBeGreaterThan(48);
  if (geometry.viewport.width <= 700) {
    const trailingDrag = geometry.boxes.trailingDrag;
    const firstRightControl = geometry.boxes.update ?? geometry.boxes.reload;
    expect(trailingDrag).not.toBeNull();
    expect(geometry.trailingDragRegion).toBe('true');
    expect(trailingDrag?.width).toBeGreaterThanOrEqual(24);
    expect(search.width).toBeGreaterThanOrEqual(260);
    if (firstRightControl) {
      expect(search.right).toBeLessThanOrEqual(firstRightControl.x - 24);
    }
  } else {
    expect(Math.abs(search.x + search.width / 2 - geometry.viewport.width / 2)).toBeLessThanOrEqual(
      0.5,
    );
    expect(geometry.boxes.trailingDrag).toBeNull();
  }

  const occupied = Object.entries(geometry.boxes).filter(
    ([name, box]) =>
      name !== 'titlebar' && name !== 'drag' && name !== 'trailingDrag' && box !== null,
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
      name === 'minimize' || name === 'maximize' || name === 'close' ? 46 : 34,
    );
    expect(box.height, `${name} has a visible height`).toBeGreaterThanOrEqual(
      name === 'minimize' || name === 'maximize' || name === 'close' ? 40 : 34,
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
          await expectPointerCenter(page, page.locator('.titlebar-drag-space'));
          if (viewport.width <= 700) {
            await expectPointerCenter(page, page.locator('.titlebar-drag-space-trailing'));
          }

          const searchInput = page.locator('.global-search input');
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
            const update = page.locator('.update-indicator');
            await expectPointerCenter(page, update);
            const updateDialog = page.getByRole('dialog', { name: 'Software update' });
            await expect(updateDialog).toBeVisible();
            await updateDialog.getByRole('button', { name: 'Close update' }).click();
          }

          await expectPointerCenter(page, page.getByRole('button', { name: 'Reload library' }));
          await expect(page.locator('.track-browser-workbench-pane')).toBeVisible();
          await expectPointerCenter(page, page.getByRole('button', { name: 'Open settings' }));
          const settings = page.getByRole('dialog', { name: 'Settings' });
          await expect(settings).toBeVisible();
          await settings.getByRole('button', { name: 'Close settings' }).click();

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
