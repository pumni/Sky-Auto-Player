import { mkdirSync } from 'node:fs';
import { writeFile } from 'node:fs/promises';
import { arch, platform, release, version as osVersion } from 'node:os';
import { relative, resolve } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';

const preferenceKey = 'sky.ui.workbench.v5';
const evidenceEnabled = process.env.GRAPHITE_PACKAGE_C_EVIDENCE === '1';
const captureSha = process.env.PACKAGE_C_CAPTURE_SHA ?? '';
const evidenceDirectory = resolve(process.cwd(), '..', 'docs/evidence/issue-563-package-c');

const previewTokens = {
  accentInk: '#aba6df',
  primaryFill: '#6b5c9e',
  primaryFillHover: '#7566a9',
  primaryFillPressed: '#5f528e',
  focusRing: '#aba6df',
} as const;

const roleNames = {
  accentInk: '--accent-ink',
  accentInkHover: '--accent-ink-hover',
  primaryFill: '--primary-fill',
  primaryFillHover: '--primary-fill-hover',
  primaryFillPressed: '--primary-fill-pressed',
  onPrimary: '--on-primary',
  focusRing: '--focus-ring',
  searchFocusRing: '--search-focus-ring',
  selectionSurface: '--selection-bg',
  selectionControl: '--selection-control',
  selectionControlHover: '--selection-control-hover',
  selectionControlPressed: '--selection-control-pressed',
  neutralHover: '--control-hover',
  neutralPressed: '--control-pressed',
  toggleActiveInk: '--toggle-active-ink',
  toggleInactiveInk: '--toggle-inactive-ink',
  canvas: '--canvas',
  panel: '--surface-panel',
  secondarySurface: '--surface-secondary',
  raised: '--surface-raised',
  searchSurface: '--workbench-search-surface',
  textPrimary: '--text-primary',
  textSecondary: '--text-secondary',
  textTertiary: '--text-tertiary',
  controlBoundary: '--control-border',
  successText: '--success-fg',
  successSurface: '--success-bg',
  warningText: '--warning-fg',
  warningSurface: '--warning-bg',
  dangerText: '--danger-fg',
  dangerSurface: '--danger-bg',
  dangerFill: '--danger-fill',
  dangerFillHover: '--danger-fill-hover',
  dangerFillPressed: '--danger-fill-pressed',
  onDanger: '--on-danger',
  workbenchAccentInk: '--workbench-accent-ink',
  workbenchToggleOn: '--workbench-toggle-on',
  workbenchSearchFocus: '--workbench-search-focus',
  workbenchFocus: '--workbench-focus',
  workbenchAccentFill: '--workbench-accent-fill',
} as const;

type Variant = 'A-current' | 'B-preview';
type SceneName =
  | 'populated-1366x768'
  | 'compact-800x560'
  | 'inspector-overlay-640x448-browser-fallback'
  | 'settings-1366x768'
  | 'playback-confirmation-1366x768'
  | 'playback-error-800x560'
  | 'diagnostics-1366x768'
  | 'populated-1920x1080'
  | 'diagnostics-1920x1080'
  | 'forced-colors-1366x768'
  | 'reduced-motion-1366x768';

interface Scene {
  name: SceneName;
  viewport: { width: number; height: number };
  query?: Record<string, string>;
  prepare: (page: Page) => Promise<void>;
}

const variants: Variant[] = ['A-current', 'B-preview'];

function parseColor(color: string): [number, number, number] {
  const hex = color.trim().match(/^#([0-9a-f]{6})$/i);
  if (hex?.[1]) {
    return [
      Number.parseInt(hex[1].slice(0, 2), 16),
      Number.parseInt(hex[1].slice(2, 4), 16),
      Number.parseInt(hex[1].slice(4, 6), 16),
    ];
  }
  const rgb = color.match(/rgba?\(([^)]+)\)/i);
  if (!rgb?.[1]) throw new Error(`Unsupported color value: ${color}`);
  const channels = rgb[1]
    .split(',')
    .slice(0, 3)
    .map((channel) => Number.parseFloat(channel.trim()));
  if (channels.length !== 3 || channels.some((channel) => Number.isNaN(channel))) {
    throw new Error(`Unsupported color value: ${color}`);
  }
  return channels as [number, number, number];
}

function toRgb(color: string) {
  return `rgb(${parseColor(color).join(', ')})`;
}

function luminance(color: string) {
  const [red, green, blue] = parseColor(color).map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return red * 0.2126 + green * 0.7152 + blue * 0.0722;
}

function contrastRatio(first: string, second: string) {
  const [lighter, darker] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return Number(((lighter + 0.05) / (darker + 0.05)).toFixed(2));
}

async function installPreferenceSeed(page: Page) {
  await page.addInitScript((key) => {
    const encodedPreference = new URLSearchParams(location.search).get('packageCWorkbench');
    localStorage.clear();
    if (encodedPreference) localStorage.setItem(key, encodedPreference);
  }, preferenceKey);
}

async function openApp(
  page: Page,
  viewport: { width: number; height: number },
  query: Record<string, string> = {},
) {
  await page.emulateMedia({ forcedColors: 'none', reducedMotion: 'no-preference' });
  await page.setViewportSize(viewport);
  const preferences = {
    version: 5,
    navigatorPreference: viewport.width <= 800 ? 'collapsed' : 'expanded',
    expandedNavigatorWidth: 240,
    utilityWidth: 320,
    utilityOpen: false,
  };
  const parameters = new URLSearchParams({
    mockPlaybackDurationMs: '120000',
    mockStartDelayMs: '10',
    packageCWorkbench: JSON.stringify(preferences),
    ...query,
  });
  await page.goto(`/?${parameters.toString()}`);
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
}

async function applyVariant(page: Page, variant: Variant) {
  if (variant === 'A-current') return;
  await page.addStyleTag({
    content: `
      html[data-palette='graphite'] {
        --accent-ink: ${previewTokens.accentInk};
        --primary-fill: ${previewTokens.primaryFill};
        --primary-fill-hover: ${previewTokens.primaryFillHover};
        --primary-fill-pressed: ${previewTokens.primaryFillPressed};
        --focus-ring: ${previewTokens.focusRing};
      }
    `,
  });
}

async function readTokens(page: Page) {
  return page.evaluate((names) => {
    const style = getComputedStyle(document.documentElement);
    return Object.fromEntries(
      Object.entries(names).map(([role, name]) => [role, style.getPropertyValue(name).trim()]),
    ) as Record<keyof typeof roleNames, string>;
  }, roleNames);
}

async function renderedColor(
  locator: Locator,
  property: 'color' | 'backgroundColor' | 'outlineColor',
) {
  return locator.evaluate(
    (element, styleProperty) => getComputedStyle(element)[styleProperty],
    property,
  );
}

async function expectContrast(
  foreground: string,
  background: string,
  minimum: number,
  role: string,
) {
  const ratio = contrastRatio(foreground, background);
  expect(
    ratio,
    `${role} contrast ${ratio.toFixed(2)}:1 must be at least ${minimum}:1`,
  ).toBeGreaterThanOrEqual(minimum);
  return ratio;
}

function contrastEvidence(tokens: Record<keyof typeof roleNames, string>) {
  return {
    accentInkOnCanvas: contrastRatio(tokens.accentInk, tokens.canvas),
    accentInkOnSelected: contrastRatio(tokens.accentInk, tokens.selectionSurface),
    primaryTextOnRest: contrastRatio(tokens.onPrimary, tokens.primaryFill),
    primaryTextOnHover: contrastRatio(tokens.onPrimary, tokens.primaryFillHover),
    primaryTextOnPressed: contrastRatio(tokens.onPrimary, tokens.primaryFillPressed),
    focusOnPanel: contrastRatio(tokens.focusRing, tokens.panel),
    focusOnRaised: contrastRatio(tokens.focusRing, tokens.raised),
    searchFocusOnSurface: contrastRatio(tokens.searchFocusRing, tokens.searchSurface),
    neutralBoundaryOnCanvas: contrastRatio(tokens.controlBoundary, tokens.canvas),
    dangerTextOnSurface: contrastRatio(tokens.dangerText, tokens.dangerSurface),
    warningTextOnSurface: contrastRatio(tokens.warningText, tokens.warningSurface),
    successTextOnSurface: contrastRatio(tokens.successText, tokens.successSurface),
    dangerButtonTextOnFill: contrastRatio(tokens.onDanger, tokens.dangerFill),
    dangerButtonTextOnHover: contrastRatio(tokens.onDanger, tokens.dangerFillHover),
    dangerButtonTextOnPressed: contrastRatio(tokens.onDanger, tokens.dangerFillPressed),
  };
}

async function startDryRunWithSelection(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const playingRow = page.getByRole('row', { name: /Candle Run/ });
  await playingRow.click();
  await player.getByRole('button', { name: 'Configure playback profile' }).click();
  await page
    .getByRole('dialog', { name: 'Playback profile' })
    .getByRole('button', { name: 'Test playback (no input)' })
    .click();
  await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
  await expect(player.locator('.player-track-status')).toHaveText('Dry run · Playing');
  const selectedRow = page.getByRole('row', { name: /Blue Bird/ });
  await selectedRow.click();
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await expect(playingRow).toHaveAttribute('aria-current', 'true');
  return player;
}

async function openInspector(page: Page, tab: 'Details' | 'Runtime') {
  const trigger = page.locator('.player-utility-button');
  if ((await trigger.getAttribute('aria-expanded')) !== 'true') await trigger.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
  await inspector.getByRole('tab', { name: tab }).click();
  return inspector;
}

async function openSettingsAppearance(page: Page) {
  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await dialog.getByRole('button', { name: 'Appearance', exact: true }).click();
  await expect(dialog.getByLabel('Color palette')).toHaveValue('graphite');
  await expect(dialog.getByLabel('Color palette').locator('option[value="graphite"]')).toHaveText(
    'Graphite (Default)',
  );
  return dialog;
}

async function preparePopulated(page: Page, tab: 'Details' | 'Runtime' = 'Details') {
  await startDryRunWithSelection(page);
  const inspector = await openInspector(page, tab);
  await expect(page.locator('.track-row.is-selected')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('.track-row.is-playing:not(.is-selected)')).toHaveAttribute(
    'aria-current',
    'true',
  );
  if (tab === 'Details') {
    await page.getByRole('row', { name: /Aurora Landing/ }).hover({ position: { x: 24, y: 22 } });
  } else {
    await expect(inspector.getByText('Mode: Dry run · no physical input sent')).toBeVisible();
    await expect(inspector.getByText('Sender samples', { exact: true })).toBeVisible();
  }
}

async function preparePlaybackConfirmation(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Playback confirmation' })).toBeVisible();
}

async function preparePlaybackError(page: Page) {
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  await expect(page.getByRole('alert')).toContainText('Sky window was not found');
}

async function captureScreenshot(page: Page, variant: Variant, name: string) {
  const path = resolve(evidenceDirectory, variant, name);
  mkdirSync(resolve(path, '..'), { recursive: true });
  await page.screenshot({ path, animations: 'disabled', caret: 'hide' });
  return relative(process.cwd(), path).replaceAll('\\', '/');
}

async function captureLocator(page: Page, variant: Variant, name: string, locator: Locator) {
  const bounds = await locator.boundingBox();
  const viewport = page.viewportSize();
  expect(bounds).not.toBeNull();
  expect(viewport).not.toBeNull();
  if (!bounds || !viewport) throw new Error(`Missing geometry for ${name}`);
  const padding = 8;
  const x = Math.max(0, Math.floor(bounds.x - padding));
  const y = Math.max(0, Math.floor(bounds.y - padding));
  const right = Math.min(viewport.width, Math.ceil(bounds.x + bounds.width + padding));
  const bottom = Math.min(viewport.height, Math.ceil(bounds.y + bounds.height + padding));
  const path = resolve(evidenceDirectory, variant, 'roles', name);
  mkdirSync(resolve(path, '..'), { recursive: true });
  await page.screenshot({
    path,
    clip: { x, y, width: right - x, height: bottom - y },
    animations: 'disabled',
    caret: 'hide',
  });
  return relative(process.cwd(), path).replaceAll('\\', '/');
}

async function readSceneMeasurement(page: Page, variant: Variant, scene: SceneName) {
  const tokens = await readTokens(page);
  const rendered = await page.evaluate(() => {
    const box = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    };
    const style = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const computed = getComputedStyle(element);
      return {
        color: computed.color,
        backgroundColor: computed.backgroundColor,
        fontSize: computed.fontSize,
        outlineColor: computed.outlineColor,
        outlineWidth: computed.outlineWidth,
      };
    };
    const selected = document.querySelector<HTMLElement>('.track-row.is-selected');
    const playing = document.querySelector<HTMLElement>('.track-row.is-playing');
    const paletteOption = document.querySelector<HTMLOptionElement>('option[value="graphite"]');
    return {
      viewport: { width: innerWidth, height: innerHeight, devicePixelRatio },
      media: {
        forcedColors: matchMedia('(forced-colors: active)').matches,
        reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
      },
      palette: document.documentElement.dataset.palette ?? null,
      colorMode: document.documentElement.dataset.colorMode ?? null,
      utilityPresentation:
        document.querySelector<HTMLElement>('.workbench')?.dataset.utilityPresentation ?? null,
      layoutFits: document.querySelector<HTMLElement>('.workbench')?.dataset.layoutFits ?? null,
      geometry: {
        workbench: box('.workbench'),
        navigator: box('.navigator-workbench-pane'),
        tracks: box('.track-browser-workbench-pane'),
        inspector: box('.utility-workbench-pane'),
        playerDock: box('.player-bar'),
        firstRow: box('.track-row'),
        selectedRow: box('.track-row.is-selected'),
        playingRow: box('.track-row.is-playing'),
        trackMinimum: getComputedStyle(
          document.querySelector<HTMLElement>('.track-browser-workbench-pane')!,
        ).minWidth,
        documentWidth: document.documentElement.scrollWidth,
      },
      state: {
        selectedSong: selected?.querySelector('.track-title-text')?.textContent?.trim() ?? null,
        selectedAria: selected?.getAttribute('aria-selected') ?? null,
        playingSong: playing?.querySelector('.track-title-text')?.textContent?.trim() ?? null,
        playingAria: playing?.getAttribute('aria-current') ?? null,
        dryRunStatus: document.querySelector('.player-track-status')?.textContent?.trim() ?? null,
        alert: document.querySelector('[role="alert"]')?.textContent?.trim() ?? null,
        settingsPaletteLabel: paletteOption?.textContent?.trim() ?? null,
      },
      typography: {
        playerStatus: style('.player-track-status')?.fontSize ?? null,
        playerAction: style('.player-primary-action')?.fontSize ?? null,
        diagnosticsStatus: style('.diagnostics-status')?.fontSize ?? null,
        diagnosticsValue: style('.diagnostics-metric strong')?.fontSize ?? null,
      },
      rendered: {
        primaryAction: style('.player-primary-action'),
        selectedRow: selected ? style('.track-row.is-selected') : null,
        playingRow: playing ? style('.track-row.is-playing') : null,
        settingsActive: style('.settings-nav-item.is-active'),
        error: style('.player-admission-error'),
        diagnostics: style('.diagnostics-playback-summary'),
      },
    };
  });
  return {
    variant,
    scene,
    tokens,
    contrast: contrastEvidence(tokens),
    rendered,
  };
}

async function captureRoleStateEvidence(page: Page, variant: Variant) {
  await openApp(page, { width: 1366, height: 768 });
  await applyVariant(page, variant);
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const primary = page.locator('.player-primary-action');
  await expect(primary).toBeEnabled();
  const controls: Record<string, unknown> = {};

  await primary.hover();
  controls.primaryHover = await primary.evaluate((element) => {
    const style = getComputedStyle(element);
    return { foreground: style.color, fill: style.backgroundColor };
  });
  controls.primaryHoverScreenshot = await captureLocator(
    page,
    variant,
    'primary-hover-1366x768.png',
    primary,
  );

  const bounds = await primary.boundingBox();
  expect(bounds).not.toBeNull();
  if (!bounds) throw new Error('Primary control bounds are missing');
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  controls.primaryPressed = await primary.evaluate((element) => {
    const style = getComputedStyle(element);
    return { foreground: style.color, fill: style.backgroundColor };
  });
  controls.primaryPressedScreenshot = await captureLocator(
    page,
    variant,
    'primary-pressed-1366x768.png',
    primary,
  );
  await page.mouse.move(0, 0);
  await page.mouse.up();

  await page.keyboard.press('Tab');
  await primary.focus();
  await expect
    .poll(() => primary.evaluate((element) => element.matches(':focus-visible')))
    .toBe(true);
  controls.primaryFocus = await primary.evaluate((element) => {
    const style = getComputedStyle(element);
    return { outlineColor: style.outlineColor, outlineWidth: style.outlineWidth };
  });
  controls.primaryFocusScreenshot = await captureLocator(
    page,
    variant,
    'primary-focus-1366x768.png',
    primary,
  );

  const close = page.getByRole('button', { name: 'Close window' });
  await close.hover();
  controls.dangerHover = await close.evaluate((element) => {
    const style = getComputedStyle(element);
    return { foreground: style.color, fill: style.backgroundColor };
  });
  controls.dangerHoverScreenshot = await captureLocator(
    page,
    variant,
    'danger-hover-1366x768.png',
    close,
  );
  return controls;
}

test('Graphite A and Soft Graphite preview consume semantic roles with contrast and input-state evidence', async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  await installPreferenceSeed(page);
  const reports: Array<Record<string, unknown>> = [];

  for (const variant of variants) {
    await openApp(page, { width: 1366, height: 768 });
    await applyVariant(page, variant);
    const tokens = await readTokens(page);
    const expected =
      variant === 'A-current'
        ? {
            accentInk: '#b2a5e9',
            primaryFill: '#705eae',
            primaryFillHover: '#7564b4',
            primaryFillPressed: '#6553a0',
            focusRing: '#b2a5e9',
          }
        : previewTokens;
    expect(tokens).toMatchObject({
      ...expected,
      accentInkHover: '#c0b4f1',
      searchFocusRing: '#9688c6',
      selectionSurface: '#25242c',
      toggleActiveInk: '#b68cff',
      workbenchAccentInk: expected.accentInk,
      workbenchToggleOn: '#b68cff',
      workbenchSearchFocus: '#9688c6',
      workbenchFocus: expected.focusRing,
      workbenchAccentFill: expected.primaryFill,
    });
    await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
    await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');

    const surfaces = [tokens.canvas, tokens.panel, tokens.secondarySurface, tokens.raised];
    const textRatios = {
      primary: surfaces.map((surface) => contrastRatio(tokens.textPrimary, surface)),
      secondary: surfaces.map((surface) => contrastRatio(tokens.textSecondary, surface)),
      tertiary: surfaces.map((surface) => contrastRatio(tokens.textTertiary, surface)),
      accent: surfaces.map((surface) => contrastRatio(tokens.accentInk, surface)),
      accentHover: surfaces.map((surface) => contrastRatio(tokens.accentInkHover, surface)),
    };
    for (const [role, ratios] of Object.entries(textRatios)) {
      expect(
        ratios.every((ratio) => ratio >= 4.5),
        `${variant} ${role} text contrast`,
      ).toBe(true);
    }
    const contrast = contrastEvidence(tokens);
    expect(contrast.primaryTextOnRest).toBeGreaterThanOrEqual(4.5);
    expect(contrast.primaryTextOnHover).toBeGreaterThanOrEqual(4.5);
    expect(contrast.primaryTextOnPressed).toBeGreaterThanOrEqual(4.5);
    expect(contrast.accentInkOnSelected).toBeGreaterThanOrEqual(3);
    expect(contrast.focusOnPanel).toBeGreaterThanOrEqual(3);
    expect(contrast.focusOnRaised).toBeGreaterThanOrEqual(3);
    expect(contrast.searchFocusOnSurface).toBeGreaterThanOrEqual(3);
    expect(contrast.neutralBoundaryOnCanvas).toBeGreaterThanOrEqual(3);
    expect(contrast.dangerTextOnSurface).toBeGreaterThanOrEqual(4.5);
    expect(contrast.warningTextOnSurface).toBeGreaterThanOrEqual(4.5);
    expect(contrast.successTextOnSurface).toBeGreaterThanOrEqual(4.5);
    expect(contrast.dangerButtonTextOnFill).toBeGreaterThanOrEqual(4.5);
    expect(contrast.dangerButtonTextOnHover).toBeGreaterThanOrEqual(4.5);
    expect(contrast.dangerButtonTextOnPressed).toBeGreaterThanOrEqual(4.5);

    const browser = page.locator('.track-browser-workbench-pane');
    const playerDock = page.locator('.player-bar');
    await expect
      .poll(() => browser.evaluate((element) => getComputedStyle(element).minWidth))
      .toBe('420px');
    await expect
      .poll(() => playerDock.evaluate((element) => element.getBoundingClientRect().height))
      .toBe(92);
    await expect
      .poll(() =>
        page
          .locator('.track-row')
          .first()
          .evaluate((element) => element.getBoundingClientRect().height),
      )
      .toBe(46);

    const selectedRow = page.getByRole('row', { name: /Blue Bird/ });
    await selectedRow.click();
    const primary = page.locator('.player-primary-action');
    await expect(primary).toBeEnabled();
    await expect
      .poll(() => renderedColor(primary, 'backgroundColor'))
      .toBe(toRgb(tokens.primaryFill));
    await expect.poll(() => renderedColor(primary, 'color')).toBe(toRgb(tokens.onPrimary));
    await primary.hover();
    await expect
      .poll(() => renderedColor(primary, 'backgroundColor'))
      .toBe(toRgb(tokens.primaryFillHover));
    const primaryBounds = await primary.boundingBox();
    expect(primaryBounds).not.toBeNull();
    if (!primaryBounds) throw new Error('Primary action bounds are unavailable');
    await page.mouse.move(
      primaryBounds.x + primaryBounds.width / 2,
      primaryBounds.y + primaryBounds.height / 2,
    );
    await page.mouse.down();
    await expect
      .poll(() => renderedColor(primary, 'backgroundColor'))
      .toBe(toRgb(tokens.primaryFillPressed));
    await page.mouse.move(0, 0);
    await page.mouse.up();
    await page.keyboard.press('Tab');
    await primary.focus();
    await expect
      .poll(() => primary.evaluate((element) => element.matches(':focus-visible')))
      .toBe(true);
    await expect.poll(() => renderedColor(primary, 'outlineColor')).toBe(toRgb(tokens.focusRing));
    await expect
      .poll(() => primary.evaluate((element) => getComputedStyle(element).outlineWidth))
      .toBe('2px');

    const search = page.getByRole('searchbox', { name: 'Search Built-in Songs' });
    await search.focus();
    await expect
      .poll(() => search.evaluate((element) => element.matches(':focus-visible')))
      .toBe(true);
    await expect
      .poll(() =>
        search.evaluate((element) => getComputedStyle(element.parentElement!).outlineColor),
      )
      .toBe(toRgb(tokens.searchFocusRing));

    const settings = await openSettingsAppearance(page);
    const activeCategory = settings.getByRole('button', { name: 'Appearance', exact: true });
    await expect.poll(() => renderedColor(activeCategory, 'color')).toBe(toRgb(tokens.accentInk));
    await activeCategory.hover();
    await expect
      .poll(() => renderedColor(activeCategory, 'backgroundColor'))
      .toBe(toRgb(tokens.selectionControlHover));
    await page.mouse.move(0, 0);
    await expect
      .poll(() => renderedColor(activeCategory, 'backgroundColor'))
      .toBe(toRgb(tokens.selectionSurface));
    await settings.getByRole('button', { name: 'Close settings' }).click();

    const close = page.getByRole('button', { name: 'Close window' });
    await close.hover();
    await expect.poll(() => renderedColor(close, 'backgroundColor')).toBe(toRgb(tokens.dangerFill));
    await expectContrast(
      await renderedColor(close, 'color'),
      await renderedColor(close, 'backgroundColor'),
      4.5,
      'close danger action',
    );

    const player = await startDryRunWithSelection(page);
    const playingRow = page.locator('.track-row.is-playing:not(.is-selected)');
    await expect(playingRow).toHaveAttribute('aria-current', 'true');
    await expect(page.locator('.track-row.is-selected')).toHaveAttribute('aria-selected', 'true');
    const selectedFill = await renderedColor(
      page.locator('.track-row.is-selected'),
      'backgroundColor',
    );
    const playingFill = await renderedColor(playingRow, 'backgroundColor');
    expect(selectedFill).not.toBe(playingFill);
    await expect(player.locator('.player-track-status')).toHaveText('Dry run · Playing');

    await page.keyboard.press('Tab');
    await primary.focus();
    await expect
      .poll(() => primary.evaluate((element) => element.matches(':focus-visible')))
      .toBe(true);
    await page.emulateMedia({ forcedColors: 'active' });
    const systemHighlight = await page.evaluate(() => {
      const probe = document.createElement('span');
      probe.style.outlineColor = 'Highlight';
      document.body.append(probe);
      const value = getComputedStyle(probe).outlineColor;
      probe.remove();
      return value;
    });
    await expect.poll(() => renderedColor(primary, 'outlineColor')).toBe(systemHighlight);
    await expect
      .poll(() => renderedColor(page.locator('.track-row.is-selected'), 'backgroundColor'))
      .toBe(
        await page.evaluate(() => {
          const probe = document.createElement('span');
          probe.style.backgroundColor = 'Highlight';
          document.body.append(probe);
          const value = getComputedStyle(probe).backgroundColor;
          probe.remove();
          return value;
        }),
      );
    await page.emulateMedia({ forcedColors: 'none', reducedMotion: 'reduce' });
    const reducedMotion = await primary.evaluate((element) => {
      const style = getComputedStyle(element);
      return { transition: style.transitionDuration, animation: style.animationDuration };
    });
    const durationMs = (value: string) =>
      Math.max(
        ...value.split(',').map((part) => {
          const duration = Number.parseFloat(part);
          return part.trim().endsWith('ms') ? duration : duration * 1000;
        }),
      );
    expect(durationMs(reducedMotion.transition)).toBeLessThanOrEqual(0.01);
    expect(durationMs(reducedMotion.animation)).toBeLessThanOrEqual(0.01);

    reports.push({
      variant,
      tokens,
      contrast,
      textRatios,
      rowHeight: 46,
      playerDockHeight: 92,
      trackBrowserMinWidth: 420,
      selectedRowBackground: selectedFill,
      playingRowBackground: playingFill,
      reducedMotion,
    });
  }
  await testInfo.attach('graphite-package-c-rendered-role-audit.json', {
    body: Buffer.from(`${JSON.stringify(reports, null, 2)}\n`),
    contentType: 'application/json',
  });
});

test('captures matched Graphite A and Soft Graphite preview states', async ({ page, browser }) => {
  test.skip(!evidenceEnabled, 'Set GRAPHITE_PACKAGE_C_EVIDENCE=1 to write matched screenshots.');
  test.setTimeout(360_000);
  expect(captureSha, 'the screenshot evidence must record its immutable source SHA').toMatch(
    /^[a-f0-9]{40}$/,
  );
  await installPreferenceSeed(page);
  const scenes: Scene[] = [
    {
      name: 'populated-1366x768',
      viewport: { width: 1366, height: 768 },
      prepare: (scenePage) => preparePopulated(scenePage, 'Details'),
    },
    {
      name: 'compact-800x560',
      viewport: { width: 800, height: 560 },
      prepare: (scenePage) => preparePopulated(scenePage, 'Details'),
    },
    {
      name: 'inspector-overlay-640x448-browser-fallback',
      viewport: { width: 640, height: 448 },
      prepare: async (scenePage) => {
        await preparePopulated(scenePage, 'Details');
        await expect(scenePage.locator('.workbench')).toHaveAttribute(
          'data-utility-presentation',
          'overlay',
        );
      },
    },
    {
      name: 'settings-1366x768',
      viewport: { width: 1366, height: 768 },
      prepare: async (scenePage) => {
        await startDryRunWithSelection(scenePage);
        await openSettingsAppearance(scenePage);
      },
    },
    {
      name: 'playback-confirmation-1366x768',
      viewport: { width: 1366, height: 768 },
      prepare: preparePlaybackConfirmation,
    },
    {
      name: 'playback-error-800x560',
      viewport: { width: 800, height: 560 },
      query: { mockStartFailure: 'target_not_found' },
      prepare: preparePlaybackError,
    },
    {
      name: 'diagnostics-1366x768',
      viewport: { width: 1366, height: 768 },
      prepare: (scenePage) => preparePopulated(scenePage, 'Runtime'),
    },
    {
      name: 'populated-1920x1080',
      viewport: { width: 1920, height: 1080 },
      prepare: (scenePage) => preparePopulated(scenePage, 'Details'),
    },
    {
      name: 'diagnostics-1920x1080',
      viewport: { width: 1920, height: 1080 },
      prepare: (scenePage) => preparePopulated(scenePage, 'Runtime'),
    },
    {
      name: 'forced-colors-1366x768',
      viewport: { width: 1366, height: 768 },
      prepare: async (scenePage) => {
        await preparePopulated(scenePage, 'Details');
        await scenePage.emulateMedia({ forcedColors: 'active' });
        await scenePage.keyboard.press('Tab');
        await scenePage.locator('.library-nav-item.is-active').first().focus();
      },
    },
    {
      name: 'reduced-motion-1366x768',
      viewport: { width: 1366, height: 768 },
      prepare: async (scenePage) => {
        await preparePopulated(scenePage, 'Details');
        await scenePage.emulateMedia({ reducedMotion: 'reduce' });
      },
    },
  ];
  const results: Record<string, unknown> = {
    captureSourceSha: captureSha,
    environment: {
      osPlatform: platform(),
      osRelease: release(),
      osVersion: osVersion(),
      architecture: arch(),
      browser: 'Chromium',
      browserVersion: browser.version(),
      deviceScaleFactor: 1,
      bridge: 'deterministic Playwright mock bridge; no Tauri/native runtime',
      viewportUnits: 'CSS pixels; screenshots are browser viewport captures',
    },
    screenshots: [] as string[],
    scenes: {} as Record<string, unknown>,
    roleStates: {} as Record<string, unknown>,
  };

  for (const variant of variants) {
    for (const scene of scenes) {
      await openApp(page, scene.viewport, scene.query);
      await applyVariant(page, variant);
      await scene.prepare(page);
      const media = await page.evaluate(() => ({
        forcedColors: matchMedia('(forced-colors: active)').matches,
        reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
      }));
      expect(media.forcedColors, `${variant}/${scene.name} forced-colors mode`).toBe(
        scene.name === 'forced-colors-1366x768',
      );
      expect(media.reducedMotion, `${variant}/${scene.name} reduced-motion mode`).toBe(
        scene.name === 'reduced-motion-1366x768',
      );
      const screenshot = await captureScreenshot(page, variant, `${scene.name}.png`);
      (results.screenshots as string[]).push(screenshot);
      results.scenes[`${variant}/${scene.name}`] = {
        screenshot,
        ...(await readSceneMeasurement(page, variant, scene.name)),
      };
    }
    results.roleStates[variant] = await captureRoleStateEvidence(page, variant);
  }

  mkdirSync(evidenceDirectory, { recursive: true });
  await writeFile(
    resolve(evidenceDirectory, 'render-measurements.json'),
    `${JSON.stringify(results, null, 2)}\n`,
  );
});
