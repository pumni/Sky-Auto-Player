import { test, expect } from '@playwright/test';

const origin = 'http://localhost:4321/Sky-Auto-Player';

test.describe('locale switch contrast (bug repro)', () => {
  for (const route of ['/', '/vi/'] as const) {
    test(`${route} active option has dark text on accent (not white)`, async ({ page }) => {
      await page.goto(origin + route);
      const active = page.locator(".locale-switch__option[aria-current='page']").first();
      await expect(active).toBeVisible();

      const color = await active.evaluate((el) => window.getComputedStyle(el).color);
      const bg = await active.evaluate((el) => window.getComputedStyle(el).backgroundColor);

      // Capture for diagnostic
      console.log(`route=${route} active.text.color=${color} active.bg.color=${bg}`);

      // Sanity: color must NOT be near-white (i.e. not --color-text #f4efe3).
      // rgb(244, 239, 227) is the white-cream token; if we see it, the bug is present.
      const whiteCream = 'rgb(244, 239, 227)';
      expect(color, 'active option must NOT use --color-text (white cream)').not.toBe(whiteCream);
    });
  }
});
