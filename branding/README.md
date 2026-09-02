# Sky Auto Player branding

The visual master is `sky-auto-player-app-icon.svg`: a flat `128 × 128` application icon with a
right-facing equilateral node skeleton. The top-left node is a medium ivory-outlined gold diamond;
the lower-left gold ring is intentionally larger than the smaller blue-gray ring on the right.
Connections are two solid lines and explicit, round-capped dash segments. The canonical master is
intended for large surfaces; it is not scaled down to supply the small Windows icon layers.

Small Windows surfaces use pixel-grid optical masters with target-sized viewBoxes:

| Asset | Target | Use |
| --- | ---: | --- |
| `sky-auto-player-app-icon-16.svg` | 16px | title bar/tray at 100% |
| `sky-auto-player-app-icon-20.svg` | 20px | title bar/tray at 125% |
| `sky-auto-player-app-icon-small.svg` / `-24.svg` | 24px | title bar/tray and taskbar at 100% |
| `sky-auto-player-app-icon-30.svg` | 30px | taskbar at 125% |
| `sky-auto-player-app-icon-32.svg` | 32px | React toolbar and exact 32px layer |
| `sky-auto-player-app-icon-36.svg` | 36px | taskbar at 150% |
| `sky-auto-player-app-icon-40.svg` | 40px | high-DPI intermediate surface |
| `sky-auto-player-app-icon-48.svg` | 48px | taskbar at 200% |

These masters simplify the dash rhythm, protect ring holes, use effective small-size strokes, and
omit the sub-pixel plate border. The React toolbar uses the dedicated `32px` master. The larger
ICO layers at `60`, `64`, `72`, `96`, `128`, and `256px` continue to derive from the visual master.

## Source variants

- `sky-auto-player-mark-no-bg.svg` — full-color transparent mark for upright, low-opacity decorative use.
- `sky-auto-player-mark-mono.svg` — light-on-dark transparent monochrome mark.
- `sky-auto-player-mark-mono-dark.svg` — dark-on-light transparent monochrome mark.
- `sky-auto-player-mark-mono-solid.svg` — light monochrome mark on the dark application plate.
- `lockup-horizontal.svg` and `lockup-stacked.svg` — flat production lockups with the approved tagline.

All sources are hand-authored flat SVG. They contain no filters, gradients, embedded raster images,
glow, texture, or generated design-tool metadata.

## Export commands

The Tauri CLI can rasterize one source at a time, so the Windows ICO is assembled from three
recursive raster sets. Each dedicated source is written to its own size directory so that the
assembler can collect all layers without overwriting same-named `NxN.png` files. Run from the
repository root after installing the pinned desktop dependencies:

```powershell
cd desktop
bun install --frozen-lockfile
bun run tauri icon ../branding/sky-auto-player-app-icon-16.svg --output <tiny-output>/16 --png 16
bun run tauri icon ../branding/sky-auto-player-app-icon-20.svg --output <small-output>/20 --png 20
bun run tauri icon ../branding/sky-auto-player-app-icon-small.svg --output <small-output>/24 --png 24
bun run tauri icon ../branding/sky-auto-player-app-icon-30.svg --output <small-output>/30 --png 30
bun run tauri icon ../branding/sky-auto-player-app-icon-32.svg --output <large-output>/32 --png 32
bun run tauri icon ../branding/sky-auto-player-app-icon-36.svg --output <small-output>/36 --png 36
bun run tauri icon ../branding/sky-auto-player-app-icon-40.svg --output <small-output>/40 --png 40
bun run tauri icon ../branding/sky-auto-player-app-icon-48.svg --output <large-output>/48 --png 48
bun run tauri icon ../branding/sky-auto-player-app-icon.svg --output <large-output>/canonical --png 60,64,72,96,128,256
cd ..
cargo xtask branding build-ico --large-dir <large-output> --small-dir <small-output> --tiny-dir <tiny-output> --output branding/exports/windows/sky-auto-player.ico
```

`cargo xtask branding build-ico` is the canonical build-time assembler. It preserves the PNG payloads
and writes the fourteen ICO layers `16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 96, 128, 256`.
Copy the resulting ICO byte-for-byte to `site/public/favicon.ico` and
`desktop/src-tauri/icons/icon.ico`.

`branding/evidence/optical-size-contact-sheet.png` records the generated `16, 20, 24, 30, 32, 36,
40, 48px` PNGs at true 1:1 size and at 8× nearest-neighbour magnification for pixel inspection.

For browser favicon routing, rasterize the dedicated 16px and 32px masters as individual PNGs, then
copy them byte-for-byte to `site/public/favicon-16x16.png` and `site/public/favicon-32x32.png` (and
keep the same files under `branding/exports/web/`). The HTML head declares these explicit PNG
candidates before the ICO fallback, so browsers that support PNG favicons receive the real optical
masters. `site/public/favicon.svg` remains the 24px small master used by the footer and is
intentionally not declared as the browser favicon.

Generate the Apple touch icon from the large master with the same Tauri CLI using `--png 180`, then
copy it to `branding/exports/web/apple-touch-icon.png` and `site/public/apple-touch-icon.png`.
Render `exports/web/og-banner.svg` in a browser at `1200 × 630` and encode the screenshot as JPEG at
both `site/public/assets/og-banner.jpg` and `site/public/assets/images/og-banner.jpg`.
