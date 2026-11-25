import { expect, test } from '@playwright/test';

test('production bundle boots the mock shell and supports a representative flow', async ({
  page,
}) => {
  const featureChunks = new Set<string>();
  const featureChunkPattern = /\/assets\/(?:SettingsPanel|DiagnosticsView|UpdateDialog)-[^/]+\.js$/;
  page.on('request', (request) => {
    if (request.resourceType() === 'script' && featureChunkPattern.test(request.url())) {
      featureChunks.add(request.url());
    }
  });

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Built-in Songs' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  const songsGrid = page.getByRole('grid', { name: 'Built-in Songs, page 1' });
  await expect(songsGrid).toBeVisible();
  expect(featureChunks.size).toBe(0);

  await page.getByLabel('Search Built-in Songs').fill('Moonlit');
  await expect(songsGrid).toHaveAttribute('aria-rowcount', '2');
  const moonlitRow = page.getByRole('row', { name: /Moonlit Village/ });
  await expect(moonlitRow).toBeVisible();
  await moonlitRow.getByText('Moonlit Village', { exact: true }).click();
  await expect(moonlitRow).toHaveAttribute('aria-selected', 'true');
  await expect(songsGrid).toBeFocused();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByText('Low timing risk')).toBeVisible();
  expect(featureChunks.size).toBe(0);

  await page.getByRole('tab', { name: 'Runtime' }).click();
  await expect(page.getByRole('tab', { name: 'Performance' })).toBeVisible();
  expect([...featureChunks].some((url) => /DiagnosticsView-/.test(url))).toBe(true);

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings).toBeVisible();
  await expect(page.getByText('500 µs')).toBeVisible();
  expect([...featureChunks].some((url) => /SettingsPanel-/.test(url))).toBe(true);
  await settings.getByRole('button', { name: 'Close settings' }).click();
  await expect(settings).toBeHidden();
  await page.getByRole('button', { name: /Open update/ }).click();
  const updateDialog = page
    .getByRole('dialog')
    .filter({ has: page.getByRole('button', { name: 'Close update' }) });
  await expect(updateDialog).toBeVisible();
  expect([...featureChunks].some((url) => /UpdateDialog-/.test(url))).toBe(true);
});
