import { expect, test } from '@playwright/test';

for (const viewport of [
  { width: 1649, height: 927 },
  { width: 987, height: 691 },
  { width: 800, height: 560 },
]) {
  test(`risk confirmation stays compact at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await page.goto('/?mockPlaybackDurationMs=120000');

    await page.getByRole('row', { name: /Aurora Landing/ }).click();
    await page.getByRole('button', { name: 'Play', exact: true }).click();
    const confirmation = page.getByRole('group', { name: 'Playback confirmation' });
    await expect(confirmation).toBeVisible();
    await expect(confirmation.getByText('Medium timing risk')).toBeVisible();
    await expect(
      confirmation.getByRole('button', { name: 'Proceed with current settings' }),
    ).toBeFocused();
    const notNow = confirmation.getByRole('button', { name: 'Not now' });
    await expect(notNow).toBeVisible();
    await expect(confirmation.locator('.admission-option')).toHaveCount(3);

    const panel = await confirmation.boundingBox();
    const dismiss = await notNow.boundingBox();
    const dock = await page.getByRole('contentinfo', { name: 'Player controls' }).boundingBox();
    expect(panel).not.toBeNull();
    expect(dismiss).not.toBeNull();
    expect(dock).not.toBeNull();
    if (!panel || !dismiss || !dock) throw new Error('Missing admission panel geometry');
    expect(panel.width).toBeLessThanOrEqual(610);
    expect(panel.width).toBeLessThanOrEqual(viewport.width - 20);
    expect(panel.height).toBeLessThanOrEqual(Math.min(viewport.height * 0.68, 520) + 2);
    expect(panel.x).toBeGreaterThanOrEqual(0);
    expect(panel.x + panel.width).toBeLessThanOrEqual(viewport.width);
    expect(panel.y).toBeGreaterThanOrEqual(0);
    expect(panel.y + panel.height).toBeLessThanOrEqual(dock.y);
    expect(dismiss.height).toBeLessThanOrEqual(42);
    expect(dismiss.width).toBeLessThanOrEqual(150);

    const estimate = confirmation.locator('.admission-option-estimate').first();
    await expect(estimate).not.toHaveAttribute('open');
    await estimate.locator('summary').click();
    await expect(estimate).toHaveAttribute('open');
    await expect(estimate.getByLabel('Offline whole-stream timing advisory')).toBeVisible();
    await estimate.locator('summary').click();
    await notNow.click();
    await expect(confirmation).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  });
}
