import { expect, test } from '@playwright/test';
import { palettes, selectPalette } from './testHelpers';

test('track status badges use semantic warning and danger roles across palettes and Light', async ({
  page,
}) => {
  await page.goto('/');
  const search = page.getByRole('searchbox', { name: 'Search Built-in Songs' });

  const readBadge = async (title: string) => {
    await search.fill(title);
    const row = page.getByRole('row', { name: new RegExp(title) });
    const badge = row.locator('[data-slot="status-badge"]');
    await expect(row).toBeVisible();
    await expect(badge).toBeVisible();
    return badge.evaluate((element) => {
      const probe = document.createElement('span');
      const tone = element.getAttribute('data-tone');
      const tokenName = tone === 'warning' ? 'warning' : 'danger';
      probe.style.color = `var(--${tokenName})`;
      probe.style.background = `var(--${tokenName}-surface)`;
      probe.style.border = `1px solid var(--${tokenName}-border)`;
      document.body.append(probe);
      const expectedText = getComputedStyle(probe).color;
      const expectedSurface = getComputedStyle(probe).backgroundColor;
      const expectedBorder = getComputedStyle(probe).borderTopColor;
      probe.remove();
      const style = getComputedStyle(element);
      return {
        tone,
        text: style.color,
        surface: style.backgroundColor,
        border: style.borderTopColor,
        expectedText,
        expectedSurface,
        expectedBorder,
      };
    });
  };

  for (const palette of palettes) {
    await selectPalette(page, palette);
    const warning = await readBadge('Composed multi-track loss demo');
    expect(warning.tone).toBe('warning');
    expect(warning.text).toBe(warning.expectedText);
    expect(warning.surface).toBe(warning.expectedSurface);
    expect(warning.border).toBe(warning.expectedBorder);

    const danger = await readBadge('Unsupported Composed v99 demo');
    expect(danger.tone).toBe('danger');
    expect(danger.text).toBe(danger.expectedText);
    expect(danger.surface).toBe(danger.expectedSurface);
    expect(danger.border).toBe(danger.expectedBorder);
  }

  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', 'light');
  const lightWarning = await readBadge('Composed multi-track loss demo');
  const lightDanger = await readBadge('Unsupported Composed v99 demo');
  expect(lightWarning.text).toBe(lightWarning.expectedText);
  expect(lightWarning.surface).toBe(lightWarning.expectedSurface);
  expect(lightWarning.border).toBe(lightWarning.expectedBorder);
  expect(lightDanger.text).toBe(lightDanger.expectedText);
  expect(lightDanger.surface).toBe(lightDanger.expectedSurface);
  expect(lightDanger.border).toBe(lightDanger.expectedBorder);
});
