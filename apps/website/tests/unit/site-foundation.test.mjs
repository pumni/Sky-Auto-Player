import { expect, test } from 'bun:test';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homeEn } from '../../src/data/home.en.ts';
import { homeVi } from '../../src/data/home.vi.ts';
import {
  PLAYBACK_DEMO_DURATION_MS,
  PLAYBACK_DEMO_EVENTS,
  PLAYBACK_DEMO_KEYS,
  formatPlaybackDuration,
  formatPlaybackTime,
} from '../../src/data/playback-demo.ts';

const sourcePath = fileURLToPath(new URL('../../src/', import.meta.url));

function filesBelow(directory, extensions) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return filesBelow(path, extensions);
    return extensions.test(entry.name) ? [path] : [];
  });
}

function valueShape(value) {
  if (Array.isArray(value)) return value.map(valueShape);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, child]) => [key, valueShape(child)]),
    );
  }
  return typeof value;
}

const sourceFiles = filesBelow(sourcePath, /.(?:astro|css|js|mjs|ts)$/);
const sourceText = sourceFiles.map((path) => readFileSync(path, 'utf8')).join('\n');

test('English and Vietnamese homepage data keep every section structurally aligned', () => {
  expect(valueShape(homeEn)).toEqual(valueShape(homeVi));
  expect(homeEn.product.views.map((view) => view.label)).toEqual([
    'Library',
    'Song details',
    'Playback settings',
  ]);
  expect(homeVi.product.views.map((view) => view.label)).toEqual([
    'Thư viện',
    'Chi tiết bài',
    'Thiết lập phát',
  ]);
  expect(homeEn.quickStart.items).toHaveLength(3);
  expect(homeVi.quickStart.items).toHaveLength(3);
  expect(homeEn.trust.supportLinks.map((link) => link.label)).toEqual([
    'Setup',
    'FAQ',
    'Troubleshooting',
  ]);
});

test('homepage composes the five approved chapters once in order', () => {
  const homePage = readFileSync(
    new URL('../../src/components/home/HomePage.astro', import.meta.url),
    'utf8',
  );
  const sectionComponents = [...homePage.matchAll(/^\s*<([A-Z]\w*)\b/gm)].map((match) => match[1]);
  expect(sectionComponents).toEqual([
    'Hero',
    'ProductShowcase',
    'PlaybackDemo',
    'QuickStart',
    'TrustDownload',
  ]);
});

test('the signature demo uses the approved event intervals, keys, and visible time format', () => {
  expect(PLAYBACK_DEMO_DURATION_MS).toBe(2600);
  expect(PLAYBACK_DEMO_EVENTS).toEqual([
    { startMs: 0, endMs: 700, kind: 'chord', keys: ['A1', 'B2', 'C3'] },
    { startMs: 700, endMs: 1400, kind: 'note', keys: ['D2'] },
    { startMs: 1400, endMs: 2600, kind: 'hold', keys: ['E3'] },
  ]);
  expect(PLAYBACK_DEMO_KEYS).toEqual([
    'A1',
    'B1',
    'C1',
    'D1',
    'E1',
    'A2',
    'B2',
    'C2',
    'D2',
    'E2',
    'A3',
    'B3',
    'C3',
    'D3',
    'E3',
  ]);
  expect(formatPlaybackTime(0)).toBe('0.0');
  expect(formatPlaybackTime(700)).toBe('0.7');
  expect(formatPlaybackTime(1400)).toBe('1.4');
  expect(formatPlaybackTime(2600)).toBe('2.6');
  expect(formatPlaybackTime(700, 'vi')).toBe('0,7');
  expect(formatPlaybackDuration(1200, 'en')).toBe('1.2 s');
  expect(formatPlaybackDuration(1200, 'vi')).toBe('1,2 giây');
});

test('the hero leads with concise product value and both locales disclose the release journey', () => {
  expect(homeEn.hero.titleEmphasis).toBe('your music');
  expect(homeVi.hero.titleEmphasis).toBe('âm nhạc');
  expect(homeEn.downloadCta).toBe('Download for Windows');
  expect(homeVi.downloadCta).toBe('Tải cho Windows');
  expect(homeEn.hero.meta).toContain('GitHub Releases');
  expect(homeVi.hero.meta).toContain('GitHub Releases');
  expect(homeEn.hero.meta).toContain('Windows 11 x64');
  expect(homeVi.hero.meta).toContain('Windows 11 64-bit');
  expect(homeEn.hero.riskNote).toMatch(/Terms of Service/);
  expect(homeVi.hero.riskNote).toMatch(/Điều khoản Dịch vụ/);
  expect(homeEn.hero.description).not.toMatch(/QPC|SendInput|foreground|packet/i);
  expect(homeVi.hero.description).not.toMatch(/QPC|SendInput|foreground|packet/i);
});

test('website copy contains no retired instructions, adaptive-lead claims, or unsupported guarantees', () => {
  const contentFiles = filesBelow(
    fileURLToPath(new URL('../../src/content/', import.meta.url)),
    /\.md$/,
  );
  const publicContent = [
    ...contentFiles,
    fileURLToPath(new URL('../../src/data/home.en.ts', import.meta.url)),
    fileURLToPath(new URL('../../src/data/home.vi.ts', import.meta.url)),
    fileURLToPath(new URL('../../src/pages/llms.txt.ts', import.meta.url)),
  ]
    .map((path) => readFileSync(path, 'utf8'))
    .join('\n');

  for (const retiredClaim of [
    'Authenticode-qualified',
    'Reload songs',
    '--tuning',
    'never clipped',
    'HUD stays smooth',
    'land on the beat',
    'keystrokes go elsewhere',
    'Library has not been reloaded',
    'Library chưa được reload',
    'Use the Library refresh control',
    'Dùng nút refresh của Library',
    'được xác minh Authenticode',
    'adaptive lead',
    'lead thích ứng',
  ]) {
    expect(publicContent).not.toContain(retiredClaim);
  }
});

test('every CSS custom property reference resolves to a declared or inline property', () => {
  const declared = new Set(
    [...sourceText.matchAll(/(?:^|[`;{\s])(--[\w-]+)\s*:/gm)].map((match) => match[1]),
  );
  for (const match of sourceText.matchAll(/setProperty\(\s*['"](--[\w-]+)['"]/g)) {
    declared.add(match[1]);
  }

  const unresolved = [...sourceText.matchAll(/var\(\s*(--[\w-]+)/g)]
    .map((match) => match[1])
    .filter((property) => !declared.has(property));
  expect([...new Set(unresolved)]).toEqual([]);
});

test('every shared foundation token is reachable from a live consumer', () => {
  const tokenPath = fileURLToPath(new URL('../../src/styles/tokens.css', import.meta.url));
  const tokenSource = readFileSync(tokenPath, 'utf8');
  const outsideTokens = sourceFiles
    .filter((path) => path !== tokenPath)
    .map((path) => readFileSync(path, 'utf8'))
    .join('\n');
  const dependencies = new Map();
  for (const match of tokenSource.matchAll(/^\s*(--[\w-]+)\s*:\s*([^;]+);/gm)) {
    const properties = dependencies.get(match[1]) || new Set();
    for (const reference of match[2].matchAll(/var\(\s*(--[\w-]+)/g)) properties.add(reference[1]);
    dependencies.set(match[1], properties);
  }

  const reachable = new Set(
    [...outsideTokens.matchAll(/var\(\s*(--[\w-]+)/g)].map((match) => match[1]),
  );
  const pending = [...reachable];
  while (pending.length > 0) {
    for (const dependency of dependencies.get(pending.pop()) || []) {
      if (reachable.has(dependency)) continue;
      reachable.add(dependency);
      pending.push(dependency);
    }
  }

  expect([...dependencies.keys()].filter((property) => !reachable.has(property))).toEqual([]);
});

test('fallback colors and the declared cascade remain available before optional OKLCH overrides', () => {
  const tokens = readFileSync(new URL('../../src/styles/tokens.css', import.meta.url), 'utf8');
  expect(tokens).toMatch(
    /@layer reset, tokens, theme, base, layout, patterns, components, utilities;/,
  );
  expect(tokens.indexOf('--color-page: var(--night-0)')).toBeLessThan(tokens.indexOf('@supports'));
  expect(tokens.indexOf('--night-0: #07090d')).toBeLessThan(tokens.indexOf('@supports'));
  expect(tokens.indexOf('--color-action: var(--gold-1)')).toBeLessThan(tokens.indexOf('@supports'));
  expect(tokens).toContain('@supports (color: oklch(50% 0.1 100))');
});

test('localized technical copy follows the current playback, install, import, and safety contracts', () => {
  const read = (relativePath) => readFileSync(new URL(relativePath, import.meta.url), 'utf8');
  const collectStrings = (value) => {
    if (typeof value === 'string') return [value];
    if (Array.isArray(value)) return value.flatMap(collectStrings);
    if (typeof value === 'object' && value !== null)
      return Object.values(value).flatMap(collectStrings);
    return [];
  };
  const enPublic = collectStrings(homeEn).join('\n');
  const viPublic = collectStrings(homeVi).join('\n');
  const enStory = collectStrings({
    hero: homeEn.hero,
    product: homeEn.product,
    playback: homeEn.playback,
    quickStart: homeEn.quickStart,
  }).join('\n');
  const viStory = collectStrings({
    hero: homeVi.hero,
    product: homeVi.product,
    playback: homeVi.playback,
    quickStart: homeVi.quickStart,
  }).join('\n');
  const enTiming = read('../../src/content/guides/en/en-timing-engine.md');
  const viTiming = read('../../src/content/guides/vi/vi-timing-engine.md');
  const enTroubleshooting = read('../../src/content/guides/en/en-troubleshooting.md');
  const viTroubleshooting = read('../../src/content/guides/vi/vi-troubleshooting.md');
  const enFormats = read('../../src/content/guides/en/en-sheet-formats.md');
  const viFormats = read('../../src/content/guides/vi/vi-sheet-formats.md');
  const enSetup = read('../../src/content/guides/en/en-windows-setup.md');
  const viSetup = read('../../src/content/guides/vi/vi-windows-setup.md');
  const enSafety = read('../../src/content/faq/en/account-safety.md');
  const viSafety = read('../../src/content/faq/vi/account-safety.md');
  const playbackContract = read('../../../../docs/playback-engine.md');

  for (const [locale, home, troubleshooting, formats, setup, safety] of [
    ['en', enPublic, enTroubleshooting, enFormats, enSetup, enSafety],
    ['vi', viPublic, viTroubleshooting, viFormats, viSetup, viSafety],
  ]) {
    expect(home, locale).not.toMatch(/adaptive lead|lead thích ứng/i);
    expect(troubleshooting, locale).not.toMatch(
      /adaptive lead|per-song settings|command palette|play hotkey|30 or 60|30 hoặc 60/i,
    );
    expect(troubleshooting, locale).toMatch(/Playback profile/i);
    expect(formats, locale).toMatch(/not\s+stored per song|không\s+được lưu riêng theo bài/i);
    expect(setup, locale).toMatch(/Playback profile/i);
    expect(setup, locale).not.toMatch(
      /command palette|play hotkey|hotkey binding|cấu hình hotkey/i,
    );
    expect(safety, locale).toContain('journal');
    expect(safety, locale).toMatch(/foreground/i);
  }

  expect(enStory).not.toMatch(/fixed wake guard|QPC|SendInput|continuously adjusting its lead/i);
  expect(viStory).not.toMatch(/wake guard cố định|QPC|SendInput|điều chỉnh lead/i);
  expect(enTiming).toMatch(/wake guard remains fixed during playback/i);
  expect(enTiming).toMatch(/does not continuously adjust/i);
  expect(viTiming).toMatch(/wake guard\s+được giữ cố định/i);
  expect(playbackContract).toMatch(/Cleanup safety Up is a separate,\s+journal-limited path/i);
  expect(playbackContract).toMatch(/H = ceiling\(hold_frames × T\) \+ timing margin/i);
  expect(enFormats).toContain('30, 60, 90, 120, 144, 165, or 240');
  expect(viFormats).toContain('30, 60, 90, 120, 144, 165 hoặc 240');
  expect(homeEn.trust.signingValue).toMatch(/unsigned installer/i);
  expect(homeVi.trust.signingValue).toMatch(/không có chữ ký Authenticode/i);
  expect(homeEn.trust.signatureNote).toContain('.exe.sig');
  expect(homeVi.trust.signatureNote).toContain('.exe.sig');
  expect(homeEn.trust.safetyNote).toMatch(/Windows SendInput/);
  expect(homeVi.trust.safetyNote).toMatch(/Windows SendInput/);
});

test('localized setup and help copy matches current distribution, import, and safety contracts', () => {
  const read = (relativePath) => readFileSync(new URL(relativePath, import.meta.url), 'utf8');
  const enSetup = read('../../src/content/guides/en/en-windows-setup.md');
  const viSetup = read('../../src/content/guides/vi/vi-windows-setup.md');
  const enTroubleshooting = read('../../src/content/guides/en/en-troubleshooting.md');
  const viTroubleshooting = read('../../src/content/guides/vi/vi-troubleshooting.md');
  const llms = read('../../src/pages/llms.txt.ts');

  expect(enSetup).toContain('not needed for manual installation');
  expect(viSetup).toContain('không cần khi cài thủ công');
  expect(viSetup).not.toContain('được xác minh Authenticode');
  expect(enSetup).toContain('use **Add songs** to import the local file or its folder');
  expect(viSetup).toContain('dùng **Add songs** để nhập file hoặc thư mục chứa file');
  expect(enSetup).not.toContain('refresh control');
  expect(viSetup).not.toContain('nút refresh');
  expect(enTroubleshooting).not.toContain('Library has not been reloaded');
  expect(viTroubleshooting).not.toContain('Library chưa được reload');
  expect(llms).toContain('Terms of Service');
  expect(llms).toContain('Unofficial community project');
});
