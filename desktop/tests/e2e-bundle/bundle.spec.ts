import { expect, test } from '@playwright/test';

test('production bundle boots the mock shell and supports a representative flow', async ({
  page,
}) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'All Songs' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();

  await page.getByLabel('Search library').fill('Moonlit');
  await expect(page.getByRole('row', { name: /Moonlit Village/ })).toBeVisible();
  await page.getByRole('row', { name: /Moonlit Village/ }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByText('Low timing risk')).toBeVisible();
});
