import { expect, test } from '@playwright/test';

test('mock desktop vertical slice can search and inspect a song', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('option', { name: /Aurora Landing/ })).toBeVisible();
  await page.getByLabel('Search songs').fill('Moonlit');
  await expect(page.getByRole('option', { name: /Moonlit Village/ })).toBeVisible();
  await page.getByRole('option', { name: /Moonlit Village/ }).click();
  await expect(page.getByText('Low timing risk')).toBeVisible();
});

test('virtualized library pages beyond the first 200 songs', async ({ page }) => {
  await page.goto('/');
  const list = page.locator('.virtual-list');
  await expect(page.getByText('500 songs')).toBeVisible();
  await list.evaluate((element) => {
    element.scrollTop = 400 * 44;
    element.dispatchEvent(new Event('scroll'));
  });
  const song = page.getByRole('option', { name: /Song 401/ });
  await expect(song).toBeVisible();
  await song.click();
  await expect(page.getByRole('heading', { name: 'Song 401' })).toBeVisible();
});
