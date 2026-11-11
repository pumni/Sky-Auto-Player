import { expect, test } from '@playwright/test';

test('Locate selected returns to an off-screen row without scrolling the list automatically', async ({
  page,
}) => {
  await page.setViewportSize({ width: 987, height: 691 });
  await page.goto('/?mockCatalogCount=112');

  const grid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  const selected = page.getByRole('row', { name: /Aurora Landing/ });
  await selected.click();
  await expect(selected).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('button', { name: 'Locate selected' })).toHaveCount(0);

  await grid.evaluate((element) => {
    element.scrollTop = 70 * 46;
  });
  const locate = page.getByRole('button', { name: 'Locate selected' });
  await expect(locate).toBeVisible();
  await expect(selected).toHaveCount(0);
  await locate.click();
  await expect(selected).toBeVisible();
  await expect(selected).toHaveAttribute('aria-selected', 'true');
  await expect(grid).toBeFocused();
  await expect(locate).toHaveCount(0);
});

test('Locate selected restores the original built-in page after browsing another page', async ({
  page,
}) => {
  await page.setViewportSize({ width: 987, height: 691 });
  await page.goto('/?mockCatalogCount=112');

  const selected = page.getByRole('row', { name: /Aurora Landing/ });
  await selected.click();
  await page.getByRole('button', { name: 'Next page' }).click();
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  const locate = page.getByRole('button', { name: 'Locate selected' });
  await expect(locate).toBeVisible();
  await locate.click();

  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeVisible();
  await expect(selected).toBeVisible();
  await expect(selected).toHaveAttribute('aria-selected', 'true');
  await expect(locate).toHaveCount(0);
});
