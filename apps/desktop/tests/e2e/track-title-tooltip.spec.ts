import { expect, test } from '@playwright/test';

test('song title tooltip appears only for clipped text at narrow and wide layouts', async ({
  page,
}) => {
  // Use the named default fixture so Aurora Landing exists in both viewport sizes.
  await page.goto('/');
  const row = page.getByRole('row', { name: /Aurora Landing/ });
  const title = row.locator('[data-track-title]');
  const cell = row.locator('[data-slot="track-title-cell"]');

  for (const viewport of [
    { width: 1649, height: 927 },
    { width: 987, height: 691 },
  ]) {
    await page.setViewportSize(viewport);
    await page.mouse.move(0, 0);
    await expect(title).toBeVisible();

    await title.evaluate((element) => {
      (element as HTMLElement).style.maxWidth = '';
    });
    expect(await title.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
      true,
    );
    await title.hover();
    await expect(page.getByRole('tooltip')).toHaveCount(0);
    expect(await title.getAttribute('title')).toBeNull();
    expect(await cell.getAttribute('title')).toBeNull();

    await page.mouse.move(0, 0);
    await title.evaluate((element) => {
      (element as HTMLElement).style.maxWidth = '48px';
    });
    expect(await title.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
    await title.hover();
    await expect(page.getByRole('tooltip')).toHaveText('Aurora Landing');
    expect(await title.getAttribute('title')).toBeNull();
    expect(await cell.getAttribute('title')).toBeNull();

    await page.mouse.move(0, 0);
    await expect(page.getByRole('tooltip')).toHaveCount(0);
  }
});
