import { mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const evidenceVariant = process.env.VISUAL_EVIDENCE_VARIANT;
const evidenceDirectory = resolve(process.cwd(), '../docs/evidence/graphite-approval');

test.skip(
  evidenceVariant !== 'before' && evidenceVariant !== 'graphite',
  'Set VISUAL_EVIDENCE_VARIANT to before or graphite to capture approval screenshots.',
);

async function showPlaybackAndDetails(page: Page) {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.getByRole('row', { name: /Candle Run/ })).toBeVisible();
  await expect(page.getByRole('button', { name: /Open update/ })).toBeVisible();

  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Candle Run/ }).click();
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await page
    .getByRole('dialog', { name: 'Playback profile' })
    .getByRole('button', { name: 'Test playback (no input)' })
    .click();
  await expect(player.getByText('Playing', { exact: true })).toBeVisible();
  await expect(player.getByText('Dry run', { exact: true })).toBeVisible();

  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await expect(player.locator('.player-track-copy strong')).toHaveText('Candle Run');
  await expect(page.getByRole('row', { name: /Blue Bird/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );

  const utilityToggle = player.locator('.player-utility-button');
  if ((await utilityToggle.getAttribute('aria-expanded')) !== 'true') await utilityToggle.click();
  await expect(page.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  await page.getByRole('searchbox', { name: 'Search All Songs' }).focus();
  await expect(page.getByRole('searchbox', { name: 'Search All Songs' })).toBeFocused();
}

async function capture(page: Page, state: 'workbench' | 'runtime', width: number, height: number) {
  const path = resolve(
    evidenceDirectory,
    evidenceVariant as string,
    `${state}-${width}x${height}.png`,
  );
  mkdirSync(dirname(path), { recursive: true });
  await page.screenshot({ path, animations: 'disabled' });
}

async function captureTitlebar(page: Page, width: number) {
  const path = resolve(evidenceDirectory, evidenceVariant as string, `titlebar-${width}x48.png`);
  mkdirSync(dirname(path), { recursive: true });
  await page.screenshot({
    path,
    clip: { x: 0, y: 0, width, height: 48 },
    animations: 'disabled',
  });
}

test('captures matched Graphite approval evidence from the desktop app', async ({ page }) => {
  test.setTimeout(120_000);
  for (const viewport of [
    { width: 640, height: 448 },
    { width: 800, height: 560 },
    { width: 1366, height: 768 },
    { width: 1920, height: 1080 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/');
    await page.evaluate(() => window.localStorage.clear());
    await showPlaybackAndDetails(page);

    if (evidenceVariant === 'graphite') {
      await page.locator('html').evaluate((root) => root.setAttribute('data-palette', 'graphite'));
      await expect
        .poll(() =>
          page
            .locator('.player-dry-run-label')
            .evaluate((label) => getComputedStyle(label).fontSize),
        )
        .toBe('11px');
    }
    await capture(page, 'workbench', viewport.width, viewport.height);
    await captureTitlebar(page, viewport.width);

    await page.getByRole('tab', { name: 'Runtime' }).click();
    await expect(
      page
        .getByRole('region', { name: 'Utility: Diagnostics' })
        .getByText('Mode: Dry run · no physical input sent', { exact: true }),
    ).toBeVisible();
    await page.getByRole('searchbox', { name: 'Search All Songs' }).focus();
    await capture(page, 'runtime', viewport.width, viewport.height);
  }
});
