import AxeBuilder from '@axe-core/playwright';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { arch, platform, release, version as osVersion } from 'node:os';
import { relative, resolve } from 'node:path';
import { expect, test, type Browser, type Page } from '@playwright/test';
import { collectRenderedTextContrast, type RenderedTextContrast } from './renderedContrast';
import { expectNoSeriousAccessibilityViolations } from './testHelpers';

const evidenceEnabled = process.env.PACKAGE_B_VISUAL_EVIDENCE === '1';
const captureSha = process.env.PACKAGE_B_CAPTURE_SHA ?? '';
const evidenceDirectory = resolve(process.cwd(), '..', 'docs/evidence/issue-565-package-b');
const viewports = [
  { width: 1366, height: 768 },
  { width: 1200, height: 760 },
  { width: 1024, height: 768 },
  { width: 800, height: 560 },
  { width: 640, height: 448 },
] as const;

const lightTokens = {
  '--canvas': '#ffffff',
  '--surface-panel': '#f7f8fa',
  '--surface-secondary': '#fafbfc',
  '--surface-raised': '#ffffff',
  '--control-hover': '#eef2f6',
  '--control-pressed': '#e2e8f0',
  '--selection-bg': '#eaf2fd',
  '--border-subtle': '#dfe4eb',
  '--border-strong': '#b1bbc6',
  '--control-border': '#767f8a',
  '--scrollbar-thumb': '#808b97',
  '--text-primary': '#252a31',
  '--text-secondary': '#535f6c',
  '--text-tertiary': '#5d6976',
  '--accent-ink': '#0b58a5',
  '--accent-fill': '#0b63c5',
  '--accent-fill-hover': '#0958ae',
  '--accent-fill-pressed': '#084c98',
  '--success-fg': '#1f7a47',
  '--success-bg': '#e9f5ed',
  '--success-border': '#1f7a47',
  '--warning-fg': '#8d5a08',
  '--warning-bg': '#fff5df',
  '--warning-border': '#8d5a08',
  '--danger-fg': '#b4232f',
  '--danger-bg': '#fcebed',
  '--danger-border': '#b4232f',
  '--workbench-content': '#ffffff',
  '--workbench-navigation': '#f7f8fa',
  '--workbench-inspector': '#fafbfc',
  '--chrome-surface': '#f7f8fa',
  '--workbench-control-disabled': '#626e7a',
  '--workbench-playing-indicator': '#767f8a',
  '--workbench-search-surface': '#ffffff',
  '--workbench-search-idle-edge': '#767f8a',
  '--workbench-search-focus': '#0b58a5',
  '--caption-text': '#535f6c',
  '--overlay-backdrop': 'rgb(37 42 49 / 38%)',
} as const;

const darkRegressionTokens = [
  '--canvas',
  '--surface-panel',
  '--surface-secondary',
  '--surface-raised',
  '--control-hover',
  '--control-pressed',
  '--selection-bg',
  '--selection-control',
  '--selection-row-indicator',
  '--border-subtle',
  '--border-strong',
  '--control-border',
  '--text-primary',
  '--text-secondary',
  '--text-tertiary',
  '--accent',
  '--accent-ink',
  '--accent-fill',
  '--focus-ring',
  '--workbench-content',
  '--workbench-navigation',
  '--workbench-inspector',
  '--chrome-surface',
  '--workbench-hover',
  '--workbench-pressed',
  '--workbench-control-rest',
  '--workbench-control-rest-edge',
  '--workbench-control-hover',
  '--workbench-control-pressed',
  '--workbench-control-disabled',
  '--workbench-action-edge',
  '--workbench-action-hover-edge',
  '--workbench-action-hover',
  '--workbench-control-danger-hover',
  '--workbench-selected',
  '--workbench-divider',
  '--workbench-playing-indicator',
  '--workbench-pane-edge',
  '--workbench-overlay-edge',
  '--workbench-critical-edge',
  '--workbench-control-boundary',
  '--workbench-text-primary',
  '--workbench-text-secondary',
  '--workbench-text-tertiary',
  '--workbench-accent',
  '--workbench-accent-hover',
  '--workbench-focus',
  '--workbench-search-surface',
  '--workbench-search-idle-edge',
  '--workbench-search-focus',
  '--primary-fill',
  '--primary-fill-hover',
  '--primary-fill-pressed',
  '--caption-text',
  '--caption-hover',
  '--caption-pressed',
  '--caption-close-hover',
  '--caption-close-pressed',
  '--scrollbar-thumb',
  '--scrollbar-thumb-hover',
] as const;

type Mode = 'light' | 'graphite-dark';
type ThemeScheme = 'light' | 'dark';

function parseColor(value: string): [number, number, number] {
  const channels = value.match(/[\d.]+/g)?.map(Number) ?? [];
  if (channels.length < 3) throw new Error(`Unsupported computed color: ${value}`);
  return [channels[0], channels[1], channels[2]];
}

function contrastRatio(first: string, second: string): number {
  const luminance = (value: string) =>
    parseColor(value)
      .map((channel) => {
        const normalized = channel / 255;
        return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
      })
      .reduce((sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index], 0);
  const left = luminance(first);
  const right = luminance(second);
  return Number(((Math.max(left, right) + 0.05) / (Math.min(left, right) + 0.05)).toFixed(2));
}

async function textAudit(page: Page, scene: string) {
  const samples = await page.evaluate(collectRenderedTextContrast);
  const failures = samples.filter((sample) => !sample.exemption && sample.ratio < sample.minimum);
  expect(failures, `${scene} rendered text contrast failures`).toEqual([]);
  return summarizeTextContrast(samples);
}

function summarizeTextContrast(samples: RenderedTextContrast[]) {
  const applicable = samples.filter((sample) => !sample.exemption);
  return {
    sampleCount: samples.length,
    inactiveControlExemptions: samples.filter((sample) => sample.exemption).length,
    minimumApplicableRatio: Math.min(...applicable.map((sample) => sample.ratio)),
    failures: applicable.filter((sample) => sample.ratio < sample.minimum),
    lowestSamples: applicable
      .slice()
      .sort((left, right) => left.ratio - right.ratio)
      .slice(0, 8),
  };
}

async function openLightApp(page: Page, query = '') {
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce', forcedColors: 'none' });
  await page.goto(query ? `/?${query}` : '/');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await page.evaluate(() => document.fonts.ready);
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
  return { player, playingRow, selectedRow };
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
  await expect(dialog.getByLabel('Theme')).toHaveValue('system');
  await expect(dialog.getByLabel('Dark palette')).toHaveValue('graphite');
  return dialog;
}

async function createEvidencePage(
  browser: Browser,
  scheme: ThemeScheme,
  viewport: { width: number; height: number },
  query = '',
) {
  const context = await browser.newContext({
    viewport,
    deviceScaleFactor: 1,
    colorScheme: scheme,
    reducedMotion: 'reduce',
  });
  const page = await context.newPage();
  const baseUrl = process.env.PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:4173';
  await page.goto(`${baseUrl}/${query ? `?${query}` : ''}`);
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', scheme);
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await expect(page.locator('.track-row').first()).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  return { context, page };
}

async function captureScene(
  page: Page,
  mode: Mode,
  name: string,
  contrastReport: Record<string, unknown>,
) {
  const samples = await page.evaluate(collectRenderedTextContrast);
  const summary = summarizeTextContrast(samples);
  expect(summary.failures, `${mode}/${name} text contrast failures`).toEqual([]);

  const path = resolve(evidenceDirectory, mode, `${name}.png`);
  await mkdir(resolve(path, '..'), { recursive: true });
  await page.screenshot({ path, animations: 'disabled', caret: 'hide' });
  contrastReport[`${mode}/${name}`] = summary;
  return relative(resolve(process.cwd(), '..'), path).replaceAll('\\', '/');
}

test('Light mode covers every Graphite alias while Dark tokens round-trip unchanged', async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');

  const readTokens = (names: readonly string[]) =>
    page.evaluate(
      (tokenNames) => {
        const style = getComputedStyle(document.documentElement);
        return Object.fromEntries(
          tokenNames.map((name) => [name, style.getPropertyValue(name).trim()]),
        );
      },
      [...names],
    );

  const darkBefore = await readTokens(darkRegressionTokens);
  const dialog = await openSettingsAppearance(page);
  await dialog.getByLabel('Theme').selectOption('light');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');

  const themeCss = await readFile(resolve(process.cwd(), 'src/styles/themes.css'), 'utf8');
  const rootBlock = themeCss.match(/:root\s*\{([\s\S]*?)\r?\n\}/)?.[1];
  const lightBlock = themeCss.match(/html\[data-color-mode='light'\]\s*\{([\s\S]*?)\r?\n\}/)?.[1];
  expect(rootBlock).toBeDefined();
  expect(lightBlock).toBeDefined();
  const declarations = (block: string) =>
    Array.from(block.matchAll(/--([\w-]+)\s*:/g), (match) => `--${match[1]}`);
  const rootNames = declarations(rootBlock ?? '');
  const lightNames = declarations(lightBlock ?? '');
  const missingLightRoles = rootNames.filter((name) => !lightNames.includes(name));
  expect(rootNames.length).toBeGreaterThan(100);
  expect(lightNames.length).toBe(rootNames.length);
  expect(missingLightRoles).toEqual([]);

  const tokens = await readTokens(Object.keys(lightTokens));
  expect(tokens).toEqual(lightTokens);
  const renderedSurfaces = await page.evaluate(() => {
    const color = (selector: string, property: 'color' | 'backgroundColor' | 'borderLeftColor') => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) throw new Error(`Missing rendered surface: ${selector}`);
      return getComputedStyle(element)[property];
    };
    return {
      canvas: color('body', 'backgroundColor'),
      titlebar: color('.app-titlebar', 'backgroundColor'),
      navigator: color('.navigator-workbench-pane', 'backgroundColor'),
      trackBrowser: color('.track-table', 'backgroundColor'),
      search: color('.global-search', 'backgroundColor'),
    };
  });
  expect(renderedSurfaces).toEqual({
    canvas: 'rgb(255, 255, 255)',
    titlebar: 'rgb(247, 248, 250)',
    navigator: 'rgb(247, 248, 250)',
    trackBrowser: 'rgb(255, 255, 255)',
    search: 'rgb(255, 255, 255)',
  });

  await dialog.getByLabel('Theme').selectOption('dark');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  await expect.poll(() => readTokens(darkRegressionTokens)).toEqual(darkBefore);
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
});

test('Light selected and playing rows remain distinct with accessible contrast', async ({
  page,
}) => {
  await openLightApp(page);
  const { playingRow, selectedRow } = await startDryRunWithSelection(page);
  expect(await selectedRow.getAttribute('aria-current')).not.toBe('true');
  expect(await playingRow.getAttribute('aria-selected')).not.toBe('true');

  const rowStyles = await page.evaluate(() => {
    const selected = document.querySelector<HTMLElement>('.track-row.is-selected');
    const playing = document.querySelector<HTMLElement>('.track-row.is-playing:not(.is-selected)');
    if (!selected || !playing) throw new Error('Selected and playing rows must both render');
    const selectedStyle = getComputedStyle(selected);
    const playingStyle = getComputedStyle(playing);
    return {
      selectedBackground: selectedStyle.backgroundColor,
      selectedIndicator: selectedStyle.borderLeftColor,
      playingBackground: playingStyle.backgroundColor,
      playingIndicator: playingStyle.borderLeftColor,
    };
  });
  expect(rowStyles.selectedBackground).toBe('rgb(234, 242, 253)');
  expect(rowStyles.playingBackground).toBe('rgb(247, 248, 250)');
  expect(rowStyles.selectedBackground).not.toBe(rowStyles.playingBackground);
  expect(rowStyles.selectedIndicator).not.toBe(rowStyles.playingIndicator);
  expect(
    contrastRatio(rowStyles.selectedIndicator, rowStyles.selectedBackground),
  ).toBeGreaterThanOrEqual(3);
  expect(
    contrastRatio(rowStyles.playingIndicator, rowStyles.playingBackground),
  ).toBeGreaterThanOrEqual(3);

  const summary = await textAudit(page, 'Light populated library');
  expect(summary.minimumApplicableRatio).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);
});

test('Graphite Dark keeps selected and playing rows distinct without changing its divider role', async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce', forcedColors: 'none' });
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'dark');
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await startDryRunWithSelection(page);

  const rowStyles = await page.evaluate(() => {
    const selected = document.querySelector<HTMLElement>('.track-row.is-selected');
    const playing = document.querySelector<HTMLElement>('.track-row.is-playing:not(.is-selected)');
    if (!selected || !playing) throw new Error('Selected and playing rows must both render');
    const selectedStyle = getComputedStyle(selected);
    const playingStyle = getComputedStyle(playing);
    const token = (name: string) =>
      getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return {
      selectedBackground: selectedStyle.backgroundColor,
      selectedIndicator: selectedStyle.borderLeftColor,
      playingBackground: playingStyle.backgroundColor,
      playingIndicator: playingStyle.borderLeftColor,
      workbenchDivider: token('--workbench-divider'),
      playingIndicatorToken: token('--workbench-playing-indicator'),
    };
  });
  expect(rowStyles.selectedBackground).toBe('rgb(37, 36, 44)');
  expect(rowStyles.playingBackground).toBe('rgb(25, 26, 27)');
  expect(rowStyles.selectedBackground).not.toBe(rowStyles.playingBackground);
  expect(rowStyles.workbenchDivider).toBe('#2a2b2c');
  expect(rowStyles.playingIndicatorToken).toBe(rowStyles.workbenchDivider);
  expect(rowStyles.playingIndicator).toBe('rgb(42, 43, 44)');
  expect(rowStyles.selectedIndicator).not.toBe(rowStyles.playingIndicator);
  const summary = await textAudit(page, 'Graphite Dark populated library');
  expect(summary.minimumApplicableRatio).toBeGreaterThanOrEqual(4.5);
  await expectNoSeriousAccessibilityViolations(page);
});

test('Light rendered text contrast holds in Settings, Diagnostics, confirmation, and errors', async ({
  page,
}) => {
  await openLightApp(page, 'mockDiagnosticsStatus=degraded');
  const { selectedRow } = await startDryRunWithSelection(page);
  await textAudit(page, 'Light populated library');
  await expectNoSeriousAccessibilityViolations(page);

  const settings = await openSettingsAppearance(page);
  await textAudit(page, 'Light Settings Appearance');
  await expectNoSeriousAccessibilityViolations(page);
  await settings.getByRole('button', { name: 'Close settings' }).click();

  const runtime = await openInspector(page, 'Runtime');
  await expect(runtime.getByText('Mode: Dry run · no physical input sent')).toBeVisible();
  await textAudit(page, 'Light Diagnostics Runtime');
  await expectNoSeriousAccessibilityViolations(page);

  await openLightApp(page, 'mockStartFailure=target_not_found');
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await player.getByRole('button', { name: 'Play', exact: true }).click();
  const proceed = player.getByRole('button', { name: 'Proceed with current settings' });
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
  await expect(page.getByRole('alert')).toContainText('Sky window was not found');
  await textAudit(page, 'Light playback error');
  await expectNoSeriousAccessibilityViolations(page);

  await openLightApp(page);
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  await page
    .getByRole('contentinfo', { name: 'Player controls' })
    .getByRole('button', { name: 'Play', exact: true })
    .click();
  await expect(page.getByRole('group', { name: 'Playback confirmation' })).toBeVisible();
  await textAudit(page, 'Light playback confirmation');
  await expectNoSeriousAccessibilityViolations(page);
});

test('Light contrast holds in pagination loading, empty library, Calibration, and Update states', async ({
  page,
}) => {
  await openLightApp(page, 'mockCatalogCount=106&mockSearchDelayMs=400');
  const nextPage = page.getByRole('button', { name: 'Next page' });
  await expect(nextPage).toBeEnabled();
  await nextPage.click();
  const pagerStatus = page.locator('.builtin-pager-status');
  await expect(pagerStatus).toContainText('Loading page 2 of 2');
  await textAudit(page, 'Light pagination loading');
  await expect(page.getByRole('grid', { name: 'Built-in Songs, page 2' })).toBeVisible();
  await textAudit(page, 'Light pagination page 2');

  await openLightApp(page, 'mockLibraryState=empty');
  await expect(page.locator('.empty-state strong')).toHaveText('No Built-in Songs are available');
  await textAudit(page, 'Light empty library');
  await expectNoSeriousAccessibilityViolations(page);

  await openLightApp(page);
  await page.getByRole('button', { name: 'Open settings' }).click();
  await page.getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.getByRole('button', { name: 'Open calibration' }).click();
  const calibration = page.getByRole('dialog', { name: 'Timing calibration' });
  await expect(calibration).toBeVisible();
  await textAudit(page, 'Light Calibration');
  await calibration.getByRole('button', { name: 'Start quick calibration' }).click();
  await expect(calibration).toContainText('Calibration complete');
  await textAudit(page, 'Light Calibration complete');
  await expectNoSeriousAccessibilityViolations(page);

  await openLightApp(page);
  await page.getByRole('button', { name: /Open update/ }).click();
  const update = page.getByRole('dialog', { name: 'Software update' });
  await expect(update).toContainText('Version 4.0.0-alpha.2-mock is available');
  await textAudit(page, 'Light Update available');
  await expectNoSeriousAccessibilityViolations(page);
});

test('Light keyboard focus, hover, pressed, disabled, forced colors, and reduced motion are covered', async ({
  page,
}) => {
  await openLightApp(page);
  await startDryRunWithSelection(page);

  const searchInput = page.locator('.global-search input');
  await searchInput.focus();
  await expect
    .poll(() => searchInput.evaluate((element) => element.matches(':focus-visible')))
    .toBe(true);
  const searchFocus = await page.locator('.global-search').evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.outlineColor, width: style.outlineWidth, style: style.outlineStyle };
  });
  expect(searchFocus).toMatchObject({ color: 'rgb(11, 88, 165)', width: '2px', style: 'solid' });
  expect(contrastRatio(searchFocus.color, 'rgb(255, 255, 255)')).toBeGreaterThanOrEqual(3);

  const minimize = page.getByRole('button', { name: 'Minimize window' });
  await minimize.hover();
  expect(await minimize.evaluate((element) => element.matches(':hover'))).toBe(true);
  expect(await minimize.evaluate((element) => getComputedStyle(element).backgroundColor)).toBe(
    'rgb(238, 242, 246)',
  );
  const captionClose = page.getByRole('button', { name: 'Close window' });
  await captionClose.hover();
  const captionCloseBounds = await captionClose.boundingBox();
  expect(captionCloseBounds).not.toBeNull();
  if (!captionCloseBounds) throw new Error('Window close control must have rendered bounds');
  await page.mouse.move(
    captionCloseBounds.x + captionCloseBounds.width / 2,
    captionCloseBounds.y + captionCloseBounds.height / 2,
  );
  await expect.poll(() => captionClose.evaluate((element) => element.matches(':hover'))).toBe(true);
  const closeCaptionStyle = () =>
    captionClose.evaluate((element) => {
      const style = getComputedStyle(element);
      return { color: style.color, background: style.backgroundColor };
    });
  await expect.poll(closeCaptionStyle).toEqual({
    color: 'rgb(255, 255, 255)',
    background: 'rgb(180, 35, 47)',
  });
  const closeCaptionColors = await closeCaptionStyle();
  expect(
    contrastRatio(closeCaptionColors.color, closeCaptionColors.background),
  ).toBeGreaterThanOrEqual(4.5);

  const likeButton = page.locator('.track-row.is-selected .track-like-button');
  await likeButton.hover();
  const likeHover = await likeButton.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      isHovered: element.matches(':hover'),
      rowIsHovered: element.closest('.track-row')?.matches(':hover'),
      color: style.color,
      background: style.backgroundColor,
      rowBackground: getComputedStyle(element.closest('.track-row')!).backgroundColor,
      renderedBackground:
        style.backgroundColor === 'rgba(0, 0, 0, 0)'
          ? getComputedStyle(element.closest('.track-row')!).backgroundColor
          : style.backgroundColor,
    };
  });
  expect(likeHover.isHovered).toBe(true);
  expect(likeHover.rowIsHovered).toBe(true);
  expect(likeHover.rowBackground).toBe('rgb(234, 242, 253)');
  expect(['rgba(0, 0, 0, 0)', 'rgb(238, 242, 246)']).toContain(likeHover.background);
  expect(contrastRatio(likeHover.color, likeHover.renderedBackground)).toBeGreaterThanOrEqual(3);

  await page.getByRole('button', { name: 'Open settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  const close = dialog.getByRole('button', { name: 'Close settings' });
  await close.hover();
  const closeHover = await close.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      isHovered: element.matches(':hover'),
      color: style.color,
      background: style.backgroundColor,
      dialogBackground: getComputedStyle(element.closest('.settings-dialog')!).backgroundColor,
    };
  });
  expect(closeHover.isHovered).toBe(true);
  expect(['rgba(0, 0, 0, 0)', 'rgb(238, 242, 246)']).toContain(closeHover.background);
  expect(
    contrastRatio(
      closeHover.color,
      closeHover.background === 'rgba(0, 0, 0, 0)'
        ? closeHover.dialogBackground
        : closeHover.background,
    ),
  ).toBeGreaterThanOrEqual(3);
  const closeBounds = await close.boundingBox();
  expect(closeBounds).not.toBeNull();
  if (!closeBounds) throw new Error('Settings close button must have rendered bounds');
  await page.mouse.move(
    closeBounds.x + closeBounds.width / 2,
    closeBounds.y + closeBounds.height / 2,
  );
  await page.mouse.down();
  expect(await close.evaluate((element) => element.matches(':active'))).toBe(true);
  await expect
    .poll(() => close.evaluate((element) => getComputedStyle(element).backgroundColor))
    .toBe('rgb(226, 232, 240)');
  await page.mouse.up();
  await expect(dialog).toBeHidden();

  await page.goto('/');
  const disabledPrimary = page.locator('.player-primary-action:disabled');
  await expect(disabledPrimary).toBeVisible();
  const disabledState = await disabledPrimary.evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.color, background: style.backgroundColor, opacity: style.opacity };
  });
  expect(disabledState.background).toBe('rgb(255, 255, 255)');
  expect(disabledState.opacity).toBe('1');

  await page.emulateMedia({ forcedColors: 'active' });
  const activeNavigation = page.locator('.library-nav-item.is-active').first();
  const expectedHighlight = await page.evaluate(() => {
    const probe = document.createElement('span');
    probe.style.backgroundColor = 'Highlight';
    document.body.append(probe);
    const resolved = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return resolved;
  });
  await expect
    .poll(() => activeNavigation.evaluate((element) => getComputedStyle(element).backgroundColor))
    .toBe(expectedHighlight);

  await page.emulateMedia({ forcedColors: 'none', reducedMotion: 'reduce' });
  const durations = await page
    .locator('.library-nav-item')
    .first()
    .evaluate((element) => {
      const style = getComputedStyle(element);
      return { transition: style.transitionDuration, animation: style.animationDuration };
    });
  const maxMs = (value: string) =>
    Math.max(
      ...value.split(',').map((part) => {
        const duration = Number.parseFloat(part);
        return part.trim().endsWith('ms') ? duration : duration * 1000;
      }),
    );
  expect(maxMs(durations.transition)).toBeLessThanOrEqual(0.01);
  expect(maxMs(durations.animation)).toBeLessThanOrEqual(0.01);
});

test('Light preserves approved geometry across native and compact browser sizes', async ({
  page,
}) => {
  await openLightApp(page);
  for (const viewport of viewports) {
    await page.setViewportSize(viewport);
    const geometry = await page.evaluate(() => {
      const rect = (selector: string) => {
        const element = document.querySelector<HTMLElement>(selector);
        if (!element) return null;
        const bounds = element.getBoundingClientRect();
        return { width: bounds.width, height: bounds.height };
      };
      return {
        titlebar: rect('.app-titlebar'),
        track: rect('.track-browser-workbench-pane'),
        row: rect('.track-row'),
        dock: rect('.player-bar'),
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight,
        documentWidth: document.documentElement.scrollWidth,
        documentHeight: document.documentElement.scrollHeight,
        utilityPresentation:
          document.querySelector<HTMLElement>('.workbench')?.dataset.utilityPresentation,
      };
    });
    expect(geometry.titlebar?.height).toBe(40);
    expect(geometry.track?.width).toBeGreaterThanOrEqual(420);
    expect(geometry.row?.height).toBe(46);
    expect(geometry.dock?.height).toBe(92);
    expect(geometry.documentWidth).toBeLessThanOrEqual(viewport.width);
    expect(geometry.documentHeight).toBeLessThanOrEqual(viewport.height);
    expect(geometry.utilityPresentation).toBe('closed');
  }
  await page.setViewportSize({ width: 640, height: 448 });
  await openInspector(page, 'Runtime');
  await expect(page.locator('.utility-workbench-pane')).toHaveClass(/is-overlay/);
  await expectNoSeriousAccessibilityViolations(page);
});

test('captures Light and Graphite Dark Package B review scenes and computed contrast evidence', async ({
  browser,
}) => {
  test.skip(!evidenceEnabled, 'Set PACKAGE_B_VISUAL_EVIDENCE=1 to write review screenshots.');
  test.setTimeout(360_000);
  expect(captureSha, 'screenshot evidence must record the immutable source SHA').toMatch(
    /^[a-f0-9]{40}$/,
  );

  const modes: { mode: Mode; scheme: ThemeScheme }[] = [
    { mode: 'light', scheme: 'light' },
    { mode: 'graphite-dark', scheme: 'dark' },
  ];
  const screenshots: string[] = [];
  const contrastReport: Record<string, unknown> = {};

  for (const { mode, scheme } of modes) {
    for (const viewport of viewports) {
      const { context, page } = await createEvidencePage(browser, scheme, viewport);
      await startDryRunWithSelection(page);
      screenshots.push(
        await captureScene(
          page,
          mode,
          `populated-${viewport.width}x${viewport.height}`,
          contrastReport,
        ),
      );
      await context.close();
    }

    const { context: settingsContext, page: settingsPage } = await createEvidencePage(
      browser,
      scheme,
      { width: 1366, height: 768 },
    );
    await startDryRunWithSelection(settingsPage);
    await openSettingsAppearance(settingsPage);
    screenshots.push(
      await captureScene(settingsPage, mode, 'settings-appearance-1366x768', contrastReport),
    );
    await settingsContext.close();

    const { context: diagnosticsContext, page: diagnosticsPage } = await createEvidencePage(
      browser,
      scheme,
      { width: 1366, height: 768 },
      'mockDiagnosticsStatus=degraded',
    );
    await startDryRunWithSelection(diagnosticsPage);
    await openInspector(diagnosticsPage, 'Runtime');
    await expect(diagnosticsPage.getByText('Mode: Dry run · no physical input sent')).toBeVisible();
    screenshots.push(
      await captureScene(diagnosticsPage, mode, 'diagnostics-runtime-1366x768', contrastReport),
    );
    await diagnosticsContext.close();

    const { context: errorContext, page: errorPage } = await createEvidencePage(
      browser,
      scheme,
      { width: 1366, height: 768 },
      'mockStartFailure=target_not_found',
    );
    await errorPage.getByRole('row', { name: /Blue Bird/ }).click();
    const errorPlayer = errorPage.getByRole('contentinfo', { name: 'Player controls' });
    await errorPlayer.getByRole('button', { name: 'Play', exact: true }).click();
    const proceed = errorPlayer.getByRole('button', { name: 'Proceed with current settings' });
    if (await proceed.isVisible().catch(() => false)) await proceed.click();
    await expect(errorPage.getByRole('alert')).toContainText('Sky window was not found');
    screenshots.push(
      await captureScene(errorPage, mode, 'playback-error-1366x768', contrastReport),
    );
    await errorContext.close();

    const { context: confirmationContext, page: confirmationPage } = await createEvidencePage(
      browser,
      scheme,
      { width: 1366, height: 768 },
    );
    await confirmationPage.getByRole('row', { name: /Aurora Landing/ }).click();
    await confirmationPage
      .getByRole('contentinfo', { name: 'Player controls' })
      .getByRole('button', { name: 'Play', exact: true })
      .click();
    await expect(
      confirmationPage.getByRole('group', { name: 'Playback confirmation' }),
    ).toBeVisible();
    screenshots.push(
      await captureScene(confirmationPage, mode, 'playback-confirmation-1366x768', contrastReport),
    );
    await confirmationContext.close();

    const { context: focusContext, page: focusPage } = await createEvidencePage(browser, scheme, {
      width: 1366,
      height: 768,
    });
    await startDryRunWithSelection(focusPage);
    await focusPage.locator('.global-search input').focus();
    await focusPage.locator('.track-row.is-selected .track-like-button').hover();
    screenshots.push(await captureScene(focusPage, mode, 'focus-hover-1366x768', contrastReport));
    await focusContext.close();

    const { context: compactContext, page: compactPage } = await createEvidencePage(
      browser,
      scheme,
      { width: 640, height: 448 },
    );
    await startDryRunWithSelection(compactPage);
    await openInspector(compactPage, 'Runtime');
    await expect(compactPage.locator('.utility-workbench-pane')).toHaveClass(/is-overlay/);
    screenshots.push(
      await captureScene(compactPage, mode, 'compact-utility-overlay-640x448', contrastReport),
    );
    await compactContext.close();
  }

  const manifest = {
    captureSourceSha: captureSha,
    capturedAt: new Date().toISOString(),
    environment: {
      osPlatform: platform(),
      osRelease: release(),
      osVersion: osVersion(),
      architecture: arch(),
      browser: 'Playwright Chromium',
      browserVersion: browser.version(),
      deviceScaleFactor: 1,
      browserThemeEmulation: 'prefers-color-scheme light/dark; not packaged Windows WebView2',
      bridge: 'deterministic browser mock bridge; no Tauri runtime or game input',
      viewportsAreCssPixels: true,
    },
    screenshots,
  };
  await mkdir(evidenceDirectory, { recursive: true });
  await writeFile(
    resolve(evidenceDirectory, 'capture-manifest.json'),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  await writeFile(
    resolve(evidenceDirectory, 'computed-contrast.json'),
    `${JSON.stringify({ captureSourceSha: captureSha, scenes: contrastReport }, null, 2)}\n`,
  );
});
