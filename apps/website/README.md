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

The shared `.container` is capped at 75rem with 16–40px side gutters; tablet documentation layouts cap it at 56rem. Midnight Editorial Home uses a scoped 80rem rail, graphite surfaces and Inter typography. Hero and real desktop preview share a two-column opening from 64rem, with the two feature explanations on the same rail below. Playback uses editorial copy beside a single vertical instrument panel at wide widths; narrow layouts preserve heading, demo and dry-run reading order. Product preview selection uses straight underline buttons with independent pending and focus states. QuickStart is an open three-step workflow with naturally sized setup and format resources; its compact closing gap leads into the installer, safety and help area, which uses shared detail lists and visible risk notes. Home has one filled download CTA and a closing stable-installer text link; the topbar instead exposes a direct locale link outside the mobile menu. Documentation retains its release action and route-preserving language group. Existing public anchors, product preview fallback and playback state contracts remain in force.

The shared header and footer are rendered by `BaseLayout` on every route. Navigation copy is typed in `src/data/home.types.ts` and localized in `src/data/home.en.ts` and `src/data/home.vi.ts`. Keep both locale shapes aligned and check product claims against their owning desktop, security, and distribution contracts.

## Homepage

`src/components/home/HomePage.astro` composes five chapters in order: the product-first Hero and preview in a shared opening, playback demonstration and supporting notes, getting started, and installation, safety and help. The visible targets `#top`, `#product`, `#playback`, `#how-it-works`, `#formats`, `#technical`, `#timing-model`, and `#faq-preview` remain stable.

Two static editorial blocks describe imports and compatibility after the preview. The compact selector updates one localized caption and enhances direct full-size image links and commits the latest successfully loaded selection as one update. The last loaded view stays visible while a new preview loads. Slow requests show a localized status. Failed requests, including initial load failure, offer a retry and direct image links. Cancelling a selection clears its timer and listeners. The browser may finish the image transfer. The playback demonstration and its browser behavior share the fixed three-event model in `src/data/playback-demo.ts`; it has one timeline, one elapsed display, one key grid, and user-triggered controls. Its static markup remains useful without JavaScript.

The page scroll container owns fragment clearance. Its padding follows the compact and sticky header breakpoint. The enhanced header applies the responsive disclosure mode after inspecting current focus so the outgoing control can hand focus to its visible replacement. The static header keeps CSS-only navigation.

The guide architecture figure uses one ordered list for the visual flow and its text equivalent, including the process boundary.

Product images are browser mock previews of published stable desktop source, not native Tauri captures or gameplay evidence. They use example data and are named `desktop-*-browser-preview-1366x768.png`; the screenshot source and method are recorded below. The older `docs/ui` capture manifest names a different desktop tree and was not reused.

### Homepage screenshot provenance

| Asset                                                                         | View              | Resolution |
| ----------------------------------------------------------------------------- | ----------------- | ---------- |
| `public/assets/images/desktop-library-browser-preview-1366x768.png`           | Library           | 1366 × 768 |
| `public/assets/images/desktop-song-detail-browser-preview-1366x768.png`       | Song detail       | 1366 × 768 |
| `public/assets/images/desktop-settings-playback-browser-preview-1366x768.png` | Playback settings | 1366 × 768 |

The previews were captured from published stable tag `v4.5.0` at source commit `17c3e72729bd03c1f1a46be534494d7f887a1ea8` (desktop tree `e3464e35d84b67325634f7a65de108c69f4a9954`). In a detached worktree, the desktop Vite and Playwright Chromium visual evidence harness used its deterministic browser mock bridge at 1366 × 768. The views show the sample library, Blue Bird with Inspector Details open, and the Playback page of Settings. The capture exercised browser UI only.

## Release destinations

The website directs stable installer requests to the official GitHub Releases Latest page. Project release policy designates stable releases as Latest and keeps prereleases out of Latest. The footer and crawler index link to the Releases collection for history. Astro builds these destinations into static HTML and the SoftwareApplication download URL without a release snapshot or network lookup.
