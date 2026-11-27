import { expect, type Locator, test } from '@playwright/test';
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
      expect(box.width).toBeLessThanOrEqual(360);
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
  await trigger.focus();
  await page.keyboard.press('Enter');

  await expect(popover).toBeVisible();
  await assertPopoverGeometry();
  await expect(popover.getByRole('switch', { name: 'Auto Play' })).toHaveCount(0);
  await expect(popover.getByLabel('Hold')).toBeVisible();
  await expect(popover.getByLabel('Tempo')).toBeVisible();
  const advancedTiming = popover.getByText('Advanced timing');
  await expect(advancedTiming).toBeVisible();
  await expect(popover.getByLabel('FPS')).toBeHidden();
  await advancedTiming.click();
  await expect(popover.getByLabel('FPS')).toBeVisible();
  await expect(popover.locator('details dt')).toHaveText([
    'Frame duration',
    'Base hold duration',
    'Effective target hold',
  ]);
  await expect(popover.locator('[data-slot="timing-margin-control"]')).toContainText(
    'Timing margin',
  );
  await expect(popover.locator('[data-slot="timing-margin-control"]')).toContainText('500 µs');
  await expect(popover.locator('[data-slot="timing-margin-control"]')).not.toContainText(
    'Recommended',
  );
  await expect(popover.getByRole('button', { name: /Use recommended/ })).toHaveCount(0);
  await expect(popover.getByText(/Source:/)).toHaveCount(0);
  await expect(popover.getByRole('button', { name: 'Test playback (no input)' })).toBeVisible();
  await expect(popover.locator('input[type="checkbox"]')).toHaveCount(0);
  await expectNoSeriousAccessibilityViolations(page);

  await popover.getByRole('combobox', { name: 'Hold' }).focus();
  await expect(popover.getByRole('combobox', { name: 'Hold' })).toBeFocused();
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
  await expect(autoPlay).toHaveAccessibleDescription(
    'Automatically continue to the next song after the current song finishes successfully.',
  );
  await expect(
    dialog.locator('[data-slot="settings-category-content"]').getByRole('switch'),
  ).toHaveCount(1);
  await expect(dialog.getByRole('button', { name: 'Playback' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(dialog.getByRole('heading', { name: 'Timing' })).toBeVisible();
  const timingMargin = dialog.getByRole('group', { name: 'Timing margin' });
  await expect(timingMargin).toContainText('500 µs');
  await expect(timingMargin).not.toContainText('Recommended');
  await expect(dialog.getByRole('button', { name: /Use recommended/ })).toHaveCount(0);
  await expect(dialog.getByText(/sender evidence, not proof/)).toHaveCount(0);
  const playbackContentHasHorizontalOverflow = await dialog
    .locator('[data-settings-content]')
    .evaluate((element) => element.scrollWidth > element.clientWidth);
  expect(playbackContentHasHorizontalOverflow).toBe(false);
  const timingHeadingBox = await dialog
    .locator('[data-slot="timing-margin-control"] > div:first-child')
    .boundingBox();
  const timingStepperBox = await dialog
    .getByRole('button', { name: 'Decrease timing margin' })
    .boundingBox();
  expect(timingHeadingBox).not.toBeNull();
  expect(timingStepperBox).not.toBeNull();
  if (timingHeadingBox && timingStepperBox) {
    const headingCenterY = timingHeadingBox.y + timingHeadingBox.height / 2;
    const stepperCenterY = timingStepperBox.y + timingStepperBox.height / 2;
    expect(Math.abs(headingCenterY - stepperCenterY)).toBeLessThanOrEqual(4);
  }
  const initialBox = await dialog.boundingBox();
  const settingsContent = dialog.locator('[data-settings-content]');
  const initialContentWidth = await settingsContent.evaluate((element) => element.clientWidth);
  expect(initialBox).not.toBeNull();
  const holdCombobox = dialog.getByRole('combobox', { name: 'Base hold' });
  await expect(holdCombobox).toContainText('1 frame');
  await expect(dialog.getByRole('combobox', { name: 'Target FPS' })).toContainText('60 FPS');
  await expect(
    dialog.getByText('Settings are validated and persisted by the native runtime.'),
  ).toHaveCount(0);
  await expect(dialog.getByText('Preferences', { exact: true })).toHaveCount(0);
  for (const category of ['Appearance', 'Diagnostics', 'Updates', 'Donate', 'About', 'Playback']) {
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
  await expectFocusClearance(dialog.getByLabel('Base hold'));
  await expectFocusClearance(dialog.getByLabel('Tempo'));
  await expectFocusClearance(dialog.getByLabel('Target FPS'));
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Dark palette'));
  // Nested Select Escape closes popup before modal
  await dialog.getByRole('combobox', { name: 'Dark palette' }).click();
  await expect(page.locator('[data-slot="select-content"]:visible')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.locator('[data-slot="select-content"]:visible')).toHaveCount(0);
  await expect(dialog).toBeVisible();

  await dialog.getByRole('button', { name: 'Updates', exact: true }).click();
  await expectFocusClearance(dialog.getByLabel('Release channel'));
  await expectFocusClearance(dialog.getByRole('button', { name: /View update|Check for updates/ }));
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

test('Donate shows the offline MBBank VietQR without clipping or native settings footer', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Donate' }).click();

  await expect(dialog.getByRole('heading', { name: 'Support Sky Auto Player' })).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Bank transfer · VietQR' })).toBeVisible();
  await expect(dialog.getByText('MBBank', { exact: true })).toHaveCount(1);
  await expect(dialog.getByText(/No fixed amount|· VND/)).toHaveCount(0);
  await expect(dialog.getByRole('link', { name: 'Open Ko-fi in browser' })).toHaveAttribute(
    'href',
    'https://ko-fi.com/pumni',
  );
  await expect(dialog.getByRole('link', { name: 'Open PayPal in browser' })).toHaveAttribute(
    'href',
    'https://www.paypal.com/paypalme/pe4ce28',
  );

  await expect(dialog.getByText('0857288363')).toBeVisible();
  await expect(dialog.getByText('Nguyen Duc Hoa')).toBeVisible();
  await expect(dialog.getByText('Sky Auto Player Donate')).toBeVisible();
  await expect(dialog.getByText('MoMo')).toHaveCount(0);
  await expect(dialog.getByText('PSP2603710800000415')).toHaveCount(0);
  await expect(dialog.getByText('Techcombank')).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: 'Copy account' })).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: 'Copy note' })).toHaveCount(0);

  const qr = dialog.getByRole('img', { name: 'VietQR bank transfer code' });
  await qr.scrollIntoViewIfNeeded();
  await expect(qr).toBeVisible();
  await expect(qr).toHaveAttribute('src', '/donate-vietqr-mbbank.png');
  expect(
    await qr.evaluate((image: HTMLImageElement) => image.complete && image.naturalWidth > 0),
  ).toBe(true);
  await expect(
    dialog.getByText('Settings are validated and persisted by the native runtime.'),
  ).toHaveCount(0);

  const content = dialog.locator('[data-settings-content]');
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    const horizontalOverflow = await content.evaluate(
      (element) => element.scrollWidth > element.clientWidth,
    );
    expect(horizontalOverflow).toBe(false);
  }
  await expectNoSeriousAccessibilityViolations(page);
});

test('Donate prioritizes QR scanning and aligns manual fallback details and GitHub icon', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Donate' }).click();

  const headingTitle = dialog.getByRole('heading', { name: 'Support Sky Auto Player' });
  await expect(headingTitle).toBeVisible();
  await expect(dialog.locator('[data-slot="donate-heading-icon"]')).toHaveCount(0);

  const bank = dialog.locator('[data-slot="donate-bank-card"]');
  const qr = bank.locator('[data-slot="donate-qr-frame"]');
  const detailRows = bank.locator('[data-slot="donate-bank-reference-row"]');
  const guidance = bank.getByText(
    'Scan with your phone’s banking app. Verify the recipient before confirming.',
  );
  await expect(guidance).toBeVisible();
  await expect(detailRows).toHaveCount(2);
  await expect(bank.getByRole('button')).toHaveCount(0);

  const bankGeometry = await bank.evaluate((element) => {
    const rect = (selector: string) => element.querySelector(selector)?.getBoundingClientRect();
    const qrBounds = rect('[data-slot="donate-qr-frame"]');
    const detailsBounds = rect('[data-slot="donate-bank-information"]');
    const accountBounds = rect('[data-slot="donate-bank-reference-row"]');
    const guidanceBounds = rect('[data-slot="donate-bank-guidance"]');
    const detailListBounds = rect('[data-slot="donate-bank-details"]');
    const referenceRows = Array.from(
      element.querySelectorAll('[data-slot="donate-bank-reference-row"]'),
    ).map((row) => {
      const rowBounds = row.getBoundingClientRect();
      const valueBounds = row.querySelector('dd')?.getBoundingClientRect();
      return {
        valueVisible: Boolean(
          valueBounds &&
            valueBounds.left >= rowBounds.left &&
            valueBounds.right <= rowBounds.right + 1,
        ),
      };
    });
    return {
      qrTop: qrBounds?.top ?? 0,
      qrRight: qrBounds?.right ?? 0,
      qrBottom: qrBounds?.bottom ?? 0,
      detailsTop: detailsBounds?.top ?? 0,
      detailsBottom: detailsBounds?.bottom ?? 0,
      guidanceTop: guidanceBounds?.top ?? 0,
      guidanceLeft: guidanceBounds?.left ?? 0,
      guidanceBottom: guidanceBounds?.bottom ?? 0,
      detailListTop: detailListBounds?.top ?? 0,
      accountTop: accountBounds?.top ?? 0,
      referenceRows,
    };
  });
  expect(Math.abs(bankGeometry.qrTop - bankGeometry.detailsTop)).toBeLessThanOrEqual(2);
  expect(Math.abs(bankGeometry.qrTop - bankGeometry.guidanceTop)).toBeLessThanOrEqual(2);
  expect(bankGeometry.guidanceLeft).toBeGreaterThanOrEqual(bankGeometry.qrRight + 12);
  expect(bankGeometry.detailListTop).toBeGreaterThan(bankGeometry.guidanceBottom);
  expect(bankGeometry.accountTop).toBeGreaterThan(bankGeometry.detailListTop);
  expect(bankGeometry.detailsBottom).toBeLessThanOrEqual(bankGeometry.qrBottom + 2);
  for (const row of bankGeometry.referenceRows) {
    expect(row.valueVisible).toBe(true);
  }

  const github = dialog.getByRole('link', { name: /star on GitHub/ });
  await github.scrollIntoViewIfNeeded();
  const iconAlignment = await github.evaluate((element) => {
    const label = element.querySelector('span')?.getBoundingClientRect();
    const icon = element.querySelector('svg')?.getBoundingClientRect();
    if (!label || !icon) return Infinity;
    return Math.abs((label.top + label.bottom) / 2 - (icon.top + icon.bottom) / 2);
  });
  expect(iconAlignment).toBeLessThanOrEqual(2);
  await expect(qr).toBeVisible();

  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    const overflow = await dialog
      .locator('[data-settings-content]')
      .evaluate((element) => element.scrollWidth > element.clientWidth);
    expect(overflow).toBe(false);
    const stacked = await bank.evaluate((element) => {
      const qrBounds = element
        .querySelector('[data-slot="donate-qr-frame"]')
        ?.getBoundingClientRect();
      const infoBounds = element
        .querySelector('[data-slot="donate-bank-information"]')
        ?.getBoundingClientRect();
      if (!qrBounds || !infoBounds) return false;
      return infoBounds.top >= qrBounds.bottom;
    });
    expect(stacked).toBe(viewport.width <= 760);
  }
});

test('Donate provider rows stack cleanly and align text with Open actions across widths', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Donate' }).click();

  const rows = dialog.locator('[data-slot="donate-provider-row"]');
  await expect(rows).toHaveCount(2);
  for (const viewport of [
    { width: 1366, height: 768 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    const layout = await rows.evaluateAll((elements) =>
      elements.map((row) => {
        const title = row.querySelector<HTMLElement>('h5');
        const subtitle = row.querySelector<HTMLElement>('p');
        const icon = row.querySelector<SVGElement>('[data-slot="donate-provider-heading"] svg');
        const action = row.querySelector<HTMLAnchorElement>('[data-slot="donate-external-link"]');
        if (!title || !subtitle || !icon || !action) return null;

        const rowRect = row.getBoundingClientRect();
        const titleRect = title.getBoundingClientRect();
        const subtitleRect = subtitle.getBoundingClientRect();
        const iconRect = icon.getBoundingClientRect();
        const actionRect = action.getBoundingClientRect();
        const copyMid = (titleRect.top + subtitleRect.bottom) / 2;
        return {
          left: rowRect.left,
          top: rowRect.top,
          bottom: rowRect.bottom,
          right: rowRect.right,
          subtitleGap: subtitleRect.top - titleRect.bottom,
          iconOffset: Math.abs((iconRect.top + iconRect.bottom) / 2 - copyMid),
          actionOffset: Math.abs((actionRect.top + actionRect.bottom) / 2 - copyMid),
          actionInside: actionRect.right <= rowRect.right + 1,
        };
      }),
    );
    expect(layout).toHaveLength(2);
    const [first, second] = layout;
    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    if (first && second) {
      expect(Math.abs(first.left - second.left)).toBeLessThanOrEqual(1);
      expect(second.top).toBeGreaterThanOrEqual(first.bottom - 1);
      for (const item of [first, second]) {
        expect(item.subtitleGap).toBeGreaterThanOrEqual(0);
        expect(item.subtitleGap).toBeLessThanOrEqual(8);
        expect(item.iconOffset).toBeLessThanOrEqual(5);
        expect(item.actionOffset).toBeLessThanOrEqual(5);
        expect(item.actionInside).toBe(true);
      }
    }
  }
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
    const content = element.querySelector<HTMLElement>('[data-settings-content]');
    const switchRow = element.querySelector<HTMLElement>('[data-slot="auto-play-setting-row"]');
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

test('Settings backdrop stays within the viewport without outer scrollbars', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });

  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
    { width: 560, height: 448 },
    { width: 440, height: 360 },
  ]) {
    await page.setViewportSize(viewport);
    await expect(dialog).toBeVisible();
    const geometry = await page.evaluate(() => {
      const backdrop = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
      const settingsDialog = document.querySelector<HTMLElement>(
        '[role="dialog"][aria-labelledby="settings-title"]',
      );
      const content = settingsDialog?.querySelector<HTMLElement>('[data-settings-content]');
      if (!backdrop || !settingsDialog || !content) return null;
      const rect = settingsDialog.getBoundingClientRect();
      return {
        backdropClientWidth: backdrop.clientWidth,
        backdropScrollWidth: backdrop.scrollWidth,
        backdropClientHeight: backdrop.clientHeight,
        backdropScrollHeight: backdrop.scrollHeight,
        left: rect.left,
        right: rect.right,
        top: rect.top,
        bottom: rect.bottom,
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight,
        contentClientHeight: content.clientHeight,
        contentScrollHeight: content.scrollHeight,
      };
    });
    expect(geometry).not.toBeNull();
    if (!geometry) continue;
    expect(geometry.backdropScrollWidth).toBeLessThanOrEqual(geometry.backdropClientWidth + 1);
    expect(geometry.backdropScrollHeight).toBeLessThanOrEqual(geometry.backdropClientHeight + 1);
    expect(geometry.left).toBeGreaterThanOrEqual(15);
    expect(geometry.top).toBeGreaterThanOrEqual(15);
    expect(geometry.right).toBeLessThanOrEqual(geometry.viewportWidth - 15);
    expect(geometry.bottom).toBeLessThanOrEqual(geometry.viewportHeight - 15);
    if (viewport.width === 440) {
      expect(geometry.contentScrollHeight).toBeGreaterThan(geometry.contentClientHeight);
    }
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
  await panel.getByRole('button', { name: 'Collapse Inspector' }).click();
  await expect(panel.getByRole('button', { name: 'Expand Inspector' })).toBeVisible();
  await panel.getByRole('button', { name: 'Expand Inspector' }).click();
  await page.getByRole('button', { name: 'Close utility panel' }).click();
  await expect(panel).toBeHidden();
  await expect(page.getByRole('button', { name: 'Open utility panel' })).toBeFocused();
});

test('all supported palettes round-trip through the settings surface', async ({ page }) => {
  const settingsButton = page.getByRole('button', { name: 'Open settings' });
  await settingsButton.click();
  await page.getByRole('button', { name: 'Appearance' }).click();
  const palette = page.getByRole('combobox', { name: 'Dark palette' });
  for (const id of ['graphite', 'aurora', 'minimalist', 'slate', 'cyberpunk', 'classic']) {
    await palette.click();
    await page.locator(`[data-slot="select-item"][data-value="${id}"]`).click();
    await expect(page.locator('[data-slot="select-content"]')).toBeHidden();
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
  const dialog = page.getByRole('dialog');
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
  await page.getByRole('button', { name: 'Close utility panel' }).click();
  await expect(diagnosticsDrawer).toBeHidden();
  await expect(page.getByRole('button', { name: 'Open utility panel' })).toBeFocused();
  await trigger.click();
  await expect(page.getByRole('region', { name: 'Utility: Diagnostics' })).toBeVisible();
  await page.getByRole('button', { name: 'Close utility panel' }).click();
});

test('retired calibration controls are not exposed', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings.getByRole('button', { name: 'Advanced', exact: true })).toHaveCount(0);
  await expect(settings.getByRole('button', { name: 'Open calibration' })).toHaveCount(0);
  await expect(page.getByRole('dialog', { name: 'Timing calibration' })).toHaveCount(0);
  await expect(settings.getByRole('group', { name: 'Timing margin' })).toContainText('500 µs');
});

test('Settings unified shell, Ctrl+, shortcut, radio arrows, and scroll reset hold across categories', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1024, height: 640 });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Open settings' })).toBeVisible();
  await page.keyboard.press('Control+,');
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog).toBeVisible();

  // Unified shell uses --card for dialog and --surface for inset content canvas
  const shellTokens = await page.evaluate(() => {
    const dialogEl = document.querySelector<HTMLElement>(
      '[role="dialog"][aria-labelledby="settings-title"]',
    );
    const canvasEl = document.querySelector<HTMLElement>(
      '[data-slot="settings-content-scroll-region"]',
    );
    if (!dialogEl || !canvasEl) return null;
    const tokenBg = (token: string) => {
      const probe = document.createElement('span');
      probe.style.backgroundColor = `var(${token})`;
      document.body.append(probe);
      const value = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return value;
    };
    return {
      dialogBg: getComputedStyle(dialogEl).backgroundColor,
      expectedDialogBg: tokenBg('--card'),
      canvasBg: getComputedStyle(canvasEl).backgroundColor,
      expectedCanvasBg: tokenBg('--surface'),
    };
  });
  expect(shellTokens).not.toBeNull();
  if (shellTokens) {
    expect(shellTokens.dialogBg).toBe(shellTokens.expectedDialogBg);
    expect(shellTokens.canvasBg).toBe(shellTokens.expectedCanvasBg);
  }

  // Default Playback fits without scroll at 1366x768, including frozen-session note
  await page.setViewportSize({ width: 1366, height: 768 });
  const content = dialog.locator('[data-settings-content]');
  const wideFit = await content.evaluate((el) => ({
    scrollHeight: el.scrollHeight,
    clientHeight: el.clientHeight,
    scrollWidth: el.scrollWidth,
    clientWidth: el.clientWidth,
  }));
  expect(wideFit.scrollHeight).toBeLessThanOrEqual(wideFit.clientHeight);
  expect(wideFit.scrollWidth).toBeLessThanOrEqual(wideFit.clientWidth);
  await expect(dialog.getByText(/Changes apply to the next prepared session/)).toBeInViewport();

  // At compact height, scroll Playback down, then switch to Appearance and confirm scroll resets to top
  await page.setViewportSize({ width: 1024, height: 640 });
  await content.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  const appearanceNav = dialog.getByRole('button', { name: 'Appearance', exact: true });
  await appearanceNav.click();
  await expect(appearanceNav).toBeFocused();
  expect(await content.evaluate((el) => el.scrollTop)).toBe(0);

  // Compact 40px Theme radio tiles (3-col at wide/native min, no clipping, checkmark cue)
  const themeTiles = dialog.locator('[data-slot="theme-radio-option"]');
  await expect(themeTiles).toHaveCount(3);
  const tileGeometry = await themeTiles.evaluateAll((elements) =>
    elements.map((el) => {
      const r = el.getBoundingClientRect();
      return {
        top: r.top,
        height: r.height,
        scrollWidth: el.scrollWidth,
        clientWidth: el.clientWidth,
        checked: el.getAttribute('aria-checked'),
      };
    }),
  );
  for (const tile of tileGeometry) {
    expect(tile.height).toBeGreaterThanOrEqual(40);
    expect(tile.height).toBeLessThanOrEqual(48);
    expect(tile.scrollWidth).toBeLessThanOrEqual(tile.clientWidth + 1);
    expect(Math.abs(tile.top - (tileGeometry[0]?.top ?? 0))).toBeLessThanOrEqual(1);
  }

  // Radio arrow navigation switches Theme and retains focus without stale data-theme-applying
  const systemRadio = dialog.getByRole('radio', { name: 'System', exact: true });
  await systemRadio.focus();
  await page.keyboard.press('ArrowRight');
  const lightRadio = dialog.getByRole('radio', { name: 'Light', exact: true });
  await expect(lightRadio).toBeFocused();
  await expect(lightRadio).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  await expect(page.locator('html')).not.toHaveAttribute('data-theme-applying', 'true');

  // Verify no horizontal overflow on Appearance across desktop & stress viewports
  for (const viewport of [
    { width: 1366, height: 768 },
    { width: 1200, height: 760 },
    { width: 800, height: 560 },
    { width: 640, height: 448 },
    { width: 640, height: 320 },
    { width: 440, height: 360 },
  ]) {
    await page.setViewportSize(viewport);
    const overflow = await content.evaluate((el) => el.scrollWidth > el.clientWidth);
    expect(overflow).toBe(false);
  }

  // Close and reopen preserves category selection (Appearance)
  await page.setViewportSize({ width: 1024, height: 640 });
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await page.keyboard.press('Control+,');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Appearance', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expectNoSeriousAccessibilityViolations(page);
});

test('Settings Updates wraps long errors without horizontal overflow at compact widths', async ({
  page,
}) => {
  await page.setViewportSize({ width: 640, height: 448 });
  await page.goto('/?mockUpdateState=error-long');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Updates', exact: true }).click();

  const alert = dialog.getByRole('alert');
  const checkAgain = dialog.getByRole('button', { name: 'Check again', exact: true });
  const viewDetails = dialog.getByRole('button', { name: 'View details', exact: true });
  await expect(alert).toBeVisible();
  await expect(checkAgain).toBeVisible();
  await expect(viewDetails).toBeVisible();
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    const bounds = await dialog.evaluate((el) => {
      const scroller = el.querySelector<HTMLElement>('[data-settings-content]');
      const statusGroup = el.querySelector<HTMLElement>('[data-slot="update-status-group"]');
      if (!scroller || !statusGroup) return null;
      const scrollerBox = scroller.getBoundingClientRect();
      const groupBox = statusGroup.getBoundingClientRect();
      return {
        overflowX: scroller.scrollWidth > scroller.clientWidth,
        groupFitsX: groupBox.right <= scrollerBox.right + 1,
      };
    });
    expect(bounds).not.toBeNull();
    expect(bounds?.overflowX).toBe(false);
    expect(bounds?.groupFitsX).toBe(true);
  }
});
