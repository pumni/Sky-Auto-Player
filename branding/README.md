# Sky Auto Player branding

The visual master is `sky-auto-player-app-icon.svg`: a flat `128 × 128` application icon with a
right-facing equilateral node skeleton. The top-left node is a medium ivory-outlined gold diamond;
the lower-left gold ring is intentionally larger than the smaller blue-gray ring on the right.
Connections are two thin solid lines and three explicit, round-capped dash segments. Each connection
has an optical inset so it does not weld into a node at small sizes.

`sky-auto-player-app-icon-small.svg` is the optical master for `24px` and `16px`. It uses slightly
stronger strokes and two clearly separated dash segments so the distinction between solid and dashed
edges survives reduction. Use the large master for `256`, `128`, `64`, `48`, and `32px`.

## Source variants

- `sky-auto-player-mark-no-bg.svg` — full-color transparent mark for upright, low-opacity decorative use.
- `sky-auto-player-mark-mono.svg` — light-on-dark transparent monochrome mark.
- `sky-auto-player-mark-mono-dark.svg` — dark-on-light transparent monochrome mark.
- `sky-auto-player-mark-mono-solid.svg` — light monochrome mark on the dark application plate.
- `lockup-horizontal.svg` and `lockup-stacked.svg` — flat production lockups with the approved tagline.

All sources are hand-authored flat SVG. They contain no filters, gradients, embedded raster images,
glow, texture, or generated design-tool metadata.

## Export commands

The Tauri CLI can rasterize one source at a time, so the Windows ICO is assembled from two raster
sets. Run from the repository root after installing the pinned desktop dependencies:

```powershell
cd desktop
bun install --frozen-lockfile
bun run tauri icon ../branding/sky-auto-player-app-icon.svg --output <large-output> --png 32,48,64,128,256
bun run tauri icon ../branding/sky-auto-player-app-icon-small.svg --output <small-output> --png 16,24
cd ..
uv run python branding/scripts/build_ico.py --large-dir <large-output> --small-dir <small-output> --output branding/exports/windows/sky-auto-player.ico
```

`branding/scripts/build_ico.py` is a build-time standard-library assembler. It preserves the PNG
payloads and writes the six ICO layers `16, 24, 32, 48, 64, 256`; the first two come from the small
optical master and the remaining layers come from the large master. Copy the resulting ICO byte-for-
byte to `site/public/favicon.ico` and `desktop/src-tauri/icons/icon.ico`.

Generate the Apple touch icon from the large master with the same Tauri CLI using `--png 180`, then
copy it to `branding/exports/web/apple-touch-icon.png` and `site/public/apple-touch-icon.png`.
Render `exports/web/og-banner.svg` in a browser at `1200 × 630` and encode the screenshot as JPEG at
both `site/public/assets/og-banner.jpg` and `site/public/assets/images/og-banner.jpg`.
