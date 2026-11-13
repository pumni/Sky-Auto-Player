import { expect, test } from '@playwright/test';

test('Built-in pager count has a readable left inset and no duplicate dock locator', async ({
  page,
}) => {
  for (const viewport of [
    { width: 987, height: 691 },
    { width: 1649, height: 927 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/?mockCatalogCount=112&mockPlaybackDurationMs=120000&mockStartDelayMs=10');

    const inset = await page.locator('.builtin-pager-range').evaluate((element) => {
      const wrap = element.closest('.builtin-pager-wrap');
      if (!wrap) throw new Error('Built-in pager footer is missing');
      return element.getBoundingClientRect().left - wrap.getBoundingClientRect().left;
    });
    expect(inset).toBeGreaterThanOrEqual(17);

    const locate = page.locator('.builtin-pager-wrap').getByRole('button', {
      name: 'Locate selected',
    });
    await expect(locate).toHaveCount(1);

    await page.getByRole('row', { name: /Blue Bird/ }).click();
    const dock = page.getByRole('contentinfo', { name: 'Player controls' });
    await dock.getByRole('button', { name: 'Play', exact: true }).click();
    await expect(dock.getByText('Playing', { exact: true })).toBeVisible();
    await expect(dock.getByRole('button', { name: /Show now playing/ })).toHaveCount(0);
    await expect(locate).toBeEnabled();
  }
});

test('Locate selected in the pager footer centers an off-screen row on demand', async ({
  page,
}) => {
  await page.setViewportSize({ width: 987, height: 691 });
  await page.goto('/?mockCatalogCount=112');

  const grid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  const selected = page.locator('.track-row[aria-rowindex="2"]');
  const locate = page.locator('.builtin-pager-wrap').getByRole('button', {
    name: 'Locate selected',
  });
  await expect(locate).toBeDisabled();
  await expect(locate.locator('svg')).toHaveClass(/lucide-locate-fixed/);
  const dock = page.getByRole('contentinfo', { name: 'Player controls' });
  await expect(dock.getByRole('button', { name: 'Locate selected' })).toHaveCount(0);
  await expect(dock.getByRole('button', { name: /Show now playing/ })).toHaveCount(0);
  await selected.click();
  await expect(selected).toHaveAttribute('aria-selected', 'true');
  await expect(locate).toBeEnabled();
  await expect(page.locator('.track-browser-locate-button')).toHaveCount(0);

  await grid.evaluate((element) => {
    element.scrollTop = 70 * 46;
  });
  await expect(selected).toHaveCount(0);
  await expect(locate).toBeEnabled();
  await locate.click();
  await expect(selected).toBeVisible();
  await expect(selected).toHaveAttribute('aria-selected', 'true');
  await expect(grid).toBeFocused();
  await expect(locate).toBeEnabled();
});

test('Locate selected in the pager footer restores the original built-in page', async ({
  page,
}) => {
  await page.setViewportSize({ width: 987, height: 691 });
  await page.goto('/?mockCatalogCount=112');

  const selected = page.locator('.track-row[aria-rowindex="2"]');
  const locate = page.locator('.builtin-pager-wrap').getByRole('button', {
    name: 'Locate selected',
  });
  await selected.click();
  await page.getByRole('button', { name: 'Next page' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  await expect(locate).toBeEnabled();
  await locate.click();

  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expect(selected).toBeVisible();
  await expect(selected).toHaveAttribute('aria-selected', 'true');
  await expect(locate).toBeEnabled();
});
