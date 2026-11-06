import { expect, test } from 'bun:test';
import { PRODUCT_RELEASE, parseProductReleaseMetadata } from '../../src/data/product-release.ts';

test('committed metadata parses as a stable release with a matching tag', () => {
  const parsed = parseProductReleaseMetadata(PRODUCT_RELEASE);
  expect(parsed.tag).toBe(`v${parsed.version}`);
});

test('rejects missing, empty, and non-object metadata', () => {
  for (const value of [null, {}, { version: '', tag: 'v' }]) {
    expect(() => parseProductReleaseMetadata(value)).toThrow();
  }
});

test('rejects invalid versions, prereleases, and build metadata', () => {
  for (const version of [
    '4.3',
    '04.3.0',
    '4.3.0-preview.1',
    '4.3.0-',
    '4.3.0+local',
    '4.3.0+build.1',
  ]) {
    expect(() => parseProductReleaseMetadata({ version, tag: `v${version}` })).toThrow();
  }
});

test('rejects prerelease and mismatched release tags', () => {
  for (const tag of ['v4.3.0-rc.1', '4.3.0', 'v4.3.1']) {
    expect(() => parseProductReleaseMetadata({ version: '4.3.0', tag })).toThrow();
  }
});

test('rejects unknown metadata fields', () => {
  expect(() =>
    parseProductReleaseMetadata({ version: '4.3.0', tag: 'v4.3.0', source: 'desktop' }),
  ).toThrow();
});
