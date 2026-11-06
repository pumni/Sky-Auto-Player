import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

test('fresh System mode follows the OS while manual mode and dark palette stay independent', async ({
  page,
}) => {
  const root = page.locator('html');
  await expect(root).toHaveAttribute('data-color-mode', 'dark');

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click();
  const theme = settings.getByLabel('Theme');
  const palette = settings.getByLabel('Dark palette');

  await expect(theme).toHaveValue('system');
  await expect(palette).toHaveValue('graphite');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(root).toHaveAttribute('data-palette', 'graphite');

  await palette.selectOption('slate');
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(root).toHaveAttribute('data-palette', 'slate');

  await theme.selectOption('dark');
  await expect(root).toHaveAttribute('data-color-mode', 'dark');
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(root).toHaveAttribute('data-color-mode', 'dark');

  await theme.selectOption('system');
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  expect(
    await page.evaluate(() =>
      Object.keys(localStorage).some((key) => /appearance|color.?mode|theme/i.test(key)),
    ),
  ).toBe(false);
});
