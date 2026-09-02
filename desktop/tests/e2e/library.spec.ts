import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

async function expectNoSeriousAccessibilityViolations(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.filter(
      (violation) => violation.impact === 'critical' || violation.impact === 'serious',
    ),
  ).toEqual([]);
}

test('mock desktop vertical slice can search and inspect a song', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('option', { name: /Aurora Landing/ })).toBeVisible();
  await page.getByLabel('Search songs').fill('Moonlit');
  await expect(page.getByRole('option', { name: /Moonlit Village/ })).toBeVisible();
  await page.getByRole('option', { name: /Moonlit Village/ }).click();
  await expect(page.getByText('Low timing risk')).toBeVisible();
});

test('virtualized library pages beyond the first 200 songs', async ({ page }) => {
  await page.goto('/');
  const list = page.locator('.virtual-list');
  await expect(page.getByText('500 songs')).toBeVisible();
  await list.evaluate((element) => {
    element.scrollTop = 400 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  const song = page.getByRole('option', { name: /Song 401/ });
  await expect(song).toBeVisible();
  await song.click();
  await expect(page.getByRole('heading', { name: 'Song 401' })).toBeVisible();
});

test('minimum viewport keeps the workbench and Player Bar bounded', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  await expect(page.getByRole('listbox', { name: 'Songs' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Choose a song' })).toBeVisible();
  await expect(page.getByRole('contentinfo', { name: 'Player controls' })).toBeVisible();
  const libraryPane = await page.locator('.library-workbench-pane').boundingBox();
  const inspectorPane = await page.locator('.inspector-workbench-pane').boundingBox();
  const playerBar = await page.getByRole('contentinfo', { name: 'Player controls' }).boundingBox();
  expect(libraryPane).not.toBeNull();
  expect(inspectorPane).not.toBeNull();
  expect(playerBar).not.toBeNull();
  if (libraryPane && inspectorPane && playerBar) {
    expect(Math.round(inspectorPane.x - (libraryPane.x + libraryPane.width))).toBe(8);
    expect(Math.round(playerBar.y - (libraryPane.y + libraryPane.height))).toBe(8);
  }
  const dimensions = await page.evaluate(() => ({
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth: window.innerWidth,
    documentHeight: document.documentElement.scrollHeight,
    viewportHeight: window.innerHeight,
  }));
  expect(dimensions.documentWidth).toBeLessThanOrEqual(dimensions.viewportWidth);
  expect(dimensions.documentHeight).toBeLessThanOrEqual(dimensions.viewportHeight);
  await expectNoSeriousAccessibilityViolations(page);
});

test('Playback Profile works through the narrow popover with focus restore', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  const trigger = page.getByRole('button', { name: 'Configure timing profile' });
  await trigger.click();

  const popover = page.getByRole('dialog', { name: 'Playback profile' });
  await expect(popover).toBeVisible();
  await expect(popover.getByLabel('Hold')).toBeVisible();
  await expect(popover.getByLabel('Tempo')).toBeVisible();
  await expect(popover.getByLabel('FPS')).toBeVisible();
  await expect(popover.getByRole('button', { name: 'Test playback (no input)' })).toBeVisible();
  await expect(popover.locator('input[type="checkbox"]')).toHaveCount(0);
  await expectNoSeriousAccessibilityViolations(page);

  await page.keyboard.press('Tab');
  await expect(popover.locator(':focus')).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(popover).toBeHidden();
  await expect(trigger).toBeFocused();
});

test('Library separator resizes by pointer and persists after reload', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  const separator = page.getByRole('separator', { name: 'Resize library pane' });
  const initial = Number(await separator.getAttribute('aria-valuenow'));
  const box = await separator.boundingBox();
  expect(box).not.toBeNull();
  if (!box) return;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2);
  await page.mouse.up();
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial + 40));
  await page.reload();
  await expect(page.getByRole('separator', { name: 'Resize library pane' })).toHaveAttribute(
    'aria-valuenow',
    String(initial + 40),
  );
});

test('selected Song Detail has no serious accessibility violations', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('option', { name: /Aurora Landing/ }).click();
  await expect(page.getByRole('heading', { name: 'Aurora Landing' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
});

test('Player Bar keeps transport geometry stable through its lifecycle', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  await page.getByRole('option', { name: /Aurora Landing/ }).click();
  const progress = page.getByRole('progressbar', { name: /Playback progress/ });
  await expect(progress).toBeVisible();
  await expect(progress).toHaveAttribute('value', '0');
  const idlePrimary = page.getByRole('button', { name: 'Play' });
  const idleBox = await idlePrimary.boundingBox();
  expect(idleBox).not.toBeNull();

  await idlePrimary.click();
  const confirmation = page.getByRole('group', { name: 'Playback confirmation' });
  await expect(confirmation).toBeVisible();
  await expect(
    confirmation.getByRole('button', { name: 'Proceed with current settings' }),
  ).toBeFocused();
  await expect(
    confirmation.getByRole('button', { name: 'Test playback (no input)' }),
  ).toBeVisible();
  await confirmation.getByRole('button', { name: 'Proceed with current settings' }).click();

  const activePrimary = page.getByRole('button', { name: 'Pause' });
  await expect(activePrimary).toBeVisible();
  const activeBox = await activePrimary.boundingBox();
  expect(activeBox).not.toBeNull();
  if (idleBox && activeBox) {
    expect(
      Math.abs(idleBox.x + idleBox.width / 2 - (activeBox.x + activeBox.width / 2)),
    ).toBeLessThanOrEqual(1);
  }
  await expect(page.getByRole('button', { name: 'Stop' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  await page.getByRole('button', { name: 'Pause' }).click();
  await expect(page.getByRole('button', { name: 'Resume' })).toBeVisible();
  await page.getByRole('button', { name: 'Resume' }).click();
  await expect(page.getByRole('button', { name: 'Pause' })).toBeVisible();
  await page.getByRole('button', { name: 'Stop' }).click();
  await expect(page.getByRole('button', { name: 'Play' })).toBeVisible();
});

test('Settings modal has no serious accessibility violations and closes accessibly', async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Playback defaults');
  await expect(dialog.getByRole('button', { name: 'Playback' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  const initialBox = await dialog.boundingBox();
  expect(initialBox).not.toBeNull();
  for (const category of ['Appearance', 'Diagnostics', 'Updates', 'Advanced', 'Playback']) {
    await dialog.getByRole('button', { name: category, exact: true }).click();
    const nextBox = await dialog.boundingBox();
    expect(nextBox).not.toBeNull();
    if (initialBox && nextBox) {
      expect(Math.abs(nextBox.x - initialBox.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.y - initialBox.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.width - initialBox.width)).toBeLessThanOrEqual(1);
      expect(Math.abs(nextBox.height - initialBox.height)).toBeLessThanOrEqual(1);
    }
  }
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

test('wide Diagnostics integrates as a workbench pane', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open diagnostics' }).click();
  const panel = page.getByRole('region', { name: 'Diagnostics' });
  await expect(panel).toBeVisible();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const panelBox = await panel.boundingBox();
  const playerBox = await player.boundingBox();
  expect(panelBox).not.toBeNull();
  expect(playerBox).not.toBeNull();
  if (panelBox && playerBox) expect(panelBox.y + panelBox.height).toBeLessThanOrEqual(playerBox.y);
  await panel.getByRole('tab', { name: 'Timing' }).click();
  await expect(panel.getByRole('img', { name: /Maximum timing lateness/ })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);
  await panel.getByRole('button', { name: 'Close diagnostics' }).click();
  await expect(panel).toBeHidden();
});

test('Diagnostics utility separator follows pointer and keyboard direction', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open diagnostics' }).click();
  const separator = page.getByRole('separator', { name: 'Resize diagnostics pane' });
  const initial = Number(await separator.getAttribute('aria-valuenow'));
  const box = await separator.boundingBox();
  expect(box).not.toBeNull();
  if (!box) return;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 20, box.y + box.height / 2);
  await page.mouse.up();
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial - 20));

  await separator.press('ArrowRight');
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial - 28));
  await separator.press('Shift+ArrowLeft');
  await expect(separator).toHaveAttribute('aria-valuenow', String(initial + 4));
});

test('all supported themes round-trip through the settings surface', async ({ page }) => {
  await page.goto('/');
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  await page.getByRole('button', { name: 'Appearance' }).click();
  const theme = page.getByLabel('Theme');
  for (const id of ['aurora', 'minimalist', 'slate', 'cyberpunk', 'classic']) {
    await theme.selectOption(id);
    await expect(page.locator('html')).toHaveAttribute('data-theme', id);
    await expectNoSeriousAccessibilityViolations(page);
  }
  await page.keyboard.press('Escape');
  await expect(settingsButton).toBeFocused();
});

test('update indicator and typed update dialog expose safe handoff states', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  const indicator = page.getByRole('button', { name: /Open update 3\.6\.0-mock/ });
  await expect(indicator).toBeVisible();
  await indicator.click();
  const dialog = page.getByRole('dialog', { name: 'Software update' });
  await expect(dialog).toContainText('Version 3.6.0-mock is available');
  await expectNoSeriousAccessibilityViolations(page);
  await dialog.getByRole('button', { name: 'Update and restart' }).click();
  await expect(dialog).toContainText('Restart handoff ready');
  await expectNoSeriousAccessibilityViolations(page);
});

test('Diagnostics drawer is bounded and accessible at the minimum viewport', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  const trigger = page.getByRole('button', { name: 'Open diagnostics' });
  await trigger.click();
  const drawer = page.getByRole('dialog', { name: 'Diagnostics' });
  await expect(drawer).toBeVisible();
  await expect(drawer).toBeFocused();
  await expect(drawer.getByRole('tab', { name: 'Performance' })).toBeVisible();
  await expectNoSeriousAccessibilityViolations(page);

  const focusables = drawer.locator('button, [tabindex]:not([tabindex="-1"])');
  await focusables.last().focus();
  await page.keyboard.press('Tab');
  await expect(drawer.locator(':focus')).toHaveCount(1);
  await focusables.first().focus();
  await page.keyboard.press('Shift+Tab');
  await expect(drawer.locator(':focus')).toHaveCount(1);

  await drawer.getByRole('tab', { name: 'Timing' }).click();
  await expect(drawer.getByRole('img', { name: /Maximum timing lateness/ })).toBeVisible();
  await expect(drawer).toContainText(/No timing samples|Latest maximum lateness/);
  await drawer.getByRole('tab', { name: 'Events' }).click();
  await expectNoSeriousAccessibilityViolations(page);
  await drawer.getByRole('button', { name: 'Close diagnostics' }).click();
  await expect(drawer).toBeHidden();
  await expect(page.getByRole('button', { name: 'Open diagnostics' })).toBeFocused();
});

test('long sheet titles remain contained in Library, Inspector, and Player Bar', async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 620 });
  await page.goto('/');
  const list = page.locator('.virtual-list');
  await list.evaluate((element) => {
    element.scrollTop = 495 * 46;
    element.dispatchEvent(new Event('scroll'));
  });
  const longTitle = /A sheet with an intentionally long title/;
  await expect(page.getByRole('option', { name: longTitle })).toBeVisible();
  await page.getByRole('option', { name: longTitle }).click();
  await expect(page.getByRole('heading', { name: longTitle })).toBeVisible();
  await expect(
    page.getByText(/This intentionally long timing-risk explanation verifies wrapping/),
  ).toBeVisible();
  const overflow = await page.evaluate(() => ({
    documentWidth: document.documentElement.scrollWidth,
    viewportWidth: window.innerWidth,
    titleOverflowHandled: [
      ...document.querySelectorAll('.song-row-title, .player-track-copy strong'),
    ].every((element) => {
      if (element.scrollWidth <= element.clientWidth) return true;
      const style = getComputedStyle(element);
      return style.overflow === 'hidden' && style.textOverflow === 'ellipsis';
    }),
    inspectorReasonOverflow: [...document.querySelectorAll('.inspector-panel')].some(
      (element) => element.scrollWidth > element.clientWidth,
    ),
  }));
  expect(overflow.documentWidth).toBeLessThanOrEqual(overflow.viewportWidth);
  expect(overflow.titleOverflowHandled).toBe(true);
  expect(overflow.inspectorReasonOverflow).toBe(false);
});

test('Calibration dialog exposes safe running and terminal states', async ({ page }) => {
  await page.setViewportSize({ width: 920, height: 620 });
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
