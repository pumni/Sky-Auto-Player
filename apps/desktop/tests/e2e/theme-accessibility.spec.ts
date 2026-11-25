import { expect, test } from '@playwright/test';
import { collectRenderedTextContrast } from './renderedContrast';
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
  '--card-foreground',
  '--popover-foreground',
  '--secondary',
  '--secondary-foreground',
  '--muted',
  '--muted-foreground',
  '--accent',
  '--accent-foreground',
  '--primary',
  '--primary-hover',
  '--primary-pressed',
  '--primary-foreground',
  '--border',
  '--overlay-border',
  '--control-hover',
  '--row-hover',
  '--control-pressed',
  '--control-active',
  '--control-disabled',
  '--scrollbar-thumb',
  '--scrollbar-thumb-active',
  '--ring',
  '--input',
  '--selection',
  '--playing',
  '--success',
  '--success-surface',
  '--success-border',
  '--warning',
  '--warning-surface',
  '--warning-border',
  '--danger',
  '--danger-surface',
  '--danger-border',
  '--destructive',
  '--destructive-hover',
  '--destructive-pressed',
  '--destructive-foreground',
  '--scrim',
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

async function expectRenderedTextContrast(page: import('@playwright/test').Page, scene: string) {
  const samples = await page.evaluate(collectRenderedTextContrast);
  const failures = samples.filter((sample) => !sample.exemption && sample.ratio < sample.minimum);
  expect(failures, `${scene} rendered text contrast`).toEqual([]);
  const applicable = samples.filter((sample) => !sample.exemption);
  expect(applicable.length, `${scene} visible text samples`).toBeGreaterThan(0);
  return Math.min(...applicable.map((sample) => sample.ratio));
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

async function resolvedComputedTheme(page: import('@playwright/test').Page) {
  return page.evaluate(
    (roles) => {
      const normalize = (value: string) => {
        const srgb = value.match(
          /^color\(srgb\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)(?:\s*\/\s*[\d.]+)?\)$/i,
        );
        if (srgb) {
          return `rgb(${Math.round(Number(srgb[1]) * 255)}, ${Math.round(Number(srgb[2]) * 255)}, ${Math.round(Number(srgb[3]) * 255)})`;
        }
        return value;
      };
      const probe = document.createElement('div');
      probe.style.position = 'fixed';
      probe.style.visibility = 'hidden';
      probe.style.pointerEvents = 'none';
      document.documentElement.appendChild(probe);
      try {
        return Object.fromEntries(
          roles.map((role) => {
            probe.style.color = `var(${role})`;
            return [role, normalize(getComputedStyle(probe).color)];
          }),
        );
      } finally {
        probe.remove();
      }
    },
    themeRoles as readonly string[],
  ) as Promise<Record<ThemeRole, string>>;
}

function mixSrgb(first: string, weightPercent: number, second: string): string {
  const [r1, g1, b1] = rgb(first);
  const [r2, g2, b2] = rgb(second);
  const rest = 100 - weightPercent;
  return `rgb(${Math.round((r1 * weightPercent + r2 * rest) / 100)}, ${Math.round((g1 * weightPercent + g2 * rest) / 100)}, ${Math.round((b1 * weightPercent + b2 * rest) / 100)})`;
}

// Independent baseline d90e79db inherited roles shared by all five non-Graphite dark palettes.
const BASELINE_SHARED_DARK_ROLES = {
  '--success': '#64c496',
  '--success-surface': '#143326',
  '--success-border': '#489e74',
  '--warning': '#f5c451',
  '--warning-surface': '#3a2c11',
  '--warning-border': '#b4862d',
  '--danger': '#f27a80',
  '--danger-surface': '#3a1c22',
  '--danger-border': '#be5962',
  '--destructive': '#c42b36',
  '--destructive-hover': '#d13a45',
  '--destructive-pressed': '#9e1f2a',
  '--destructive-foreground': '#fff',
  '--scrim': '#08090c',
} as const;

// Independent baseline d90e79db declarations for the five non-Graphite dark palettes.
const BASELINE_NON_GRAPHITE_PALETTES = {
  aurora: {
    '--background': '#0b1018',
    '--surface': '#0d141e',
    '--card': '#101722',
    '--popover': '#151e2b',
    '--card-foreground': '#e7edf6',
    '--popover-foreground': '#e7edf6',
    '--foreground': '#e7edf6',
    '--secondary': '#1a2533',
    '--secondary-foreground': '#e7edf6',
    '--muted': '#151e2b',
    '--muted-foreground': '#8a98ad',
    '--text-secondary': '#b7c2d1',
    '--text-tertiary': '#8a98ad',
    '--border': '#243145',
    '--overlay-border': '#384a66',
    '--input': '#617390',
    '--primary': '#785ddd',
    '--primary-hover': '#825bdb',
    '--primary-pressed': '#6647bd',
    '--primary-foreground': '#fff',
    '--accent-ink': '#9b8cff',
    '--ring': '#9b8cff',
    '--accent': '#1a2533',
    '--accent-foreground': '#e7edf6',
    '--selection': '#152132',
    '--playing': mixSrgb('#9b8cff', 8, '#0b1018'),
    '--control-hover': '#1a2533',
    '--row-hover': '#1a2533',
    '--control-pressed': '#223044',
    '--control-active': '#30425a',
    '--control-disabled': '#718096',
  },
  minimalist: {
    '--background': '#17191d',
    '--surface': '#1b1e22',
    '--card': '#202328',
    '--popover': '#292d33',
    '--card-foreground': '#f0f2f4',
    '--popover-foreground': '#f0f2f4',
    '--foreground': '#f0f2f4',
    '--secondary': '#333941',
    '--secondary-foreground': '#f0f2f4',
    '--muted': '#292d33',
    '--muted-foreground': '#9da5ae',
    '--text-secondary': '#c9ced4',
    '--text-tertiary': '#9da5ae',
    '--border': '#3d444d',
    '--overlay-border': '#4e5662',
    '--input': '#77818d',
    '--primary': '#a0b8d5',
    '--primary-hover': '#c4d0df',
    '--primary-pressed': '#88a4c4',
    '--primary-foreground': '#17191d',
    '--accent-ink': '#a0b8d5',
    '--ring': '#e1e8f0',
    '--accent': '#333941',
    '--accent-foreground': '#f0f2f4',
    '--selection': '#2d343d',
    '--playing': mixSrgb('#a0b8d5', 8, '#17191d'),
    '--control-hover': '#333941',
    '--row-hover': '#333941',
    '--control-pressed': '#3b424b',
    '--control-active': '#4c5661',
    '--control-disabled': '#858d97',
  },
  slate: {
    '--background': '#10171d',
    '--surface': '#13202a',
    '--card': '#17232c',
    '--popover': '#20313d',
    '--card-foreground': '#e6f0f5',
    '--popover-foreground': '#e6f0f5',
    '--foreground': '#e6f0f5',
    '--secondary': '#273946',
    '--secondary-foreground': '#e6f0f5',
    '--muted': '#20313d',
    '--muted-foreground': '#8fa9b8',
    '--text-secondary': '#bad0dc',
    '--text-tertiary': '#8fa9b8',
    '--border': '#304755',
    '--overlay-border': '#425e70',
    '--input': '#6a8699',
    '--primary': '#6ca7d8',
    '--primary-hover': '#83b8e3',
    '--primary-pressed': '#568fbe',
    '--primary-foreground': '#10171d',
    '--accent-ink': '#a8dded',
    '--ring': '#a8dded',
    '--accent': '#273946',
    '--accent-foreground': '#e6f0f5',
    '--selection': '#2a3d4a',
    '--playing': mixSrgb('#a8dded', 8, '#10171d'),
    '--control-hover': '#273946',
    '--row-hover': '#273946',
    '--control-pressed': '#314754',
    '--control-active': '#405b6a',
    '--control-disabled': '#879ca9',
  },
  cyberpunk: {
    '--background': '#130f1d',
    '--surface': '#191225',
    '--card': '#1d1529',
    '--popover': '#291c37',
    '--card-foreground': '#f7ecfb',
    '--popover-foreground': '#f7ecfb',
    '--foreground': '#f7ecfb',
    '--secondary': '#352445',
    '--secondary-foreground': '#f7ecfb',
    '--muted': '#291c37',
    '--muted-foreground': '#b297bd',
    '--text-secondary': '#dbc6e4',
    '--text-tertiary': '#b297bd',
    '--border': '#4b2d59',
    '--overlay-border': '#663d78',
    '--input': '#976595',
    '--primary': '#d95abf',
    '--primary-hover': '#ef78d4',
    '--primary-pressed': '#c954b1',
    '--primary-foreground': '#130f1d',
    '--accent-ink': '#f4a8e3',
    '--ring': '#f4a8e3',
    '--accent': '#352445',
    '--accent-foreground': '#f7ecfb',
    '--selection': '#3a274a',
    '--playing': mixSrgb('#f4a8e3', 8, '#130f1d'),
    '--control-hover': '#352445',
    '--row-hover': '#352445',
    '--control-pressed': '#422d54',
    '--control-active': '#553b6a',
    '--control-disabled': '#ae8fb8',
  },
  classic: {
    '--background': '#171512',
    '--surface': '#1c1915',
    '--card': '#211d18',
    '--popover': '#2c261f',
    '--card-foreground': '#f2eadc',
    '--popover-foreground': '#f2eadc',
    '--foreground': '#f2eadc',
    '--secondary': '#393126',
    '--secondary-foreground': '#f2eadc',
    '--muted': '#2c261f',
    '--muted-foreground': '#a89882',
    '--text-secondary': '#d2c2aa',
    '--text-tertiary': '#a89882',
    '--border': '#4b4033',
    '--overlay-border': '#615342',
    '--input': '#897864',
    '--primary': '#c99242',
    '--primary-hover': '#e0aa5b',
    '--primary-pressed': '#a87832',
    '--primary-foreground': '#171512',
    '--accent-ink': '#e8c17a',
    '--ring': '#e8c17a',
    '--accent': '#393126',
    '--accent-foreground': '#f2eadc',
    '--selection': '#352c21',
    '--playing': mixSrgb('#e8c17a', 8, '#171512'),
    '--control-hover': '#393126',
    '--row-hover': '#393126',
    '--control-pressed': '#463b2d',
    '--control-active': '#5a4a32',
    '--control-disabled': '#a69a87',
  },
} as const;

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

    for (const surfaceRole of [
      '--secondary',
      '--control-hover',
      '--control-pressed',
      '--control-active',
    ] as const) {
      expect(
        contrast(theme['--foreground'], theme[surfaceRole]),
        `${palette} foreground on ${surfaceRole}`,
      ).toBeGreaterThanOrEqual(4.5);
    }
    expect(
      contrast(theme['--secondary-foreground'], theme['--secondary']),
      `${palette} secondary foreground on its control track`,
    ).toBeGreaterThanOrEqual(4.5);
    expect(theme['--secondary']).not.toBe(theme['--control-active']);

    for (const [textRole, surfaceRole] of [
      ['--success', '--success-surface'],
      ['--warning', '--warning-surface'],
      ['--danger', '--danger-surface'],
      ['--destructive-foreground', '--destructive'],
      ['--destructive-foreground', '--destructive-hover'],
      ['--destructive-foreground', '--destructive-pressed'],
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

test('primary and outlined actions use computed theme colors in every dark palette', async ({
  page,
}) => {
  await page.goto('/');
  const createPlaylist = page.locator('[data-slot="library-empty-playlists-create"]');
  await expect(createPlaylist).toBeVisible();
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const primaryAction = page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true });
  await expect(primaryAction).toBeVisible();
  await expect(primaryAction).toBeEnabled();

  for (const palette of palettes) {
    await selectPalette(page, palette);
    const theme = await resolvedTheme(page);
    const createRest = await createPlaylist.evaluate((element) => {
      const style = getComputedStyle(element);
      let ancestor = element.parentElement;
      let surfaceBackground = 'rgba(0, 0, 0, 0)';
      while (ancestor) {
        const bg = getComputedStyle(ancestor).backgroundColor;
        if (bg !== 'rgba(0, 0, 0, 0)' && bg !== 'transparent') {
          surfaceBackground = bg;
          break;
        }
        ancestor = ancestor.parentElement;
      }
      return {
        background: style.backgroundColor,
        border: style.borderColor,
        color: style.color,
        surfaceBackground,
      };
    });
    expect(createRest.background).toBe('rgba(0, 0, 0, 0)');
    expect(createRest.border).toBe(computedRgb(theme['--border']));
    expect(contrast(createRest.color, createRest.surfaceBackground)).toBeGreaterThanOrEqual(4.5);
    await createPlaylist.hover();
    await expect
      .poll(() => createPlaylist.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe(computedRgb(theme['--control-hover']));
    await page.mouse.down();
    await expect
      .poll(() => createPlaylist.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe(computedRgb(theme['--control-pressed']));
    await page.mouse.move(0, 0);
    await page.mouse.up();

    const rest = await primaryAction.evaluate((element) => {
      const style = getComputedStyle(element);
      return { background: style.backgroundColor, foreground: style.color };
    });
    expect(rest.background, `${palette} primary fill`).toBe(computedRgb(theme['--primary']));
    expect(rest.foreground, `${palette} primary foreground`).toBe(
      computedRgb(theme['--primary-foreground']),
    );

    await primaryAction.hover();
    await expect
      .poll(() => primaryAction.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe(computedRgb(theme['--primary-hover']));

    await page.mouse.down();
    await expect
      .poll(() => primaryAction.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe(computedRgb(theme['--primary-pressed']));
    await page.mouse.move(0, 0);
    await page.mouse.up();
  }
});

test('PlayerTransport Play uses the shared primary action states in Dark and Light', async ({
  page,
}) => {
  await page.goto('/');
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const play = page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true });
  await expect(play).toBeEnabled();

  const assertPrimaryStates = async () => {
    const theme = await resolvedTheme(page);
    await expect
      .poll(async () => {
        const colors = await play.evaluate((element) => {
          const style = getComputedStyle(element);
          return { background: style.backgroundColor, foreground: style.color };
        });
        return colors;
      })
      .toEqual({
        background: computedRgb(theme['--primary']),
        foreground: computedRgb(theme['--primary-foreground']),
      });

    await play.hover();
    await expect
      .poll(() => play.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe(computedRgb(theme['--primary-hover']));

    const bounds = await play.boundingBox();
    expect(bounds).not.toBeNull();
    if (bounds) {
      await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
      await page.mouse.down();
      await expect
        .poll(() => play.evaluate((element) => getComputedStyle(element).backgroundColor))
        .toBe(computedRgb(theme['--primary-pressed']));
      await page.mouse.move(0, 0);
      await page.mouse.up();
    }
    await expect(play).toBeVisible();
    await expect(play).toHaveAccessibleName('Play');
  };

  for (const palette of palettes) {
    await selectPalette(page, palette);
    await assertPrimaryStates(palette);
  }

  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  await assertPrimaryStates('Light');
});

test('PlayerTransport Pause and Resume retain the filled shared primary role in a dry run', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await page
    .getByRole('dialog', { name: 'Playback profile' })
    .getByRole('button', { name: 'Test playback (no input)' })
    .click();
  await page.mouse.move(0, 0);
  const pauseControl = player.getByRole('button', { name: 'Pause', exact: true });
  expect(await pauseControl.evaluate((element) => element.matches(':hover'))).toBe(false);

  const theme = await resolvedTheme(page);
  const expectPrimaryState = async (name: 'Pause' | 'Resume') => {
    await page.mouse.move(0, 0);
    const action = player.getByRole('button', { name, exact: true });
    await expect(action).toBeVisible();
    await expect
      .poll(() =>
        action.evaluate((element) => {
          const style = getComputedStyle(element);
          return { background: style.backgroundColor, foreground: style.color };
        }),
      )
      .toEqual({
        background: computedRgb(theme['--primary']),
        foreground: computedRgb(theme['--primary-foreground']),
      });
  };

  await expectPrimaryState('Pause');
  await player.getByRole('button', { name: 'Pause', exact: true }).click();
  await expect(player.locator('[data-slot="player-track-status"]')).toHaveText('Paused');
  await expectPrimaryState('Resume');
  await player.getByRole('button', { name: 'Resume', exact: true }).click();
  await expect(player.locator('[data-slot="player-track-status"]')).toHaveText('Playing');
  await expectPrimaryState('Pause');
});

test('destructive confirmation uses its rest, hover, and pressed theme colors', async ({
  page,
}) => {
  await page.goto('/');
  await page.locator('[data-slot="library-add-button"]').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await createDialog.getByLabel('Playlist name').fill('Variant check');
  await createDialog.getByRole('button', { name: 'Create' }).click();

  for (const palette of palettes) {
    await selectPalette(page, palette);
    const theme = await resolvedTheme(page);
    await page.getByRole('button', { name: 'More actions for Variant check' }).click();
    await page.getByRole('menuitem', { name: 'Delete playlist' }).click();
    const dialog = page.getByRole('alertdialog', { name: /Delete/ });
    const remove = dialog.getByRole('button', { name: 'Delete playlist' });

    const expectFill = async (
      role: '--destructive' | '--destructive-hover' | '--destructive-pressed',
    ) => {
      await expect
        .poll(() => remove.evaluate((element) => getComputedStyle(element).backgroundColor))
        .toBe(computedRgb(theme[role]));
    };

    await expectFill('--destructive');
    await remove.hover();
    await expectFill('--destructive-hover');
    const [buttonBounds, titleBounds] = await Promise.all([
      remove.boundingBox(),
      dialog.getByRole('heading').boundingBox(),
    ]);
    expect(buttonBounds).not.toBeNull();
    expect(titleBounds).not.toBeNull();
    if (buttonBounds && titleBounds) {
      await page.mouse.move(
        buttonBounds.x + buttonBounds.width / 2,
        buttonBounds.y + buttonBounds.height / 2,
      );
      await page.mouse.down();
      await expectFill('--destructive-pressed');
      await page.mouse.move(
        titleBounds.x + titleBounds.width / 2,
        titleBounds.y + titleBounds.height / 2,
      );
      await page.mouse.up();
    }

    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toBeHidden();
  }
  await expectNoSeriousAccessibilityViolations(page);
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
  await expect(settings.locator('[data-slot="appearance-active-summary"]')).toHaveText(
    'Active: Light. Saved dark palette: Slate.',
  );
  await settings.getByRole('button', { name: 'Close settings' }).click();

  for (const viewport of [
    { width: 800, height: 560 },
    { width: 1024, height: 640 },
    { width: 1200, height: 760 },
    { width: 1366, height: 768 },
    { width: 1920, height: 1080 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await expect(page.locator('[data-slot="workbench"]')).toHaveAttribute(
      'data-workbench-width',
      String(viewport.width),
    );
    await expectWorkbenchWithinBounds(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      viewport.width,
    );
  }

  await expectNoSeriousAccessibilityViolations(page);
});

test('Light rendered text remains readable across library, Settings, Diagnostics, recovery, and Update', async ({
  page,
}) => {
  test.setTimeout(180_000);
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/?mockDiagnosticsStatus=degraded');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  expect(await expectRenderedTextContrast(page, 'Light library')).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  expect(await expectRenderedTextContrast(page, 'Light Settings')).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);
  await settings.getByRole('button', { name: 'Close settings' }).click();

  await page.locator('button[aria-label$="utility panel"]').click();
  const utility = page.getByRole('region', { name: 'Utility: Song Details' });
  await expect(utility).toBeVisible();
  await utility.getByRole('tab', { name: 'Runtime' }).click();
  const diagnostics = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(diagnostics).toBeVisible();
  await diagnostics.getByRole('tab', { name: 'Timing' }).click();
  await expect(diagnostics).toContainText(
    /No active playback session|Timing unavailable|Latest sender pre-call max/,
  );
  expect(
    await expectRenderedTextContrast(page, 'Light Runtime Diagnostics'),
  ).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);

  await page.goto('/?mockStartFailure=target_not_found');
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  const recovery = page.getByRole('alert');
  await expect(recovery).toContainText('Sky window was not found');
  await expect.poll(() => recovery.evaluate((el) => getComputedStyle(el).opacity)).toBe('1');
  expect(await expectRenderedTextContrast(page, 'Light playback recovery')).toBeGreaterThanOrEqual(
    4.5,
  );
  await expectNoSeriousAccessibilityViolations(page);

  await page.goto('/?mockReleaseNotes=short');
  await page.getByRole('button', { name: /Open update 4\.0\.0-alpha\.2-mock/ }).click();
  const update = page.getByRole('dialog');
  await expect(update).toContainText('Version 4.0.0-alpha.2-mock is available');
  expect(await expectRenderedTextContrast(page, 'Light Update dialog')).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);

  await page.goto('/?mockLibraryState=empty');
  await expect(page.locator('[data-slot="library-empty-state"]')).toContainText(
    'No Built-in Songs are available',
  );
  expect(await expectRenderedTextContrast(page, 'Light empty library')).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);

  await page.goto('/?mockLibraryState=unavailable');
  await expect(page.locator('[data-slot="library-empty-state"]')).toContainText(
    'Built-in Songs are unavailable',
  );
  expect(
    await expectRenderedTextContrast(page, 'Light unavailable catalog'),
  ).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);

  await page.goto('/?mockNavigationFailures=1');
  await expect(page.getByRole('alert')).toContainText('Couldn’t check Built-in Songs or playlists');
  expect(
    await expectRenderedTextContrast(page, 'Light library navigation error'),
  ).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);

  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/?mockLibraryState=empty');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  expect(await expectRenderedTextContrast(page, 'Graphite empty library')).toBeGreaterThanOrEqual(
    4.5,
  );
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
        selectedRail: getComputedStyle(selected).borderLeftColor,
        playingBackground: getComputedStyle(playing).backgroundColor,
        playingForeground: getComputedStyle(playingTitle).color,
        playingRail: getComputedStyle(playing).borderLeftColor,
      };
    });

    expect(rowColors).not.toBeNull();
    if (rowColors) {
      expect(rowColors.selectedBackground).not.toBe(rowColors.playingBackground);
      expect(rowColors.selectedRail).toBe(computedRgb(theme['--accent-ink']));
      expect(rowColors.playingRail).not.toBe(computedRgb(theme['--accent-ink']));
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

test('selected and playing row indicators and actions remain visible without hover', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=5');
  await startSelectedSong(page);
  const row = page.getByRole('row', { name: /Blue Bird/ });
  await expect(row).toHaveAttribute('data-state', 'selected-playing');
  await page.mouse.move(0, 0);

  const theme = await resolvedTheme(page);
  const state = await row.evaluate((element) => {
    const style = getComputedStyle(element);
    const like = element.querySelector<HTMLButtonElement>('button[aria-label*="Liked Songs"]');
    const actions = element.querySelector<HTMLButtonElement>('button[aria-label^="More actions"]');
    return {
      border: style.borderLeftColor,
      background: style.backgroundColor,
      likeOpacity: like ? getComputedStyle(like).opacity : null,
      actionsOpacity: actions ? getComputedStyle(actions).opacity : null,
      nowPlaying: element.querySelector('svg.lucide-music-2[aria-hidden="true"]') !== null,
    };
  });
  expect(state.border).toBe(computedRgb(theme['--accent-ink']));
  expect(state.background).toBe(computedRgb(theme['--selection']));
  expect(state.likeOpacity).toBe('1');
  expect(state.actionsOpacity).toBe('1');
  expect(state.nowPlaying).toBe(true);

  await row.hover();
  await expect
    .poll(() => row.evaluate((element) => getComputedStyle(element).backgroundColor))
    .toBe(computedRgb(theme['--selection']));
  await expectNoSeriousAccessibilityViolations(page);
});

test('forced colors retain control feedback and reduced motion removes overlay movement', async ({
  page,
}) => {
  await page.goto('/');
  await expectNoSeriousAccessibilityViolations(page);
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  const idleRow = page.locator('[data-slot="track-row"][data-state="idle"]').first();
  const idleRowColors = await idleRow.evaluate((element) => {
    const grid = element.closest<HTMLElement>('[role="grid"]');
    const backgroundSurfaces: string[] = [];
    let ancestor = element.parentElement;
    while (ancestor) {
      backgroundSurfaces.push(getComputedStyle(ancestor).backgroundColor);
      ancestor = ancestor.parentElement;
    }
    const surfaceBackground = backgroundSurfaces.find(
      (color) => color !== 'rgba(0, 0, 0, 0)' && color !== 'transparent',
    );
    return {
      rowBorder: getComputedStyle(element).borderLeftColor,
      gridSurface: grid ? getComputedStyle(grid).backgroundColor : null,
      surfaceBackground: surfaceBackground ?? null,
    };
  });
  expect(idleRowColors.gridSurface).not.toBeNull();
  expect(idleRowColors.surfaceBackground).not.toBeNull();
  expect(idleRowColors.rowBorder).toBe(idleRowColors.surfaceBackground);

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

test('Light row hover, segmented boundary, forced-colors segmented label, and nested Select border meet design contracts', async ({
  page,
}) => {
  await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=5');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  const lightTheme = await resolvedTheme(page);

  // Light row hover vs selection vs playing vs card
  await startSelectedSong(page);
  await page.getByRole('row', { name: /Candle Run/ }).click();
  const idleRow = page.locator('[data-slot="track-row"][data-state="idle"]').first();
  await idleRow.hover();
  await expect
    .poll(() => idleRow.evaluate((el) => getComputedStyle(el).backgroundColor))
    .toBe(computedRgb(lightTheme['--row-hover']));
  expect(lightTheme['--row-hover']).not.toBe(lightTheme['--selection']);
  expect(lightTheme['--row-hover']).not.toBe(lightTheme['--playing']);
  expect(lightTheme['--row-hover']).not.toBe(lightTheme['--card']);
  const lightOrdinalColor = await idleRow
    .locator('[role="gridcell"]')
    .first()
    .evaluate((el) => getComputedStyle(el).color);
  expect(contrast(lightOrdinalColor, lightTheme['--row-hover'])).toBeGreaterThanOrEqual(4.5);
  expect(
    contrast(lightTheme['--text-secondary'], lightTheme['--row-hover']),
  ).toBeGreaterThanOrEqual(4.5);

  // Light segmented indicator boundary and contrast
  await page.locator('button[aria-label$="utility panel"]').click();
  const utility = page.getByRole('region', { name: 'Utility: Song Details' });
  await expect(utility).toBeVisible();
  await utility.getByRole('tab', { name: 'Runtime' }).click();
  const diagnostics = page.getByRole('region', { name: 'Utility: Diagnostics' });
  await expect(diagnostics).toBeVisible();
  const segList = diagnostics.getByRole('tablist', { name: 'Diagnostics views' });
  const segIndicator = segList.locator('[data-slot="tabs-indicator"]');
  const activeTab = segList.getByRole('tab', { name: 'Performance' });
  const segStyles = await Promise.all([
    segIndicator.evaluate((el) => {
      const s = getComputedStyle(el);
      return { bg: s.backgroundColor, border: s.borderColor };
    }),
    activeTab.evaluate((el) => getComputedStyle(el).color),
  ]);
  expect(segStyles[0].bg).toBe(computedRgb(lightTheme['--control-active']));
  expect(segStyles[0].border).toBe(computedRgb(lightTheme['--input']));
  expect(contrast(segStyles[1], segStyles[0].bg)).toBeGreaterThanOrEqual(4.5);

  // Light + forced-colors segmented active tab readability (HighlightText on Highlight)
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'active' });
  const forcedSeg = await Promise.all([
    segIndicator.evaluate((el) => getComputedStyle(el).backgroundColor),
    activeTab.evaluate((el) => getComputedStyle(el).color),
  ]);
  expect(forcedSeg[0]).not.toBe('rgba(0, 0, 0, 0)');
  expect(forcedSeg[1]).not.toBe(forcedSeg[0]);

  // Nested Select popup uses --overlay-border in Light and Graphite
  await page.emulateMedia({ colorScheme: 'light', forcedColors: 'none' });
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  const profileDialog = page.getByRole('dialog', { name: 'Playback profile' });
  await profileDialog.getByText('Advanced timing', { exact: true }).click();
  await profileDialog.getByRole('combobox', { name: 'Base hold' }).click();
  const selectPopup = page.locator('[data-slot="select-content"]:visible');
  await expect(selectPopup).toBeVisible();
  expect(await selectPopup.evaluate((el) => getComputedStyle(el).borderColor)).toBe(
    computedRgb(lightTheme['--overlay-border']),
  );

  await page.emulateMedia({ colorScheme: 'dark', forcedColors: 'none' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  const darkTheme = await resolvedTheme(page);
  await expect(selectPopup).toBeVisible();
  expect(await selectPopup.evaluate((el) => getComputedStyle(el).borderColor)).toBe(
    computedRgb(darkTheme['--overlay-border']),
  );
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await expect(profileDialog).toBeHidden();

  // Graphite row hover ordinal contrast and distinct idle/hover/selected/playing states
  await idleRow.hover();
  await expect
    .poll(() => idleRow.evaluate((el) => getComputedStyle(el).backgroundColor))
    .toBe(computedRgb(darkTheme['--row-hover']));
  const darkOrdinalColor = await idleRow
    .locator('[role="gridcell"]')
    .first()
    .evaluate((el) => getComputedStyle(el).color);
  expect(contrast(darkOrdinalColor, darkTheme['--row-hover'])).toBeGreaterThanOrEqual(4.5);
  expect(darkTheme['--row-hover']).not.toBe(darkTheme['--card']);
  expect(darkTheme['--row-hover']).not.toBe(darkTheme['--selection']);
  expect(darkTheme['--row-hover']).not.toBe(darkTheme['--playing']);
});

test('Graphite 2.0 47-role surfaces, shared Light across all saved palettes, and non-Graphite baseline isolation hold', async ({
  page,
}) => {
  await page.goto('/');
  expect(themeRoles).toHaveLength(47);

  const graphiteDark = await resolvedComputedTheme(page);
  expect(graphiteDark['--background']).toBe(computedRgb('#121314'));
  expect(graphiteDark['--surface']).toBe(computedRgb('#121314'));
  expect(graphiteDark['--card']).toBe(computedRgb('#191a1b'));
  expect(graphiteDark['--popover']).toBe(computedRgb('#202122'));
  expect(graphiteDark['--selection']).toBe(computedRgb('#303132'));
  expect(graphiteDark['--playing']).toBe(computedRgb('#202326'));
  expect(graphiteDark['--scrollbar-thumb']).toBe(computedRgb('#777a7e'));
  expect(graphiteDark['--scrollbar-thumb-active']).toBe(computedRgb('#a4a6aa'));
  expect(graphiteDark['--scrim']).toBe(computedRgb('#08090c'));

  // Content canvas vs chrome separation in Dark and Light
  for (const mode of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: mode });
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', mode);
    const activeTheme = await resolvedComputedTheme(page);
    const regions = await page.evaluate(() => ({
      titlebar: getComputedStyle(document.querySelector('[data-slot="titlebar"]')!).backgroundColor,
      navigator: getComputedStyle(document.querySelector('[data-pane="navigator"]')!)
        .backgroundColor,
      browser: getComputedStyle(document.querySelector('[data-pane="browser"]')!).backgroundColor,
      dock: getComputedStyle(document.querySelector('[data-slot="player-dock"]')!).backgroundColor,
    }));
    expect(regions.titlebar).toBe(activeTheme['--card']);
    expect(regions.navigator).toBe(activeTheme['--card']);
    expect(regions.dock).toBe(activeTheme['--card']);
    expect(regions.browser).toBe(activeTheme['--surface']);
    expect(regions.browser).not.toBe(regions.titlebar);
  }

  // Every saved dark palette resolves identical 47 shared Light roles and restores cleanly on Dark
  await page.emulateMedia({ colorScheme: 'light' });
  const referenceLight = await resolvedComputedTheme(page);
  expect(referenceLight['--surface']).toBe(computedRgb('#fff'));
  expect(referenceLight['--card']).toBe(computedRgb('#f7f7f8'));
  expect(referenceLight['--selection']).toBe(computedRgb('#e4e5e7'));
  expect(referenceLight['--playing']).toBe(computedRgb('#edf0f4'));
  expect(referenceLight['--scrollbar-thumb']).toBe(computedRgb('#85888e'));
  expect(referenceLight['--scrollbar-thumb-active']).toBe(computedRgb('#5d6066'));
  expect(referenceLight['--scrim']).toBe(computedRgb('#16181b'));

  for (const palette of palettes) {
    await page.emulateMedia({ colorScheme: 'dark' });
    await selectPalette(page, palette);
    const darkSnapshot = await resolvedComputedTheme(page);

    if (palette !== 'graphite') {
      const baseline = BASELINE_NON_GRAPHITE_PALETTES[palette];
      for (const role of themeRoles) {
        if (role === '--playing' || role === '--scrollbar-thumb') {
          const expectedRgb =
            role === '--playing'
              ? rgb(baseline['--playing'])
              : rgb(mixSrgb(baseline['--text-secondary'], 70, baseline['--background']));
          const actualRgb = rgb(darkSnapshot[role]);
          for (let channel = 0; channel < 3; channel += 1) {
            expect(
              Math.abs((actualRgb[channel] ?? 0) - (expectedRgb[channel] ?? 0)),
              `${palette} ${role} channel ${channel}`,
            ).toBeLessThanOrEqual(1);
          }
          continue;
        }
        const raw =
          role === '--scrollbar-thumb-active'
            ? baseline['--text-secondary']
            : role in baseline
              ? baseline[role as keyof typeof baseline]
              : BASELINE_SHARED_DARK_ROLES[role as keyof typeof BASELINE_SHARED_DARK_ROLES];
        expect(darkSnapshot[role], `${palette} ${role} baseline oracle`).toBe(computedRgb(raw));
      }
    }

    await page.emulateMedia({ colorScheme: 'light' });
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
    await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
    const lightSnapshot = await resolvedComputedTheme(page);
    expect(lightSnapshot, `${palette} saved palette in Light`).toEqual(referenceLight);

    await page.emulateMedia({ colorScheme: 'dark' });
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
    await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
    const restoredDark = await resolvedComputedTheme(page);
    expect(restoredDark, `${palette} restored after Light roundtrip`).toEqual(darkSnapshot);
  }
});

test('rendered control contrast, Switch visual cues, M15 child instant switch, and input boundaries hold in Dark and Light', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');

  for (const mode of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: mode, reducedMotion: 'no-preference' });
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', mode);
    const theme = await resolvedComputedTheme(page);

    // M15 guarantees immediate child update on mode switch, no stuck data-theme-applying,
    // and immediate restoration of normal child transition durations afterward.
    expect(
      await page.evaluate(() => document.documentElement.hasAttribute('data-theme-applying')),
    ).toBe(false);
    const firstRow = page.locator('[data-slot="track-row"]').first();
    expect(await firstRow.evaluate((el) => getComputedStyle(el).color)).toBe(theme['--foreground']);
    expect(await firstRow.evaluate((el) => getComputedStyle(el).transitionDuration)).toBe('0.12s');
    expect(
      await page
        .locator('[data-slot="library-empty-playlists-create"]')
        .evaluate((el) => getComputedStyle(el).transitionDuration),
    ).toBe('0.12s');
    expect(
      await page
        .locator('[data-slot="library-nav-item"][data-active="true"]')
        .evaluate((el) => getComputedStyle(el).backgroundColor),
    ).toBe(theme['--selection']);

    // GlobalSearch essential border + placeholder rendered contrast
    const searchBox = page.locator('[data-slot="global-search"]');
    const searchStyles = await searchBox.evaluate((el) => {
      const style = getComputedStyle(el);
      return { border: style.borderTopColor, bg: style.backgroundColor };
    });
    expect(searchStyles.border).toBe(theme['--input']);
    expect(searchStyles.bg).toBe(theme['--surface']);
    expect(contrast(searchStyles.border, theme['--card'])).toBeGreaterThanOrEqual(3);
    expect(contrast(searchStyles.border, searchStyles.bg)).toBeGreaterThanOrEqual(3);
    expect(await expectRenderedTextContrast(page, `${mode} workbench`)).toBeGreaterThanOrEqual(4.5);

    // TrackRow wide typography (14/20/46px)
    const rowTypography = await firstRow.evaluate((el) => {
      const style = getComputedStyle(el);
      return { fontSize: style.fontSize, lineHeight: style.lineHeight, height: style.height };
    });
    expect(rowTypography).toEqual({ fontSize: '14px', lineHeight: '20px', height: '46px' });

    // PlaylistCreateDialog uses elevated popover surface + overlay border + high-contrast Input
    await page.locator('[data-slot="library-add-button"]').click();
    const createDialog = page.getByRole('dialog', { name: 'New playlist' });
    await expect(createDialog).toBeVisible();
    await createDialog.getByRole('button', { name: 'Cancel' }).focus();
    await expect
      .poll(() =>
        createDialog.locator('input').evaluate((el) => getComputedStyle(el).borderTopColor),
      )
      .toBe(theme['--input']);
    const dialogMetrics = await createDialog.evaluate((el) => {
      const style = getComputedStyle(el);
      const input = el.querySelector<HTMLInputElement>('input');
      const inputStyle = input ? getComputedStyle(input) : null;
      return {
        bg: style.backgroundColor,
        border: style.borderTopColor,
        inputBg: inputStyle?.backgroundColor ?? null,
        inputBorder: inputStyle?.borderTopColor ?? null,
      };
    });
    expect(dialogMetrics.bg).toBe(theme['--popover']);
    expect(dialogMetrics.border).toBe(theme['--overlay-border']);
    expect(dialogMetrics.inputBg).toBe(theme['--surface']);
    expect(dialogMetrics.inputBorder).toBe(theme['--input']);
    expect(contrast(dialogMetrics.inputBorder!, dialogMetrics.inputBg!)).toBeGreaterThanOrEqual(3);
    expect(contrast(dialogMetrics.inputBorder!, dialogMetrics.bg)).toBeGreaterThanOrEqual(3);
    expect(
      await expectRenderedTextContrast(page, `${mode} playlist create dialog`),
    ).toBeGreaterThanOrEqual(4.5);
    await page.keyboard.press('Escape');
    await expect(createDialog).toBeHidden();

    // Settings elevated surface, Switch track/thumb visual cues (checked & unchecked), and Select border
    await page.getByRole('button', { name: 'Open settings' }).click();
    const settings = page.getByRole('dialog', { name: 'Settings' });
    await expect(settings).toBeVisible();
    const settingsSurface = await settings.evaluate((el) => {
      const s = getComputedStyle(el);
      return { bg: s.backgroundColor, border: s.borderTopColor };
    });
    expect(settingsSurface.bg).toBe(theme['--popover']);
    expect(settingsSurface.border).toBe(theme['--overlay-border']);

    const switchEl = settings.locator('[data-slot="switch"]');
    const switchThumb = switchEl.locator('[data-slot="switch-thumb"]');
    const checkedSwitch = await Promise.all([
      switchEl.evaluate((el) => getComputedStyle(el).backgroundColor),
      switchThumb.evaluate((el) => getComputedStyle(el).backgroundColor),
    ]);
    expect(checkedSwitch[0]).toBe(theme['--primary']);
    expect(contrast(checkedSwitch[1], checkedSwitch[0])).toBeGreaterThanOrEqual(3);
    await switchEl.click();
    await expect
      .poll(() => switchEl.evaluate((el) => getComputedStyle(el).backgroundColor))
      .toBe(theme['--input']);
    const uncheckedSwitch = await Promise.all([
      switchEl.evaluate((el) => getComputedStyle(el).backgroundColor),
      switchThumb.evaluate((el) => getComputedStyle(el).backgroundColor),
    ]);
    expect(contrast(uncheckedSwitch[0], theme['--surface'])).toBeGreaterThanOrEqual(3);
    expect(contrast(uncheckedSwitch[1], uncheckedSwitch[0])).toBeGreaterThanOrEqual(3);
    await switchEl.click();

    const selectTrigger = settings.locator('[data-slot="select-trigger"]').first();
    const triggerBorder = await selectTrigger.evaluate((el) => getComputedStyle(el).borderTopColor);
    expect(triggerBorder).toBe(theme['--input']);
    expect(contrast(triggerBorder, theme['--background'])).toBeGreaterThanOrEqual(3);
    expect(
      await expectRenderedTextContrast(page, `${mode} settings playback`),
    ).toBeGreaterThanOrEqual(4.5);

    // M15 lifecycle while Settings dialog is open: switching color scheme updates dialog
    // surface immediately, never leaves data-theme-applying stuck, and keeps dialog usable.
    const oppositeMode = mode === 'dark' ? 'light' : 'dark';
    await page.emulateMedia({ colorScheme: oppositeMode });
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', oppositeMode);
    const oppositeTheme = await resolvedComputedTheme(page);
    expect(
      await page.evaluate(() => document.documentElement.hasAttribute('data-theme-applying')),
    ).toBe(false);
    expect(await settings.evaluate((el) => getComputedStyle(el).backgroundColor)).toBe(
      oppositeTheme['--popover'],
    );
    await page.emulateMedia({ colorScheme: mode });
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', mode);
    await page.keyboard.press('Escape');
    await expect(settings).toBeHidden();
  }
});

test('shared primitive motion durations, normal-motion overlay interruption, and reduced motion hold', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'no-preference' });

  // M01 Button (120ms), M02 TrackRow (120ms), M04 Transport icon (100ms), M13 Scrollbar (180ms)
  expect(
    await page
      .locator('[data-slot="library-empty-playlists-create"]')
      .evaluate((el) => getComputedStyle(el).transitionDuration),
  ).toBe('0.12s');
  expect(
    await page
      .locator('[data-slot="track-row"]')
      .first()
      .evaluate((el) => getComputedStyle(el).transitionDuration),
  ).toBe('0.12s');
  expect(
    await page
      .locator('[data-slot="primary-transport"] svg')
      .first()
      .evaluate((el) => getComputedStyle(el).transitionDuration),
  ).toBe('0.1s');
  expect(
    await page
      .locator('[data-slot="scroll-area-scrollbar"]')
      .first()
      .evaluate((el) => {
        const s = getComputedStyle(el);
        return { prop: s.transitionProperty, dur: s.transitionDuration };
      }),
  ).toEqual({ prop: 'opacity', dur: '0.18s' });

  // M06 Tooltip (100ms enter)
  await page.getByRole('button', { name: 'Open settings' }).hover();
  const tooltip = page.locator('[data-slot="tooltip-content"]:visible');
  await expect(tooltip).toBeVisible();
  expect(
    await tooltip.evaluate((el) => {
      const s = getComputedStyle(el);
      return { prop: s.transitionProperty, dur: s.transitionDuration };
    }),
  ).toEqual({ prop: 'opacity', dur: '0.1s' });
  await page.mouse.move(0, 0);

  // M05 TabsIndicator (150ms)
  const utilityToggle = page.locator('button[aria-label$="utility panel"]');
  await utilityToggle.click();
  expect(
    await page
      .locator('[data-slot="tabs-indicator"]')
      .first()
      .evaluate((el) => getComputedStyle(el).transitionDuration),
  ).toBe('0.15s');
  await utilityToggle.click();

  // M07 Select (fade-only 150ms) & M09 Dialog (opacity + scale 200ms)
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await expect(settings).toBeVisible();
  const dialogMotion = await settings.evaluate((el) => {
    const s = getComputedStyle(el);
    return { prop: s.transitionProperty, dur: s.transitionDuration };
  });
  expect(dialogMotion.prop).toContain('opacity');
  expect(dialogMotion.prop).toContain('scale');
  expect(dialogMotion.dur).toContain('0.2s');

  await settings.locator('[data-slot="select-trigger"]').first().click();
  const selectPopup = page.locator('[data-slot="select-content"]:visible');
  await expect(selectPopup).toBeVisible();
  expect(
    await selectPopup.evaluate((el) => {
      const s = getComputedStyle(el);
      return { prop: s.transitionProperty, dur: s.transitionDuration };
    }),
  ).toEqual({ prop: 'opacity', dur: '0.15s' });
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await expect(settings).toBeHidden();

  // M08 Popover (fade-only 180ms)
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  const popover = page.locator('[data-slot="popover-content"]:visible');
  await expect(popover).toBeVisible();
  expect(
    await popover.evaluate((el) => {
      const s = getComputedStyle(el);
      return { prop: s.transitionProperty, dur: s.transitionDuration };
    }),
  ).toEqual({ prop: 'opacity', dur: '0.18s' });
  await page.keyboard.press('Escape');
  await expect(popover).toBeHidden();

  // M14 Admission entrance (120ms opacity) + 640x448 cancel reachability
  await page.setViewportSize({ width: 640, height: 448 });
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const admission = page.locator('[data-slot="playback-admission"]');
  await expect(admission).toBeVisible();
  expect(
    await admission.evaluate((el) => {
      const s = getComputedStyle(el);
      return { prop: s.transitionProperty, dur: s.transitionDuration };
    }),
  ).toEqual({ prop: 'opacity', dur: '0.12s' });
  const notNow = admission.getByRole('button', { name: 'Not now' });
  await notNow.scrollIntoViewIfNeeded();
  await expect(notNow).toBeInViewport();
  await notNow.click();
  await expect(admission).toBeHidden();

  // M10 Normal-motion interrupted close/reopen on overlay Inspector (220ms enter, no stuck inert)
  await utilityToggle.click();
  const overlayPane = page.locator('[data-pane="inspector"][data-presentation-mode="overlay"]');
  await expect(overlayPane).toBeVisible();
  const overlayMotion = await overlayPane.evaluate((el) => {
    const s = getComputedStyle(el);
    return { prop: s.transitionProperty, dur: s.transitionDuration };
  });
  expect(overlayMotion.prop).toContain('opacity');
  expect(overlayMotion.prop).toContain('translate');
  expect(overlayMotion.dur).toContain('0.22s');
  // Trigger exit via Close utility, confirm exit actually started (data-open=false, inert, 160ms exit),
  // then interrupt mid-exit by reopening before transitionend unmounts the pane.
  await overlayPane.getByRole('button', { name: 'Close utility' }).click();
  await expect(overlayPane).toHaveAttribute('data-open', 'false');
  await expect(overlayPane).toHaveAttribute('inert', '');
  const exitDuration = await overlayPane.evaluate((el) => getComputedStyle(el).transitionDuration);
  expect(exitDuration).toContain('0.16s');
  await page.evaluate(() => {
    document.querySelector<HTMLButtonElement>('button[aria-label="Open utility panel"]')?.click();
  });
  await expect(overlayPane).toBeVisible();
  await expect(overlayPane).toHaveAttribute('data-open', 'true');
  await expect(overlayPane).not.toHaveAttribute('inert');
  await page.keyboard.press('Escape');
  await expect(overlayPane).toBeHidden();
  await expect(utilityToggle).toBeFocused();

  // Reduced motion removes transitions across Button and overlay Inspector
  await page.emulateMedia({ reducedMotion: 'reduce' });
  expect(await utilityToggle.evaluate((el) => getComputedStyle(el).transitionProperty)).toBe(
    'none',
  );
  await utilityToggle.click();
  await expect(overlayPane).toBeVisible();
  expect(await overlayPane.evaluate((el) => getComputedStyle(el).transitionProperty)).toBe('none');
  await page.keyboard.press('Escape');
  await expect(overlayPane).toBeHidden();
  await expect(utilityToggle).toBeFocused();
});
