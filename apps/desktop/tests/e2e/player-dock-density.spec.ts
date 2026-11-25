import { expect, test } from '@playwright/test';
import { expectPlayerBarGeometry, readPlayerBarGeometry } from './testHelpers';

for (const viewport of [
  { width: 1649, height: 927 },
  { width: 1005, height: 615 },
  { width: 987, height: 691 },
  { width: 800, height: 560 },
  { width: 640, height: 448 },
]) {
  test(`player dock and compact pager fit ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await page.goto('/?mockCatalogCount=112');

    const dock = page.getByRole('contentinfo', { name: 'Player controls' });
    const profileTrigger = dock.getByRole('button', { name: 'Configure playback profile' });
    await expect(dock).toBeVisible();
    await expect(profileTrigger).toBeVisible();
    const autoPlay = dock.getByRole('button', { name: 'Auto Play' });
    await expect(autoPlay).toBeVisible();
    await expect(autoPlay).toBeEnabled();
    await expect(autoPlay).toHaveAccessibleName('Auto Play');
    await autoPlay.hover();
    await expect(page.getByRole('tooltip')).toContainText('Auto Play on');
    await page.mouse.move(0, 0);
    await autoPlay.focus();
    await expect(page.getByRole('tooltip')).toContainText('Auto Play on');
    await autoPlay.click();
    await expect(autoPlay).toHaveAttribute('aria-pressed', 'false');
    await page.mouse.move(0, 0);
    await autoPlay.hover();
    await expect(page.getByRole('tooltip')).toContainText('Auto Play off');
    await page.mouse.move(0, 0);
    await expect(dock.getByRole('button', { name: 'Test playback (no input)' })).toHaveCount(0);
    await expect(dock.locator('button[aria-label="Configure playback profile"]')).not.toContainText(
      'FPS',
    );
    expect(
      await dock.locator('button[aria-label="Configure playback profile"]').textContent(),
    ).toBe('');
    expect(await dock.getByRole('button', { name: 'Auto Play' }).textContent()).toBe('');

    const geometry = await page.evaluate(() => {
      const required = (selector: string) => {
        const element = document.querySelector<HTMLElement>(selector);
        if (!element) throw new Error(`Missing required element: ${selector}`);
        return element;
      };
      const rect = (element: Element) => element.getBoundingClientRect();
      const tools = required('[data-slot="player-tools"]');
      const dock = rect(required('[data-slot="player-dock"]'));
      const browser = rect(required('[data-slot="track-browser"]'));
      const pager = rect(required('[data-slot="builtin-pager"]'));
      return {
        pagerHeight: pager.height,
        progressHeight: rect(required('[data-slot="player-timeline"] progress')).height,
        bottomGap: browser.bottom - pager.bottom,
        documentWidth: document.documentElement.scrollWidth,
        dock: { left: dock.left, right: dock.right, top: dock.top, bottom: dock.bottom },
        tools: Array.from(tools.querySelectorAll('button')).map((button) => {
          const bounds = rect(button);
          return {
            label: button.getAttribute('aria-label') ?? '',
            left: bounds.left,
            right: bounds.right,
            top: bounds.top,
            bottom: bounds.bottom,
          };
        }),
      };
    });
    expect(geometry.pagerHeight).toBeLessThanOrEqual(38);
    expect(geometry.progressHeight).toBe(4);
    expect(Math.abs(geometry.bottomGap)).toBeLessThanOrEqual(1);
    expect(geometry.documentWidth).toBeLessThanOrEqual(viewport.width);
    expect(geometry.tools).toHaveLength(3);
    expect(geometry.tools.map((tool) => tool.label)).not.toContain('Locate selected');
    await expect(page.locator('[data-slot="builtin-pager-locate"]')).toBeVisible();
    for (const tool of geometry.tools) {
      expect(tool.left, tool.label).toBeGreaterThanOrEqual(geometry.dock.left);
      expect(tool.right, tool.label).toBeLessThanOrEqual(geometry.dock.right);
      expect(tool.top, tool.label).toBeGreaterThanOrEqual(geometry.dock.top);
      expect(tool.bottom, tool.label).toBeLessThanOrEqual(geometry.dock.bottom);
    }

    await profileTrigger.click();
    const profile = page.getByRole('dialog', { name: 'Playback profile' });
    await expect(profile.getByRole('heading', { name: 'Playback profile' })).toBeVisible();
    await expect(profile.getByRole('button', { name: 'Test playback (no input)' })).toBeVisible();
    const advancedTiming = profile.getByText('Advanced timing');
    await expect(advancedTiming).toBeVisible();
    await expect(profile.getByLabel('FPS')).toBeHidden();
    await expect(
      profile.getByText(
        'Select a song to enable a dry-run preview. No keyboard input will be sent.',
      ),
    ).toBeVisible();
    const basicControls = await profile
      .locator('[data-slot="profile-fields"] > div > [data-slot="select-trigger"]')
      .evaluateAll((selects) =>
        selects.map((select) => {
          const box = select.getBoundingClientRect();
          return { x: box.x, y: box.y, right: box.right, bottom: box.bottom, width: box.width };
        }),
      );
    expect(basicControls).toHaveLength(2);
    expect(
      Math.max(...basicControls.map((box) => box.y)) -
        Math.min(...basicControls.map((box) => box.y)),
    ).toBeLessThanOrEqual(1);
    for (const control of basicControls) expect(control.width).toBeGreaterThanOrEqual(60);
    await advancedTiming.click();
    await expect(profile.getByLabel('FPS')).toBeVisible();
    await expect(profile.getByText('Effective target hold')).toBeVisible();
    const fpsBounds = await profile.getByLabel('FPS').boundingBox();
    const timingMarginControlBounds = await profile
      .locator('[data-slot="timing-margin-control"] > div:nth-child(2)')
      .boundingBox();
    expect(fpsBounds).not.toBeNull();
    expect(timingMarginControlBounds).not.toBeNull();
    if (fpsBounds && timingMarginControlBounds) {
      expect(Math.abs(fpsBounds.y - timingMarginControlBounds.y)).toBeLessThanOrEqual(1);
      expect(Math.abs(fpsBounds.height - timingMarginControlBounds.height)).toBeLessThanOrEqual(1);
    }
    const marginBounds = await profile.locator('[data-slot="timing-margin-control"]').boundingBox();
    expect(marginBounds).not.toBeNull();
    const advancedBounds = await profile.locator('details[open]').boundingBox();
    expect(advancedBounds).not.toBeNull();
    expect(marginBounds!.x).toBeGreaterThanOrEqual(advancedBounds!.x);
    expect(marginBounds!.x + marginBounds!.width).toBeLessThanOrEqual(
      advancedBounds!.x + advancedBounds!.width,
    );
    expect(marginBounds!.y).toBeGreaterThanOrEqual(advancedBounds!.y);
    expect(marginBounds!.y + marginBounds!.height).toBeLessThanOrEqual(
      advancedBounds!.y + advancedBounds!.height,
    );
    if (viewport.width <= 640) {
      const profileViewport = profile.locator('[data-slot="scroll-area-viewport"]');
      await profileViewport.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
        element.dispatchEvent(new Event('scroll'));
      });
      await expect
        .poll(() => profileViewport.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(0);
      await expect(
        profile.getByRole('button', { name: 'Test playback (no input)' }),
      ).toBeInViewport();
      await profileViewport.evaluate((element) => {
        element.scrollTop = 0;
        element.dispatchEvent(new Event('scroll'));
      });
    }
    const popoverBounds = await page
      .locator('[role="dialog"][aria-label="Playback profile"]')
      .boundingBox();
    expect(popoverBounds).not.toBeNull();
    if (popoverBounds) {
      expect(popoverBounds.x).toBeGreaterThanOrEqual(0);
      expect(popoverBounds.x + popoverBounds.width).toBeLessThanOrEqual(viewport.width);
      expect(popoverBounds.y).toBeGreaterThanOrEqual(0);
      expect(popoverBounds.y + popoverBounds.height).toBeLessThanOrEqual(viewport.height);
    }

    const tempoSelect = profile.getByRole('combobox', { name: 'Tempo' });
    await tempoSelect.focus();
    await page.keyboard.press('ArrowDown');
    await expect(page.getByRole('listbox')).toBeVisible();
    await page.keyboard.press('Tab');
    await expect(page.getByRole('listbox')).toBeHidden();
  });
}

test('compact dock preserves title, active status, test mode, progress, and centered transport', async ({
  page,
}) => {
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/?mockPlaybackDurationMs=120000&mockStartDelayMs=10');
    const dock = page.getByRole('contentinfo', { name: 'Player controls' });
    await page.getByRole('row', { name: /Blue Bird/ }).click();
    await dock.getByRole('button', { name: 'Configure playback profile' }).click();
    const profile = page.getByRole('dialog', { name: 'Playback profile' });
    await profile.getByRole('button', { name: 'Test playback (no input)' }).click();
    await expect(dock.locator('[data-slot="player-track-status"]')).toHaveText('Playing');

    const measurements = await page.evaluate(() => {
      const dock = document.querySelector<HTMLElement>('[data-slot="player-dock"]');
      const info = dock?.querySelector<HTMLElement>('[role="group"][aria-label^="Playback for"]');
      const title = info?.querySelector<HTMLElement>('strong');
      const status = info?.querySelector<HTMLElement>('[data-slot="player-track-status"]');
      const badge = info?.querySelector<HTMLElement>('[data-slot="status-badge"]');
      const noteIcon = info?.querySelector<HTMLElement>('span[aria-hidden="true"]');
      const progress = dock?.querySelector<HTMLElement>('[data-slot="player-timeline"] progress');
      const bounds = (element: HTMLElement | null | undefined) => {
        if (!element) return null;
        const rect = element.getBoundingClientRect();
        return { x: rect.x, right: rect.right, width: rect.width, height: rect.height };
      };
      return {
        viewportWidth: innerWidth,
        dock: bounds(dock),
        title: title ? { ...bounds(title)!, clipped: title.scrollWidth > title.clientWidth } : null,
        status: status
          ? {
              ...bounds(status)!,
              clipped: status.scrollWidth > status.clientWidth,
              text: status.textContent?.trim() ?? '',
            }
          : null,
        badge: badge
          ? {
              ...bounds(badge)!,
              text: [...badge.childNodes]
                .map((node) => {
                  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? '';
                  if (node instanceof HTMLElement && getComputedStyle(node).display !== 'none') {
                    return node.textContent ?? '';
                  }
                  return '';
                })
                .join('')
                .trim(),
              accessibleName: badge.getAttribute('aria-label'),
            }
          : null,
        decorativeNoteIcon: noteIcon ? getComputedStyle(noteIcon).display !== 'none' : false,
        dockBackground: dock ? getComputedStyle(dock).backgroundColor : null,
        progressTrack: progress
          ? getComputedStyle(progress, '::-webkit-progress-bar').backgroundColor
          : null,
        documentWidth: document.documentElement.scrollWidth,
      };
    });
    expect(measurements.dock?.height).toBe(92);
    expect(measurements.title?.clipped).toBe(false);
    expect(measurements.status?.clipped).toBe(false);
    expect(measurements.status?.text).toBe('Playing');
    expect(measurements.badge?.text).toBe('Test mode');
    expect(measurements.badge?.accessibleName).toBe('Test mode: no keyboard input is sent.');
    expect(measurements.decorativeNoteIcon).toBe(false);
    expect(measurements.progressTrack).not.toBe(measurements.dockBackground);
    expect(measurements.documentWidth).toBeLessThanOrEqual(viewport.width);
    if (measurements.status && measurements.badge) {
      expect(measurements.status.right).toBeLessThanOrEqual(measurements.badge.x + 1);
    }

    const geometry = await readPlayerBarGeometry(page);
    expectPlayerBarGeometry(geometry);
    await page.mouse.move(0, 0);
  }
});

test('compact dock keeps a prepared risk label and confirmation state visible at 800 and 640', async ({
  page,
}) => {
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/?mockPlaybackDurationMs=120000');
    const dock = page.getByRole('contentinfo', { name: 'Player controls' });
    await page.getByRole('row', { name: /Aurora Landing/ }).click();
    await dock.getByRole('button', { name: 'Play', exact: true }).click();
    await expect(page.getByRole('group', { name: 'Playback confirmation' })).toBeVisible();
    const info = dock.getByRole('group', { name: 'Playback for Aurora Landing' });
    const title = info.locator('strong');
    const status = info.locator('[data-slot="player-track-status"]');
    const risk = info.locator('[data-slot="status-badge"]');
    const like = info.locator('button[aria-pressed]');
    await expect(title).toHaveText('Aurora Landing');
    await expect(status).toBeVisible();
    await expect(risk).toHaveText('Medium risk');
    await expect(like).toBeVisible();

    const state = await page.evaluate(() => {
      const dock = document.querySelector<HTMLElement>('[data-slot="player-dock"]');
      const info = dock?.querySelector<HTMLElement>('[role="group"][aria-label^="Playback for"]');
      const title = info?.querySelector<HTMLElement>('strong');
      const status = info?.querySelector<HTMLElement>('[data-slot="player-track-status"]');
      const risk = info?.querySelector<HTMLElement>('[data-slot="status-badge"]');
      const like = info?.querySelector<HTMLElement>('button[aria-pressed]');
      const rect = (element: HTMLElement | null | undefined) => {
        if (!element) return null;
        const box = element.getBoundingClientRect();
        return { x: box.x, right: box.right, y: box.y, bottom: box.bottom, height: box.height };
      };
      return {
        dock: rect(dock),
        info: rect(info),
        title: rect(title),
        titleClipped: title ? title.scrollWidth > title.clientWidth : true,
        status: rect(status),
        statusClipped: status ? status.scrollWidth > status.clientWidth : true,
        risk: rect(risk),
        riskClipped: risk ? risk.scrollWidth > risk.clientWidth : true,
        like: rect(like),
        width: document.documentElement.scrollWidth,
      };
    });
    expect(state.dock?.bottom).toBe(viewport.height);
    expect(state.titleClipped, JSON.stringify(state)).toBe(false);
    expect(state.statusClipped, JSON.stringify(state)).toBe(false);
    expect(state.riskClipped, JSON.stringify(state)).toBe(false);
    expect(state.width).toBeLessThanOrEqual(viewport.width);
    for (const element of [state.info, state.title, state.status, state.risk, state.like]) {
      expect(element).not.toBeNull();
      if (element && state.dock) {
        expect(element.x).toBeGreaterThanOrEqual(state.dock.x);
        expect(element.right).toBeLessThanOrEqual(state.dock.right);
        expect(element.y).toBeGreaterThanOrEqual(state.dock.y);
        expect(element.bottom).toBeLessThanOrEqual(state.dock.bottom);
      }
    }
    expectPlayerBarGeometry(await readPlayerBarGeometry(page));
    await page.getByRole('button', { name: 'Not now' }).click();
    await expect(page.getByRole('group', { name: 'Playback confirmation' })).toHaveCount(0);
  }
});

test('compact dock bounds a long prepared title with fallback fonts at 800 and 640', async ({
  page,
}) => {
  const longTitle = 'A sheet with an intentionally long title for layout verification';
  for (const viewport of [
    { width: 800, height: 560 },
    { width: 640, height: 448 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/?mockPlaybackDurationMs=120000');
    await page.addStyleTag({
      content: 'html, body, body * { font-family: sans-serif !important; }',
    });
    const search = page.getByRole('searchbox', { name: 'Search Built-in Songs' });
    await search.fill(longTitle);
    const row = page.getByRole('row', { name: /A sheet with an intentionally long title/ });
    await expect(row).toBeVisible();
    await row.click();

    const dock = page.getByRole('contentinfo', { name: 'Player controls' });
    await dock.getByRole('button', { name: 'Play', exact: true }).click();
    await expect(page.getByRole('group', { name: 'Playback confirmation' })).toBeVisible();
    const info = dock.getByRole('group', { name: `Playback for ${longTitle}` });
    const title = info.locator('strong');
    const status = info.locator('[data-slot="player-track-status"]');
    const risk = info.locator('[data-slot="status-badge"]');
    const like = info.locator('button[aria-pressed]');
    await expect(title).toHaveText(longTitle);
    await expect(title).toHaveAttribute('title', longTitle);
    await expect(status).toHaveText('Prepared');
    await expect(risk).toHaveText('Medium risk');
    await expect(like).toBeVisible();

    const state = await page.evaluate(() => {
      const dock = document.querySelector<HTMLElement>('[data-slot="player-dock"]');
      const info = dock?.querySelector<HTMLElement>('[role="group"][aria-label^="Playback for"]');
      const title = info?.querySelector<HTMLElement>('strong');
      const status = info?.querySelector<HTMLElement>('[data-slot="player-track-status"]');
      const risk = info?.querySelector<HTMLElement>('[data-slot="status-badge"]');
      const like = info?.querySelector<HTMLElement>('button[aria-pressed]');
      const rect = (element: HTMLElement | null | undefined) => {
        if (!element) return null;
        const box = element.getBoundingClientRect();
        return { x: box.x, right: box.right, y: box.y, bottom: box.bottom, height: box.height };
      };
      return {
        dock: rect(dock),
        info: rect(info),
        infoName: info?.getAttribute('aria-label'),
        title: rect(title),
        titleText: title?.textContent?.trim(),
        titleAttribute: title?.getAttribute('title'),
        titleClipped: title ? title.scrollWidth > title.clientWidth : false,
        titleWhiteSpace: title ? getComputedStyle(title).whiteSpace : null,
        titleOverflow: title ? getComputedStyle(title).overflow : null,
        titleTextOverflow: title ? getComputedStyle(title).textOverflow : null,
        status: rect(status),
        statusClipped: status ? status.scrollWidth > status.clientWidth : true,
        risk: rect(risk),
        riskClipped: risk ? risk.scrollWidth > risk.clientWidth : true,
        like: rect(like),
        likeName: like?.getAttribute('aria-label'),
        width: document.documentElement.scrollWidth,
      };
    });
    expect(state.dock?.height, JSON.stringify(state)).toBe(92);
    expect(state.dock?.bottom, JSON.stringify(state)).toBe(viewport.height);
    expect(state.infoName, JSON.stringify(state)).toBe(`Playback for ${longTitle}`);
    expect(state.titleText, JSON.stringify(state)).toBe(longTitle);
    expect(state.titleAttribute, JSON.stringify(state)).toBe(longTitle);
    expect(state.titleClipped, JSON.stringify(state)).toBe(true);
    expect(state.titleWhiteSpace, JSON.stringify(state)).toBe('nowrap');
    expect(state.titleOverflow, JSON.stringify(state)).toBe('hidden');
    expect(state.titleTextOverflow, JSON.stringify(state)).toBe('ellipsis');
    expect(state.statusClipped, JSON.stringify(state)).toBe(false);
    expect(state.riskClipped, JSON.stringify(state)).toBe(false);
    expect(state.likeName, JSON.stringify(state)).toMatch(/Liked Songs/);
    expect(state.width, JSON.stringify(state)).toBeLessThanOrEqual(viewport.width);
    for (const element of [state.info, state.title, state.status, state.risk, state.like]) {
      expect(element, JSON.stringify(state)).not.toBeNull();
      if (element && state.dock) {
        expect(element.x, JSON.stringify(state)).toBeGreaterThanOrEqual(state.dock.x);
        expect(element.right, JSON.stringify(state)).toBeLessThanOrEqual(state.dock.right);
        expect(element.y, JSON.stringify(state)).toBeGreaterThanOrEqual(state.dock.y);
        expect(element.bottom, JSON.stringify(state)).toBeLessThanOrEqual(state.dock.bottom);
      }
    }
    expectPlayerBarGeometry(await readPlayerBarGeometry(page));
    await page.getByRole('button', { name: 'Not now' }).click();
  }
});
