import { expect, test } from '@playwright/test';

test('song title tooltip appears only for clipped text at narrow and wide layouts', async ({
  page,
}) => {
  await page.goto('/?mockCatalogCount=112');
  const row = page.getByRole('row', { name: /Aurora Landing/ });
  const title = row.locator('.track-title-text');
  const cell = row.locator('.track-cell-title');

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
    expect(await title.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await title.hover();
    expect(await title.getAttribute('title')).toBeNull();
    expect(await cell.getAttribute('title')).toBeNull();

    await page.mouse.move(0, 0);
    await title.evaluate((element) => {
      (element as HTMLElement).style.maxWidth = '48px';
    });
    expect(await title.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
    await title.hover();
    await expect(title).toHaveAttribute('title', 'Aurora Landing');
    expect(await cell.getAttribute('title')).toBeNull();

    await page.mouse.move(0, 0);
    expect(await title.getAttribute('title')).toBeNull();
  }
});
