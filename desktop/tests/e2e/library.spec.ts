import { expect, test } from '@playwright/test';

test('mock desktop vertical slice can search and inspect a song', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('option', { name: /Aurora Landing/ })).toBeVisible();
  await page.getByLabel('Search songs').fill('Moonlit');
  await expect(page.getByRole('option', { name: /Moonlit Village/ })).toBeVisible();
  await page.getByRole('option', { name: /Moonlit Village/ }).click();
  await expect(page.getByText('Low timing risk')).toBeVisible();
});
