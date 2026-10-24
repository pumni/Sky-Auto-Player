import { expect, test } from '@playwright/test';

test.describe('static startup fallback', () => {
  test.use({ javaScriptEnabled: false });

  test('shows a branded dark loading surface when frontend JavaScript cannot run', async ({
    page,
  }) => {
    await page.goto('/');

    await expect(page.getByRole('status')).toContainText('Starting the desktop application…');
    await expect(page.getByText('Preparing your library and saved settings.')).toBeVisible();
    await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(11, 16, 24)');
  });
});
