# Sky Auto Player branding

The visual master is `sky-auto-player-app-icon.svg`: a flat `128 × 128` application icon with a
right-facing equilateral node skeleton. The top-left node is a medium ivory-outlined gold diamond;
the lower-left gold ring is intentionally larger than the smaller blue-gray ring on the right.
Connections are two solid lines and explicit, round-capped dash segments. These three source masters
are owner-approved and immutable; rendering and delivery may change, but brand geometry may not:

| Asset | Target | Use |
| --- | ---: | --- |
| `sky-auto-player-app-icon-16.svg` | 16px | dedicated tiny Windows surface |
| `sky-auto-player-app-icon-small.svg` | 20–24px | approved small Windows surface |
| `sky-auto-player-app-icon.svg` | 32–256px | canonical Windows, toolbar and marketing surfaces |

`approved-brand.lock.json` pins all three source files to the owner-approved commit and SHA-256
content. The SHA-256 check normalizes CRLF and legacy bare-CR line endings to LF so a Windows
working-tree checkout cannot invalidate the lock without changing the approved SVG content. Do not
redraw, optically reinterpret, or change these files to solve raster or native delivery problems.

## Source variants

- `sky-auto-player-mark-no-bg.svg` — full-color transparent mark for upright, low-opacity decorative use.
- `sky-auto-player-mark-mono.svg` — light-on-dark transparent monochrome mark.
- `sky-auto-player-mark-mono-dark.svg` — dark-on-light transparent monochrome mark.
- `sky-auto-player-mark-mono-solid.svg` — light monochrome mark on the dark application plate.
- `lockup-horizontal.svg` and `lockup-stacked.svg` — flat production lockups with the approved tagline.

All sources are hand-authored flat SVG. They contain no filters, gradients, embedded raster images,
glow, texture, or generated design-tool metadata.

## Export commands

The Tauri CLI rasterizes the approved masters into one Windows raster directory. The source routing
is recorded in `raster-sources.json` and is checked by `cargo xtask check static`. Run from the
repository root after installing the pinned desktop dependencies:

```powershell
cd desktop
bun install --frozen-lockfile
bun run tauri icon ../branding/sky-auto-player-app-icon-16.svg --output ../branding/exports/windows/raster --png 16
bun run tauri icon ../branding/sky-auto-player-app-icon-small.svg --output ../branding/exports/windows/raster --png 20,24
bun run tauri icon ../branding/sky-auto-player-app-icon.svg --output ../branding/exports/windows/raster --png 32,40,48,64,96,128,256
bun run tauri icon ../branding/sky-auto-player-app-icon.svg --output src/assets/brand --png 32,40,48,64
cd ..
cargo xtask branding build-ico --layers-dir branding/exports/windows/raster --output branding/exports/windows/sky-auto-player.ico
```

`cargo xtask branding build-ico` is the canonical build-time assembler. It uses the `ico` crate,
writes the defensive fallback layer `32px` first, encodes all layers below `256px` as BMP ICO
entries, and encodes only the `256px` layer as PNG. The output has the ten layers
`16, 20, 24, 32, 40, 48, 64, 96, 128, 256`. Copy it byte-for-byte to
`site/public/favicon.ico` and `desktop/src-tauri/icons/icon.ico`.

The running Windows window is a separate pipeline. `desktop/src-tauri/src/windows_icon.rs` obtains
the HWND through `raw-window-handle`, reads `SM_CXSMICON/SM_CYSMICON` and
`SM_CXICON/SM_CYICON` with `GetSystemMetricsForDpi`, loads resource `32512` with `LoadImageW`
at those dimensions, and applies separate `WM_SETICON/ICON_SMALL` and `WM_SETICON/ICON_BIG`
handles. It reapplies both handles after a scale-factor change; the EXE resource remains the
Explorer/shortcut source of truth.

`branding/evidence/optical-size-contact-sheet.png` records the generated `16, 20, 24, 32, 40, 48px`
PNGs at true 1:1 size and at 8× nearest-neighbour magnification for pixel inspection.

For browser favicon routing, rasterize the approved 16px and canonical 32px masters as individual
PNGs, then
copy them byte-for-byte to `site/public/favicon-16x16.png` and `site/public/favicon-32x32.png` (and
keep the same files under `branding/exports/web/`). The HTML head declares these explicit PNG
candidates before the ICO fallback, so browsers that support PNG favicons receive the real optical
masters. `site/public/favicon.svg` remains the approved small master used by the footer and is
intentionally not declared as the browser favicon.

Generate the Apple touch icon from the large master with the same Tauri CLI using `--png 180`, then
copy it to `branding/exports/web/apple-touch-icon.png` and `site/public/apple-touch-icon.png`.
Render `exports/web/og-banner.svg` in a browser at `1200 × 630` and encode the screenshot as JPEG at
both `site/public/assets/og-banner.jpg` and `site/public/assets/images/og-banner.jpg`.
