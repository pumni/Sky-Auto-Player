import { expect, test } from '@playwright/test';

test('production bundle boots the mock shell and supports a representative flow', async ({
  page,
}) => {
  const featureChunks = new Set<string>();
  const featureChunkPattern =
    /\/assets\/(?:SettingsPanel|DiagnosticsView|CalibrationDialog|UpdateDialog)-[^/]+\.js$/;
  page.on('request', (request) => {
    if (request.resourceType() === 'script' && featureChunkPattern.test(request.url())) {
      featureChunks.add(request.url());
    }
  });

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'All Songs' })).toBeVisible();
  await expect(page.getByRole('row', { name: /Aurora Landing/ })).toBeVisible();
  expect(featureChunks.size).toBe(0);

  await page.getByLabel('Search library').fill('Moonlit');
  await expect(page.getByRole('row', { name: /Moonlit Village/ })).toBeVisible();
  await page.getByRole('row', { name: /Moonlit Village/ }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByText('Low timing risk')).toBeVisible();
  expect(featureChunks.size).toBe(0);

  await page.getByRole('tab', { name: 'Runtime' }).click();
  await expect(page.getByRole('tab', { name: 'Performance' })).toBeVisible();
  expect([...featureChunks].some((url) => /DiagnosticsView-/.test(url))).toBe(true);

  await page.getByRole('button', { name: 'Open settings' }).click();
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
  expect([...featureChunks].some((url) => /SettingsPanel-/.test(url))).toBe(true);

  await page.getByRole('button', { name: 'Advanced' }).click();
  await page.getByRole('button', { name: 'Open calibration' }).click();
  await expect(page.getByRole('dialog', { name: 'Timing calibration' })).toBeVisible();
  expect([...featureChunks].some((url) => /CalibrationDialog-/.test(url))).toBe(true);

  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: /Open update/ }).click();
  await expect(page.getByRole('dialog', { name: 'Software update' })).toBeVisible();
  expect([...featureChunks].some((url) => /UpdateDialog-/.test(url))).toBe(true);
});
