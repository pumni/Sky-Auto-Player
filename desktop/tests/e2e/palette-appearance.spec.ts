import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from '@playwright/test';

const palettes = ['aurora', 'minimalist', 'slate', 'cyberpunk', 'classic'] as const;
type Palette = (typeof palettes)[number];

const auroraTokenNames = [
  '--canvas',
  '--surface-panel',
  '--surface-raised',
  '--control-hover',
  '--control-pressed',
  '--selection-bg',
  '--selection-indicator',
  '--border-subtle',
  '--border-strong',
  '--control-border',
  '--text-primary',
  '--text-secondary',
  '--text-tertiary',
  '--accent',
  '--accent-hover',
  '--on-accent',
  '--focus-ring',
] as const;

const expected = {
  aurora: {
    controlBorder: '#617390',
    selectionBg: '#152132',
    accent: '#785ddd',
    accentHover: '#825bdb',
    borderStrong: '#33445e',
    canvas: '#0b1018',
    surfaceRaised: '#151e2b',
    onAccent: '#ffffff',
    focusRing: '#9b8cff',
    onDanger: '#ffffff',
  },
  minimalist: {
    controlBorder: '#77818d',
    selectionBg: '#2d343d',
    accent: '#a0b8d5',
    accentHover: '#c4d0df',
    borderStrong: '#58616d',
    canvas: '#17191d',
    surfaceRaised: '#292d33',
    onAccent: '#17191d',
    focusRing: '#e1e8f0',
    onDanger: '#ffffff',
  },
  slate: {
    controlBorder: '#6a8699',
    selectionBg: '#2a3d4a',
    accent: '#6ca7d8',
    accentHover: '#83b8e3',
    borderStrong: '#486274',
    canvas: '#10171d',
    surfaceRaised: '#20313d',
    onAccent: '#10171d',
    focusRing: '#a8dded',
    onDanger: '#ffffff',
  },
  cyberpunk: {
    controlBorder: '#976595',
    selectionBg: '#3a274a',
    accent: '#d95abf',
    accentHover: '#ef78d4',
    borderStrong: '#654872',
    canvas: '#130f1d',
    surfaceRaised: '#291c37',
    onAccent: '#130f1d',
    focusRing: '#f4a8e3',
    onDanger: '#ffffff',
  },
  classic: {
    controlBorder: '#897864',
    selectionBg: '#352c21',
    accent: '#c99242',
    accentHover: '#e0aa5b',
    borderStrong: '#695845',
    canvas: '#171512',
    surfaceRaised: '#2c261f',
    onAccent: '#171512',
    focusRing: '#e8c17a',
    onDanger: '#ffffff',
  },
} satisfies Record<Palette, Record<string, string>>;

const stableStatusTokens = {
  '--success-fg': '#56d69a',
  '--success-bg': '#143326',
  '--success-border': '#3fa976',
  '--warning-fg': '#f5c451',
  '--warning-bg': '#3a2c11',
  '--warning-border': '#b4862d',
  '--danger-fg': '#f27a80',
  '--danger-bg': '#3a1c22',
  '--danger-border': '#be5962',
  '--danger-fill': '#c42b36',
  '--danger-fill-hover': '#d13a45',
  '--on-danger': '#ffffff',
} as const;

function parseColor(value: string): [number, number, number] {
  const hex = value.trim().match(/^#([0-9a-f]{6})$/i);
  if (hex) {
    return [
      Number.parseInt(hex[1].slice(0, 2), 16),
      Number.parseInt(hex[1].slice(2, 4), 16),
      Number.parseInt(hex[1].slice(4, 6), 16),
    ];
  }
  const rgb = value.match(/rgba?\(([^)]+)\)/i);
  if (!rgb) throw new Error(`Unsupported color value: ${value}`);
  const channels = rgb[1]
    .split(',')
    .slice(0, 3)
    .map((channel) => Number.parseFloat(channel.trim()));
  if (channels.length !== 3 || channels.some((channel) => Number.isNaN(channel))) {
    throw new Error(`Unsupported color value: ${value}`);
  }
  return channels as [number, number, number];
}

function relativeLuminance(color: string): number {
  return parseColor(color)
    .map((channel) => {
      const normalized = channel / 255;
      return normalized <= 0.03928 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
    })
    .reduce((sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index], 0);
}

function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

async function selectPalette(page: Page, palette: Palette) {
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  await dialog.getByLabel('Color palette').selectOption(palette);
  await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  await dialog.getByRole('button', { name: 'Close settings' }).click();
  await expect(dialog).toBeHidden();
}

async function token(page: Page, name: string): Promise<string> {
  return page.evaluate(
    (tokenName) => getComputedStyle(document.documentElement).getPropertyValue(tokenName).trim(),
    name,
  );
}

async function resolvedTokenColor(page: Page, name: string): Promise<string> {
  return page.evaluate((tokenName) => {
    const probe = document.createElement('span');
    probe.style.color = `var(${tokenName})`;
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  }, name);
}

async function styleValue(
  locator: Locator,
  property: 'color' | 'backgroundColor' | 'borderTopColor' | 'borderLeftColor' | 'outlineColor',
) {
  return locator.evaluate(
    (element, styleProperty) => getComputedStyle(element)[styleProperty],
    property,
  );
}

async function assertBorderUsesToken(
  page: Page,
  locator: Locator,
  tokenName: string,
  property: 'borderTopColor' | 'borderLeftColor' = 'borderTopColor',
) {
  await expect(styleValue(locator, property)).resolves.toBe(
    await resolvedTokenColor(page, tokenName),
  );
}

async function assertBackgroundUsesToken(page: Page, locator: Locator, tokenName: string) {
  const expectedColor = await resolvedTokenColor(page, tokenName);
  await expect.poll(() => styleValue(locator, 'backgroundColor')).toBe(expectedColor);
}

async function pressAndReadBackground(
  page: Page,
  locator: Locator,
  expectedColor: string,
): Promise<string> {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  if (!box) throw new Error('Cannot press an element without a box');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  try {
    await page.waitForTimeout(50);
    const background = await styleValue(locator, 'backgroundColor');
    expect(background).toBe(expectedColor);
    return background;
  } finally {
    await page.mouse.move(0, 0);
    await page.mouse.up();
  }
}

async function focusSeparatorWithKeyboard(page: Page, separator: Locator) {
  await separator.evaluate((element) => {
    const sentinel = document.createElement('button');
    sentinel.id = 'palette-audit-focus-sentinel';
    sentinel.type = 'button';
    sentinel.tabIndex = 0;
    element.parentElement?.insertBefore(sentinel, element);
    sentinel.focus();
  });
  await page.keyboard.press('Tab');
  await expect(separator).toBeFocused();
  await page.locator('#palette-audit-focus-sentinel').evaluate((element) => element.remove());
}

async function expectNoSeriousAccessibilityViolations(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.filter(
      (violation) => violation.impact === 'critical' || violation.impact === 'serious',
    ),
  ).toEqual([]);
}

test('all palette tokens and contrast gates match the locked contract', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1366, height: 768 });
  for (const palette of palettes) {
    await page.goto('/');
    await selectPalette(page, palette);
    const values = expected[palette];
    for (const [name, value] of Object.entries({
      '--control-border': values.controlBorder,
      '--selection-bg': values.selectionBg,
      '--accent': values.accent,
      '--accent-hover': values.accentHover,
      '--border-strong': values.borderStrong,
    })) {
      await expect.poll(() => token(page, name)).toBe(value);
    }
    for (const [name, value] of Object.entries(stableStatusTokens)) {
      await expect.poll(() => token(page, name)).toBe(value);
    }

    const colors = {
      accent: await resolvedTokenColor(page, '--accent'),
      accentHover: await resolvedTokenColor(page, '--accent-hover'),
      controlBorder: await resolvedTokenColor(page, '--control-border'),
      selectionBg: await resolvedTokenColor(page, '--selection-bg'),
      canvas: await resolvedTokenColor(page, '--canvas'),
      surfaceRaised: await resolvedTokenColor(page, '--surface-raised'),
      onAccent: await resolvedTokenColor(page, '--on-accent'),
      onDanger: await resolvedTokenColor(page, '--on-danger'),
      dangerFill: await resolvedTokenColor(page, '--danger-fill'),
      dangerFillHover: await resolvedTokenColor(page, '--danger-fill-hover'),
      focusRing: await resolvedTokenColor(page, '--focus-ring'),
    };
    expect(contrastRatio(colors.onAccent, colors.accent)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(colors.onAccent, colors.accentHover)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(colors.onDanger, colors.dangerFill)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(colors.onDanger, colors.dangerFillHover)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(colors.controlBorder, colors.surfaceRaised)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(colors.controlBorder, colors.canvas)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(colors.accent, colors.selectionBg)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(colors.focusRing, colors.surfaceRaised)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(colors.focusRing, colors.canvas)).toBeGreaterThanOrEqual(3);
  }

  await selectPalette(page, 'aurora');
  const auroraTokens = await Promise.all(auroraTokenNames.map((name) => token(page, name)));

  await page.locator('html').evaluate((root) => root.removeAttribute('data-palette'));
  await expect(page.locator('html')).not.toHaveAttribute('data-palette');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  for (const [index, name] of auroraTokenNames.entries()) {
    await expect.poll(() => token(page, name)).toBe(auroraTokens[index]);
  }
  await selectPalette(page, 'aurora');
});

test('semantic consumers and interaction states use the locked hierarchy', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await selectPalette(page, 'aurora');
  const transparentBackground = await page.evaluate(() => {
    const probe = document.createElement('span');
    probe.style.backgroundColor = 'transparent';
    document.body.append(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  });

  const openSettings = page.getByRole('button', { name: 'Open settings' });
  await pressAndReadBackground(page, openSettings, transparentBackground);

  await assertBorderUsesToken(page, page.locator('.global-search'), '--control-border');
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const selectedRow = page.locator('.track-row.is-selected');
  await expect(selectedRow).toHaveCount(1);
  await expect(selectedRow).toHaveClass(/is-selected/);
  await assertBackgroundUsesToken(page, selectedRow, '--selection-bg');
  await assertBorderUsesToken(page, selectedRow, '--accent', 'borderLeftColor');
  await assertBackgroundUsesToken(
    page,
    page.locator('.library-nav-item.is-active'),
    '--selection-bg',
  );

  const separator = page.getByRole('separator', { name: 'Resize library navigator' });
  const separatorLine = separator.locator('span');
  await separator.hover();
  await assertBorderUsesToken(page, separatorLine, '--control-border', 'borderLeftColor');
  await focusSeparatorWithKeyboard(page, separator);
  await expect(styleValue(separatorLine, 'borderLeftColor')).resolves.toBe(
    await resolvedTokenColor(page, '--focus-ring'),
  );
  const separatorBox = await separator.boundingBox();
  expect(separatorBox).not.toBeNull();
  if (!separatorBox) throw new Error('Cannot drag separator without a box');
  await page.mouse.move(
    separatorBox.x + separatorBox.width / 2,
    separatorBox.y + separatorBox.height / 2,
  );
  await page.mouse.down();
  await expect(styleValue(separatorLine, 'borderLeftColor')).resolves.toBe(
    await resolvedTokenColor(page, '--accent'),
  );
  await page.mouse.up();

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Playback', exact: true }).click();
  const settingsSelect = settings.locator('.settings-section select').first();
  await assertBorderUsesToken(page, settingsSelect, '--control-border');
  await settings
    .locator('.settings-section')
    .first()
    .evaluate((section) => {
      const input = document.createElement('input');
      input.className = 'palette-audit-settings-text-input';
      input.type = 'text';
      input.setAttribute('aria-label', 'Palette audit text input');
      section.append(input);
    });
  const settingsTextInput = settings.locator('.palette-audit-settings-text-input');
  await assertBorderUsesToken(page, settingsTextInput, '--control-border');
  await settingsTextInput.evaluate((element) => element.remove());
  const closeSettings = settings.getByRole('button', { name: 'Close settings' });
  await closeSettings.hover();
  await assertBackgroundUsesToken(page, closeSettings, '--control-hover');
  await pressAndReadBackground(
    page,
    closeSettings,
    await resolvedTokenColor(page, '--control-pressed'),
  );
  const autoPlay = settings.getByRole('switch', { name: 'Auto Play' });
  await autoPlay.hover();
  await assertBackgroundUsesToken(page, autoPlay, '--control-hover');
  await pressAndReadBackground(page, autoPlay, await resolvedTokenColor(page, '--control-pressed'));
  await closeSettings.click();

  const profileTrigger = page.getByRole('button', { name: 'Configure playback profile' });
  await pressAndReadBackground(page, profileTrigger, transparentBackground);
  await profileTrigger.click();
  const profile = page.getByRole('dialog', { name: 'Playback profile' });
  const profilePopover = page.locator('.profile-popover');
  await assertBorderUsesToken(
    page,
    profile.locator('.profile-fields select').first(),
    '--control-border',
  );
  await assertBorderUsesToken(page, profilePopover, '--border-strong');
  await page.keyboard.press('Escape');
  await expect(profile).toBeHidden();

  const update = page.locator('.update-indicator');
  await assertBorderUsesToken(page, update, '--border-strong');
  await assertBackgroundUsesToken(page, update, '--surface-raised');
  await update.hover();
  await assertBackgroundUsesToken(page, update, '--control-hover');
  await assertBackgroundUsesToken(page, page.locator('.update-indicator-dot'), '--accent');

  const navigator = page.getByRole('navigation', { name: 'Library' });
  const collapseNavigator = navigator.getByRole('button', { name: 'Collapse library navigator' });
  await pressAndReadBackground(
    page,
    collapseNavigator,
    await resolvedTokenColor(page, '--control-pressed'),
  );
  await collapseNavigator.click();
  const expandNavigator = navigator.getByRole('button', { name: 'Expand library navigator' });
  await expandNavigator.hover();
  await pressAndReadBackground(
    page,
    expandNavigator,
    await resolvedTokenColor(page, '--control-pressed'),
  );
  await expandNavigator.click();

  const createPlaylist = navigator.getByRole('button', { name: 'Create playlist' });
  await createPlaylist.hover();
  await assertBackgroundUsesToken(page, createPlaylist, '--control-hover');
  await pressAndReadBackground(
    page,
    createPlaylist,
    await resolvedTokenColor(page, '--control-hover'),
  );
  await createPlaylist.click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  const createDialogSurface = page.locator('.library-dialog').filter({ has: createDialog });
  await assertBorderUsesToken(page, createDialog.getByLabel('Playlist name'), '--control-border');
  await assertBorderUsesToken(page, createDialogSurface, '--border-strong');
  const cancel = createDialog.getByRole('button', { name: 'Cancel' });
  await assertBackgroundUsesToken(page, cancel, '--surface-raised');
  await cancel.hover();
  await assertBackgroundUsesToken(page, cancel, '--control-hover');
  await pressAndReadBackground(page, cancel, await resolvedTokenColor(page, '--control-pressed'));
  const primary = createDialog.getByRole('button', { name: 'Create' });
  await assertBackgroundUsesToken(page, primary, '--accent');
  await primary.hover();
  await assertBackgroundUsesToken(page, primary, '--accent-hover');
  await createDialog.getByLabel('Playlist name').fill('Palette Contract');
  await primary.click();
  await page.getByRole('button', { name: 'More actions for Palette Contract' }).click();
  await page.getByRole('menuitem', { name: 'Delete playlist' }).click();
  const deleteDialog = page.getByRole('dialog', { name: /Delete/ });
  const danger = deleteDialog.getByRole('button', { name: 'Delete playlist' });
  await assertBackgroundUsesToken(page, danger, '--danger-fill');
  await danger.hover();
  await assertBackgroundUsesToken(page, danger, '--danger-fill-hover');
  await deleteDialog.getByRole('button', { name: 'Cancel' }).click();

  expect(await page.locator('[data-theme]').count()).toBe(0);
  await expectNoSeriousAccessibilityViolations(page);
});

test('forced colors retain system-state mappings and no palette leakage', async ({ page }) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await selectPalette(page, 'cyberpunk');
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await expect
    .poll(() => page.evaluate(() => window.matchMedia('(forced-colors: active)').matches))
    .toBe(true);
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const selectedRow = page.locator('.track-row.is-selected');
  const systemColor = async (
    property: 'color' | 'backgroundColor' | 'borderLeftColor' | 'outlineColor',
    value: string,
  ) =>
    page.evaluate(
      ({ property: styleProperty, value: color }) => {
        const probe = document.createElement('span');
        probe.style[styleProperty] = color;
        document.body.append(probe);
        const resolved = getComputedStyle(probe)[styleProperty];
        probe.remove();
        return resolved;
      },
      { property, value },
    );
  const expectSystemColor = async (
    locator: Locator,
    property: 'color' | 'backgroundColor' | 'borderLeftColor' | 'outlineColor',
    value: string,
  ) => {
    const expected = await systemColor(property, value);
    await expect
      .poll(() => styleValue(locator, property), {
        message: `wait for forced-colors ${property}=${value} to apply`,
        timeout: 2_000,
      })
      .toBe(expected);
  };
  await expectSystemColor(selectedRow, 'backgroundColor', 'Highlight');
  await expectSystemColor(selectedRow, 'color', 'HighlightText');

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Playback', exact: true }).click();
  const activeSettingsNav = settings.getByRole('button', { name: 'Playback', exact: true });
  await expect(activeSettingsNav).toHaveClass(/is-active/);
  await expectSystemColor(activeSettingsNav, 'backgroundColor', 'Highlight');
  const autoPlay = settings.getByRole('switch', { name: 'Auto Play' });
  await expectSystemColor(
    autoPlay.locator('.auto-play-switch-track'),
    'backgroundColor',
    'Highlight',
  );
  await settings.getByRole('button', { name: 'Close settings' }).click();

  const separator = page.getByRole('separator', { name: 'Resize library navigator' });
  await focusSeparatorWithKeyboard(page, separator);
  await expectSystemColor(separator, 'outlineColor', 'Highlight');
  await expectSystemColor(separator.locator('span'), 'borderLeftColor', 'Highlight');
});
