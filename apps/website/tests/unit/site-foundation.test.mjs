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
  expect(homeEn.playback.description).toContain('authored QPC deadlines');
  expect(homeEn.playback.description).toContain('calibrated fixed wake guard');
  expect(homeEn.playback.description).toContain('does not continuously adjust its lead');
  expect(homeVi.playback.description).toContain('deadline QPC');
  expect(homeVi.playback.description).toContain('không liên tục điều chỉnh lead');
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
    '--section-xs',
    '--section-sm',
    '--section-md',
    '--section-lg',
    '--section-xl',
    '--section-space-compact',
    '--section-space-default',
    '--section-space-feature',
    '--section-space-opening',
    '--section-space-transition',
    '--section-space-explanation',
    '--section-space-product',
    '--section-space-utility',
    '--section-space-closure',
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

test('localized public timing and safety copy matches current playback and desktop contracts', () => {
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
  const nativeOptions = read('../../../../apps/desktop/src-tauri/src/native_runtime.rs');
  const settings = read('../../../../rust/crates/sky_app_core/src/settings.rs');

  for (const [locale, home, troubleshooting, formats, setup, safety] of [
    ['en', enPublic, enTroubleshooting, enFormats, enSetup, enSafety],
    ['vi', viPublic, viTroubleshooting, viFormats, viSetup, viSafety],
  ]) {
    expect(home, locale).not.toMatch(/adaptive lead/i);
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

  expect(enTiming).toMatch(/wake guard remains fixed during playback/i);
  expect(enTiming).toMatch(/does not continuously adjust/i);
  expect(viTiming).toMatch(/wake guard\s+được giữ cố định/i);
  expect(playbackContract).toMatch(/Cleanup safety Up is a separate,\s+journal-limited path/i);
  expect(playbackContract).toMatch(/H = ceiling\(hold_frames × T\) \+ timing margin/i);

  expect(nativeOptions).toContain('hold_frames: vec![1.0, 1.25, 1.5]');
  expect(nativeOptions).toContain('tempo_scales: vec![0.90, 0.95, 1.0, 1.05, 1.10]');
  expect(settings).toContain('VALID_FPS: [u16; 7] = [30, 60, 90, 120, 144, 165, 240]');
  expect(settings).toContain('TIMING_MARGIN_STEP_US: u64 = 100');
  expect(enFormats).toContain('30, 60, 90, 120, 144, 165, or 240');
  expect(viFormats).toContain('30, 60, 90, 120, 144, 165 hoặc 240');
});

test('replaced header and pattern mechanisms have no remaining source implementation', () => {
  const read = (relativePath) => readFileSync(new URL(relativePath, import.meta.url), 'utf8');
  const header = read('../../src/components/layout/SiteHeader.astro');
  const patterns = read('../../src/styles/patterns.css');
  const controls = read('../../src/styles/components.css');
  const hero = read('../../src/components/home/Hero.astro');
  const layout = read('../../src/styles/layout.css');
  const intro = read('../../src/components/ui/SectionIntro.astro');

  for (const retiredRule of [
    '.ui-editorial-rule',
    '.ui-data-ledger',
    '.ui-instrument-panel',
    '.ui-instrument-panel--active',
    '.ui-instrument-panel--product',
    '.ui-timing-rail',
    '.ui-timing-rail__node',
    '.ui-action-link',
    '.ui-data-row',
    '.ui-data-ledger',
    '.ui-section-intro--narrow',
    '.ui-section-intro--wide',
    '.ui-section-intro--center',
  ]) {
    expect(patterns).not.toContain(retiredRule);
  }
  for (const retiredMechanism of [
    'closeTimer',
    'openFrame',
    'clearPendingMotion',
    'reducedMotionQuery',
    "data-state='opening'",
    "data-state='closing'",
    'nav.dataset.state',
  ]) {
    expect(header).not.toContain(retiredMechanism);
  }
  expect(header).toContain(".menu-toggle[aria-expanded='true']");
  expect(controls).not.toMatch(/\.button:is\(\s*:disabled/);
  expect(hero).not.toMatch(/\.hero__(?:stage-meta|rail|rail-label)/);
  expect(layout).not.toMatch(/section-shell--(?:opening|utility|open|peak|closure|proof)/);
  expect(intro).not.toContain('align?:');
  expect(intro).not.toContain('measure?:');
  expect(intro).not.toContain('wide');
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
