import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

async function expectNoSeriousAccessibilityViolations(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.filter(
      (violation) => violation.impact === 'critical' || violation.impact === 'serious',
    ),
  ).toEqual([]);
}

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

test('default Library has no serious accessibility violations', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  await expect(page.getByRole('listbox', { name: 'Songs' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
});

test('selected Song Detail has no serious accessibility violations', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('option', { name: /Aurora Landing/ }).click();
  await expect(page.getByRole('heading', { name: 'Aurora Landing' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
});

test('Player Dock completes a dry-run lifecycle accessibly', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  await page.getByRole('option', { name: /Aurora Landing/ }).click();

  await page.getByRole('button', { name: 'Play' }).click();
  const confirmation = page.getByRole('dialog', { name: 'Playback confirmation' });
  await expect(confirmation).toBeVisible();
  await confirmation.getByRole('button', { name: 'Proceed with current settings' }).click();

  await expect(page.getByRole('button', { name: 'Pause' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  await page.getByRole('button', { name: 'Pause' }).click();
  await expect(page.getByRole('button', { name: 'Resume' })).toBeVisible();
  await page.getByRole('button', { name: 'Resume' }).click();
  await expect(page.getByRole('button', { name: 'Pause' })).toBeVisible();
  await page.getByRole('button', { name: 'Stop' }).click();
  await expect(page.getByRole('button', { name: 'Play' })).toBeVisible();
});

test('Settings modal has no serious accessibility violations and closes accessibly', async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Playback defaults');
  await expectNoSeriousAccessibilityViolations(page);

  const focusables = dialog.locator('button, select, input');
  await focusables.first().focus();
  await page.keyboard.press('Shift+Tab');
  await expect(dialog.locator(':focus')).toHaveCount(1);
  await focusables.last().focus();
  await page.keyboard.press('Tab');
  await expect(dialog.locator(':focus')).toHaveCount(1);

  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(settingsButton).toBeFocused();
});
