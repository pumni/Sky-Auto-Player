import releaseMetadata from './product-release.json';

export interface ProductReleaseMetadata {
  version: string;
  tag: string;
}

const STABLE_SEMVER =
  /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

export function parseProductReleaseMetadata(value: unknown): ProductReleaseMetadata {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Product release metadata must be an object with version and tag fields');
  }

  const metadata = value as Record<string, unknown>;
  const keys = Object.keys(metadata).sort();
  if (keys.length !== 2 || keys[0] !== 'tag' || keys[1] !== 'version') {
    throw new Error('Product release metadata must contain exactly version and tag fields');
  }

  const { version, tag } = metadata;
  if (typeof version !== 'string' || !STABLE_SEMVER.test(version)) {
    throw new Error('Product release version must be a stable SemVer version without a prerelease');
  }
  if (typeof tag !== 'string' || tag !== `v${version}`) {
    throw new Error('Product release tag must be the matching stable v-prefixed version tag');
  }

  return Object.freeze({ version, tag });
}

export const PRODUCT_RELEASE = parseProductReleaseMetadata(releaseMetadata);
