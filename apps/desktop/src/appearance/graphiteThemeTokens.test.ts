import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { PALETTES } from '../components/settings/SettingsCategoryContent';

const globalsCss = readFileSync(resolve(__dirname, '../styles/globals.css'), 'utf8');
const startupCss = readFileSync(resolve(__dirname, '../../public/startup.css'), 'utf8');
const tailwindTheme = block(/@theme inline\s*\{([\s\S]*?)\r?\n\}/);

function block(selector: RegExp, source = globalsCss): Record<string, string> {
  const declarations = source.match(selector)?.[1];
  if (!declarations) throw new Error(`Missing theme block for ${selector}`);
  return Object.fromEntries(
    [...declarations.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].flatMap((match) =>
      match[1] && match[2] ? [[match[1], match[2].trim()]] : [],
    ),
  );
}

function hexRgb(color: string): [number, number, number] {
  if (!/^#(?:[\da-f]{3}|[\da-f]{6})$/i.test(color)) {
    throw new Error(`Expected a three- or six-digit hex color, received ${color}`);
  }
  const normalized =
    color.length === 4
      ? [...color.slice(1)].map((digit) => `${digit}${digit}`).join('')
      : color.slice(1);
  return [0, 2, 4].map((start) => Number.parseInt(normalized.slice(start, start + 2), 16)) as [
    number,
    number,
    number,
  ];
}

function luminance(color: string): number {
  return hexRgb(color)
    .map((channel) => channel / 255)
    .map((value) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4))
    .reduce((total, value, index) => total + value * ([0.2126, 0.7152, 0.0722][index] ?? 0), 0);
}

function contrast(first: string, second: string): number {
  const left = luminance(first);
  const right = luminance(second);
  return (Math.max(left, right) + 0.05) / (Math.min(left, right) + 0.05);
}

describe('Sky Graphite 2 semantic theme', () => {
  const graphite = block(/:root\s*\{([\s\S]*?)\r?\n\}/);
  const light = block(/html\[data-color-mode=['"]light['"]\]\s*\{([\s\S]*?)\r?\n\}/);
  const startupDark = block(/:root\s*\{([\s\S]*?)\r?\n\}/, startupCss);
  const startupLight = block(
    /@media\s*\(prefers-color-scheme:\s*light\)\s*\{\s*:root\s*\{([\s\S]*?)\r?\n\s*\}\s*\}/,
    startupCss,
  );
  const startupExplicitLight = block(
    /html\[data-color-mode=['"]light['"]\]\s*\{([\s\S]*?)\r?\n\}/,
    startupCss,
  );

  it('keeps pre-React first-paint roles synchronized with the runtime themes', () => {
    for (const [runtime, startup] of [
      [graphite, startupDark],
      [light, startupLight],
      [light, startupExplicitLight],
    ] as const) {
      expect(startup['--startup-background']).toBe(runtime['--background']);
      expect(startup['--startup-foreground']).toBe(runtime['--foreground']);
      expect(startup['--startup-secondary']).toBe(runtime['--text-secondary']);
      expect(startup['--startup-accent']).toBe(runtime['--accent-ink']);
    }
  });

  it('uses the approved Graphite 2 indigo actions and preserves the light pairing', () => {
    expect(graphite['--primary']).toBe('#8e9dff');
    expect(graphite['--primary-hover']).toBe('#a3b0ff');
    expect(graphite['--primary-pressed']).toBe('#7888f0');
    expect(graphite['--primary-foreground']).toBe('#11131a');
    expect(light['--primary']).toBe('#3c54cf');
    expect(light['--primary-hover']).toBe('#3248ba');
    expect(light['--primary-pressed']).toBe('#293da3');
    expect(light['--primary-foreground']).toBe('#fff');
  });

  it('maps hover, pressed, destructive, and scrim tokens into Tailwind utilities', () => {
    for (const [utility, token] of [
      ['--color-primary-hover', '--primary-hover'],
      ['--color-primary-pressed', '--primary-pressed'],
      ['--color-destructive-hover', '--destructive-hover'],
      ['--color-destructive-pressed', '--destructive-pressed'],
      ['--color-control-active', '--control-active'],
      ['--color-scrollbar-thumb', '--scrollbar-thumb'],
      ['--color-scrollbar-thumb-active', '--scrollbar-thumb-active'],
      ['--color-scrim', '--scrim'],
      ['--color-overlay-border', '--overlay-border'],
    ] as const) {
      expect(tailwindTheme[utility], utility).toBe(`var(${token})`);
    }
  });

  it('keeps text and primary action pairs above WCAG AA contrast', () => {
    for (const [mode, theme, surfaces] of [
      ['dark', graphite, ['--background', '--surface', '--card']],
      ['light', light, ['--background', '--surface', '--card']],
    ] as const) {
      for (const textRole of [
        '--foreground',
        '--text-secondary',
        '--text-tertiary',
        '--accent-ink',
      ]) {
        const text = theme[textRole];
        if (!text) throw new Error(`${mode} theme is missing ${textRole}`);
        for (const surfaceRole of surfaces) {
          const surface = theme[surfaceRole];
          if (!surface) throw new Error(`${mode} theme is missing ${surfaceRole}`);
          expect(
            contrast(text, surface),
            `${mode} ${textRole} on ${surfaceRole}`,
          ).toBeGreaterThanOrEqual(4.5);
        }
      }
      for (const fillRole of ['--primary', '--primary-hover', '--primary-pressed']) {
        const fill = theme[fillRole];
        const ink = theme['--primary-foreground'];
        if (!fill || !ink) throw new Error(`${mode} theme is missing its primary pair`);
        expect(
          contrast(ink, fill),
          `${mode} primary foreground on ${fillRole}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(graphite['--selection']).not.toBe(graphite['--playing']);
    expect(light['--selection']).not.toBe(light['--playing']);
  });

  it('keeps all six dark palettes color-only and matches Settings preview swatches', () => {
    const paletteRules = [
      ...globalsCss.matchAll(/html\[data-palette=['"]([\w-]+)['"]\]\s*\{([\s\S]*?)\r?\n\}/g),
    ];
    expect(paletteRules).toHaveLength(5);
    const themes = new Map<string, Record<string, string>>([['graphite', graphite]]);
    for (const rule of paletteRules) {
      const id = rule[1];
      const declarations = rule[2];
      if (!id || !declarations) throw new Error('Palette rule is incomplete');
      const tokens = Object.fromEntries(
        [...declarations.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].flatMap((match) =>
          match[1] && match[2] ? [[match[1], match[2].trim()]] : [],
        ),
      );
      expect(declarations).not.toMatch(
        /(?:min|max)?(?:width|height)|gap|padding|margin|radius|font|transition/i,
      );
      for (const [name, value] of Object.entries(tokens)) {
        if (name === '--playing') {
          expect(value).toMatch(/^color-mix\(in srgb, #[\da-f]{6} \d+%, #[\da-f]{6}\)$/i);
        } else {
          expect(value).toMatch(/^#[\da-f]{3}(?:[\da-f]{3})?$/i);
        }
      }
      themes.set(id, tokens);
    }

    for (const palette of PALETTES) {
      const theme = themes.get(palette.id);
      if (!theme) throw new Error(`Missing ${palette.id} palette tokens`);
      expect(palette.swatches).toEqual([
        theme['--background'],
        theme['--card'],
        theme['--popover'],
        theme['--primary'],
      ]);
      expect(contrast(theme['--primary-foreground']!, theme['--primary']!)).toBeGreaterThanOrEqual(
        4.5,
      );
    }
  });
});
