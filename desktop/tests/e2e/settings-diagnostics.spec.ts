import { expect, test, type Locator } from '@playwright/test';
import { expectNoSeriousAccessibilityViolations } from './testHelpers';

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

test('Playback Profile works through the narrow popover with focus restore', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  const trigger = page.getByRole('button', { name: 'Configure playback profile' });
  const popover = page.getByRole('dialog', { name: 'Playback profile' });

  await trigger.click();
  await expect(popover).toBeVisible();
  await expect(popover.getByRole('switch', { name: 'Auto Play' })).toHaveCount(0);
  const assertPopoverGeometry = async () => {
    const box = await popover.boundingBox();
    expect(box).not.toBeNull();
    if (box) {
      expect(box.width).toBeGreaterThanOrEqual(260);
      expect(box.width).toBeLessThanOrEqual(320);
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize()?.width ?? 0);
    }
    const hasHorizontalOverflow = await popover.evaluate(
      (element) => element.scrollWidth > element.clientWidth,
    );
    expect(hasHorizontalOverflow).toBe(false);
  };
  await assertPopoverGeometry();
  await page.keyboard.press('Escape');
  await expect(popover).toBeHidden();

  await page.setViewportSize({ width: 800, height: 560 });
  await trigger.click();

  await expect(popover).toBeVisible();
  await assertPopoverGeometry();
  await expect(popover.getByRole('switch', { name: 'Auto Play' })).toHaveCount(0);
  await expect(popover.getByLabel('Hold')).toBeVisible();
  await expect(popover.getByLabel('Tempo')).toBeVisible();
  await expect(popover.getByLabel('FPS')).toBeVisible();
  await expect(popover.locator('.profile-timing-summary dt')).toHaveText([
    'Target hold',
    'Release gap',
  ]);
  await expect(popover.locator('.timing-margin-control')).toContainText(
    'Timing Margin · rec. 500 µs',
  );
  await expect(popover.getByRole('button', { name: /Use recommended/ })).toHaveCount(0);
  await expect(popover.getByText(/Source:/)).toHaveCount(0);
  await expect(popover.getByRole('button', { name: 'Test playback (no input)' })).toBeVisible();
  await expect(popover.locator('input[type="checkbox"]')).toHaveCount(0);
  await expectNoSeriousAccessibilityViolations(page);

  await page.keyboard.press('Tab');
  await expect(popover.locator(':focus')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(popover).toBeHidden();
  await expect(trigger).toBeFocused();
});

test('selected Song Detail has no serious accessibility violations', async ({ page }) => {
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await expect(page.getByRole('heading', { name: 'Aurora Landing' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
});

test('Settings modal has no serious accessibility violations and closes accessibly', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Behavior' })).toBeVisible();
  await expect(dialog).toContainText('Playback defaults');
  const autoPlay = dialog.getByRole('switch', { name: 'Auto Play' });
  await expect(autoPlay).toContainText(
    'Automatically continue to the next song after the current song finishes successfully.',
  );
  await expect(dialog.locator('.settings-grid').getByRole('switch')).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: 'Playback' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(dialog.getByRole('heading', { name: 'Timing' })).toBeVisible();
  await expect(
    dialog.getByText('Source: Default fallback (no valid calibration cache)'),
  ).toBeVisible();
  await expect(dialog.getByRole('button', { name: /Use recommended/ })).toHaveCount(0);
  await expect(dialog.getByText(/sender evidence, not proof/)).toHaveCount(0);
  const playbackContentHasHorizontalOverflow = await dialog
    .locator('.settings-content')
    .evaluate((element) => element.scrollWidth > element.clientWidth);
  expect(playbackContentHasHorizontalOverflow).toBe(false);
  const timingHeadingBox = await dialog.locator('.timing-margin-heading').boundingBox();
  const timingStepperBox = await dialog.locator('.timing-margin-stepper').boundingBox();
  expect(timingHeadingBox).not.toBeNull();
  expect(timingStepperBox).not.toBeNull();
  if (timingHeadingBox && timingStepperBox) {
    const headingCenterY = timingHeadingBox.y + timingHeadingBox.height / 2;
    const stepperCenterY = timingStepperBox.y + timingStepperBox.height / 2;
    expect(Math.abs(headingCenterY - stepperCenterY)).toBeLessThanOrEqual(2);
  }
  const initialBox = await dialog.boundingBox();
  const settingsContent = dialog.locator('.settings-content');
  const initialContentWidth = await settingsContent.evaluate((element) => element.clientWidth);
  expect(initialBox).not.toBeNull();
  await expect(dialog.getByLabel('Hold').locator('option[value="1"]')).toHaveText('1 frame');
  for (const category of [
    'Appearance',
    'Diagnostics',
    'Updates',
    'Advanced',
    'About',
    'Playback',
  ]) {
    await dialog.getByRole('button', { name: category, exact: true }).click();
    const nextBox = await dialog.boundingBox();
    const nextContentWidth = await settingsContent.evaluate((element) => element.clientWidth);
    expect(nextBox).not.toBeNull();
    expect(nextContentWidth).toBe(initialContentWidth);
    if (initialBox && nextBox) {
      expect(Math.abs(nextBox.x - initialBox.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.y - initialBox.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.width - initialBox.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.height - initialBox.height)).toBeLessThanOrEqual(1);
    }
  }

  const expectFocusClearance = async (control: Locator) => {
    await control.focus();
    const controlBox = await control.boundingBox();
    const contentBox = await settingsContent.boundingBox();
    expect(controlBox).not.toBeNull();
    expect(contentBox).not.toBeNull();
    if (controlBox && contentBox) {
      expect(
        contentBox.x + contentBox.width - (controlBox.x + controlBox.width),
      ).toBeGreaterThanOrEqual(4);
    }
  };

  await dialog.getByRole('button', { name: 'Playback', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Hold'));
  await expectFocusClearance(dialog.getByLabel('Tempo'));
  await expectFocusClearance(dialog.getByLabel('FPS'));
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Color palette'));
  await dialog.getByRole('button', { name: 'Updates', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Channel'));
  await expectFocusClearance(dialog.getByRole('button', { name: 'Check for updates' }));
  await dialog.getByRole('button', { name: 'About', exact: true }).click();
  await expect(dialog).toContainText('Native ABI');
  await expectNoSeriousAccessibilityViolations(page);

  const focusables = dialog.locator('button, select, input');
  await focusables.first().focus();
  await page.keyboard.press('Shift+Tab');
  await expect(dialog.locator(':focus')).toHaveCount(1);
  await focusables.last().focus();
  await page.keyboard.press('Tab');
  await expect(dialog.locator(':focus')).toHaveCount(1);

  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(settingsButton).toBeFocused();
});

test('Playback Behavior stays usable in a compact Settings viewport', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 448 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  const row = dialog.getByRole('switch', { name: 'Auto Play' });
  await expect(row).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Playback defaults' })).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Timing' })).toBeVisible();
  const geometry = await dialog.evaluate((element) => {
    const content = element.querySelector<HTMLElement>('.settings-content');
    const switchRow = element.querySelector<HTMLElement>('.auto-play-switch');
    if (!content || !switchRow) return null;
    const contentBox = content.getBoundingClientRect();
    const rowBox = switchRow.getBoundingClientRect();
    return {
      contentClientWidth: content.clientWidth,
      contentScrollWidth: content.scrollWidth,
      rowLeft: rowBox.left,
      rowRight: rowBox.right,
      contentLeft: contentBox.left,
      contentRight: contentBox.right,
    };
  });
  expect(geometry).not.toBeNull();
  if (geometry) {
    expect(geometry.contentScrollWidth).toBeLessThanOrEqual(geometry.contentClientWidth);
    expect(geometry.rowLeft).toBeGreaterThanOrEqual(geometry.contentLeft);
    expect(geometry.rowRight).toBeLessThanOrEqual(geometry.contentRight);
  }
});

test('wide Diagnostics integrates as a workbench pane', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  await page.getByRole('tab', { name: 'Runtime' }).click();
  const panel = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(panel).toBeVisible();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const panelBox = await panel.boundingBox();
  const playerBox = await player.boundingBox();
  expect(panelBox).not.toBeNull();
  expect(playerBox).not.toBeNull();
  if (panelBox && playerBox) expect(panelBox.y + panelBox.height).toBeLessThanOrEqual(playerBox.y);
  await panel.getByRole('tab', { name: 'Timing' }).click();
  await expect(panel).toContainText(
    /No active playback session|Timing unavailable|Latest sender pre-call max/,
  );
  await expectNoSeriousAccessibilityViolations(page);
  await panel.getByRole('button', { name: 'Close utility' }).click();
  await expect(panel).toBeHidden();
  await expect(page.getByRole('button', { name: 'Open utility panel' })).toBeFocused();
});

test('all supported palettes round-trip through the settings surface', async ({ page }) => {
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  await page.getByRole('button', { name: 'Appearance' }).click();
  const palette = page.getByLabel('Color palette');
  for (const id of ['graphite', 'aurora', 'minimalist', 'slate', 'cyberpunk', 'classic']) {
    await palette.selectOption(id);
    await expect(page.locator('html')).toHaveAttribute('data-palette', id);
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
    await expect
      .poll(() => page.locator('html').evaluate((element) => getComputedStyle(element).colorScheme))
      .toContain('dark');
    await expectNoSeriousAccessibilityViolations(page);
  }
  await page.keyboard.press('Escape');
  await expect(settingsButton).toBeFocused();
});

test('update indicator and typed update dialog expose safe install states', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const indicator = page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ });
  await expect(indicator).toBeVisible();
  await indicator.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Software update' });
  await expect(dialog).toContainText('Version 4.0.0-alpha.2-mock is available');
  await expectNoSeriousAccessibilityViolations(page);
  await dialog.getByRole('button', { name: 'Update and restart' }).click();
  await expect(dialog).toContainText('Installing update…');
  await expectNoSeriousAccessibilityViolations(page);
});

test('Diagnostics drawer is bounded and accessible at the minimum viewport', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  const trigger = page.getByRole('button', { name: 'Open utility panel' });
  await trigger.click();
  const drawer = page.getByRole('region', { name: 'Utility: Song Details' });
  await expect(drawer).toBeVisible();
  await expect(drawer).not.toHaveAttribute('aria-modal');
  await drawer.getByRole('tab', { name: 'Details' }).focus();
  await page.keyboard.press('ArrowRight');
  const diagnosticsDrawer = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(diagnosticsDrawer.getByRole('tab', { name: 'Performance' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
  await diagnosticsDrawer.getByRole('tab', { name: 'Timing' }).click();
  await expect(diagnosticsDrawer).toContainText(
    /No active playback session|Timing unavailable|Latest sender pre-call max/,
  );
  await diagnosticsDrawer.getByRole('tab', { name: 'Events' }).click();
  await expectNoSeriousAccessibilityViolations(page);
  await diagnosticsDrawer.getByRole('button', { name: 'Close utility' }).click();
  await expect(diagnosticsDrawer).toBeHidden();
  await expect(page.getByRole('button', { name: 'Open utility panel' })).toBeFocused();
  await trigger.click();
  await expect(page.getByRole('region', { name: 'Utility: Diagnostics' })).toBeVisible();
  await page.getByRole('button', { name: 'Close utility panel' }).click();
});

test('Calibration dialog exposes safe running and terminal states', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  await page.getByRole('button', { name: 'Advanced' }).click();
  await page.getByRole('button', { name: 'Open calibration' }).click();
  const dialog = page.getByRole('dialog', { name: 'Timing calibration' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Start quick calibration' }).click();
  await expect(dialog).toContainText('Calibration complete');
  await expectNoSeriousAccessibilityViolations(page);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(dialog).toBeHidden();
});
