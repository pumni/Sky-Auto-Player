import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { PALETTES } from '../components/settings/SettingsCategoryContent';

const themesCssPath = resolve(__dirname, '../styles/themes.css');
const themesCss = readFileSync(themesCssPath, 'utf8');

function extractBlock(selectorPattern: RegExp): Record<string, string> {
  const match = themesCss.match(selectorPattern);
  if (!match?.[1]) {
    throw new Error(`Missing CSS block for ${selectorPattern}`);
  }
  const tokens: Record<string, string> = {};
  for (const decl of match[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    const name = decl[1];
    const value = decl[2]?.trim();
    if (name && value) {
      tokens[name] = value;
    }
  }
  return tokens;
}

function resolveToken(tokens: Record<string, string>, name: string): string {
  let current = tokens[name];
  for (let depth = 0; depth < 8 && current?.startsWith('var('); depth += 1) {
    const ref = current.match(/^var\((--[\w-]+)\)$/)?.[1];
    if (!ref) break;
    current = tokens[ref];
  }
  if (!current) throw new Error(`Unresolved token: ${name}`);
  return current;
}

function parseHex(hex: string): [number, number, number] {
  const clean = hex.trim().replace(/^#/, '');
  if (!/^[0-9a-f]{6}$/i.test(clean)) {
    throw new Error(`Expected 6-digit hex color, received: ${hex}`);
  }
  return [
    Number.parseInt(clean.slice(0, 2), 16),
    Number.parseInt(clean.slice(2, 4), 16),
    Number.parseInt(clean.slice(4, 6), 16),
  ];
}

function contrastRatio(first: string, second: string): number {
  const luminance = (hex: string) =>
    parseHex(hex)
      .map((channel) => {
        const normalized = channel / 255;
        return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
      })
      .reduce((sum, value, index) => sum + value * ([0.2126, 0.7152, 0.0722][index] ?? 0), 0);
  const left = luminance(first);
  const right = luminance(second);
  return Number(((Math.max(left, right) + 0.05) / (Math.min(left, right) + 0.05)).toFixed(2));
}

function hueDegrees(hex: string): number {
  const [rChannel, gChannel, bChannel] = parseHex(hex);
  const r = rChannel / 255;
  const g = gChannel / 255;
  const b = bChannel / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  if (delta === 0) return 0;
  let hue: number;
  if (max === r) hue = ((g - b) / delta + (g < b ? 6 : 0)) * 60;
  else if (max === g) hue = ((b - r) / delta + 2) * 60;
  else hue = ((r - g) / delta + 4) * 60;
  return Math.round(hue);
}

describe('Graphite Unified Sky Indigo theme tokens', () => {
  const darkTokens = extractBlock(/:root\s*\{([\s\S]*?)\r?\n\}/);
  const lightTokens = extractBlock(/html\[data-color-mode='light'\]\s*\{([\s\S]*?)\r?\n\}/);

  it('keeps exact CSS custom property parity between Dark Graphite and Light Graphite', () => {
    const darkKeys = Object.keys(darkTokens).sort();
    const lightKeys = Object.keys(lightTokens).sort();
    expect(lightKeys).toEqual(darkKeys);
  });

  it('aligns Dark and Light Graphite accents in the same Sky Indigo hue family', () => {
    const darkHue = hueDegrees(resolveToken(darkTokens, '--accent-fill'));
    const lightHue = hueDegrees(resolveToken(lightTokens, '--accent-fill'));
    const distance = Math.min(Math.abs(darkHue - lightHue), 360 - Math.abs(darkHue - lightHue));
    expect(darkHue).toBeGreaterThanOrEqual(220);
    expect(darkHue).toBeLessThanOrEqual(242);
    expect(lightHue).toBeGreaterThanOrEqual(220);
    expect(lightHue).toBeLessThanOrEqual(242);
    expect(distance).toBeLessThanOrEqual(15);
  });

  it('meets WCAG AA contrast across Dark Graphite surfaces, primary states, and row indicators', () => {
    const surfaces = ['--canvas', '--surface-panel', '--surface-raised', '--selection-bg'].map(
      (name) => resolveToken(darkTokens, name),
    );
    for (const textToken of [
      '--text-primary',
      '--text-secondary',
      '--text-tertiary',
      '--accent-ink',
    ]) {
      const ink = resolveToken(darkTokens, textToken);
      for (const surface of surfaces) {
        expect(
          contrastRatio(ink, surface),
          `Dark ${textToken} on ${surface}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }

    const onAccent = resolveToken(darkTokens, '--on-accent');
    for (const fillToken of ['--accent-fill', '--accent-fill-hover', '--accent-fill-pressed']) {
      const fill = resolveToken(darkTokens, fillToken);
      expect(
        contrastRatio(onAccent, fill),
        `Dark --on-accent on ${fillToken}`,
      ).toBeGreaterThanOrEqual(4.5);
    }

    expect(
      contrastRatio(
        resolveToken(darkTokens, '--accent-fill'),
        resolveToken(darkTokens, '--surface-panel'),
      ),
    ).toBeGreaterThanOrEqual(6);
    expect(
      contrastRatio(
        resolveToken(darkTokens, '--selection-row-indicator'),
        resolveToken(darkTokens, '--selection-bg'),
      ),
    ).toBeGreaterThanOrEqual(3);
    expect(resolveToken(darkTokens, '--selection-bg')).not.toBe(
      resolveToken(darkTokens, '--workbench-playing-bg'),
    );
  });

  it('meets WCAG AA contrast across Light Graphite surfaces, primary states, and row indicators', () => {
    const surfaces = [
      '--canvas',
      '--surface-panel',
      '--selection-bg',
      '--workbench-playing-bg',
    ].map((name) => resolveToken(lightTokens, name));
    for (const textToken of [
      '--text-primary',
      '--text-secondary',
      '--text-tertiary',
      '--accent-ink',
    ]) {
      const ink = resolveToken(lightTokens, textToken);
      for (const surface of surfaces) {
        expect(
          contrastRatio(ink, surface),
          `Light ${textToken} on ${surface}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }

    const onAccent = resolveToken(lightTokens, '--on-accent');
    for (const fillToken of ['--accent-fill', '--accent-fill-hover', '--accent-fill-pressed']) {
      const fill = resolveToken(lightTokens, fillToken);
      expect(
        contrastRatio(onAccent, fill),
        `Light --on-accent on ${fillToken}`,
      ).toBeGreaterThanOrEqual(4.5);
    }

    expect(
      contrastRatio(
        resolveToken(lightTokens, '--selection-row-indicator'),
        resolveToken(lightTokens, '--selection-bg'),
      ),
    ).toBeGreaterThanOrEqual(3);
    expect(
      contrastRatio(
        resolveToken(lightTokens, '--workbench-playing-indicator'),
        resolveToken(lightTokens, '--workbench-playing-bg'),
      ),
    ).toBeGreaterThanOrEqual(3);
    expect(resolveToken(lightTokens, '--selection-bg')).not.toBe(
      resolveToken(lightTokens, '--workbench-playing-bg'),
    );
  });

  it('locks exact Dark Graphite and Light primary action states and foreground contrasts', () => {
    // Dark Graphite: light Indigo background with dark on-accent foreground
    const darkOnAccent = resolveToken(darkTokens, '--on-accent');
    expect(darkOnAccent).toBe('#11131a');
    expect(resolveToken(darkTokens, '--accent-fill')).toBe('#8e9dff');
    expect(resolveToken(darkTokens, '--accent-fill-hover')).toBe('#a3b0ff');
    expect(resolveToken(darkTokens, '--accent-fill-pressed')).toBe('#7888f0');
    expect(
      contrastRatio(darkOnAccent, resolveToken(darkTokens, '--accent-fill')),
    ).toBeGreaterThanOrEqual(4.5);
    expect(
      contrastRatio(darkOnAccent, resolveToken(darkTokens, '--accent-fill-hover')),
    ).toBeGreaterThanOrEqual(4.5);
    expect(
      contrastRatio(darkOnAccent, resolveToken(darkTokens, '--accent-fill-pressed')),
    ).toBeGreaterThanOrEqual(4.5);

    // Light appearance: deep Indigo background with white on-accent foreground
    const lightOnAccent = resolveToken(lightTokens, '--on-accent');
    expect(lightOnAccent).toBe('#ffffff');
    expect(resolveToken(lightTokens, '--accent-fill')).toBe('#3c54cf');
    expect(resolveToken(lightTokens, '--accent-fill-hover')).toBe('#3248ba');
    expect(resolveToken(lightTokens, '--accent-fill-pressed')).toBe('#293da3');
    expect(
      contrastRatio(lightOnAccent, resolveToken(lightTokens, '--accent-fill')),
    ).toBeGreaterThanOrEqual(4.5);
    expect(
      contrastRatio(lightOnAccent, resolveToken(lightTokens, '--accent-fill-hover')),
    ).toBeGreaterThanOrEqual(4.5);
    expect(
      contrastRatio(lightOnAccent, resolveToken(lightTokens, '--accent-fill-pressed')),
    ).toBeGreaterThanOrEqual(4.5);
  });

  it('meets WCAG AA contrast across all six Dark Palette primary fill and on-accent pairs', () => {
    const paletteIds = [
      'graphite',
      'aurora',
      'minimalist',
      'slate',
      'cyberpunk',
      'classic',
    ] as const;
    for (const id of paletteIds) {
      const tokens =
        id === 'graphite'
          ? darkTokens
          : extractBlock(new RegExp(`html\\[data-palette='${id}'\\]\\s*\\{([\\s\\S]*?)\\r?\\n\\}`));
      const fill = resolveToken(tokens, '--accent-fill');
      const onAccent = resolveToken(tokens, '--on-accent');
      const ratio = contrastRatio(onAccent, fill);
      expect(
        ratio,
        `Palette ${id} primary fill (${fill}) with on-accent (${onAccent}) contrast ratio: ${ratio}`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('guarantees palette preview swatches match canonical theme declarations across all six palettes', () => {
    for (const palette of PALETTES) {
      const tokens =
        palette.id === 'graphite'
          ? darkTokens
          : extractBlock(
              new RegExp(`html\\[data-palette='${palette.id}'\\]\\s*\\{([\\s\\S]*?)\\r?\\n\\}`),
            );
      const expectedCanvas = resolveToken(tokens, '--canvas');
      const expectedPanel = resolveToken(tokens, '--surface-panel');
      const expectedRaised = resolveToken(tokens, '--surface-raised');
      const expectedAccentFill = resolveToken(tokens, '--accent-fill');

      expect(palette.swatches[0], `${palette.id} canvas swatch`).toBe(expectedCanvas);
      expect(palette.swatches[1], `${palette.id} panel swatch`).toBe(expectedPanel);
      expect(palette.swatches[2], `${palette.id} raised surface swatch`).toBe(expectedRaised);
      expect(palette.swatches[3], `${palette.id} primary accent fill swatch`).toBe(
        expectedAccentFill,
      );
    }
  });
});
