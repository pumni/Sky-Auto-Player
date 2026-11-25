# Sky Auto Player Pages

Static Astro site for the Sky Auto Player project, published under the `/Sky-Auto-Player/` GitHub Pages base path.

## Local development

Run from this directory:

    bun install --frozen-lockfile
    bun run dev

## Quality gates

    bun run format
    bun run format:check
    bun run lint
    bun run check
    bun run test:unit
    bun run build
    bun run verify:dist
    bun run verify:seo
    bun run test:functional

The production build is written to `dist/`. Functional tests cover routing, metadata, accessibility, responsive layout, the product view selector, playback behavior, and static fallbacks.

## Shared site foundation

Global styles use the declared cascade order in `src/styles/tokens.css`: reset, tokens, theme, base, layout, patterns, components, and utilities. Shared semantic color, type, spacing, and motion tokens live in that file; shared controls live in `src/styles/components.css`. Astro component styles belong to the components layer.

Non-home pages retain the shared section spacing roles. The homepage uses one 75rem container with 16–40px side gutters, a six-to-seven text-to-image opening grid on wide screens, and a 32–80px chapter rhythm. Its footer aligns to the same container only on localized home routes.

The shared header and footer are rendered by `BaseLayout` on every route. Navigation copy is typed in `src/data/home.types.ts` and localized in `src/data/home.en.ts` and `src/data/home.vi.ts`. Keep both locale shapes aligned and check product claims against their owning desktop, security, and distribution contracts.

## Homepage

`src/components/home/HomePage.astro` composes four visual chapters: the Hero and product showcase share one opening composition; one compact playback demonstration follows; three getting-started steps include a compact formats row; and one download section closes with safety and support links. The visible targets `#top`, `#product`, `#playback`, `#how-it-works`, `#formats`, `#technical`, `#timing-model`, and `#faq-preview` remain stable.

The product selector enhances direct full-size image links and commits the latest successfully loaded selection as one update. The playback demonstration and its browser behavior share the fixed three-event model in `src/data/playback-demo.ts`; it has one timeline, one elapsed display, one key grid, and user-triggered controls. Its static markup remains useful without JavaScript.

Product images are desktop interface captures documented in [the UI asset guide](../../docs/ui/README.md). They show the interface and are not claims about current release qualification.

## Published application version

`src/data/product-release.json` records the latest publicly published stable release used by the site's `SoftwareApplication` JSON-LD and guide download copy. Project policy requires the version to be exactly `MAJOR.MINOR.PATCH`, with no prerelease or build metadata, and the `v`-prefixed tag must match. SemVer permits build metadata generally; this snapshot intentionally follows the narrower published-release tag policy in `rust/xtask/src/version.rs`. `src/data/product-release.ts` validates both when the site is built. This site-owned snapshot is independent of the desktop development version in Cargo and requires no network access during the Astro build.

After a stable release has been publicly published and verified, update the version and tag in `src/data/product-release.json` together. Do not update this snapshot from an unreleased desktop version, a prerelease, or a version with build metadata. Run `bun run test:unit`, `bun run build`, `bun run verify:dist`, and `bun run verify:seo` before submitting the metadata refresh.
