import { expect, test } from 'bun:test';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homeEn } from '../../src/data/home.en.ts';
import { homeVi } from '../../src/data/home.vi.ts';

const sourcePath = fileURLToPath(new URL('../../src/', import.meta.url));

function filesBelow(directory, extensions) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return filesBelow(path, extensions);
    return extensions.test(entry.name) ? [path] : [];
  });
}

const sourceFiles = filesBelow(sourcePath, /\.(?:astro|css|js|mjs|ts)$/);
const sourceText = sourceFiles.map((path) => readFileSync(path, 'utf8')).join('\n');

test('localized home data keeps the shared navigation and content shape aligned', () => {
  expect(Object.keys(homeEn.navigation)).toEqual(Object.keys(homeVi.navigation));
  expect(Object.keys(homeEn.hero)).toEqual(Object.keys(homeVi.hero));
  expect(Object.keys(homeEn.comparison)).toEqual(Object.keys(homeVi.comparison));
  expect(homeEn.comparison.rows).toHaveLength(6);
  expect(homeVi.comparison.rows).toHaveLength(6);
  expect(homeEn.steps.items).toHaveLength(3);
  expect(homeVi.steps.items).toHaveLength(3);
});

test('public home content states the supported OS and sender-side timing boundary', () => {
  expect(homeEn.hero.kicker).toContain('Windows 11 x64');
  expect(homeVi.hero.kicker).toContain('Windows 11 64-bit');
  expect(homeEn.hero.primaryCta).toBe('Get the Windows installer');
  expect(homeEn.hero.primaryCtaDestination).toContain('GitHub Releases');
  expect(homeEn.hero.metadata).toContain('FREE · OPEN SOURCE');
  expect(homeVi.hero.primaryCta).toBe('Tải bộ cài Windows');
  expect(homeVi.hero.primaryCtaDestination).toContain('GitHub Releases');
  expect(homeVi.hero.metadata).toContain('MIỄN PHÍ · MÃ NGUỒN MỞ');
  expect(homeEn.playback.description).toContain('cannot guarantee in-game sound or frame timing');
  expect(homeVi.playback.description).toContain('không thể bảo đảm thời điểm phát âm thanh');
  expect(homeEn.hero.affiliationDisclaimer).toContain('Unofficial community project');
  expect(homeVi.hero.affiliationDisclaimer).toContain('không chính thức');
  const finalCta = readFileSync(
    new URL('../../src/components/home/FinalCta.astro', import.meta.url),
    'utf8',
  );
  expect(finalCta).toContain('WINDOWS 11 X64');
  expect(finalCta).not.toContain('WINDOWS 10–11');
});

test('website content contains no retired product instructions or unsupported guarantees', () => {
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
  ]) {
    expect(publicContent).not.toContain(retiredClaim);
  }
});

test('every CSS custom property reference resolves to a declared or inline property', () => {
  const declared = new Set(
    [...sourceText.matchAll(/(?:^|[;{\s])(--[\w-]+)\s*:/gm)].map((match) => match[1]),
  );
  for (const match of sourceText.matchAll(/setProperty\(\s*['"](--[\w-]+)['"]/g)) {
    declared.add(match[1]);
  }

  const unresolved = [...sourceText.matchAll(/var\(\s*(--[\w-]+)/g)]
    .map((match) => match[1])
    .filter((property) => !declared.has(property));
  expect([...new Set(unresolved)]).toEqual([]);
});

test('obsolete compatibility tokens are absent after caller migration', () => {
  for (const retiredToken of [
    '--color-bg',
    '--color-surface-1',
    '--color-surface-2',
    '--color-text-muted',
    '--color-text-subtle',
    '--color-accent',
    '--color-accent-strong',
    '--color-sky',
    '--color-danger',
    '--border-faint',
    '--border-default',
    '--border-accent',
    '--font-size-xs',
    '--font-size-sm',
    '--font-size-md',
    '--font-size-lg',
    '--space-12',
    '--space-16',
    '--space-9',
    '--space-11',
    '--color-action-soft',
  ]) {
    expect(sourceText).not.toContain(retiredToken);
  }
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

  const unused = [...dependencies.keys()].filter((property) => !reachable.has(property));
  expect(unused).toEqual([]);
});
test('fallback tokens and cascade order remain available before optional OKLCH overrides', () => {
  const tokens = readFileSync(new URL('../../src/styles/tokens.css', import.meta.url), 'utf8');
  expect(tokens).toMatch(
    /@layer reset, tokens, theme, base, layout, patterns, components, utilities;/,
  );
  expect(tokens.indexOf('--color-page: var(--night-0)')).toBeLessThan(tokens.indexOf('@supports'));
  expect(tokens.indexOf('--night-0: #07090d')).toBeLessThan(tokens.indexOf('@supports'));
  expect(tokens.indexOf('--color-action: var(--gold-1)')).toBeLessThan(tokens.indexOf('@supports'));
  expect(tokens).toContain('@supports (color: oklch(50% 0.1 100))');
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
