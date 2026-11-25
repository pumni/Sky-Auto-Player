# Sky Auto Player Pages

Static Astro site for the Sky Auto Player project, published under the /Sky-Auto-Player/ GitHub Pages base path.

## Local development

Run from this directory:

    bun install --frozen-lockfile
    bun run dev

## Quality gates

    bun run check
    bun run lint
    bun run format:check
    bun run test:unit
    bun run build
    bun run verify:dist
    bun run verify:seo
    bun run test:functional

The site is static output. The production build is written to dist/, and the E2E suite checks routing, canonical metadata, accessibility, responsive behavior, the project favicon, and the mobile screenshot crop.

## Shared site foundation

Global styles use the declared cascade order in src/styles/tokens.css: reset, tokens, theme, base, layout, patterns, components, and utilities. Semantic color, type, spacing, and motion tokens live in that file; shared controls live in src/styles/components.css. Astro component styles belong to the components layer.

Section spacing is per side and uses the semantic compact (32–64px), default (48–96px), feature (64–128px), and product (80–144px) ranges. The proof transition keeps a separate 40–64px role. Use the matching section-shell modifier; intro placement under the header is handled by its contextual modifier and does not replace a spacing role.

The shared header and footer are rendered by BaseLayout on every localized route. Navigation labels are typed in src/data/home.types.ts and provided in both home.en.ts and home.vi.ts; route targets use the base-aware URL helpers. Keep localized home data structurally aligned, and reconcile public product claims against the owning library, playback, and distribution contracts before publishing.

## Published application version

`src/data/product-release.json` records the latest publicly published stable
release used by the site's `SoftwareApplication` JSON-LD and guide download
copy. Project policy requires the version to be exactly `MAJOR.MINOR.PATCH`,
with no prerelease or build metadata, and the `v`-prefixed tag must match.
SemVer permits build metadata generally; this snapshot intentionally follows
the narrower published-release tag policy in `rust/xtask/src/version.rs`.
`src/data/product-release.ts` validates both when the site is built.
This site-owned snapshot is independent of the desktop development version in
Cargo and requires no network access during the Astro build.

After a stable release has been publicly published and verified, update the
version and tag in `src/data/product-release.json` together. Do not update this
snapshot from an unreleased desktop version, a prerelease, or a version with
build metadata. Run
`bun run test:unit`, `bun run build`, `bun run verify:dist`, and
`bun run verify:seo` before submitting the metadata refresh.
