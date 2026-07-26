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
    bun run build
    bun run verify:dist
    bun run test:e2e

The site is static output. The production build is written to dist/, and the E2E suite checks routing, canonical metadata, accessibility, responsive behavior, the project favicon, and the mobile screenshot crop.
