import { expect, test } from '@playwright/test';
import {
  chooseSelectOption,
  expectNoSeriousAccessibilityViolations,
  expectWorkbenchWithinBounds,
  palettes,
  selectPalette,
  startSelectedSong,
} from './testHelpers';

const surfaces = ['--background', '--surface', '--card', '--popover'] as const;
const bodyTextRoles = [
  '--foreground',
  '--text-secondary',
  '--text-tertiary',
  '--accent-ink',
] as const;
const themeRoles = [
  ...surfaces,
  ...bodyTextRoles,
  '--primary',
  '--primary-hover',
  '--primary-pressed',
  '--primary-foreground',
  '--ring',
  '--input',
  '--selection',
  '--playing',
  '--success',
  '--success-surface',
  '--warning',
  '--warning-surface',
  '--danger',
  '--danger-surface',
  '--destructive',
  '--destructive-foreground',
] as const;

type ThemeRole = (typeof themeRoles)[number];

function rgb(color: string): [number, number, number] {
  const shortHex = color.match(/^#([\da-f])([\da-f])([\da-f])$/i);
  const longHex = color.match(/^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i);
  if (shortHex) {
    return shortHex.slice(1).map((channel) => Number.parseInt(`${channel}${channel}`, 16)) as [
      number,
      number,
      number,
    ];
  }
  if (longHex) {
    return longHex.slice(1).map((channel) => Number.parseInt(channel, 16)) as [
      number,
      number,
      number,
    ];
  }
  const channels = color.match(/^rgba?\((\d+(?:\.\d+)?),\s*(\d+(?:\.\d+)?),\s*(\d+(?:\.\d+)?)/i);
  if (channels) return channels.slice(1).map(Number) as [number, number, number];
  throw new Error(`Unsupported computed theme color: ${color}`);
}

function computedRgb(color: string): string {
  return `rgb(${rgb(color).join(', ')})`;
}

function luminance(color: string): number {
  return rgb(color)
    .map((channel) => channel / 255)
    .map((value) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4))
    .reduce((total, value, index) => total + value * ([0.2126, 0.7152, 0.0722][index] ?? 0), 0);
}

function contrast(foreground: string, background: string): number {
  const first = luminance(foreground);
  const second = luminance(background);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

async function resolvedTheme(page: import('@playwright/test').Page) {
  return page.evaluate(
    (roles) => {
      const root = getComputedStyle(document.documentElement);
      return Object.fromEntries(roles.map((role) => [role, root.getPropertyValue(role).trim()]));
    },
    themeRoles as readonly string[],
  ) as Promise<Record<ThemeRole, string>>;
}

test('all six dark palettes meet semantic text, action, status, and focus contrast', async ({
  page,
}) => {
  await page.goto('/');

  for (const palette of palettes) {
    await selectPalette(page, palette);
    const theme = await resolvedTheme(page);

    for (const textRole of bodyTextRoles) {
      for (const surfaceRole of surfaces) {
        expect(
          contrast(theme[textRole], theme[surfaceRole]),
          `${palette} ${textRole} on ${surfaceRole}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }

    for (const actionRole of ['--primary', '--primary-hover', '--primary-pressed'] as const) {
      expect(
        contrast(theme['--primary-foreground'], theme[actionRole]),
        `${palette} primary foreground on ${actionRole}`,
      ).toBeGreaterThanOrEqual(4.5);
    }

    for (const [textRole, surfaceRole] of [
      ['--success', '--success-surface'],
      ['--warning', '--warning-surface'],
      ['--danger', '--danger-surface'],
      ['--destructive-foreground', '--destructive'],
    ] as const) {
      expect(
        contrast(theme[textRole], theme[surfaceRole]),
        `${palette} ${textRole} on ${surfaceRole}`,
      ).toBeGreaterThanOrEqual(4.5);
    }

    expect(
      contrast(theme['--ring'], theme['--background']),
      `${palette} focus ring`,
    ).toBeGreaterThanOrEqual(3);
    expect(
      contrast(theme['--input'], theme['--background']),
      `${palette} input boundary`,
    ).toBeGreaterThanOrEqual(3);
    expect(theme['--selection']).not.toBe(theme['--playing']);
    await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
  }
});

test('Light appearance stays independent of the saved dark palette and fits desktop sizes', async ({
  page,
}) => {
  await page.goto('/');
  await page.emulateMedia({ colorScheme: 'light' });
  const root = page.locator('html');
  await expect(root).toHaveAttribute('data-color-mode', 'light');

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click();
  await chooseSelectOption(page, settings.getByRole('combobox', { name: 'Dark palette' }), 'slate');
  await expect(root).toHaveAttribute('data-color-mode', 'light');
  await expect(root).toHaveAttribute('data-palette', 'slate');

  const theme = await resolvedTheme(page);
  expect(theme['--background']).toBe('#fff');
  expect(theme['--primary']).toBe('#3c54cf');
  expect(theme['--primary-hover']).toBe('#3248ba');
  expect(theme['--primary-pressed']).toBe('#293da3');
  expect(contrast(theme['--primary-foreground'], theme['--primary'])).toBeGreaterThanOrEqual(4.5);
  expect(theme['--selection']).not.toBe(theme['--playing']);
  await expect(
    settings.getByText('Saved for Dark mode (Light mode is currently active).'),
  ).toBeVisible();
  await settings.getByRole('button', { name: 'Close settings' }).click();

  for (const viewport of [
    { width: 800, height: 560 },
    { width: 1366, height: 768 },
  ]) {
    await page.setViewportSize(viewport);
    await expectWorkbenchWithinBounds(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      viewport.width,
    );
  }

  await expectNoSeriousAccessibilityViolations(page);
});

test('selected and playing rows keep separate accessible surfaces in Dark and Light', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=5');
  const player = await startSelectedSong(page);
  await expect(player.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  await page.getByRole('row', { name: /Candle Run/ }).click();

  const playingRow = page.locator('[data-slot="track-row"][data-playing="true"]');
  const selectedRow = page.locator('[data-slot="track-row"][data-selected="true"]');
  await expect(playingRow).toHaveAttribute('aria-current', 'true');
  await expect(playingRow).toHaveAttribute('aria-selected', 'false');
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await page.mouse.move(0, 0);

  for (const mode of ['dark', 'light'] as const) {
    if (mode === 'light') await page.emulateMedia({ colorScheme: 'light' });
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', mode);
    const theme = await resolvedTheme(page);
    await expect
      .poll(() => selectedRow.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe(computedRgb(theme['--selection']));
    await expect
      .poll(() =>
        selectedRow
          .locator('[data-track-title]')
          .evaluate((element) => getComputedStyle(element).color),
      )
      .toBe(computedRgb(theme['--foreground']));
    await expect
      .poll(() => playingRow.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe(computedRgb(theme['--playing']));
    await expect
      .poll(() =>
        playingRow
          .locator('[data-track-title]')
          .evaluate((element) => getComputedStyle(element).color),
      )
      .toBe(computedRgb(theme['--accent-ink']));
    const rowColors = await page.evaluate(() => {
      const selected = document.querySelector<HTMLElement>(
        '[data-slot="track-row"][data-selected="true"]',
      );
      const playing = document.querySelector<HTMLElement>(
        '[data-slot="track-row"][data-playing="true"]',
      );
      if (!selected || !playing) return null;
      const selectedTitle = selected.querySelector<HTMLElement>('[data-track-title]');
      const playingTitle = playing.querySelector<HTMLElement>('[data-track-title]');
      if (!selectedTitle || !playingTitle) return null;
      return {
        selectedBackground: getComputedStyle(selected).backgroundColor,
        selectedForeground: getComputedStyle(selectedTitle).color,
        playingBackground: getComputedStyle(playing).backgroundColor,
        playingForeground: getComputedStyle(playingTitle).color,
      };
    });

    expect(rowColors).not.toBeNull();
    if (rowColors) {
      expect(rowColors.selectedBackground).not.toBe(rowColors.playingBackground);
      expect(
        contrast(rowColors.selectedForeground, rowColors.selectedBackground),
        `${mode} selected row: ${rowColors.selectedForeground} on ${rowColors.selectedBackground}`,
      ).toBeGreaterThanOrEqual(4.5);
      expect(
        contrast(rowColors.playingForeground, rowColors.playingBackground),
        `${mode} playing row: ${rowColors.playingForeground} on ${rowColors.playingBackground}`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  }
});

test('forced colors retain control feedback and reduced motion removes overlay movement', async ({
  page,
}) => {
  await page.goto('/');
  await expectNoSeriousAccessibilityViolations(page);
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).focus();
  const accessibilityStyles = await dialog
    .getByRole('button', { name: 'Appearance', exact: true })
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return { outlineStyle: style.outlineStyle, outlineWidth: style.outlineWidth };
    });
  expect(accessibilityStyles.outlineStyle).not.toBe('none');
  expect(Number.parseFloat(accessibilityStyles.outlineWidth)).toBeGreaterThanOrEqual(2);

  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  const paletteTrigger = dialog.getByRole('combobox', { name: 'Dark palette' });
  await paletteTrigger.click();
  const popup = page.locator('[data-slot="select-content"]');
  await expect(popup).toBeVisible();
  const popupMotion = await popup.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      transitionProperty: style.transitionProperty,
      transitionDuration: style.transitionDuration,
    };
  });
  expect(
    popupMotion.transitionProperty === 'none' ||
      popupMotion.transitionDuration
        .split(',')
        .every((duration) => Number.parseFloat(duration) === 0),
  ).toBe(true);
  await page.keyboard.press('Escape');
});
