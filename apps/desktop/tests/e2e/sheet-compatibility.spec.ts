import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Page } from '@playwright/test';

const evidenceDirectory = resolve(process.cwd(), '../docs/evidence/issue-567-wp-c');

async function openSongDetails(page: Page, title: string) {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.addInitScript(() => {
    window.localStorage.clear();
    window.localStorage.setItem(
      'sky.ui.workbench.v5',
      JSON.stringify({
        version: 5,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 240,
        utilityWidth: 320,
        utilityOpen: true,
      }),
    );
  });
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
  await page.getByRole('searchbox', { name: 'Search Built-in Songs' }).fill(title);
  const row = page.getByRole('row', { name: new RegExp(title, 'i') });
  await expect(row).toBeVisible();
  await expect(page.getByText(/Showing 1–1 of 1 songs/)).toBeVisible();
  await row.click();
  await expect(row).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('heading', { name: title })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Utility: Song Details' })).toBeVisible();
}

async function capture(page: Page, name: string) {
  mkdirSync(evidenceDirectory, { recursive: true });
  await page.screenshot({
    path: resolve(evidenceDirectory, name),
    animations: 'disabled',
    caret: 'hide',
  });
}

test('legacy flat remains ready in selected-song details', async ({ page }) => {
  await openSongDetails(page, 'Aurora Landing');
  await expect(page.getByText('Legacy flat · ready')).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Target Sky Piano pitch' })).toHaveCount(0);
  await capture(page, 'legacy-ready.png');
});

test('loss proposal requires explicit acceptance before preparing a multitrack plan', async ({
  page,
}) => {
  await openSongDetails(page, 'Composed multi-track loss demo');
  await expect(page.getByText('Composed v5 · schema 5 · requires confirmation')).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Conversion mode' })).toBeVisible();
  const primarySlot = page.getByTestId('player-primary-slot');
  await primarySlot.getByRole('button', { name: 'Play' }).click();
  const accept = page.getByRole('button', { name: 'Accept conversion losses and prepare' });
  await expect(accept).toBeVisible();
  await page.locator('.sheet-consent-panel').scrollIntoViewIfNeeded();
  await capture(page, 'multitrack-loss-proposal.png');

  await accept.click();
  await expect(page.getByText('Conversion losses accepted for this plan.')).toBeVisible();
  await expect(primarySlot.getByRole('button', { name: 'Play' })).toBeVisible();
  await page.locator('.sheet-consent-accepted').scrollIntoViewIfNeeded();
  await capture(page, 'multitrack-loss-accepted.png');
});

test('unsupported schema is explained and has no active conversion controls', async ({ page }) => {
  await openSongDetails(page, 'Unsupported Composed v99 demo');
  await expect(page.getByText('Composed v99 · schema 99 · unsupported')).toBeVisible();
  await expect(page.getByText(/Unsupported composed sheet version 99/)).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Conversion mode' })).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Target Sky Piano pitch' })).toHaveCount(0);
  await capture(page, 'unsupported-v99-blocked.png');
});

test('source pitch selection is labeled as an in-game tuning assumption', async ({ page }) => {
  await openSongDetails(page, 'Source pitch assumption demo');
  await expect(page.getByText('Composed v4 · schema 4 · ready')).toBeVisible();
  await expect(page.getByText(/Sky Piano tuned to Ab/)).toBeVisible();
  await expect(page.getByText(/assumed from the source song/i)).toBeVisible();
  await capture(page, 'source-pitch-assumption.png');
});
