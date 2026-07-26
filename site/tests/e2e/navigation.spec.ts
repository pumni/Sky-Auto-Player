import { test, expect } from '@playwright/test';

test('navigation works', async ({ page }) => {
  await page.goto('/Sky-Auto-Player/');
  await expect(page).toHaveTitle(/Sky Auto Player/);
  
  // Click FAQ
  await page.locator('.site-nav a').filter({ hasText: 'FAQ' }).click();
  await expect(page).toHaveTitle(/FAQ/);
});

test('i18n switching works', async ({ page }) => {
  await page.goto('/Sky-Auto-Player/');
  // Switch to Vietnamese
  await page.locator('.locale-links a').filter({ hasText: 'VI' }).click();
  await expect(page).toHaveTitle(/Trình phát nhạc/);
});
