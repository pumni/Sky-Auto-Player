import { expect, test } from '@playwright/test';
import {
  expectPlayerBarGeometry,
  expectRecoveryBannerWithinViewport,
  expectStablePlayerBar,
  readPlayerBarGeometry,
  startSelectedSong,
} from './testHelpers';

test.setTimeout(60_000);
test.beforeEach(async ({ page }) => {
  await page.clock.install();
});
test('status Retry restores Playing and clears the recovery banner without moving the transport', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 760 });
  await page.goto(
    '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10&mockStartResponseDelayMs=60000&mockStatusQueryFailures=1&mockDropPlaybackSnapshots=1&mockDropPlaybackStartConfirmation=1',
  );
  const player = await startSelectedSong(page);
  await expect(player.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
  const beforeRecovery = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(beforeRecovery);

  await page.clock.fastForward(15_000);
  await expectRecoveryBannerWithinViewport(page);
  await expect(page.getByRole('alert')).toContainText('Playback status is unavailable');
  await expect(player.getByRole('button', { name: 'Retry status' })).toBeVisible();
  await expect(player.getByRole('button', { name: 'Stop playback' })).toBeVisible();
  const issueGeometry = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(issueGeometry);
  expectStablePlayerBar(beforeRecovery, issueGeometry);

  await player.getByRole('button', { name: 'Retry status' }).click();
  await expect(player.getByRole('button', { name: 'Pause' })).toBeEnabled();
  await expect(player.getByRole('alert')).toHaveCount(0);
  const recoveredGeometry = await readPlayerBarGeometry(page);
  expectStablePlayerBar(beforeRecovery, recoveredGeometry);
});

test('Stop playback remains explicit and actionable from a recovery banner at 800×560', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto(
    '/?mockPlaybackDurationMs=120000&mockStartDelayMs=10&mockStartResponseDelayMs=60000&mockStatusQueryFailures=1&mockDropPlaybackSnapshots=1&mockDropPlaybackStartConfirmation=1',
  );
  const player = await startSelectedSong(page);
  await expect(player.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
  await page.clock.fastForward(15_000);
  await expectRecoveryBannerWithinViewport(page);
  const beforeStop = await readPlayerBarGeometry(page);
  expectPlayerBarGeometry(beforeStop);

  await player.getByRole('button', { name: 'Stop playback' }).click();
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  await expect(player.getByRole('alert')).toHaveCount(0);
  const afterStop = await readPlayerBarGeometry(page);
  expectStablePlayerBar(beforeStop, afterStop);
});

test('authoritative no-session recovery returns to Play and Try again at 800×560', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/?mockNeverCreateSession=1');
  await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  const idleGeometry = await readPlayerBarGeometry(page);
  const player = await startSelectedSong(page);
  await expect(player.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
  await page.clock.fastForward(15_000);
  await expectRecoveryBannerWithinViewport(page);
  await expect(page.getByRole('alert')).toContainText('No active playback session was created');
  await expect(player.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
  await expect(player.getByRole('button', { name: 'Try again' })).toBeEnabled();
  expectStablePlayerBar(idleGeometry, await readPlayerBarGeometry(page));
});
