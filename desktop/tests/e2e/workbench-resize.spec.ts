import { expect, test, type Page } from '@playwright/test';

const key = 'sky.ui.workbench.v5';
async function seed(page: Page, width: number, collapsed = false) {
  await page.setViewportSize({ width, height: 768 });
  await page.goto('/');
  await page.evaluate(
    ({ key, collapsed }) => {
      localStorage.clear();
      localStorage.setItem(
        key,
        JSON.stringify({
          version: 5,
          navigatorPreference: collapsed ? 'collapsed' : 'expanded',
          expandedNavigatorWidth: 300,
          utilityWidth: 388,
          utilityOpen: true,
        }),
      );
    },
    { key, collapsed },
  );
  await page.reload();
  await expect(page.getByRole('separator', { name: 'Resize utility pane' })).toBeVisible();
}
async function bounds(page: Page) {
  return page.evaluate(() => {
    const width = (selector: string) =>
      document.querySelector(selector)?.getBoundingClientRect().width ?? null;
    return {
      navigator: width('.navigator-workbench-pane'),
      tracks: width('.track-browser-workbench-pane'),
      inspector: width('.utility-workbench-pane'),
    };
  });
}

for (const pane of ['utility', 'navigator'] as const) {
  test(`continuous ${pane} drag keeps CSS, ARIA, focus and preferences synchronized`, async ({
    page,
  }) => {
    await seed(page, 1366);
    const utility = page.getByRole('separator', { name: 'Resize utility pane' });
    const navigator = page.getByRole('separator', { name: 'Resize library navigator' });
    await utility.press('Home');
    await navigator.press('Enter');
    const separator = pane === 'utility' ? utility : navigator;
    const before = await page.evaluate((key) => localStorage.getItem(key), key);
    const box = (await separator.boundingBox())!;
    const x = box.x + box.width / 2;
    await page.mouse.move(x, box.y + 100);
    await page.mouse.down();
    for (const delta of [8, 16, 32, 48, 64, 48, 32, 16, 8]) {
      await page.mouse.move(x + (pane === 'utility' ? -delta : delta), box.y + 100);
      const width = (pane === 'utility' ? 280 : 240) + delta;
      await expect(separator).toHaveAttribute('aria-valuenow', String(width));
      await expect
        .poll(() => bounds(page))
        .toEqual({
          navigator: pane === 'navigator' ? width : 240,
          inspector: pane === 'utility' ? width : 280,
          tracks: 830 - delta,
        });
      // Re-render the parent through a genuine utility state change while capture remains active.
      await page
        .getByRole('tab', { name: delta === 32 ? 'Runtime' : 'Details', exact: true })
        .evaluate((element) => (element as HTMLElement).click());
      await expect(separator).toHaveAttribute('aria-valuenow', String(width));
      // Pane width must not become an inherited input to every row/content node.
      expect(
        await page
          .locator(pane === 'utility' ? '.utility-surface' : '.library-navigator')
          .evaluate(
            (element, property) => getComputedStyle(element).getPropertyValue(property),
            pane === 'utility' ? '--utility-pane-width' : '--navigator-pane-width',
          ),
      ).toBe(pane === 'utility' ? '320px' : '240px');
      expect(await page.evaluate((key) => localStorage.getItem(key), key)).toBe(before);
    }
    await page.mouse.up();
    await expect(page.locator('body')).not.toHaveClass(/is-resizing/);
    expect(await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!), key)).toMatchObject(
      pane === 'utility'
        ? { utilityWidth: 288, expandedNavigatorWidth: 240 }
        : { utilityWidth: 280, expandedNavigatorWidth: 248 },
    );
    await separator.focus();
    await page.keyboard.press('Enter');
    await expect(separator).toHaveAttribute('aria-valuenow', pane === 'utility' ? '320' : '240');
  });
}

test('all palettes preserve Inspector 388/389/388 geometry and persist once', async ({
  page,
}, info) => {
  test.setTimeout(180_000);
  const trace = [];
  for (const palette of ['graphite', 'aurora', 'minimalist', 'slate', 'cyberpunk', 'classic']) {
    await seed(page, 1024);
    await page.locator('html').evaluate((root, value) => {
      (root as HTMLElement).dataset.palette = value;
    }, palette);
    const separator = page.getByRole('separator', { name: 'Resize utility pane' });
    const box = (await separator.boundingBox())!;
    await page.evaluate(() => {
      const original = Storage.prototype.setItem;
      Object.assign(window, { resizeWrites: 0 });
      Storage.prototype.setItem = function (key, value) {
        if (key === 'sky.ui.workbench.v5')
          Object.assign(window, { resizeWrites: Reflect.get(window, 'resizeWrites') + 1 });
        return original.call(this, key, value);
      };
    });
    await page.mouse.move(box.x + box.width / 2, box.y + 80);
    await page.mouse.down();
    const paletteTrace = [];
    for (const offset of [0, -1, 0]) {
      await page.mouse.move(box.x + box.width / 2 + offset, box.y + 80);
      await expect
        .poll(() => bounds(page))
        .toEqual({ navigator: 200, tracks: 420, inspector: 388 });
      paletteTrace.push({ requested: 388 - offset, ...(await bounds(page)) });
    }
    expect(await page.evaluate(() => Reflect.get(window, 'resizeWrites'))).toBe(0);
    await page.mouse.up();
    await expect(page.locator('body')).not.toHaveClass(/is-resizing/);
    expect(await page.evaluate(() => Reflect.get(window, 'resizeWrites'))).toBe(1);
    trace.push({ palette, samples: paletteTrace });
    if (palette === 'graphite' && process.env.WORKBENCH_RESIZE_CAPTURE === '1') {
      await info.attach('inspector-clamped-1024', {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
    }
  }
  await info.attach('geometry-oracle', {
    body: JSON.stringify(trace, null, 2),
    contentType: 'application/json',
  });
  const separator = page.getByRole('separator', { name: 'Resize utility pane' });
  await page.reload();
  await expect(separator).toBeVisible();
  await expect.poll(() => bounds(page)).toEqual({ navigator: 200, tracks: 420, inspector: 388 });
  await separator.focus();
  await page.keyboard.press('ArrowRight');
  await expect(separator).toHaveAttribute('aria-valuenow', '380');
  await page.keyboard.press('End');
  await expect(separator).toHaveAttribute('aria-valuenow', '388');
  await page.keyboard.press('Enter');
  await expect(separator).toHaveAttribute('aria-valuenow', '320');
  await page.getByRole('button', { name: 'Collapse library navigator' }).click();
  await separator.focus();
  await page.keyboard.press('End');
  await expect.poll(() => bounds(page)).toEqual({ navigator: 48, tracks: 520, inspector: 440 });
  await page.keyboard.press('Tab');
  await expect(separator).not.toBeFocused();
});

test('all palette IDs share workbench geometry across the dock and overlay thresholds', async ({
  page,
}) => {
  test.setTimeout(120_000);
  await seed(page, 1366);
  for (const width of [1366, 1024, 800, 764, 763, 640]) {
    await page.setViewportSize({ width, height: width === 640 ? 448 : 768 });
    const docked = width >= 764;
    const expectedInspector = docked
      ? Math.min(388, Math.max(0, width - 484))
      : Math.min(width - 16, Math.min(352, Math.max(288, Math.floor(width * 0.45))));
    await expect
      .poll(() =>
        page
          .locator('.utility-workbench-pane')
          .evaluate((element) => element.getBoundingClientRect().width),
      )
      .toBe(expectedInspector);
    let sharedGeometry: unknown;
    for (const palette of ['graphite', 'aurora', 'minimalist', 'slate', 'cyberpunk', 'classic']) {
      await page.locator('html').evaluate((root, value) => {
        (root as HTMLElement).dataset.palette = value;
      }, palette);
      await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
      const geometry = await page.evaluate(() => {
        const rect = (selector: string) => {
          const element = document.querySelector<HTMLElement>(selector);
          if (!element) return null;
          const bounds = element.getBoundingClientRect();
          return [bounds.x, bounds.y, bounds.width, bounds.height];
        };
        return {
          titlebar: rect('.app-titlebar'),
          workbench: rect('.workbench'),
          navigator: rect('.navigator-workbench-pane'),
          tracks: rect('.track-browser-workbench-pane'),
          inspector: rect('.utility-workbench-pane'),
          trackHeader: rect('.track-browser-header'),
          trackRow: rect('.track-row'),
          player: rect('.player-bar'),
          separators: Array.from(
            document.querySelectorAll<HTMLElement>('.resizable-separator'),
          ).map((separator) => {
            const bounds = separator.getBoundingClientRect();
            return [bounds.x, bounds.y, bounds.width, bounds.height];
          }),
          presentation:
            document.querySelector<HTMLElement>('.workbench')?.dataset.utilityPresentation,
        };
      });
      expect(geometry.presentation).toBe(width < 764 ? 'overlay' : 'docked');
      if (sharedGeometry === undefined) sharedGeometry = geometry;
      else expect(geometry, `${width}px using ${palette}`).toEqual(sharedGeometry);
    }
  }
});

for (const [viewport, inspector] of [
  [900, 264],
  [915, 279],
  [916, 280],
  [917, 281],
] as const) {
  test(`release preserves docked geometry at ${viewport}px and v5 survives reopen`, async ({
    page,
  }) => {
    await seed(page, 1024);
    const separator = page.getByRole('separator', { name: 'Resize utility pane' });
    const box = (await separator.boundingBox())!;
    await page.mouse.move(box.x + 2, box.y + 100);
    await page.mouse.down();
    await page.setViewportSize({ width: viewport!, height: 768 });
    const geometry = { navigator: 200, tracks: 420, inspector };
    await expect.poll(() => bounds(page)).toEqual(geometry);
    await page.mouse.up();
    await expect.poll(() => bounds(page)).toEqual(geometry);
    expect(await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!), key)).toMatchObject(
      {
        version: 5,
        navigatorPreference: 'expanded',
        utilityWidth: Math.max(280, inspector!),
      },
    );
    const trigger = page.locator('.player-utility-button');
    await trigger.click();
    await trigger.click();
    await expect(separator).toBeVisible();
    await expect.poll(() => bounds(page)).toEqual(geometry);
    await page.setViewportSize({ width: 1366, height: 768 });
    await expect
      .poll(() => bounds(page))
      .toEqual({
        navigator: 300,
        tracks: 1050 - Math.max(280, inspector!),
        inspector: Math.max(280, inspector!),
      });
  });
}

test('wide and narrow drag geometry, overlay focus, and v5 preference survive viewport changes', async ({
  page,
}) => {
  await seed(page, 1366);
  const separator = page.getByRole('separator', { name: 'Resize utility pane' });
  await separator.focus();
  await page.keyboard.press('End');
  await expect.poll(() => bounds(page)).toEqual({ navigator: 300, tracks: 610, inspector: 440 });
  await page.keyboard.press('Enter');
  await page.setViewportSize({ width: 960, height: 768 });
  await expect(separator).toHaveAttribute('aria-valuemax', '324');
  const box = (await separator.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + 80);
  await page.mouse.down();
  await page.mouse.move(box.x - 150, box.y + 80);
  await expect.poll(() => bounds(page)).toEqual({ navigator: 200, tracks: 420, inspector: 324 });
  await page.setViewportSize({ width: 900, height: 768 });
  await expect.poll(() => bounds(page)).toEqual({ navigator: 200, tracks: 420, inspector: 264 });
  await page.mouse.up();
  await expect.poll(() => bounds(page)).toEqual({ navigator: 200, tracks: 420, inspector: 264 });
  const saved = await page.evaluate((key) => localStorage.getItem(key), key);
  await page.setViewportSize({ width: 640, height: 448 });
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(page.locator('body')).not.toHaveClass(/is-resizing/);
  await page.getByRole('button', { name: 'Close utility pane' }).click();
  await expect(page.locator('.player-utility-button')).toBeFocused();
  await page.setViewportSize({ width: 1024, height: 768 });
  const persisted = JSON.parse(saved!);
  expect(persisted).toMatchObject({
    version: 5,
    navigatorPreference: 'expanded',
    expandedNavigatorWidth: 300,
  });
  expect(JSON.parse((await page.evaluate((key) => localStorage.getItem(key), key))!)).toMatchObject(
    {
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 300,
    },
  );
  const navigatorSeparator = page.getByRole('separator', { name: 'Resize library navigator' });
  await navigatorSeparator.focus();
  await page.keyboard.press('Home');
  await expect(navigatorSeparator).toHaveAttribute('aria-valuenow', '200');
  await page.keyboard.press('Shift+ArrowRight');
  await expect(navigatorSeparator).toHaveAttribute('aria-valuenow', '232');
});

test('closing the Inspector during capture cleans up without persisting a stale drag', async ({
  page,
}) => {
  await seed(page, 1024);
  const separator = page.getByRole('separator', { name: 'Resize utility pane' });
  const box = (await separator.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + 80);
  await page.mouse.down();
  await page.mouse.move(box.x + 40, box.y + 80);
  await expect(separator).toHaveAttribute('aria-valuenow', '350');
  const trigger = page.locator('.player-utility-button');
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(separator).toHaveCount(0);
  await expect(page.locator('body')).not.toHaveClass(/is-resizing/);
  await page.mouse.up();
  expect(
    await page.evaluate((key) => JSON.parse(localStorage.getItem(key)!).utilityWidth, key),
  ).toBe(388);
  await page.keyboard.press('Enter');
  // Closing discards the session-owned visual width; preferences were never changed.
  await expect(separator).toHaveAttribute('aria-valuenow', '388');
  await page.reload();
  await expect(separator).toHaveAttribute('aria-valuenow', '388');
});
