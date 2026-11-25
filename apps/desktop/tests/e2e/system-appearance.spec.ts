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
  const themeGroup = settings.getByRole('radiogroup', { name: 'Theme' });
  const systemRadio = themeGroup.getByRole('radio', { name: 'System', exact: true });
  const darkRadio = themeGroup.getByRole('radio', { name: 'Dark', exact: true });
  const palette = settings.getByRole('combobox', { name: 'Dark palette' });
  const appearanceSummary = settings.locator('[data-slot="appearance-active-summary"]');
  const preview = settings.locator('[data-slot="appearance-preview"]');
  const assertPreviewUsesRuntimeTokens = async () => {
    const colors = await page.evaluate(() => {
      const previewEl = document.querySelector<HTMLElement>('[data-slot="appearance-preview"]');
      const canvasEl = document.querySelector<HTMLElement>(
        '[data-slot="appearance-preview-canvas"]',
      );
      const actionEl = document.querySelector<HTMLElement>(
        '[data-slot="appearance-preview-primary-action"]',
      );
      if (!previewEl || !canvasEl || !actionEl) return null;
      const tokenColor = (property: string, token: string) => {
        const probe = document.createElement('span');
        probe.style.setProperty(property, `var(${token})`);
        document.body.append(probe);
        const value = getComputedStyle(probe).getPropertyValue(property);
        probe.remove();
        return value;
      };
      const isUnified =
        document.documentElement.dataset.colorMode === 'light' ||
        document.documentElement.dataset.palette === 'graphite';
      return {
        shell: getComputedStyle(previewEl).backgroundColor,
        expectedShell: tokenColor('background-color', isUnified ? '--card' : '--background'),
        canvas: getComputedStyle(canvasEl).backgroundColor,
        expectedCanvas: tokenColor('background-color', '--surface'),
        action: getComputedStyle(actionEl).backgroundColor,
        expectedAction: tokenColor('background-color', '--primary'),
        actionText: getComputedStyle(actionEl).color,
        expectedActionText: tokenColor('color', '--primary-foreground'),
      };
    });
    expect(colors).not.toBeNull();
    if (colors) {
      expect(colors.shell).toBe(colors.expectedShell);
      expect(colors.canvas).toBe(colors.expectedCanvas);
      expect(colors.action).toBe(colors.expectedAction);
      expect(colors.actionText).toBe(colors.expectedActionText);
    }
  };

  await expect(themeGroup).toHaveAccessibleDescription('System follows Windows appearance.');
  await expect(systemRadio).toHaveAttribute('aria-checked', 'true');
  await expect(palette).toContainText('Graphite (Default)');
  await expect(appearanceSummary).toHaveText(
    'Active: Dark. Saved dark palette: Graphite (Default).',
  );
  await expect(preview).toHaveAttribute('data-effective-mode', 'dark');
  await expect(preview).toHaveAttribute(
    'aria-label',
    'Dark appearance preview (Graphite (Default) palette)',
  );
  await assertPreviewUsesRuntimeTokens();
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(root).toHaveAttribute('data-palette', 'graphite');
  await expect(appearanceSummary).toHaveText(
    'Active: Light. Saved dark palette: Graphite (Default).',
  );
  await expect(preview).toHaveAttribute('data-effective-mode', 'light');
  await expect(preview).toHaveAttribute(
    'aria-label',
    'Light appearance preview (saved dark palette: Graphite (Default))',
  );
  await assertPreviewUsesRuntimeTokens();

  await palette.click();
  await page.locator('[data-slot="select-item"][data-value="slate"]').click();
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(root).toHaveAttribute('data-palette', 'slate');
  await expect(appearanceSummary).toHaveText('Active: Light. Saved dark palette: Slate.');
  await expect(preview).toHaveAttribute('data-effective-mode', 'light');
  await assertPreviewUsesRuntimeTokens();

  await darkRadio.click();
  await expect(root).toHaveAttribute('data-color-mode', 'dark');
  await expect(root).not.toHaveAttribute('data-theme-applying', 'true');
  await expect(appearanceSummary).toHaveText('Active: Dark. Saved dark palette: Slate.');
  await expect(preview).toHaveAttribute('data-effective-mode', 'dark');
  await assertPreviewUsesRuntimeTokens();
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(root).toHaveAttribute('data-color-mode', 'dark');

  await systemRadio.click();
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(appearanceSummary).toHaveText('Active: Light. Saved dark palette: Slate.');
  expect(
    await page.evaluate(() =>
      Object.keys(localStorage).some((key) => /appearance|color.?mode|theme/i.test(key)),
    ),
  ).toBe(false);
});
