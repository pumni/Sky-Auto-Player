# Sky Auto Player branding

`sky-auto-player-app-icon.svg` is the canonical application mark. It is a flat `128 × 128` SVG
with the approved `#07090D`, `#F4EFE3`, `#F7DDA2`, and `#B8CCD6` palette. The node centers are
`(40, 33)`, `(40, 95)`, and `(93.6936, 64)`, an equilateral triangle pointing right. The two
edges from the diamond are solid; the circle-to-circle edge is three explicit six-unit,
round-capped segments at the approved coordinates.

The application plate is part of the canonical icon. `sky-auto-player-mark-mono.svg` is the
transparent monochrome derivative for the site's low-opacity decorative mark; it retains all three
edges and the same geometry.

## Exports

- `exports/windows/sky-auto-player.ico` is generated from the canonical SVG with the pinned Tauri
  CLI (`cd desktop; bun install --frozen-lockfile; bun run tauri icon ../branding/sky-auto-player-app-icon.svg
  --output <temporary-output>`) and contains the Windows size layers.
- `exports/web/apple-touch-icon.png` is the 180 px application-plate render.
- `exports/web/og-banner.svg` is the deterministic source composition for the 1200 × 630 JPEG
  social card. Render it with a local browser screenshot and encode the result as
  `site/public/assets/og-banner.jpg` (and its legacy `assets/images` copy).
- `site/public/favicon.ico` and `desktop/src-tauri/icons/icon.ico` are byte-identical copies of the
  canonical Windows export.

Keep the SVG flat: no filters, gradients, embedded raster data, glow, or generated design-tool
metadata. Judge the canonical export at 256, 128, 64, 48, 32, 24, and 16 px before changing the
geometry. If an optical small master is ever needed, preserve the equilateral node skeleton and
document the exact sizes that use it here.
