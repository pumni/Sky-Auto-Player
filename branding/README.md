# Sky Auto Player branding

The visual master is `sky-auto-player-app-icon.svg`: a flat `128 × 128` application icon with a
right-facing equilateral node skeleton. The top-left node is a medium ivory-outlined gold diamond;
the lower-left gold ring is intentionally larger than the smaller blue-gray ring on the right.
Connections are two thin solid lines and three explicit, round-capped dash segments. Each connection
has an optical inset so it does not weld into a node at small sizes.

`sky-auto-player-app-icon-small.svg` is the optical master for `24px`; it uses slightly stronger
strokes and two clearly separated dash segments. `sky-auto-player-app-icon-16.svg` is a dedicated
16px optical master with protected ring holes and the same two-pulse rhythm. Use the large master for
`256`, `128`, `64`, `48`, and `32px`.

## Source variants

- `sky-auto-player-mark-no-bg.svg` — full-color transparent mark for upright, low-opacity decorative use.
- `sky-auto-player-mark-mono.svg` — light-on-dark transparent monochrome mark.
- `sky-auto-player-mark-mono-dark.svg` — dark-on-light transparent monochrome mark.
- `sky-auto-player-mark-mono-solid.svg` — light monochrome mark on the dark application plate.
- `lockup-horizontal.svg` and `lockup-stacked.svg` — flat production lockups with the approved tagline.

All sources are hand-authored flat SVG. They contain no filters, gradients, embedded raster images,
glow, texture, or generated design-tool metadata.

## Export commands

The Tauri CLI can rasterize one source at a time, so the Windows ICO is assembled from three raster
sets. Run from the repository root after installing the pinned desktop dependencies:

```powershell
cd desktop
bun install --frozen-lockfile
bun run tauri icon ../branding/sky-auto-player-app-icon.svg --output <large-output> --png 32,48,64,128,256
bun run tauri icon ../branding/sky-auto-player-app-icon-small.svg --output <small-output> --png 24
bun run tauri icon ../branding/sky-auto-player-app-icon-16.svg --output <tiny-output> --png 16
# Browser favicon PNGs: use the dedicated 16px master and large 32px master
bun run tauri icon ../branding/sky-auto-player-app-icon-16.svg --output <favicon-16-output> --png 16
bun run tauri icon ../branding/sky-auto-player-app-icon.svg --output <favicon-32-output> --png 32
cd ..
cargo xtask branding build-ico --large-dir <large-output> --small-dir <small-output> --tiny-dir <tiny-output> --output branding/exports/windows/sky-auto-player.ico
```

`cargo xtask branding build-ico` is the canonical build-time assembler. It preserves the PNG payloads
and writes the seven ICO layers `16, 24, 32, 48, 64, 128, 256`; `16` comes from the dedicated
16px master, `24` comes from the small optical master, and the remaining layers come from the large
master. Copy the resulting ICO byte-for-byte to `site/public/favicon.ico` and
`desktop/src-tauri/icons/icon.ico`.

For browser favicon routing, also rasterize the dedicated 16px master and the large master as
individual PNGs, then copy them byte-for-byte to `site/public/favicon-16x16.png` and
`site/public/favicon-32x32.png` (and keep the same files under `branding/exports/web/`). The HTML
head declares these explicit PNG candidates before the ICO fallback, so browsers that support PNG
favicons receive the real 16px optical master. `site/public/favicon.svg` remains the 24px small
master used by the footer and is intentionally not declared as the browser favicon.

Generate the Apple touch icon from the large master with the same Tauri CLI using `--png 180`, then
copy it to `branding/exports/web/apple-touch-icon.png` and `site/public/apple-touch-icon.png`.
Render `exports/web/og-banner.svg` in a browser at `1200 × 630` and encode the screenshot as JPEG at
both `site/public/assets/og-banner.jpg` and `site/public/assets/images/og-banner.jpg`.
