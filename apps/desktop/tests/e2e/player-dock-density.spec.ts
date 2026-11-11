import { expect, test } from '@playwright/test';

for (const viewport of [
  { width: 1649, height: 927 },
  { width: 1005, height: 615 },
  { width: 800, height: 560 },
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
    await expect(dock.getByRole('button', { name: 'Auto Play' })).toBeVisible();
    await expect(dock.locator('.player-test-button')).toHaveCount(0);
    await expect(dock.locator('.player-auto-play-label')).toHaveCount(0);
    await expect(dock.locator('.profile-summary-button')).not.toContainText('FPS');

    const geometry = await page.evaluate(() => {
      const required = (selector: string) => {
        const element = document.querySelector<HTMLElement>(selector);
        if (!element) throw new Error(`Missing required element: ${selector}`);
        return element;
      };
      const rect = (element: Element) => element.getBoundingClientRect();
      const tools = required('.player-tools');
      const dock = rect(required('.player-bar'));
      const browser = rect(required('.track-browser'));
      const pager = rect(required('.builtin-pager-wrap'));
      return {
        pagerHeight: pager.height,
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
    expect(Math.abs(geometry.bottomGap)).toBeLessThanOrEqual(1);
    expect(geometry.documentWidth).toBeLessThanOrEqual(viewport.width);
    expect(geometry.tools).toHaveLength(3);
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
    await expect(profile.getByText('Calculated timing')).toBeVisible();
    const popoverBounds = await page.locator('.profile-popover').boundingBox();
    expect(popoverBounds).not.toBeNull();
    if (popoverBounds) {
      expect(popoverBounds.x).toBeGreaterThanOrEqual(0);
      expect(popoverBounds.x + popoverBounds.width).toBeLessThanOrEqual(viewport.width);
      expect(popoverBounds.y).toBeGreaterThanOrEqual(0);
      expect(popoverBounds.y + popoverBounds.height).toBeLessThanOrEqual(viewport.height);
    }
  });
}
