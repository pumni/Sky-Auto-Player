import { expect, test } from '@playwright/test';
import { palettes, selectPalette } from './testHelpers';

function channels(color: string): [number, number, number] {
  const rgbMatch = color.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)/);
  if (rgbMatch) return [Number(rgbMatch[1]), Number(rgbMatch[2]), Number(rgbMatch[3])];

  const oklabMatch = color.match(
    /^oklab\(\s*([+-]?(?:\d+\.?\d*|\.\d+))\s+([+-]?(?:\d+\.?\d*|\.\d+))\s+([+-]?(?:\d+\.?\d*|\.\d+))/,
  );
  if (oklabMatch) {
    const lightness = Number(oklabMatch[1]);
    const a = Number(oklabMatch[2]);
    const b = Number(oklabMatch[3]);
    const light = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const medium = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const short = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
    const toSrgb = (value: number) => {
      const linear = Math.max(0, Math.min(1, value));
      const encoded = linear <= 0.0031308 ? 12.92 * linear : 1.055 * linear ** (1 / 2.4) - 0.055;
      return Math.round(encoded * 255);
    };
    return [
      toSrgb(4.0767416621 * light - 3.3077115913 * medium + 0.2309699292 * short),
      toSrgb(-1.2684380046 * light + 2.6097574011 * medium - 0.3413193965 * short),
      toSrgb(-0.0041960863 * light - 0.7034186147 * medium + 1.707614701 * short),
    ];
  }

  throw new Error(`Unsupported computed color: ${color}`);
}

function luminance(color: string): number {
  return channels(color)
    .map((channel) => channel / 255)
    .map((value) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4))
    .reduce((total, value, index) => total + value * ([0.2126, 0.7152, 0.0722][index] ?? 0), 0);
}

function contrast(first: string, second: string): number {
  const left = luminance(first);
  const right = luminance(second);
  return (Math.max(left, right) + 0.05) / (Math.min(left, right) + 0.05);
}

test('Runtime tabs fill the available width and use one raised segmented indicator', async ({
  page,
}) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Open utility panel' }).click();
  const songDetails = page.getByRole('region', { name: 'Utility: Song Details' });
  await songDetails.getByRole('tab', { name: 'Runtime' }).click();

  const assertColorsAndGeometry = async () => {
    await expect
      .poll(() =>
        page.evaluate(() => {
          const active = document.querySelector<HTMLElement>(
            '[data-slot="tabs-list"][aria-label="Diagnostics views"] [role="tab"][aria-selected="true"]',
          );
          const probe = document.createElement('span');
          document.body.append(probe);
          probe.style.color = 'var(--foreground)';
          const expected = getComputedStyle(probe).color;
          probe.remove();
          return active ? getComputedStyle(active).color === expected : false;
        }),
      )
      .toBe(true);
    const result = await page.evaluate(() => {
      const measure = (label: string) => {
        const list = document.querySelector<HTMLElement>(
          `[data-slot="tabs-list"][aria-label="${label}"]`,
        );
        const indicator = list?.querySelector<HTMLElement>('[data-slot="tabs-indicator"]');
        const active = list?.querySelector<HTMLElement>('[role="tab"][data-active]');
        const triggers = Array.from(list?.querySelectorAll<HTMLElement>('[role="tab"]') ?? []);
        if (!list || !indicator || !active) return null;
        const listRect = list.getBoundingClientRect();
        const indicatorRect = indicator.getBoundingClientRect();
        const activeRect = active.getBoundingClientRect();
        const probe = document.createElement('span');
        document.body.append(probe);
        const color = (role: string) => {
          probe.style.color = `var(${role})`;
          return getComputedStyle(probe).color;
        };
        const result = {
          variant: list.dataset.variant,
          indicatorCount: list.querySelectorAll('[data-slot="tabs-indicator"]').length,
          listWidth: listRect.width,
          parentWidth: (() => {
            const parent = list.parentElement;
            if (!parent) return 0;
            const parentStyle = getComputedStyle(parent);
            return (
              parent.clientWidth -
              Number.parseFloat(parentStyle.paddingLeft) -
              Number.parseFloat(parentStyle.paddingRight)
            );
          })(),
          track: getComputedStyle(list).backgroundColor,
          indicator: getComputedStyle(indicator).backgroundColor,
          indicatorBorder: getComputedStyle(indicator).borderTopColor,
          indicatorHeight: indicatorRect.height,
          indicatorTopDelta: indicatorRect.top - activeRect.top,
          indicatorBottomDelta: activeRect.bottom - indicatorRect.bottom,
          indicatorLeftDelta: indicatorRect.left - activeRect.left,
          indicatorWidthDelta: indicatorRect.width - activeRect.width,
          indicatorZ: Number.parseInt(getComputedStyle(indicator).zIndex, 10),
          activeZ: Number.parseInt(getComputedStyle(active).zIndex, 10),
          activeBackground: getComputedStyle(active).backgroundColor,
          activeText: getComputedStyle(active).color,
          activeWeight: Number.parseInt(getComputedStyle(active).fontWeight, 10),
          triggerWidths: triggers.map((trigger) => trigger.getBoundingClientRect().width),
          secondary: color('--secondary'),
          controlActive: color('--control-active'),
          foreground: color('--foreground'),
          border: color('--border'),
          accentInk: color('--accent-ink'),
        };
        probe.remove();
        return result;
      };
      return { line: measure('Utility views'), segmented: measure('Diagnostics views') };
    });
    expect(result.line).not.toBeNull();
    expect(result.segmented).not.toBeNull();
    if (!result.line || !result.segmented) return;

    expect(result.line.variant).toBe('line');
    expect(result.line.indicatorCount).toBe(1);
    expect(result.line.listWidth).toBeCloseTo(result.line.parentWidth, 0);
    expect(result.line.track).toBe('rgba(0, 0, 0, 0)');
    expect(result.line.triggerWidths[0]).toBeCloseTo(result.line.triggerWidths[1] ?? 0, 0);
    expect(result.line.indicator).toBe(result.line.accentInk);
    expect(result.line.indicatorHeight).toBeCloseTo(2, 0);
    expect(Math.abs(result.line.indicatorLeftDelta)).toBeLessThanOrEqual(1);
    expect(result.line.indicatorWidthDelta).toBeCloseTo(0, 0);

    expect(result.segmented.variant).toBe('segmented');
    expect(result.segmented.indicatorCount).toBe(1);
    expect(result.segmented.listWidth).toBeCloseTo(result.segmented.parentWidth, 0);
    expect(result.segmented.triggerWidths[0]).toBeCloseTo(
      result.segmented.triggerWidths[1] ?? 0,
      0,
    );
    expect(result.segmented.triggerWidths[1]).toBeCloseTo(
      result.segmented.triggerWidths[2] ?? 0,
      0,
    );
    expect(result.segmented.track).toBe(result.segmented.secondary);
    expect(result.segmented.indicator).toBe(result.segmented.controlActive);
    expect(result.segmented.indicatorBorder).toBe(result.segmented.border);
    expect(result.segmented.activeBackground).toBe('rgba(0, 0, 0, 0)');
    expect(result.segmented.activeText).toBe(result.segmented.foreground);
    expect(result.segmented.activeWeight).toBeGreaterThanOrEqual(600);
    expect(result.segmented.indicatorZ).toBeLessThan(result.segmented.activeZ);
    expect(Math.abs(result.segmented.indicatorLeftDelta)).toBeLessThanOrEqual(1);
    expect(result.segmented.indicatorWidthDelta).toBeCloseTo(0, 0);
    expect(Math.abs(result.segmented.indicatorTopDelta)).toBeLessThanOrEqual(1);
    expect(Math.abs(result.segmented.indicatorBottomDelta)).toBeLessThanOrEqual(1);
  };

  for (const palette of palettes) {
    await selectPalette(page, palette);
    await assertColorsAndGeometry();
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  await assertColorsAndGeometry();
  await page.emulateMedia({ forcedColors: 'active' });
  await expect
    .poll(() =>
      page
        .locator(
          '[data-slot="tabs-list"][aria-label="Diagnostics views"] [data-slot="tabs-indicator"]',
        )
        .evaluate((element) => getComputedStyle(element).backgroundColor),
    )
    .not.toBe('rgba(0, 0, 0, 0)');
});

test('scrollbar visibility follows scrolling and thumb drag, not hover or focus', async ({
  page,
}) => {
  await page.goto('/');
  const grid = page.getByRole('grid');
  const scrollArea = page.locator('[data-slot="scroll-area"]:has([role="grid"])');
  const scrollbar = scrollArea.locator(
    '[data-slot="scroll-area-scrollbar"][data-orientation="vertical"]',
  );
  const thumb = scrollbar.locator('[data-slot="scroll-area-thumb"]');
  await expect(grid).toBeVisible();

  const expectHidden = async () => {
    await expect
      .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).opacity))
      .toBe('0');
    await expect
      .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).pointerEvents))
      .toBe('none');
  };
  const expectActive = async () => {
    await expect
      .poll(
        () =>
          scrollbar.evaluate((element) => {
            const active =
              element.hasAttribute('data-scrolling') ||
              element.getAttribute('data-dragging') === 'true';
            const style = getComputedStyle(element);
            return active && Number.parseFloat(style.opacity) > 0 && style.pointerEvents === 'auto';
          }),
        { intervals: [10, 20, 40] },
      )
      .toBe(true);
  };
  const waitForScrollToSettle = async () => {
    await expect
      .poll(() => scrollbar.evaluate((element) => element.hasAttribute('data-scrolling')))
      .toBe(false);
  };

  for (const [paletteIndex, palette] of palettes.entries()) {
    await selectPalette(page, palette);
    await grid.evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.mouse.move(0, 0);
    await expectHidden();
    await page.getByRole('row', { name: /Aurora Landing/ }).hover();
    await expectHidden();
    if (paletteIndex === 0) {
      await grid.focus();
      await expect(grid).toBeFocused();
      await expectHidden();

      const previousTopBeforeKeyboard = await grid.evaluate((element) => element.scrollTop);
      await page.keyboard.press('PageDown');
      await page.keyboard.press('PageDown');
      await expect
        .poll(() => grid.evaluate((element) => element.scrollTop), { intervals: [10, 20, 40] })
        .toBeGreaterThan(previousTopBeforeKeyboard);
      await expectActive();
      await waitForScrollToSettle();
      await expectHidden();
    }

    const previousTop = await grid.evaluate((element) => element.scrollTop);
    await grid.hover();
    await page.mouse.wheel(0, 320);
    await page.mouse.wheel(0, 320);
    await expect
      .poll(() => grid.evaluate((element) => element.scrollTop), { intervals: [10, 20, 40] })
      .toBeGreaterThan(previousTop);
    await expectActive();

    const colors = await scrollArea.evaluate((area) => {
      const track = area.querySelector<HTMLElement>('[role="grid"]');
      const thumb = area.querySelector<HTMLElement>('[data-slot="scroll-area-thumb"]');
      const scrollbar = area.querySelector<HTMLElement>('[data-slot="scroll-area-scrollbar"]');
      if (!track || !thumb || !scrollbar) return null;
      return {
        thumb: getComputedStyle(thumb).backgroundColor,
        surface: getComputedStyle(track).backgroundColor,
        track: getComputedStyle(scrollbar).backgroundColor,
      };
    });
    expect(colors).not.toBeNull();
    if (colors) {
      expect(colors.track).toBe('rgba(0, 0, 0, 0)');
      expect(contrast(colors.thumb, colors.surface)).toBeGreaterThanOrEqual(3);
    }

    if (paletteIndex === 0) {
      const thumbBox = await thumb.boundingBox();
      expect(thumbBox).not.toBeNull();
      if (!thumbBox) throw new Error('The overflowing Library thumb has no bounds.');
      const thumbX = thumbBox.x + thumbBox.width / 2;
      const thumbY = thumbBox.y + thumbBox.height / 2;
      await page.mouse.move(thumbX, thumbY);
      await page.mouse.wheel(0, 32);
      await page.mouse.wheel(0, 32);
      await expect
        .poll(() => scrollbar.evaluate((element) => element.hasAttribute('data-scrolling')))
        .toBe(true);
      await page.mouse.down();
      await page.mouse.move(thumbX, thumbY + 14);
      await expect.poll(() => scrollbar.getAttribute('data-dragging')).toBe('true');
      await expectActive();
      await waitForScrollToSettle();
      await expect
        .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).opacity))
        .toBe('1');
      await expect
        .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).pointerEvents))
        .toBe('auto');

      await page.mouse.up();
      await expect.poll(() => scrollbar.getAttribute('data-dragging')).toBeNull();
      await waitForScrollToSettle();
      await expectHidden();

      const cancelThumbBox = await thumb.boundingBox();
      expect(cancelThumbBox).not.toBeNull();
      if (!cancelThumbBox) throw new Error('The overflowing Library thumb has no bounds.');
      const cancelX = cancelThumbBox.x + cancelThumbBox.width / 2;
      const cancelY = cancelThumbBox.y + cancelThumbBox.height / 2;
      await page.mouse.move(cancelX, cancelY);
      await page.mouse.wheel(0, 32);
      await page.mouse.wheel(0, 32);
      await expect
        .poll(() => scrollbar.evaluate((element) => element.hasAttribute('data-scrolling')))
        .toBe(true);
      await page.mouse.down();
      await page.mouse.move(cancelX, cancelY + 14);
      await expect.poll(() => scrollbar.getAttribute('data-dragging')).toBe('true');
      await scrollbar.evaluate((element) =>
        element.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true, pointerId: 1 })),
      );
      await expect.poll(() => scrollbar.getAttribute('data-dragging')).toBeNull();
      await page.mouse.up();
      await waitForScrollToSettle();
      await expectHidden();
    } else {
      await waitForScrollToSettle();
      await expectHidden();
    }
  }

  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  await grid.evaluate((element) => {
    element.scrollTop = 0;
  });
  await page.mouse.move(0, 0);
  await expectHidden();
  const lightPreviousTop = await grid.evaluate((element) => element.scrollTop);
  await grid.hover();
  await page.mouse.wheel(0, 320);
  await page.mouse.wheel(0, 320);
  await expect
    .poll(() => grid.evaluate((element) => element.scrollTop), { intervals: [10, 20, 40] })
    .toBeGreaterThan(lightPreviousTop);
  await expectActive();
  const lightColors = await scrollArea.evaluate((area) => {
    const track = area.querySelector<HTMLElement>('[role="grid"]');
    const thumb = area.querySelector<HTMLElement>('[data-slot="scroll-area-thumb"]');
    if (!track || !thumb) return null;
    return {
      thumb: getComputedStyle(thumb).backgroundColor,
      surface: getComputedStyle(track).backgroundColor,
    };
  });
  expect(lightColors).not.toBeNull();
  if (lightColors)
    expect(contrast(lightColors.thumb, lightColors.surface)).toBeGreaterThanOrEqual(3);

  await page.emulateMedia({ forcedColors: 'active' });
  await expect
    .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).opacity))
    .toBe('1');
  await expect
    .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).pointerEvents))
    .toBe('auto');
  await expect
    .poll(() => thumb.evaluate((element) => getComputedStyle(element).backgroundColor))
    .not.toBe('rgba(0, 0, 0, 0)');
});

test('expanded Playback Profile fits at 800 by 560 and keeps Select inside the viewport', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('row', { name: /Blue Bird/ }).click();
  await page.getByRole('button', { name: 'Configure playback profile' }).click();
  const profile = page.getByRole('dialog', { name: 'Playback profile' });
  await profile.getByText('Advanced timing', { exact: true }).click();
  await expect(profile.getByLabel('FPS')).toBeVisible();
  await expect(profile.getByText('Frame duration')).toBeVisible();
  await expect(profile.getByText('Base hold duration')).toBeVisible();
  await expect(profile.getByText('Effective target hold')).toBeVisible();
  await expect(profile.getByRole('button', { name: 'Test playback (no input)' })).toBeVisible();

  for (const viewportSize of [
    { width: 800, height: 560 },
    { width: 1200, height: 760 },
    { width: 1366, height: 768 },
  ]) {
    await page.setViewportSize(viewportSize);
    const geometry = await profile.evaluate((element) => {
      const viewport = element.querySelector<HTMLElement>('[data-slot="scroll-area-viewport"]');
      if (!viewport) return null;
      const viewportRect = viewport.getBoundingClientRect();
      const controls = Array.from(
        element.querySelectorAll<HTMLElement>('button, summary, [role="combobox"]'),
      ).filter((control) => getComputedStyle(control).display !== 'none');
      return {
        viewport: {
          top: viewportRect.top,
          bottom: viewportRect.bottom,
          left: viewportRect.left,
          right: viewportRect.right,
          clientWidth: viewport.clientWidth,
          scrollWidth: viewport.scrollWidth,
          clientHeight: viewport.clientHeight,
          scrollHeight: viewport.scrollHeight,
        },
        scrollbars: element.querySelectorAll('[data-slot="scroll-area-scrollbar"]').length,
        controls: controls.map((control) => {
          const rect = control.getBoundingClientRect();
          return { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
        }),
      };
    });
    expect(
      geometry,
      `${viewportSize.width}x${viewportSize.height} profile viewport`,
    ).not.toBeNull();
    if (!geometry) throw new Error('Playback Profile has no scroll viewport.');
    expect(geometry.viewport.scrollHeight).toBeLessThanOrEqual(geometry.viewport.clientHeight + 1);
    expect(geometry.viewport.scrollWidth).toBeLessThanOrEqual(geometry.viewport.clientWidth + 1);
    expect(geometry.scrollbars).toBe(0);
    for (const control of geometry.controls) {
      expect(control.top).toBeGreaterThanOrEqual(geometry.viewport.top - 1);
      expect(control.bottom).toBeLessThanOrEqual(geometry.viewport.bottom + 1);
      expect(control.left).toBeGreaterThanOrEqual(geometry.viewport.left - 1);
      expect(control.right).toBeLessThanOrEqual(geometry.viewport.right + 1);
    }
  }

  await page.setViewportSize({ width: 800, height: 560 });

  const baseHold = profile.getByRole('combobox', { name: 'Base Hold' });
  await baseHold.click();
  const select = page.locator('[data-slot="select-content"]:visible');
  await expect(select).toBeVisible();
  const selectBox = await select.boundingBox();
  expect(selectBox).not.toBeNull();
  if (selectBox) {
    expect(selectBox.x).toBeGreaterThanOrEqual(0);
    expect(selectBox.x + selectBox.width).toBeLessThanOrEqual(800);
    expect(selectBox.y).toBeGreaterThanOrEqual(0);
    expect(selectBox.y + selectBox.height).toBeLessThanOrEqual(560);
  }
});

test('Settings scrollbars stay hidden through hover, focus, and an open Select', async ({
  page,
}) => {
  await page.setViewportSize({ width: 640, height: 320 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings' }).click();
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click();
  const scrollbars = settings.locator('[data-slot="scroll-area-scrollbar"]');
  await expect.poll(() => scrollbars.count()).toBeGreaterThan(0);

  const expectAllHidden = async () => {
    for (const scrollbar of await scrollbars.all()) {
      await expect
        .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).opacity))
        .toBe('0');
      await expect
        .poll(() => scrollbar.evaluate((element) => getComputedStyle(element).pointerEvents))
        .toBe('none');
    }
  };
  await expectAllHidden();
  await settings.locator('[data-slot="settings-content-scroll-region"]').hover();
  await expectAllHidden();
  const categoryNavigation = settings.getByRole('navigation', { name: 'Settings categories' });
  await categoryNavigation.focus();
  await expect(categoryNavigation).toBeFocused();
  await expectAllHidden();
  await settings.getByRole('combobox', { name: 'Dark palette' }).click();
  await expect(page.locator('[data-slot="select-content"]:visible')).toBeVisible();
  await expectAllHidden();
});

test('Settings keeps all six category rows vertical and visible at compact supported sizes', async ({
  page,
}) => {
  const categoryNames = ['Playback', 'Appearance', 'Diagnostics', 'Updates', 'Donate', 'About'];
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/');
    await page.getByRole('button', { name: 'Open settings' }).click();
    const navigation = page
      .getByRole('dialog', { name: 'Settings' })
      .getByRole('navigation', { name: 'Settings categories' });
    const rows = categoryNames.map((name) => navigation.getByRole('button', { name, exact: true }));
    for (const row of rows) await expect(row).toBeInViewport();
    const rowTops = await Promise.all(rows.map((row) => row.boundingBox()));
    for (let index = 1; index < rowTops.length; index += 1) {
      expect(rowTops[index]?.y).toBeGreaterThan(rowTops[index - 1]?.y ?? 0);
    }
  }
});

test('Previous and Next use filled glyphs with the existing centered transport geometry', async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 560 });
  await page.goto('/');
  await page.getByRole('row', { name: /Aurora Landing/ }).click();
  const player = page.getByRole('contentinfo', { name: 'Player controls' });
  const geometry = await player.evaluate((element) => {
    const bounds = (selector: string) => {
      const target = element.querySelector<HTMLElement>(selector);
      if (!target) return null;
      const rect = target.getBoundingClientRect();
      return {
        width: rect.width,
        height: rect.height,
        centerX: rect.left + rect.width / 2,
        centerY: rect.top + rect.height / 2,
      };
    };
    const iconFill = (name: string) => {
      const button = Array.from(element.querySelectorAll<HTMLButtonElement>('button')).find(
        (candidate) => candidate.getAttribute('aria-label') === name,
      );
      const icon = button?.querySelector<SVGElement>('svg');
      return icon
        ? { attribute: icon.getAttribute('fill'), computed: getComputedStyle(icon).fill }
        : null;
    };
    return {
      previous: bounds('button[aria-label="Previous"]'),
      primary: bounds('[data-slot="primary-transport"]'),
      next: bounds('button[aria-label="Next"]'),
      previousFill: iconFill('Previous'),
      nextFill: iconFill('Next'),
    };
  });
  expect(geometry.previous).not.toBeNull();
  expect(geometry.primary).not.toBeNull();
  expect(geometry.next).not.toBeNull();
  expect(geometry.previousFill?.attribute).toBe('currentColor');
  expect(geometry.nextFill?.attribute).toBe('currentColor');
  expect(geometry.previousFill?.computed).not.toBe('none');
  expect(geometry.nextFill?.computed).not.toBe('none');
  if (geometry.previous && geometry.primary && geometry.next) {
    expect(geometry.previous.width).toBe(34);
    expect(geometry.previous.height).toBe(34);
    expect(geometry.primary.width).toBe(42);
    expect(geometry.primary.height).toBe(42);
    expect(geometry.next.width).toBe(34);
    expect(geometry.next.height).toBe(34);
    expect(geometry.previous.centerY).toBeCloseTo(geometry.primary.centerY, 0);
    expect(geometry.next.centerY).toBeCloseTo(geometry.primary.centerY, 0);
  }
});
