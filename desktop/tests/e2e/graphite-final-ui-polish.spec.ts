import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';

const captureStage = process.env.GRAPHITE_FINAL_UI_CAPTURE;
const evidenceDirectory =
  process.env.GRAPHITE_FINAL_UI_EVIDENCE_DIR ??
  resolve(process.cwd(), '..', 'docs/evidence/graphite-final-ui-polish/after');
const workbenchPreferenceKey = 'sky.ui.workbench.v5';

async function openApp(
  page: Page,
  viewport: { width: number; height: number },
  utilityOpen = false,
  updateAvailable = true,
) {
  await page.setViewportSize(viewport);
  const updateQuery = updateAvailable ? '' : '&mockUpdateState=current';
  await page.addInitScript(
    ({ key, openUtility, viewportWidth }) => {
      window.localStorage.clear();
      window.localStorage.setItem(
        key,
        JSON.stringify({
          version: 5,
          navigatorPreference: viewportWidth <= 800 ? 'collapsed' : 'expanded',
          expandedNavigatorWidth: 240,
          utilityWidth: 320,
          utilityOpen: openUtility,
        }),
      );
    },
    { key: workbenchPreferenceKey, openUtility: utilityOpen, viewportWidth: viewport.width },
  );
  await page.goto(`/?mockPlaybackDurationMs=120000&mockStartDelayMs=10${updateQuery}`);
  await expect(page.locator('.app-shell')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'graphite');
  await expect(page.getByText('500 songs', { exact: true })).toBeVisible();
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

async function token(page: Page, name: string) {
  return page.evaluate(
    (tokenName) => getComputedStyle(document.documentElement).getPropertyValue(tokenName).trim(),
    name,
  );
}

function channels(color: string) {
  if (color.startsWith('#')) {
    const hex = color.slice(1);
    if (hex.length !== 6) throw new Error(`Unsupported color ${color}`);
    return [0, 2, 4].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16)).concat(1);
  }
  const values = color.match(/[\d.]+/g)?.map(Number);
  if (!values || (values.length !== 3 && values.length !== 4)) {
    throw new Error(`Cannot parse computed color ${color}`);
  }
  return values.length === 3 ? [...values, 1] : values;
}

function sameColor(first: string, second: string) {
  return channels(first).every((value, index) => Math.abs(value - channels(second)[index]) < 1);
}

function luminance(color: string) {
  const [red, green, blue] = channels(color)
    .slice(0, 3)
    .map((channel) => {
      const normalized = channel / 255;
      return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
    });
  return red * 0.2126 + green * 0.7152 + blue * 0.0722;
}

function contrastRatio(first: string, second: string) {
  const [lighter, darker] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

async function colors(control: Locator) {
  return control.evaluate((element) => {
    const style = getComputedStyle(element);
    const icon = element.querySelector('svg');
    return {
      foreground: style.color,
      background: style.backgroundColor,
      border: style.borderTopColor,
      borderBottom: style.borderBottomColor,
      outlineColor: style.outlineColor,
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
      opacity: style.opacity,
      iconColor: icon ? getComputedStyle(icon).color : null,
      iconStroke: icon?.getAttribute('stroke') ?? null,
    };
  });
}

async function expectFilledWhite(control: Locator, expectedBackground: string) {
  const style = await colors(control);
  expect(
    sameColor(style.background, expectedBackground),
    `expected ${expectedBackground}; received ${style.background}`,
  ).toBe(true);
  expect(style.foreground).toBe('rgb(255, 255, 255)');
  expect(contrastRatio(style.foreground, style.background)).toBeGreaterThanOrEqual(4.5);
  if (style.iconColor !== null) {
    expect(style.iconColor).toBe(style.foreground);
    expect(style.iconStroke).toBe('currentColor');
    expect(contrastRatio(style.iconColor, style.background)).toBeGreaterThanOrEqual(3);
  }
}

async function expectVisibleHover(control: Locator, hoverToken: string) {
  await control.hover();
  const style = await colors(control);
  expect(sameColor(style.background, hoverToken)).toBe(true);
  expect(sameColor(style.background, 'rgba(0, 0, 0, 0)')).toBe(false);
  return style;
}

async function pressWithoutClick(page: Page, control: Locator) {
  await control.hover();
  await page.mouse.down();
  const pressed = await colors(control);
  await page.mouse.move(0, 0);
  await page.mouse.up();
  return pressed;
}

async function capture(
  page: Page,
  name: string,
  clip?: { x: number; y: number; width: number; height: number },
) {
  mkdirSync(evidenceDirectory, { recursive: true });
  await page.screenshot({
    path: resolve(evidenceDirectory, name),
    animations: 'disabled',
    ...(clip ? { clip } : {}),
  });
}

test('Graphite transport keeps Play, Pause, and Resume in the filled primary role', async ({
  page,
}) => {
  await openApp(page, { width: 1366, height: 768 });

  const disabledPlay = page.locator('.player-primary-action');
  await expect(disabledPlay).toBeDisabled();
  const disabledStyle = await colors(disabledPlay);
  expect(sameColor(disabledStyle.background, await token(page, '--surface-raised'))).toBe(true);
  expect(disabledStyle.foreground).toBe('rgb(139, 140, 144)');
  expect(disabledStyle.opacity).toBe('1');
  await disabledPlay.hover();
  expect(sameColor((await colors(disabledPlay)).background, disabledStyle.background)).toBe(true);

  await page.getByRole('row', { name: /Candle Run/ }).click();
  const enabledPlay = page.locator('.player-primary-action');
  await expect(enabledPlay).toBeEnabled();
  const fill = await token(page, '--workbench-accent-fill');
  const fillHover = await token(page, '--workbench-accent-fill-hover');
  const fillPressed = await token(page, '--workbench-accent-fill-pressed');
  await expectFilledWhite(enabledPlay, fill);
  const hoverStyle = await expectVisibleHover(enabledPlay, fillHover);
  expect(hoverStyle.foreground).toBe('rgb(255, 255, 255)');
  const pressedStyle = await pressWithoutClick(page, enabledPlay);
  expect(sameColor(pressedStyle.background, fillPressed)).toBe(true);
  expect(pressedStyle.foreground).toBe('rgb(255, 255, 255)');
  expect(contrastRatio(pressedStyle.foreground, pressedStyle.background)).toBeGreaterThanOrEqual(
    4.5,
  );

  const player = await startDryRunWithSelection(page);
  const pause = player.getByRole('button', { name: 'Pause' });
  await expectFilledWhite(pause, fill);
  await pause.click();
  const resume = player.getByRole('button', { name: 'Resume' });
  await expect(resume).toBeVisible();
  await page.mouse.move(0, 0);
  await expectFilledWhite(resume, fill);
  await resume.click();
  await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
});

test('Graphite navigation, tabs, toggles, and track selection stay persistent', async ({
  page,
}) => {
  await openApp(page, { width: 1366, height: 768 });

  const selectionFill = await token(page, '--selection-bg');
  const selectedControl = await token(page, '--workbench-selected-control');
  const selectedHover = await token(page, '--workbench-selected-control-hover');
  const activeLibrary = page.locator('.library-nav-item.is-active').first();
  const activeLibraryStyle = await colors(activeLibrary);
  expect(sameColor(activeLibraryStyle.background, selectionFill)).toBe(true);
  await activeLibrary.hover();
  expect(sameColor((await colors(activeLibrary)).background, selectionFill)).toBe(true);

  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  const activeSettings = settings.getByRole('button', { name: 'Playback', exact: true });
  await expect(activeSettings).toHaveAttribute('aria-current', 'page');
  expect(sameColor((await colors(activeSettings)).background, selectionFill)).toBe(true);
  await activeSettings.hover();
  expect(sameColor((await colors(activeSettings)).background, selectedHover)).toBe(true);
  await settings.getByRole('button', { name: 'Appearance', exact: true }).focus();
  await page.keyboard.press('Shift+Tab');
  await expect(activeSettings).toBeFocused();
  expect(await activeSettings.evaluate((element) => element.matches(':focus-visible'))).toBe(true);
  const activeSettingsFocus = await colors(activeSettings);
  expect(activeSettingsFocus.outlineStyle).toBe('solid');
  expect(activeSettingsFocus.outlineWidth).toBe('2px');

  await settings.getByRole('button', { name: 'Close settings' }).click();
  const shuffle = page.getByRole('button', { name: 'Shuffle' });
  await shuffle.click();
  await expect(shuffle).toHaveAttribute('aria-pressed', 'true');
  await page.mouse.move(0, 0);
  expect(sameColor((await colors(shuffle)).background, selectedControl)).toBe(true);
  await shuffle.hover();
  expect(sameColor((await colors(shuffle)).background, selectedHover)).toBe(true);
  await page.mouse.move(0, 0);
  expect(sameColor((await colors(shuffle)).background, selectedControl)).toBe(true);

  const autoPlay = page.getByRole('button', { name: 'Auto Play' });
  await expect(autoPlay).toHaveAttribute('aria-pressed', 'true');
  expect(sameColor((await colors(autoPlay)).background, selectedControl)).toBe(true);

  const utility = page.locator('.player-utility-button');
  await utility.click();
  await expect(utility).toHaveAttribute('aria-expanded', 'true');
  await page.mouse.move(0, 0);
  expect(sameColor((await colors(utility)).background, selectedControl)).toBe(true);
  const detailsTab = page.locator('.utility-view-tabs [role=tab][aria-selected=true]');
  await expect(detailsTab).toHaveAccessibleName('Details');
  expect(sameColor((await colors(detailsTab)).background, selectedControl)).toBe(true);
  await detailsTab.focus();
  await page.keyboard.press('ArrowRight');
  const runtimeTab = page.getByRole('tab', { name: 'Runtime' });
  await expect(runtimeTab).toHaveAttribute('aria-selected', 'true');
  await expect(runtimeTab).toBeFocused();
  expect(await runtimeTab.evaluate((element) => element.matches(':focus-visible'))).toBe(true);
  expect(sameColor((await colors(runtimeTab)).background, selectedControl)).toBe(true);
  expect((await colors(runtimeTab)).outlineWidth).toBe('2px');
  await runtimeTab.hover();
  expect(sameColor((await colors(runtimeTab)).background, selectedHover)).toBe(true);
  expect((await colors(runtimeTab)).outlineWidth).toBe('2px');

  const player = await startDryRunWithSelection(page);
  const selectedRow = page.getByRole('row', { name: /Blue Bird/ });
  const playingRow = page.getByRole('row', { name: /Candle Run/ });
  await expect(selectedRow).toHaveAttribute('aria-selected', 'true');
  await expect(playingRow).toHaveAttribute('aria-current', 'true');
  const selectedRowBackground = await selectedRow.evaluate(
    (element) => getComputedStyle(element).backgroundColor,
  );
  const playingRowBackground = await playingRow.evaluate(
    (element) => getComputedStyle(element).backgroundColor,
  );
  expect(sameColor(selectedRowBackground, playingRowBackground)).toBe(false);
  await expect(player.getByRole('button', { name: 'Pause' })).toBeVisible();
});

test('Graphite secondary controls stay quiet at rest and show clear hover, focus, and danger states', async ({
  page,
}) => {
  await openApp(page, { width: 1366, height: 768 });

  const transparent = 'rgba(0, 0, 0, 0)';
  const hoverToken = await token(page, '--workbench-control-hover');
  const pressedToken = await token(page, '--workbench-control-pressed');
  const utility = page.getByRole('button', { name: 'Open utility panel' });
  expect(sameColor((await colors(utility)).background, transparent)).toBe(true);
  await expectVisibleHover(utility, hoverToken);
  const pressedUtility = await pressWithoutClick(page, utility);
  expect(sameColor(pressedUtility.background, pressedToken)).toBe(true);
  await page.mouse.move(0, 0);
  expect(sameColor((await colors(utility)).background, transparent)).toBe(true);

  const shuffle = page.getByRole('button', { name: 'Shuffle' });
  expect(sameColor((await colors(shuffle)).background, transparent)).toBe(true);
  await expectVisibleHover(shuffle, hoverToken);

  const minimize = page.getByRole('button', { name: 'Minimize window' });
  expect(sameColor((await colors(minimize)).background, transparent)).toBe(true);
  await expectVisibleHover(minimize, hoverToken);
  const close = page.getByRole('button', { name: 'Close window' });
  await close.hover();
  const closeStyle = await colors(close);
  const dangerFill = await token(page, '--danger-fill');
  expect(
    sameColor(closeStyle.background, dangerFill),
    `expected danger close ${dangerFill}; received ${closeStyle.background}`,
  ).toBe(true);
  expect(closeStyle.foreground).toBe('rgb(255, 255, 255)');
  expect(contrastRatio(closeStyle.foreground, closeStyle.background)).toBeGreaterThanOrEqual(4.5);

  await page.keyboard.press('Tab');
  await utility.evaluate((element) => element.focus());
  expect(await utility.evaluate((element) => element.matches(':focus-visible'))).toBe(true);
  const utilityFocus = await colors(utility);
  expect(utilityFocus.outlineStyle).toBe('solid');
  expect(utilityFocus.outlineWidth).toBe('2px');
  expect(
    contrastRatio(utilityFocus.outlineColor, await token(page, '--workbench-inspector')),
  ).toBeGreaterThanOrEqual(3);

  await openApp(page, { width: 1366, height: 768 });
  await page.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  const createButton = createDialog.getByRole('button', { name: 'Create' });
  await createDialog.getByLabel('Playlist name').fill('Graphite Hierarchy Test');
  await expect(createButton).toBeEnabled();
  const fill = await token(page, '--workbench-accent-fill');
  const fillHover = await token(page, '--workbench-accent-fill-hover');
  const fillPressed = await token(page, '--workbench-accent-fill-pressed');
  await expectFilledWhite(createButton, fill);
  await expectVisibleHover(createButton, fillHover);
  const pressedCreate = await pressWithoutClick(page, createButton);
  expect(sameColor(pressedCreate.background, fillPressed)).toBe(true);
  expect(contrastRatio(pressedCreate.foreground, pressedCreate.background)).toBeGreaterThanOrEqual(
    4.5,
  );
  await createButton.click();

  await page.getByRole('button', { name: 'More actions for Graphite Hierarchy Test' }).click();
  const deleteMenuItem = page.getByRole('menuitem', { name: 'Delete playlist' });
  const dangerHover = await token(page, '--workbench-control-danger-hover');
  await deleteMenuItem.hover();
  const deleteMenuStyle = await colors(deleteMenuItem);
  expect(sameColor(deleteMenuStyle.background, dangerHover)).toBe(true);
  expect(
    contrastRatio(deleteMenuStyle.foreground, deleteMenuStyle.background),
  ).toBeGreaterThanOrEqual(4.5);
  await deleteMenuItem.click();

  const deleteDialog = page.getByRole('dialog', { name: /Delete/ });
  const deleteButton = deleteDialog.getByRole('button', { name: 'Delete' });
  const dangerFillHover = await token(page, '--danger-fill-hover');
  await expectFilledWhite(deleteButton, await token(page, '--danger-fill'));
  await expectVisibleHover(deleteButton, dangerFillHover);
  const deleteHoverStyle = await colors(deleteButton);
  expect(
    contrastRatio(deleteHoverStyle.foreground, deleteHoverStyle.background),
  ).toBeGreaterThanOrEqual(4.5);
});

test('Graphite hierarchy leaves the five existing palette accent contracts unchanged', async ({
  page,
}) => {
  await openApp(page, { width: 1366, height: 768 });
  const expected = {
    aurora: ['rgb(120, 93, 221)', 'rgb(130, 91, 219)', 'rgb(255, 255, 255)'],
    minimalist: ['rgb(160, 184, 213)', 'rgb(196, 208, 223)', 'rgb(23, 25, 29)'],
    slate: ['rgb(108, 167, 216)', 'rgb(131, 184, 227)', 'rgb(16, 23, 29)'],
    cyberpunk: ['rgb(217, 90, 191)', 'rgb(239, 120, 212)', 'rgb(19, 15, 29)'],
    classic: ['rgb(201, 146, 66)', 'rgb(224, 170, 91)', 'rgb(23, 21, 18)'],
  };
  const readPalette = () =>
    page.evaluate(() => {
      const probe = document.createElement('span');
      document.body.append(probe);
      const getColor = (variable: string) => {
        probe.style.color = `var(${variable})`;
        return getComputedStyle(probe).color;
      };
      const values = [getColor('--accent'), getColor('--accent-hover'), getColor('--on-accent')];
      probe.remove();
      return values;
    });

  for (const [palette, values] of Object.entries(expected)) {
    await page
      .locator('html')
      .evaluate((element, name) => element.setAttribute('data-palette', name), palette);
    expect(await readPalette(), `${palette} token contract`).toEqual(values);
  }

  await page
    .locator('html')
    .evaluate((element) => element.setAttribute('data-palette', 'graphite'));
  expect(await readPalette()).toEqual([
    'rgb(178, 165, 233)',
    'rgb(192, 180, 241)',
    'rgb(255, 255, 255)',
  ]);
  expect(sameColor(await token(page, '--workbench-accent-fill'), '#705eae')).toBe(true);
});

test('Graphite primary, selected, and focus roles use Windows forced colors and reduced motion', async ({
  page,
}) => {
  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('row', { name: /Candle Run/ }).click();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  const inspector = page.locator('.utility-surface');
  const selectedTab = inspector.getByRole('tab', { name: 'Details' });
  await expect(selectedTab).toHaveAttribute('aria-selected', 'true');

  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await expect
    .poll(() => page.evaluate(() => window.matchMedia('(forced-colors: active)').matches))
    .toBe(true);
  const systemColors = await page.evaluate(() => {
    const probe = document.createElement('span');
    probe.style.color = 'ButtonText';
    probe.style.backgroundColor = 'ButtonFace';
    probe.style.outlineColor = 'Highlight';
    document.body.append(probe);
    const style = getComputedStyle(probe);
    const highlightTextProbe = document.createElement('span');
    highlightTextProbe.style.color = 'HighlightText';
    document.body.append(highlightTextProbe);
    const values = {
      buttonText: style.color,
      buttonFace: style.backgroundColor,
      highlight: style.outlineColor,
      highlightText: getComputedStyle(highlightTextProbe).color,
    };
    probe.remove();
    highlightTextProbe.remove();
    return values;
  });
  const primary = player.locator('.player-primary-action');
  const primaryStyle = await colors(primary);
  expect(primaryStyle.background).toBe(systemColors.buttonFace);
  expect(primaryStyle.foreground).toBe(systemColors.buttonText);

  const activeNavigation = page.locator('.library-nav-item.is-active').first();
  const navigationStyle = await colors(activeNavigation);
  expect(navigationStyle.background, JSON.stringify({ navigationStyle, systemColors })).toBe(
    systemColors.highlight,
  );
  expect(navigationStyle.foreground).toBe(systemColors.highlightText);

  const tabStyle = await colors(selectedTab);
  expect(tabStyle.background).toBe(systemColors.buttonFace);
  expect(tabStyle.foreground).toBe(systemColors.highlight);
  expect(tabStyle.borderBottom).toBe(systemColors.highlight);

  await page.keyboard.press('Tab');
  await page
    .locator('.window-caption-controls button[aria-label="Minimize window"]')
    .evaluate((element) => element.focus());
  const focusStyle = await colors(
    page.locator('.window-caption-controls button[aria-label="Minimize window"]'),
  );
  expect(focusStyle.outlineStyle).toBe('solid');
  expect(Number.parseFloat(focusStyle.outlineWidth)).toBeGreaterThanOrEqual(2);
  expect(focusStyle.outlineColor).toBe(systemColors.highlight);
  expect(
    await page.evaluate(() => window.matchMedia('(prefers-reduced-motion: reduce)').matches),
  ).toBe(true);
});

test('captures the three matched Graphite hierarchy evidence views', async ({ page }) => {
  test.skip(
    captureStage !== 'after',
    'three matched Graphite browser captures are opt-in; set GRAPHITE_FINAL_UI_CAPTURE=after',
  );
  test.setTimeout(90_000);

  await openApp(page, { width: 1366, height: 768 });
  await startDryRunWithSelection(page);
  await page.mouse.move(0, 0);
  await capture(page, 'player-playing-selected-track-1366x768.png');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
  await capture(page, 'settings-1366x768.png');

  await openApp(page, { width: 1366, height: 768 });
  const close = page.getByRole('button', { name: 'Close window' });
  await close.hover();
  const rect = await close.boundingBox();
  if (!rect) throw new Error('Caption Close button has no visible bounds');
  await capture(page, 'titlebar-close-hover-1366x768.png', rect);
});
