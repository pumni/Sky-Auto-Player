import { expect, test } from '@playwright/test';

test('status badges inherit semantic tokens across dark palettes and Light', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => {
    const host = document.createElement('div');
    host.id = 'status-badge-theme-probe';
    for (const tone of ['neutral', 'warning', 'danger']) {
      const badge = document.createElement('span');
      badge.className = 'status-badge';
      badge.dataset.tone = tone;
      badge.dataset.appearance = 'soft';
      badge.textContent = tone;
      host.append(badge);
    }
    const plain = document.createElement('span');
    plain.className = 'status-badge';
    plain.dataset.tone = 'neutral';
    plain.dataset.appearance = 'plain';
    plain.textContent = 'Test · no keys';
    host.append(plain);
    document.body.append(host);
  });

  for (const [mode, palette] of [
    ['dark', 'graphite'],
    ['dark', 'aurora'],
    ['dark', 'cyberpunk'],
    ['light', 'graphite'],
  ] as const) {
    await page.evaluate(
      ({ mode, palette }) => {
        document.documentElement.dataset.colorMode = mode;
        document.documentElement.dataset.palette = palette;
      },
      { mode, palette },
    );

    const styles = await page.evaluate(() => {
      const host = document.getElementById('status-badge-theme-probe');
      if (!host) throw new Error('Badge theme probe is missing');
      const neutral = host.querySelector<HTMLElement>(
        '[data-tone="neutral"][data-appearance="soft"]',
      );
      const warning = host.querySelector<HTMLElement>('[data-tone="warning"]');
      const danger = host.querySelector<HTMLElement>('[data-tone="danger"]');
      const plain = host.querySelector<HTMLElement>('[data-appearance="plain"]');
      if (!neutral || !warning || !danger || !plain) {
        throw new Error('Badge theme variants are missing');
      }
      const sample = document.createElement('span');
      sample.style.color = 'var(--text-secondary)';
      sample.style.background = 'var(--surface-raised)';
      sample.style.border = '1px solid var(--border-subtle)';
      document.body.append(sample);
      const expected = getComputedStyle(sample);
      const result = {
        text: getComputedStyle(neutral).color,
        expectedText: expected.color,
        surface: getComputedStyle(neutral).backgroundColor,
        expectedSurface: expected.backgroundColor,
        edge: getComputedStyle(neutral).borderTopColor,
        expectedEdge: expected.borderTopColor,
        warningSurface: getComputedStyle(warning).backgroundColor,
        dangerSurface: getComputedStyle(danger).backgroundColor,
        warningDot: getComputedStyle(warning, '::before').backgroundColor,
        dangerDot: getComputedStyle(danger, '::before').backgroundColor,
        plainSurface: getComputedStyle(plain).backgroundColor,
        plainBorder: getComputedStyle(plain).borderTopWidth,
      };
      sample.remove();
      return result;
    });
    expect(styles.text).toBe(styles.expectedText);
    expect(styles.surface).toBe(styles.expectedSurface);
    expect(styles.edge).toBe(styles.expectedEdge);
    expect(styles.warningSurface).toBe(styles.surface);
    expect(styles.dangerSurface).toBe(styles.surface);
    expect(styles.warningDot).not.toBe(styles.dangerDot);
    expect(styles.plainSurface).toBe('rgba(0, 0, 0, 0)');
    expect(styles.plainBorder).toBe('0px');
  }
});
