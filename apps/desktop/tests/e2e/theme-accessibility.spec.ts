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
  '--primary',
  '--primary-hover',
  '--primary-pressed',
  '--primary-foreground',
  '--border',
  '--overlay-border',
  '--secondary',
  '--secondary-foreground',
  '--control-hover',
  '--row-hover',
  '--control-pressed',
  '--control-active',
  '--scrollbar-thumb',
  '--scrollbar-thumb-active',
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
  '--destructive-hover',
  '--destructive-pressed',
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
  await profileDialog.getByRole('combobox', { name: 'Base Hold' }).click();
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
