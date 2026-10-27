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
      document.querySelector(selector)!.getBoundingClientRect().width;
    return {
      navigator: width('.navigator-workbench-pane'),
      tracks: width('.track-browser-workbench-pane'),
      inspector: width('.utility-workbench-pane'),
    };
  });
}

test('Inspector 388/389/388 stays expanded, caps at 388, and persists once', async ({
  page,
}, info) => {
  await seed(page, 1024);
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
  const trace = [];
  for (const offset of [0, -1, 0]) {
    await page.mouse.move(box.x + box.width / 2 + offset, box.y + 80);
    await expect.poll(() => bounds(page)).toEqual({ navigator: 200, tracks: 420, inspector: 388 });
    trace.push({ requested: 388 - offset, ...(await bounds(page)) });
  }
  expect(await page.evaluate(() => Reflect.get(window, 'resizeWrites'))).toBe(0);
  await page.mouse.up();
  await expect(page.locator('body')).not.toHaveClass(/is-resizing/);
  expect(await page.evaluate(() => Reflect.get(window, 'resizeWrites'))).toBe(1);
  await info.attach('geometry-oracle', {
    body: JSON.stringify(trace, null, 2),
    contentType: 'application/json',
  });
  await page.screenshot({ path: '../docs/evidence/issue-553/inspector-clamped-1024.png' });
  await page.reload();
  expect(await bounds(page)).toEqual({ navigator: 200, tracks: 420, inspector: 388 });
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

test('wide and narrow drag geometry, overlay focus, and v5 preference survive viewport changes', async ({
  page,
}) => {
  await seed(page, 1366);
  const separator = page.getByRole('separator', { name: 'Resize utility pane' });
  await separator.focus();
  await page.keyboard.press('End');
  expect(await bounds(page)).toEqual({ navigator: 300, tracks: 610, inspector: 440 });
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
  // An uncommitted live width may remain in the mounted layout; storage owns the roundtrip.
  await expect(separator).toHaveAttribute('aria-valuenow', '350');
  await page.reload();
  await expect(separator).toHaveAttribute('aria-valuenow', '388');
});
