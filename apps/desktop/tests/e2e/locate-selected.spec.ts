import { expect, test } from '@playwright/test';

test('Locate selected in the pager footer centers an off-screen row on demand', async ({ page }) => {
  await page.setViewportSize({ width: 987, height: 691 });
  await page.goto('/?mockCatalogCount=112');

  const grid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  const selected = page.locator('.track-row[aria-rowindex="2"]');
  const locate = page.locator('.builtin-pager-wrap').getByRole('button', {
    name: 'Locate selected',
  });
  await expect(locate).toBeDisabled();
  await expect(locate.locator('svg')).toHaveClass(/lucide-target/);
  await expect(page.getByRole('contentinfo', { name: 'Player controls' }).getByRole('button', { name: 'Locate selected' })).toHaveCount(0);
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

test('Locate selected in the pager footer restores the original built-in page', async ({ page }) => {
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
