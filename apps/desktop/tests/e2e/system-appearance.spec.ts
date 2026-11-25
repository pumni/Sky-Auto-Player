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
  const palette = settings.getByRole('combobox', { name: 'Dark palette' });
  const appearanceSummary = settings.locator('[data-slot="appearance-active-summary"]');
  const preview = settings.locator('[data-slot="appearance-preview"]');
  const assertPreviewUsesRuntimeTokens = async () => {
    const colors = await page.evaluate(() => {
      const preview = document.querySelector<HTMLElement>('[data-slot="appearance-preview"]');
      const action = document.querySelector<HTMLElement>(
        '[data-slot="appearance-preview-primary-action"]',
      );
      if (!preview || !action) return null;
      const tokenColor = (property: string, token: string) => {
        const probe = document.createElement('span');
        probe.style.setProperty(property, `var(${token})`);
        document.body.append(probe);
        const value = getComputedStyle(probe).getPropertyValue(property);
        probe.remove();
        return value;
      };
      return {
        background: getComputedStyle(preview).backgroundColor,
        expectedBackground: tokenColor('background-color', '--background'),
        action: getComputedStyle(action).backgroundColor,
        expectedAction: tokenColor('background-color', '--primary'),
        actionText: getComputedStyle(action).color,
        expectedActionText: tokenColor('color', '--primary-foreground'),
      };
    });
    expect(colors).not.toBeNull();
    if (colors) {
      expect(colors.background).toBe(colors.expectedBackground);
      expect(colors.action).toBe(colors.expectedAction);
      expect(colors.actionText).toBe(colors.expectedActionText);
    }
  };

  await expect(theme).toContainText('Follow Windows (recommended)');
  await expect(palette).toContainText('Graphite (Default)');
  await expect(appearanceSummary).toHaveText(
    'Active: Dark. Saved dark palette: Graphite (Default).',
  );
  await expect(preview).toHaveAttribute('data-effective-mode', 'dark');
  await assertPreviewUsesRuntimeTokens();
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(root).toHaveAttribute('data-palette', 'graphite');
  await expect(appearanceSummary).toHaveText(
    'Active: Light. Saved dark palette: Graphite (Default).',
  );
  await expect(preview).toHaveAttribute('data-effective-mode', 'light');
  await assertPreviewUsesRuntimeTokens();

  await palette.click();
  await page.locator('[data-slot="select-item"][data-value="slate"]').click();
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(root).toHaveAttribute('data-palette', 'slate');
  await expect(appearanceSummary).toHaveText('Active: Light. Saved dark palette: Slate.');
  await expect(preview).toHaveAttribute('data-effective-mode', 'light');
  await assertPreviewUsesRuntimeTokens();

  await theme.click();
  await page.locator('[data-slot="select-item"][data-value="dark"]').click();
  await expect(root).toHaveAttribute('data-color-mode', 'dark');
  await expect(appearanceSummary).toHaveText('Active: Dark. Saved dark palette: Slate.');
  await expect(preview).toHaveAttribute('data-effective-mode', 'dark');
  await assertPreviewUsesRuntimeTokens();
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(root).toHaveAttribute('data-color-mode', 'dark');

  await theme.click();
  await page.locator('[data-slot="select-item"][data-value="system"]').click();
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(appearanceSummary).toHaveText('Active: Light. Saved dark palette: Slate.');
  expect(
    await page.evaluate(() =>
      Object.keys(localStorage).some((key) => /appearance|color.?mode|theme/i.test(key)),
    ),
  ).toBe(false);
});
