import AxeBuilder from '@axe-core/playwright';
import { expect, type Locator, type Page } from '@playwright/test';

export const palettes = [
  'graphite',
  'aurora',
  'minimalist',
  'slate',
  'cyberpunk',
  'classic',
] as const;
export type PaletteId = (typeof palettes)[number];

// Independent E2E oracle for the approved Inspector breakpoint. Keep this
// separate from the production layout helper so browser tests can detect drift.
export function expectedUtilityPresentation(viewportWidth: number): 'docked' | 'overlay' {
  return viewportWidth >= 764 ? 'docked' : 'overlay';
}

export async function expectNoSeriousAccessibilityViolations(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.filter(
      (violation) => violation.impact === 'critical' || violation.impact === 'serious',
    ),
  ).toEqual([]);
}

export async function expectWorkbenchWithinBounds(
  page: Page,
  minimumTrackWidth = 420,
  minimumUtilityWidth = 280,
) {
  const geometry = await page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, right: rect.right, bottom: rect.bottom, width: rect.width };
    };
    return {
      viewportWidth: window.innerWidth,
      presentation: document.querySelector<HTMLElement>('.workbench')?.dataset.utilityPresentation,
      gutter: Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue('--workbench-gap'),
      ),
      solverGutter: Number.parseFloat(
        document.querySelector<HTMLElement>('.workbench')?.dataset.workbenchGap ?? 'NaN',
      ),
      workbench: box('.workbench'),
      navigator: box('.navigator-workbench-pane'),
      track: box('.track-browser-workbench-pane'),
      utility: box('.utility-workbench-pane'),
      player: box('.player-bar'),
      rootMinWidth: getComputedStyle(document.documentElement).minWidth,
      bodyMinWidth: getComputedStyle(document.body).minWidth,
    };
  });

  expect(geometry.workbench).not.toBeNull();
  expect(geometry.navigator).not.toBeNull();
  expect(geometry.track).not.toBeNull();
  if (geometry.workbench && geometry.navigator && geometry.track) {
    expect(geometry.solverGutter).toBe(geometry.gutter);
    expect(geometry.navigator.x).toBeGreaterThanOrEqual(geometry.workbench.x);
    expect(geometry.navigator.x + geometry.navigator.width).toBeLessThanOrEqual(geometry.track.x);
    expect(geometry.track.x + geometry.track.width).toBeLessThanOrEqual(
      geometry.workbench.x + geometry.workbench.width - geometry.gutter,
    );
    expect(geometry.track.width).toBeGreaterThanOrEqual(minimumTrackWidth);
  }
  if (geometry.utility && geometry.workbench) {
    expect(geometry.utility.x).toBeGreaterThanOrEqual(geometry.workbench.x);
    expect(geometry.utility.x + geometry.utility.width).toBeLessThanOrEqual(
      geometry.workbench.x + geometry.workbench.width - geometry.gutter,
    );
    if (geometry.presentation === 'docked') {
      if (geometry.track) {
        expect(geometry.track.x + geometry.track.width).toBeLessThanOrEqual(geometry.utility.x);
      }
      expect(geometry.utility.width).toBeGreaterThanOrEqual(minimumUtilityWidth);
      await expect(page.getByRole('separator', { name: 'Resize utility pane' })).toBeVisible();
    } else {
      expect(geometry.presentation).toBe('overlay');
      expect(geometry.utility.width).toBeGreaterThanOrEqual(280);
      expect(geometry.utility.width).toBeLessThanOrEqual(400);
      expect(geometry.player).not.toBeNull();
      if (geometry.player) {
        expect(geometry.utility.bottom).toBeLessThanOrEqual(geometry.player.y - geometry.gutter);
      }
      await expect(page.locator('.utility-workbench-pane')).toHaveClass(/is-overlay/);
      await expect(page.getByRole('separator', { name: 'Resize utility pane' })).toHaveCount(0);
    }
    expect(Math.round(geometry.viewportWidth - geometry.utility.right)).toBeGreaterThanOrEqual(
      geometry.gutter,
    );
  } else {
    expect(geometry.presentation).toBe('closed');
  }
  expect(geometry.rootMinWidth).toBe('0px');
  expect(geometry.bodyMinWidth).toBe('0px');
  await expect(page.locator('.workbench')).toHaveAttribute('data-layout-fits', 'true');
}

export async function readPlayerBarGeometry(page: Page) {
  return page.evaluate(() => {
    const player = document.querySelector<HTMLElement>('.player-bar');
    const primary = player?.querySelector<HTMLElement>('[data-testid="player-primary-slot"]');
    const controls = player?.querySelector<HTMLElement>('[data-testid="player-controls-row"]');
    const timeline = player?.querySelector<HTMLElement>('.player-timeline');
    const rail = player?.querySelector<HTMLElement>('.player-timeline progress');
    if (!player || !primary || !controls || !timeline || !rail) return null;

    const rect = (element: Element) => {
      const box = element.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height };
    };
    const controlSelectors: Record<string, string> = {
      Shuffle: '.player-shuffle-action',
      Previous: '.player-previous-action',
      Primary: '.player-primary-action',
      Next: '.player-next-action',
      Stop: '.player-stop-action',
    };
    const visibleControlCenters: Record<string, number> = {};
    const visibleControls: Record<
      string,
      { x: number; y: number; width: number; height: number; disabled: boolean; label: string }
    > = {};
    for (const [name, selector] of Object.entries(controlSelectors)) {
      const button = player.querySelector<HTMLButtonElement>(selector);
      if (button && button.getClientRects().length > 0) {
        const box = button.getBoundingClientRect();
        visibleControlCenters[name] = box.y + box.height / 2;
        visibleControls[name] = {
          x: box.x,
          y: box.y,
          width: box.width,
          height: box.height,
          disabled: button.disabled,
          label: button.getAttribute('aria-label') ?? '',
        };
      }
    }
    const primaryBox = rect(primary);
    return {
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      viewportCenterX: window.innerWidth / 2,
      player: rect(player),
      controls: rect(controls),
      primary: primaryBox,
      primaryCenterX: primaryBox.x + primaryBox.width / 2,
      primaryCenterY: primaryBox.y + primaryBox.height / 2,
      timeline: rect(timeline),
      rail: rect(rail),
      visibleControlCenters,
      visibleControls,
      documentWidth: document.documentElement.scrollWidth,
      documentHeight: document.documentElement.scrollHeight,
    };
  });
}

export async function expectFiveTransportControls(player: Locator, primaryName: string) {
  for (const name of ['Shuffle', 'Previous', primaryName, 'Next', 'Stop']) {
    await expect(player.getByRole('button', { name, exact: true })).toBeVisible();
  }
  await expect(player.locator('.player-controls-row button')).toHaveCount(5);
}

export async function readRightDockGeometry(page: Page) {
  return page.evaluate(() => {
    const selectors = {
      autoPlay: 'button[aria-label="Auto Play"]',
      profile: 'button[aria-label="Configure playback profile"]',
      utility: 'button[aria-label="Open utility panel"], button[aria-label="Close utility panel"]',
    };
    const result: Record<string, { x: number; right: number; centerY: number }> = {};
    for (const [name, selector] of Object.entries(selectors)) {
      const button = document.querySelector<HTMLElement>(`.player-tools ${selector}`);
      if (!button) return null;
      const box = button.getBoundingClientRect();
      result[name] = { x: box.x, right: box.right, centerY: box.y + box.height / 2 };
    }
    return result;
  });
}

export function expectPlayerBarGeometry(
  geometry: Awaited<ReturnType<typeof readPlayerBarGeometry>>,
) {
  expect(geometry).not.toBeNull();
  if (!geometry) throw new Error('Player Bar geometry is unavailable');
  expect(Math.abs(geometry.primaryCenterX - geometry.viewportCenterX)).toBeLessThanOrEqual(2);
  expect(geometry.player.height).toBe(92);
  expect(
    geometry.timeline.y - (geometry.primary.y + geometry.primary.height),
  ).toBeGreaterThanOrEqual(6);
  expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewportWidth);
  expect(geometry.documentHeight).toBeLessThanOrEqual(geometry.viewportHeight);
  expect(Object.keys(geometry.visibleControls)).toHaveLength(5);
  for (const centerY of Object.values(geometry.visibleControlCenters)) {
    expect(Math.abs(centerY - geometry.primaryCenterY)).toBeLessThanOrEqual(1);
  }
}

export function expectStablePlayerBar(
  before: Awaited<ReturnType<typeof readPlayerBarGeometry>>,
  after: Awaited<ReturnType<typeof readPlayerBarGeometry>>,
) {
  expect(before).not.toBeNull();
  expect(after).not.toBeNull();
  if (!before || !after) throw new Error('Player Bar geometry is unavailable');
  expect(Math.abs(after.primaryCenterX - before.primaryCenterX)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.primaryCenterY - before.primaryCenterY)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.timeline.y - before.timeline.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.rail.y - before.rail.y)).toBeLessThanOrEqual(1);
  expect(after.player.height).toBe(before.player.height);
}

export async function expectRecoveryBannerWithinViewport(page: Page) {
  const banner = page.getByRole('alert');
  await expect(banner).toBeVisible({ timeout: 20_000 });
  const box = await banner.boundingBox();
  const viewport = page.viewportSize();
  expect(box).not.toBeNull();
  expect(viewport).not.toBeNull();
  if (box && viewport) {
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(viewport.width);
    expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
  }
  const documentWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(documentWidth).toBeLessThanOrEqual(viewport?.width ?? 0);
}

export async function startSelectedSong(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  return player;
}

export async function selectPalette(page: Page, palette: PaletteId) {
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  await dialog.getByLabel('Dark palette').selectOption(palette);
  await expect(page.locator('html')).toHaveAttribute('data-palette', palette);
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  await dialog.getByRole('button', { name: 'Close settings' }).click();
  await expect(dialog).toBeHidden();
}
