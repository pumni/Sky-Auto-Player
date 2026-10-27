import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';

const captureStage = process.env.GRAPHITE_FINAL_UI_CAPTURE;
const evidenceDirectory =
  process.env.GRAPHITE_FINAL_UI_EVIDENCE_DIR ??
  resolve(
    process.cwd(),
    '..',
    'docs/evidence/graphite-final-ui-polish',
    captureStage === 'before' ? 'before' : 'after',
  );
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

async function openInspector(page: Page, tab: 'Details' | 'Runtime' = 'Details') {
  const toggle = page.getByRole('button', { name: 'Open utility panel' });
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  const inspector = page.locator('.utility-surface');
  await expect(inspector).toBeVisible();
  await inspector.getByRole('tab', { name: tab }).click();
  return inspector;
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

async function captureCaptionHover(page: Page, label: string, name: string) {
  const control = page.getByRole('button', { name: label });
  await control.hover();
  const rect = await control.boundingBox();
  if (!rect) throw new Error(`Caption control ${label} has no visible bounds`);
  await capture(page, name, rect);
}

async function readControls(page: Page, view: string) {
  return page.evaluate((viewName) => {
    const selectors = {
      captionMinimize: '.window-caption-controls button[aria-label="Minimize window"]',
      captionMaximize: '.window-caption-controls button[aria-label="Maximize window"]',
      captionClose: '.caption-button-close',
      update: '.update-indicator',
      libraryActive: '.library-nav-item.is-active',
      playerPrimary: '.player-primary-action',
      playerSecondary: '.player-secondary-action',
      playerTool: '.player-tool-button',
      focusUtility: '.player-utility-button',
      selectedTrackLike: '.track-row.is-selected .track-like-button',
      inspectorTab: '.utility-view-tabs [role="tab"]',
      settingsPrimary: '.settings-nav-item.is-active',
      menuItem: '.library-menu-item',
      primaryAction: '.button-primary',
      dangerAction: '.button-danger',
    } as const;
    const style = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const computed = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return {
        color: computed.color,
        backgroundColor: computed.backgroundColor,
        borderColor: computed.borderTopColor,
        borderWidth: computed.borderTopWidth,
        outlineColor: computed.outlineColor,
        outlineStyle: computed.outlineStyle,
        outlineWidth: computed.outlineWidth,
        disabled: element.matches(':disabled, [aria-disabled="true"]'),
        focusVisible: element.matches(':focus-visible'),
        width: rect.width,
        height: rect.height,
      };
    };
    return {
      view: viewName,
      viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
      parentSurfaces: {
        app: getComputedStyle(document.querySelector('.app-shell')!).backgroundColor,
        browser: getComputedStyle(document.querySelector('.track-browser-workbench-pane')!)
          .backgroundColor,
        player: getComputedStyle(document.querySelector('.player-bar')!).backgroundColor,
      },
      controls: Object.fromEntries(
        Object.entries(selectors).map(([name, selector]) => [name, style(selector)]),
      ),
    };
  }, view);
}

async function expectRestingControlsTransparent(page: Page) {
  await page.mouse.move(0, 0);
  const controls = await page.evaluate(() =>
    Array.from(
      document.querySelectorAll<HTMLElement>(
        'button, [role="button"], [role="tab"], [role="menuitem"]',
      ),
    )
      .filter((element) => element.getClientRects().length > 0)
      .map((element) => ({
        name:
          element.getAttribute('aria-label') ?? element.textContent?.trim() ?? element.className,
        background: getComputedStyle(element).backgroundColor,
        disabled: element.matches(':disabled, [aria-disabled="true"]'),
        opacity: getComputedStyle(element).opacity,
      })),
  );
  const filledControls = controls.filter(({ background }) => background !== 'rgba(0, 0, 0, 0)');
  const obscuredDisabledControls = controls.filter(
    ({ disabled, opacity }) => disabled && opacity !== '1',
  );
  expect(controls.length).toBeGreaterThan(0);
  expect(filledControls, 'visible controls should be transparent at rest').toEqual([]);
  expect(obscuredDisabledControls, 'disabled controls should not be dimmed with opacity').toEqual(
    [],
  );
}

test.skip(
  captureStage !== 'before' && captureStage !== 'after',
  'matched Graphite visual evidence capture; set GRAPHITE_FINAL_UI_CAPTURE=before or after',
);

test('captures matched Graphite final polish evidence states', async ({ page }) => {
  test.setTimeout(180_000);
  const measurements: Record<string, unknown> = {};

  await openApp(page, { width: 800, height: 560 });
  await page.mouse.move(0, 0);
  measurements['player-idle-800x560'] = await readControls(page, 'player-idle-800x560');
  await capture(page, 'player-idle-800x560.png');

  await openApp(page, { width: 1366, height: 768 });
  await captureCaptionHover(page, 'Minimize window', 'titlebar-minimize-hover-1366x768.png');
  await captureCaptionHover(page, 'Maximize window', 'titlebar-maximize-hover-1366x768.png');
  await captureCaptionHover(page, 'Close window', 'titlebar-close-hover-1366x768.png');
  await page.locator('.window-caption-controls').evaluate((element) => {
    element.classList.add('is-inactive');
  });
  await captureCaptionHover(
    page,
    'Maximize window',
    'titlebar-inactive-maximize-hover-1366x768.png',
  );
  await page.locator('.window-caption-controls').evaluate((element) => {
    element.classList.remove('is-inactive');
  });

  const widePlayer = await startDryRunWithSelection(page);
  await page.mouse.move(0, 0);
  measurements['player-playing-1366x768'] = await readControls(page, 'player-playing-1366x768');
  await capture(page, 'player-playing-selected-track-1366x768.png');
  const focusUtility = page.locator('.player-utility-button');
  await page.keyboard.press('Tab');
  await focusUtility.evaluate((element) => element.focus());
  await expect
    .poll(() => focusUtility.evaluate((element) => element.matches(':focus-visible')))
    .toBe(true);
  measurements['focus-utility-1366x768'] = await readControls(page, 'focus-utility-1366x768');
  await capture(page, 'focus-utility-1366x768.png');
  await page.mouse.move(0, 0);
  await widePlayer.getByRole('button', { name: 'Pause' }).click();
  await expect(widePlayer.getByRole('button', { name: 'Resume' })).toBeVisible();
  measurements['player-paused-1366x768'] = await readControls(page, 'player-paused-1366x768');
  await page.mouse.move(0, 0);
  await capture(page, 'player-paused-1366x768.png');
  const wideInspector = await openInspector(page, 'Details');
  await expect(wideInspector.getByRole('heading', { name: 'Blue Bird' })).toBeVisible();
  measurements['inspector-docked-1366x768'] = await readControls(page, 'inspector-docked-1366x768');
  await page.mouse.move(0, 0);
  await capture(page, 'inspector-docked-1366x768.png');

  await openApp(page, { width: 640, height: 448 });
  await startDryRunWithSelection(page);
  const compactInspector = await openInspector(page, 'Details');
  await expect(page.locator('.workbench')).toHaveAttribute('data-utility-presentation', 'overlay');
  await expect(compactInspector).toBeVisible();
  measurements['inspector-overlay-640x448'] = await readControls(page, 'inspector-overlay-640x448');
  await page.mouse.move(0, 0);
  await capture(page, 'inspector-overlay-640x448.png');

  await openApp(page, { width: 1920, height: 1080 });
  await startDryRunWithSelection(page);
  const runtimeInspector = await openInspector(page, 'Runtime');
  await expect(page.getByRole('region', { name: 'Utility: Diagnostics' })).toBeVisible();
  await expect(
    runtimeInspector.getByText('Mode: Dry run · no physical input sent', { exact: true }),
  ).toBeVisible();
  measurements['runtime-1920x1080'] = await readControls(page, 'runtime-1920x1080');
  await page.mouse.move(0, 0);
  await capture(page, 'runtime-1920x1080.png');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible();
  await capture(page, 'settings-1366x768.png');
  await page.getByRole('button', { name: 'Close settings' }).click();

  await openApp(page, { width: 1366, height: 768 });
  await page.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  await expect(createDialog).toBeVisible();
  measurements['new-playlist-dialog-1366x768'] = await readControls(
    page,
    'new-playlist-dialog-1366x768',
  );
  await capture(page, 'new-playlist-dialog-1366x768.png');
  await createDialog.getByLabel('Playlist name').fill('Graphite Evidence Playlist');
  await createDialog.getByRole('button', { name: 'Create' }).click();
  const playlistActions = page.getByRole('button', {
    name: 'More actions for Graphite Evidence Playlist',
  });
  await playlistActions.click();
  const menu = page.locator('.library-menu-popover');
  await expect(menu).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Delete playlist' })).toBeVisible();
  measurements['playlist-menu-1366x768'] = await readControls(page, 'playlist-menu-1366x768');
  await capture(page, 'playlist-menu-1366x768.png');
  await page.getByRole('menuitem', { name: 'Delete playlist' }).click();
  const confirmation = page.getByRole('dialog', { name: /Delete/ });
  await expect(confirmation).toBeVisible();
  measurements['delete-confirmation-1366x768'] = await readControls(
    page,
    'delete-confirmation-1366x768',
  );
  await capture(page, 'delete-confirmation-1366x768.png');

  await openApp(page, { width: 1366, height: 768 }, false, false);
  await expect(page.locator('.app-titlebar')).toHaveAttribute('data-update-available', 'false');
  await openApp(page, { width: 1366, height: 768 }, false, true);
  await expect(page.locator('.app-titlebar')).toHaveAttribute('data-update-available', 'true');

  const browser = page.context().browser();
  measurements.captureMetadata = {
    stage: captureStage,
    sourceRevision: process.env.GRAPHITE_FINAL_UI_SOURCE_SHA ?? 'unspecified',
    browserVersion: browser?.version() ?? 'unknown',
    userAgent: await page.evaluate(() => navigator.userAgent),
    hostPlatform: process.platform,
    mockPlayback: true,
    physicalInputSent: false,
  };

  mkdirSync(evidenceDirectory, { recursive: true });
  await import('node:fs/promises').then(({ writeFile }) =>
    writeFile(
      resolve(evidenceDirectory, 'computed-controls.json'),
      `${JSON.stringify(measurements, null, 2)}\n`,
    ),
  );
});

test('Graphite controls expose transparent rest and visible hover and focus states', async ({
  page,
}) => {
  test.skip(captureStage !== 'after', 'state assertions run against the after implementation');
  test.setTimeout(90_000);
  await openApp(page, { width: 1366, height: 768 });

  const minimize = page.getByRole('button', { name: 'Minimize window' });
  const maximize = page.getByRole('button', { name: 'Maximize window' });
  const close = page.getByRole('button', { name: 'Close window' });
  const hoverState = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--workbench-control-hover').trim(),
  );
  const pressedState = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue('--workbench-control-pressed')
      .trim(),
  );
  const rgb = (color: string) => {
    if (color.startsWith('#')) {
      const hex = color.slice(1);
      if (hex.length !== 6) throw new Error(`Cannot read color ${color}`);
      return [...[0, 2, 4].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16)), 1];
    }
    const channels = color.match(/[\d.]+/g)?.map(Number);
    if (!channels || (channels.length !== 3 && channels.length !== 4)) {
      throw new Error(`Cannot read color ${color}`);
    }
    return channels.length === 3 ? [...channels, 1] : channels;
  };
  const sameColor = (first: string, second: string) =>
    rgb(first).every((channel, index) => channel === rgb(second)[index]);
  const luminance = (color: string) => {
    const channels = rgb(color)
      .slice(0, 3)
      .map((channel) => {
        const normalized = channel / 255;
        return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
      });
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
  };
  const contrastRatio = (first: string, second: string) => {
    const [lighter, darker] = [luminance(first), luminance(second)].sort((a, b) => b - a);
    return (lighter + 0.05) / (darker + 0.05);
  };
  const background = (control: Locator) =>
    control.evaluate((element) => getComputedStyle(element).backgroundColor);
  const hoverLuminance = luminance(hoverState);
  const parentSurfaceColors = await page.evaluate(() =>
    ['.app-titlebar', '.player-bar', '.workbench-pane']
      .map((selector) => document.querySelector<HTMLElement>(selector))
      .filter((element): element is HTMLElement => element !== null)
      .map((element) => getComputedStyle(element).backgroundColor),
  );
  const contrastTokens = await page.evaluate(() => {
    const style = getComputedStyle(document.documentElement);
    return {
      accent: style.getPropertyValue('--workbench-accent').trim(),
      accentHover: style.getPropertyValue('--workbench-accent-hover').trim(),
      focus: style.getPropertyValue('--workbench-focus').trim(),
      dangerText: style.getPropertyValue('--danger-fg').trim(),
      dangerFill: style.getPropertyValue('--danger-fill').trim(),
      onDanger: style.getPropertyValue('--on-danger').trim(),
      dangerHover: style.getPropertyValue('--workbench-control-danger-hover').trim(),
    };
  });
  expect(parentSurfaceColors.length).toBeGreaterThan(2);
  expect(parentSurfaceColors.every((surface) => hoverLuminance > luminance(surface))).toBe(true);
  for (const surface of parentSurfaceColors) {
    expect(contrastRatio(contrastTokens.accent, surface)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(contrastTokens.focus, surface)).toBeGreaterThanOrEqual(3);
  }
  expect(contrastRatio(contrastTokens.accent, pressedState)).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(contrastTokens.accentHover, hoverState)).toBeGreaterThanOrEqual(4.5);
  expect(
    contrastRatio(contrastTokens.dangerText, contrastTokens.dangerHover),
  ).toBeGreaterThanOrEqual(4.5);
  expect(contrastRatio(contrastTokens.onDanger, contrastTokens.dangerFill)).toBeGreaterThanOrEqual(
    4.5,
  );

  await expectRestingControlsTransparent(page);

  const disabledPlay = page.locator('.player-primary-action');
  await expect(disabledPlay).toBeDisabled();
  const disabledText = await disabledPlay.evaluate((element) => getComputedStyle(element).color);
  await disabledPlay.hover();
  expect(sameColor(await background(disabledPlay), 'rgba(0, 0, 0, 0)')).toBe(true);
  expect(await disabledPlay.evaluate((element) => getComputedStyle(element).opacity)).toBe('1');
  expect(await disabledPlay.evaluate((element) => getComputedStyle(element).color)).toBe(
    disabledText,
  );
  expect(disabledText).toBe('rgb(139, 140, 144)');

  await page.getByRole('row', { name: /Candle Run/ }).click();
  const enabledPlay = page.locator('.player-primary-action');
  await expect(enabledPlay).toBeEnabled();
  await expectRestingControlsTransparent(page);
  expect(sameColor(await background(enabledPlay), 'rgba(0, 0, 0, 0)')).toBe(true);
  await enabledPlay.hover();
  expect(sameColor(await background(enabledPlay), hoverState)).toBe(true);
  expect(await enabledPlay.evaluate((element) => getComputedStyle(element).color)).toBe(
    'rgb(178, 165, 233)',
  );
  await page.mouse.down();
  expect(sameColor(await background(enabledPlay), pressedState)).toBe(true);
  await page.mouse.up();
  await page.mouse.move(0, 0);
  expect(sameColor(await background(enabledPlay), 'rgba(0, 0, 0, 0)')).toBe(true);

  for (const control of [minimize, maximize]) {
    await control.hover();
    const hoverColor = await background(control);
    expect(sameColor(hoverColor, 'rgba(0, 0, 0, 0)')).toBe(false);
    expect(
      sameColor(
        hoverColor,
        await page.evaluate(
          () => getComputedStyle(document.querySelector('.app-titlebar')!).backgroundColor,
        ),
      ),
    ).toBe(false);
    await page.mouse.move(0, 0);
    expect(sameColor(await background(control), 'rgba(0, 0, 0, 0)')).toBe(true);
  }

  await page.locator('.window-caption-controls').evaluate((element) => {
    element.classList.add('is-inactive');
  });
  for (const control of [minimize, maximize]) {
    await control.hover();
    expect(sameColor(await background(control), 'rgba(0, 0, 0, 0)')).toBe(false);
    const color = await control.evaluate((element) => getComputedStyle(element).color);
    expect(color).not.toBe('rgb(146, 146, 146)');
  }
  await close.hover();
  const closeHoverColor = await background(close);
  expect(sameColor(closeHoverColor, await background(maximize))).toBe(false);
  const dangerFill = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--danger-fill').trim(),
  );
  expect(sameColor(closeHoverColor, dangerFill)).toBe(true);
  await page.locator('.window-caption-controls').evaluate((element) => {
    element.classList.remove('is-inactive');
  });

  const utilityToggle = page.locator('.player-utility-button');
  await utilityToggle.hover();
  expect(sameColor(await background(utilityToggle), hoverState)).toBe(true);
  await page.mouse.down();
  const utilityPressedColor = await background(utilityToggle);
  expect(sameColor(utilityPressedColor, pressedState)).toBe(true);
  await page.mouse.up();
  await page.mouse.move(0, 0);
  expect(sameColor(await background(utilityToggle), 'rgba(0, 0, 0, 0)')).toBe(true);
  await page.keyboard.press('Tab');
  await utilityToggle.evaluate((element) => element.focus());
  expect(await utilityToggle.evaluate((element) => element.matches(':focus-visible'))).toBe(true);
  const outline = await utilityToggle.evaluate((element) => {
    const style = getComputedStyle(element);
    return { style: style.outlineStyle, width: style.outlineWidth, color: style.outlineColor };
  });
  expect(outline.style).toBe('solid');
  expect(outline.width).toBe('2px');
  expect(outline.color).toBe('rgb(178, 165, 233)');

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  const activeSetting = settings.getByRole('button', { name: 'Playback' });
  await expect(activeSetting).toHaveAttribute('aria-current', 'page');
  await expectRestingControlsTransparent(page);
  expect(await activeSetting.evaluate((element) => getComputedStyle(element).borderLeftColor)).toBe(
    'rgb(178, 165, 233)',
  );
  await activeSetting.hover();
  expect(sameColor(await background(activeSetting), hoverState)).toBe(true);
  await settings.getByRole('button', { name: 'Close settings' }).click();

  await openApp(page, { width: 1366, height: 768 });
  await page.locator('.library-add-button').click();
  const createDialog = page.getByRole('dialog', { name: 'New playlist' });
  const createButton = createDialog.getByRole('button', { name: 'Create' });
  await expectRestingControlsTransparent(page);
  await createButton.hover();
  expect(sameColor(await background(createButton), hoverState)).toBe(true);
  await createDialog.getByLabel('Playlist name').fill('Graphite State Test');
  await createButton.click();

  await page.getByRole('button', { name: 'More actions for Graphite State Test' }).click();
  const deleteMenuItem = page.getByRole('menuitem', { name: 'Delete playlist' });
  await expectRestingControlsTransparent(page);
  const dangerHover = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue('--workbench-control-danger-hover')
      .trim(),
  );
  await deleteMenuItem.hover();
  expect(sameColor(await background(deleteMenuItem), dangerHover)).toBe(true);
  await deleteMenuItem.click();

  const deleteDialog = page.getByRole('dialog', { name: /Delete/ });
  const deleteButton = deleteDialog.getByRole('button', { name: 'Delete' });
  await expectRestingControlsTransparent(page);
  await deleteButton.hover();
  expect(sameColor(await background(deleteButton), dangerHover)).toBe(true);

  await openApp(page, { width: 1366, height: 768 });
  await page.getByRole('row', { name: /Candle Run/ }).click();
  await openInspector(page, 'Details');
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await expect
    .poll(() => page.evaluate(() => window.matchMedia('(forced-colors: active)').matches))
    .toBe(true);
  await page.keyboard.press('Tab');
  const forcedColors = await page.evaluate(() => {
    const primary = getComputedStyle(document.querySelector('.player-primary-action')!);
    const activeNavigation = getComputedStyle(
      document.querySelector('.library-nav-item.is-active')!,
    );
    const selectedTab = getComputedStyle(
      document.querySelector('.utility-view-tabs [role="tab"][aria-selected="true"]')!,
    );
    const minimizeElement = document.querySelector<HTMLElement>(
      '.window-caption-controls button[aria-label="Minimize window"]',
    )!;
    const probe = document.createElement('span');
    probe.style.outlineColor = 'Highlight';
    probe.style.color = 'ButtonText';
    probe.style.backgroundColor = 'ButtonFace';
    document.body.append(probe);
    const highlight = getComputedStyle(probe).outlineColor;
    const buttonText = getComputedStyle(probe).color;
    const buttonFace = getComputedStyle(probe).backgroundColor;
    probe.remove();
    minimizeElement.focus();
    const minimizeButton = getComputedStyle(minimizeElement);
    return {
      primaryBackground: primary.backgroundColor,
      primaryColor: primary.color,
      captionBackground: minimizeButton.backgroundColor,
      activeNavigationBackground: activeNavigation.backgroundColor,
      activeNavigationColor: activeNavigation.color,
      selectedTabBackground: selectedTab.backgroundColor,
      selectedTabColor: selectedTab.color,
      selectedTabEdge: selectedTab.borderBottomColor,
      focusOutline: minimizeButton.outlineStyle,
      focusWidth: minimizeButton.outlineWidth,
      focusColor: minimizeButton.outlineColor,
      buttonText,
      buttonFace,
      highlight,
    };
  });
  expect(forcedColors.primaryBackground).toBe(forcedColors.buttonFace);
  expect(forcedColors.captionBackground).toBe(forcedColors.buttonFace);
  expect(forcedColors.primaryColor).toBe(forcedColors.buttonText);
  expect(forcedColors.activeNavigationBackground).toBe(forcedColors.highlight);
  expect(forcedColors.activeNavigationColor).not.toBe(forcedColors.activeNavigationBackground);
  expect(
    contrastRatio(forcedColors.activeNavigationColor, forcedColors.activeNavigationBackground),
  ).toBeGreaterThanOrEqual(4.5);
  expect(forcedColors.selectedTabBackground).toBe(forcedColors.buttonFace);
  expect(forcedColors.selectedTabColor).toBe(forcedColors.highlight);
  expect(forcedColors.selectedTabEdge).toBe(forcedColors.highlight);
  expect(forcedColors.focusOutline).toBe('solid');
  expect(Number.parseFloat(forcedColors.focusWidth)).toBeGreaterThanOrEqual(2);
  expect(forcedColors.focusColor).toBe(forcedColors.highlight);
});
