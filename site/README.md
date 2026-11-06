# Sky Auto Player Pages

Static Astro site for the Sky Auto Player project, published under the /Sky-Auto-Player/ GitHub Pages base path.

## Local development

Run from this directory:

    bun install
    bun run dev

## Quality gates

    bun run check
    bun run lint
    bun run format:check
    bun run test:unit
    bun run build
    bun run verify:dist
    bun run verify:seo
    bun run test:e2e

The site is static output. The production build is written to dist/, and the E2E suite checks routing, canonical metadata, accessibility, responsive behavior, the project favicon, and the mobile screenshot crop.

## Published application version

`src/data/product-release.json` records the latest publicly published stable
release used by the site's `SoftwareApplication` JSON-LD and guide download
copy. Its version must be a stable SemVer value and its `v`-prefixed tag must
match; `src/data/product-release.ts` validates both when the site is built.
This site-owned snapshot is independent of the desktop development version in
Cargo and requires no network access during the Astro build.

After a stable release has been publicly published and verified, update the
version and tag in `src/data/product-release.json` together. Do not update this
snapshot from an unreleased desktop version or a prerelease. Run
`bun run test:unit`, `bun run build`, `bun run verify:dist`, and
`bun run verify:seo` before submitting the metadata refresh.
