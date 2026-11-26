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

The shared `.container` is capped at 75rem with 16–40px side gutters; tablet layouts cap it at 56rem. The homepage changes from a bounded single-column opening to a six-to-seven text-to-image grid at 64rem and keeps a 32–80px chapter rhythm. Header, page content, and footer use the same container on every route.

The shared header and footer are rendered by `BaseLayout` on every route. Navigation copy is typed in `src/data/home.types.ts` and localized in `src/data/home.en.ts` and `src/data/home.vi.ts`. Keep both locale shapes aligned and check product claims against their owning desktop, security, and distribution contracts.

## Homepage

`src/components/home/HomePage.astro` composes four visual chapters: the Hero and product showcase share one opening composition; one compact playback demonstration follows; three getting-started steps include a compact formats row; and one download section closes with safety and support links. The visible targets `#top`, `#product`, `#playback`, `#how-it-works`, `#formats`, `#technical`, `#timing-model`, and `#faq-preview` remain stable.

The product selector enhances direct full-size image links and commits the latest successfully loaded selection as one update. The last loaded view stays visible while a new preview loads. Slow requests show a localized status. Failed requests, including initial load failure, offer a retry and direct image links. Cancelling a selection clears its timer and listeners. The browser may finish the image transfer. The playback demonstration and its browser behavior share the fixed three-event model in `src/data/playback-demo.ts`; it has one timeline, one elapsed display, one key grid, and user-triggered controls. Its static markup remains useful without JavaScript.

The enhanced header applies the responsive disclosure mode after inspecting current focus so the outgoing control can hand focus to its visible replacement. The static header keeps CSS-only navigation.

The guide architecture figure uses one ordered list for the visual flow and its text equivalent, including the process boundary.

Product images are browser mock previews of the current desktop source, not native Tauri captures or gameplay evidence. They use example data and are named `desktop-*-browser-preview-1366x768.png`; the screenshot source and method are recorded below. The older `docs/ui` capture manifest names a different desktop tree and was not reused.

### Homepage screenshot provenance

| Asset                                                                         | View              | Resolution |
| ----------------------------------------------------------------------------- | ----------------- | ---------- |
| `public/assets/images/desktop-library-browser-preview-1366x768.png`           | Library           | 1366 × 768 |
| `public/assets/images/desktop-song-detail-browser-preview-1366x768.png`       | Song detail       | 1366 × 768 |
| `public/assets/images/desktop-settings-playback-browser-preview-1366x768.png` | Playback settings | 1366 × 768 |

The previews were captured from desktop source commit `711201a1392cd34a3ff191de22a58c554a640a05`, whose `apps/desktop` tree SHA is `155e9e999f4f072e81cb15fb224347c967088ff5`. The existing desktop Vite and Playwright Chromium test harness rendered the current interface with its deterministic browser mock bridge and example song data. These assets describe the rendered desktop source at that revision; they do not show a native Tauri window, a Windows release build, or live gameplay. No capture framework or dependency was added.

## Published application version

`src/data/product-release.json` records the latest publicly published stable release used by the site's `SoftwareApplication` JSON-LD and guide download copy. Project policy requires the version to be exactly `MAJOR.MINOR.PATCH`, with no prerelease or build metadata, and the `v`-prefixed tag must match. SemVer permits build metadata generally; this snapshot intentionally follows the narrower published-release tag policy in `rust/xtask/src/version.rs`. `src/data/product-release.ts` validates both when the site is built. This site-owned snapshot is independent of the desktop development version in Cargo and requires no network access during the Astro build.

After a stable release has been publicly published and verified, update the version and tag in `src/data/product-release.json` together. Do not update this snapshot from an unreleased desktop version, a prerelease, or a version with build metadata. Run `bun run test:unit`, `bun run build`, `bun run verify:dist`, and `bun run verify:seo` before submitting the metadata refresh.
