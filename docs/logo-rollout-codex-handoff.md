# Sky Auto Player — New Logo Rollout Handoff for Codex

> **Status:** design approved by project owner; implementation pending.
>
> **Purpose:** temporary implementation brief for Codex. `AGENTS.md` remains the repository authority.
> This file exists only to carry the approved branding task across sessions. After the rollout is
> complete, remove this temporary handoff from the implementation branch or replace only the durable
> parts with a concise `branding/README.md` if the repository benefits from a permanent asset spec.

## 0. Executive summary

Replace the current constellation mark everywhere with the approved **right-pointing equilateral
play-triangle mark**. The logo is intentionally built from three different node shapes and three edge
relationships rather than drawing a literal play glyph.

The approved mark has these immutable characteristics:

1. The three node centers form an **equilateral triangle pointing right**.
2. Top-left node: a **diamond**, light gold with a warm ivory edge.
3. Bottom-left node: a **gold outlined circle**.
4. Right node: a **sky-grey outlined circle**.
5. Top-left → bottom-left edge: **solid** warm-ivory stroke.
6. Top-left → right edge: **solid** warm-ivory stroke.
7. Bottom-left → right edge: **dashed** warm-ivory stroke.
8. The dashed edge connects the **two circular nodes** and must have deliberately balanced dash
   spacing. It must not look like an arbitrary SVG `stroke-dasharray` accident.
9. The overall silhouette must remain a clean, right-facing play triangle at every size.
10. No music-note glyph, play triangle fill, wing, cloud, sparkle field, neon gradient, glass effect,
    3D bevel, heavy glow, or AI-generated decorative texture may be added.

This is a branding-only task. Do not change playback behavior, native timing, input dispatch, updater
security, configuration semantics, or unrelated UI behavior.

---

## 1. Design intent

The identity should communicate three things simultaneously without becoming illustrative:

- **Play / automatic playback:** the right-facing equilateral triangle is immediately legible as a
  play direction without copying a YouTube-style filled play button.
- **Sky music vocabulary:** circle / diamond / connection geometry recalls the music interface and
  constellation language without copying a game asset.
- **Precision / timing:** the intentionally different third edge (dashed between the circular nodes)
  introduces rhythm and sequence instead of a generic graph/network symbol.

The design must feel quiet, precise, musical, community-made, and technically intentional. It should
not look like a generic 2020s AI-generated “premium tech” logo.

---

## 2. Approved palette

Use the repository's existing brand colors. Do not invent a replacement palette.

| Role | Hex | Usage |
| --- | --- | --- |
| Night | `#07090D` | application icon plate / circle interiors |
| Ivory | `#F4EFE3` | solid edges, dashed edge, diamond outline |
| Light Gold | `#F7DDA2` | diamond fill, gold circle stroke |
| Sky Grey | `#B8CCD6` | right circle stroke |
| Deep Gold | `#DCAE55` | optional supporting brand accent only; not required inside primary mark |

The website already uses the same night / ivory / gold / sky-grey family. Do not retheme the site or
desktop application as part of this task.

---

## 3. Canonical geometry

### 3.1 Coordinate system

Primary source uses `viewBox="0 0 128 128"`.

Node centers:

- `A = (40, 33)` — top-left diamond
- `B = (40, 95)` — bottom-left gold circle
- `C = (93.6936, 64)` — right sky-grey circle

`AB = BC = CA = 62`, so the node centers form an exact equilateral triangle. The apex points right.
Do not casually nudge one center until the triangle becomes merely “approximately” equilateral.
Optical adjustments belong in shape dimensions / stroke trimming, not in the triangle skeleton.

### 3.2 Primary plate

- canvas: `128 × 128`
- plate: `128 × 128`
- corner radius: `28`
- fill: `#07090D`
- no gradient
- no drop shadow baked into the asset
- no outer border required

The rounded square is the **application-icon plate**. The triangle mark is the identity. If a future
surface genuinely needs a transparent mark, derive it from the same geometry rather than redesigning
it.

### 3.3 Nodes

#### Top-left diamond

- centered at A
- square size: `18 × 18`
- rotate `45°` around A
- fill: `#F7DDA2`
- stroke: `#F4EFE3`
- stroke width: `2.5`
- very small corner radius is allowed (`rx` around `1`) but do not round it into a lozenge

#### Bottom-left circle

- center B
- radius: `9.5`
- fill: `#07090D`
- stroke: `#F7DDA2`
- stroke width: `5.5`

#### Right circle

- center C
- radius: `9.5`
- fill: `#07090D`
- stroke: `#B8CCD6`
- stroke width: `5.5`

### 3.4 Solid edges

Draw edges before nodes so the nodes visually terminate the lines cleanly.

- stroke: `#F4EFE3`
- stroke width: `5`
- line cap: `round`
- no glow / filter

Two solid center-to-center skeleton edges:

- A → B
- A → C

The nodes are drawn afterward and mask the line under their footprints. This keeps the geometry exact
and the visible connections clean.

### 3.5 Dashed edge — critical detail

**Do not use a free-running `stroke-dasharray` on the entire B → C line for the canonical master.**
At different render sizes that makes the first/last dash land unpredictably against the two circle
strokes, which is exactly the visual defect this redesign must avoid.

Instead, render the dashed edge as **three explicit round-capped segments** placed along B → C. This
makes the rhythm deterministic and symmetric.

B → C side length is 62. Let `t` measure distance from B along that side. Use:

- dash 1: `t = 16 → 22`
- dash 2: `t = 28 → 34`
- dash 3: `t = 40 → 46`

This yields:

- 16 units of optical clearance from B center before the first segment
- three equal 6-unit dash bodies
- two equal 6-unit internal gaps
- 16 units of optical clearance before C center

Since the circle outer footprint is smaller than 16 units, both ends retain breathing room instead of
crashing into the circular strokes.

Exact segment coordinates, using the unit vector `(sqrt(3)/2, -1/2)`:

```text
segment 1: (53.8564, 87.0000) → (59.0526, 84.0000)
segment 2: (64.2487, 81.0000) → (69.4449, 78.0000)
segment 3: (74.6410, 75.0000) → (79.8372, 72.0000)
```

Each dash segment:

- stroke: `#F4EFE3`
- stroke width: `5`
- line cap: `round`

The visual result should read as **three deliberate rhythmic pulses**, centered on the side between
the two circles.

---

## 4. Canonical primary SVG

Use this as the starting production source. Minor syntax cleanup is fine; geometry and visual output
are not negotiable unless the owner explicitly changes the design.

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" role="img" aria-labelledby="title">
  <title id="title">Sky Auto Player</title>

  <rect width="128" height="128" rx="28" fill="#07090D"/>

  <!-- Solid triangle edges. Nodes are drawn later and mask the line ends. -->
  <line
    x1="40" y1="33" x2="40" y2="95"
    stroke="#F4EFE3" stroke-width="5" stroke-linecap="round"
  />
  <line
    x1="40" y1="33" x2="93.6936" y2="64"
    stroke="#F4EFE3" stroke-width="5" stroke-linecap="round"
  />

  <!-- Explicit dashed edge: deterministic, symmetrical rhythm. -->
  <line
    x1="53.8564" y1="87" x2="59.0526" y2="84"
    stroke="#F4EFE3" stroke-width="5" stroke-linecap="round"
  />
  <line
    x1="64.2487" y1="81" x2="69.4449" y2="78"
    stroke="#F4EFE3" stroke-width="5" stroke-linecap="round"
  />
  <line
    x1="74.6410" y1="75" x2="79.8372" y2="72"
    stroke="#F4EFE3" stroke-width="5" stroke-linecap="round"
  />

  <!-- Top-left diamond -->
  <rect
    x="31" y="24" width="18" height="18" rx="1"
    transform="rotate(45 40 33)"
    fill="#F7DDA2" stroke="#F4EFE3" stroke-width="2.5"
  />

  <!-- Bottom-left timing/rhythm node -->
  <circle
    cx="40" cy="95" r="9.5"
    fill="#07090D" stroke="#F7DDA2" stroke-width="5.5"
  />

  <!-- Right melody/response node -->
  <circle
    cx="93.6936" cy="64" r="9.5"
    fill="#07090D" stroke="#B8CCD6" stroke-width="5.5"
  />
</svg>
```

### 4.1 Flat-render requirement

The committed source SVG must remain flat vector artwork:

- no `<filter>`
- no blur
- no raster image embedded in the SVG
- no gradient unless a future owner-approved redesign explicitly introduces one
- no autogenerated metadata payload from design software
- no unnecessarily huge decimal precision beyond what is needed for the equilateral geometry

The design-board glow was presentation-only and is **not** part of the production logo.

---

## 5. Small-size behavior

The logo must be judged at actual display size, not only enlarged in an SVG viewer.

Required checks:

- 256 px
- 128 px
- 64 px
- 48 px
- 32 px
- 24 px
- 16 px

### 5.1 32 px and above

The canonical source should normally be used without geometric changes. Confirm that:

- the diamond remains visibly different from the circles
- the gold and sky-grey rings remain open
- the three dash pulses stay distinct
- no dash visually touches either circle
- the right-facing play silhouette is immediate

### 5.2 24 px and 16 px

If rasterization of the canonical vector causes the three dashed pulses to merge, disappear, or become
uneven, make a **small optical master** rather than changing the primary geometry.

Allowed small-master changes:

- slightly thicken the three connection strokes
- slightly enlarge the node strokes
- reduce the dashed side to two explicit pills if three cannot survive 16 px cleanly
- increase the clearance between dashed pills
- simplify the diamond border while preserving the gold diamond identity

Not allowed:

- replacing the dashed edge with a solid edge
- moving the node centers out of the right-pointing equilateral skeleton
- converting all three nodes to circles
- introducing a literal filled play triangle

If a small optical master is created, name it clearly, e.g.
`branding/sky-auto-player-app-icon-small.svg`, and document which raster sizes use it.

---

## 6. Repository current state relevant to this rollout

Codex must inspect the current branch before editing, but as of the handoff the important surfaces are:

### Website

- `site/public/assets/sky-auto-player-mark.svg` — current old constellation SVG.
- `site/public/favicon.svg` — currently the same old mark source.
- `site/public/favicon.ico` — old Windows-style multi-resolution icon.
- `site/public/assets/og-banner.jpg` — existing social preview banner.
- `site/src/components/layout/SiteHeader.astro` — renders
  `/assets/sky-auto-player-mark.svg` at 32×32.
- `site/src/components/layout/SiteFooter.astro` — renders the same mark at 24×24.
- `site/src/layouts/BaseLayout.astro` — references the SVG mark, `favicon.ico`, `favicon.svg`, and OG
  banner.
- `site/tests/e2e/site-contracts.spec.ts` — asserts critical brand assets return HTTP 200.

### Tauri desktop

- `desktop/src-tauri/icons/icon.ico` — current desktop icon.
- `desktop/src-tauri/tauri.conf.json` — currently points bundle icon to `icons/icon.ico`.
- `desktop/package.json` includes `@tauri-apps/cli` and the `tauri` script.

### Current Windows/PyInstaller release

- `Sky-Auto-Player.spec` currently creates `Sky-Auto-Player.exe` without an explicit `icon=` argument.
- `src/build_app.py` drives the release build and uses the spec.

This means the branding rollout is incomplete if only the website and Tauri `icon.ico` are changed.
The shipping PyInstaller executable must also receive the approved icon unless current source evidence
shows that this executable is no longer part of the supported release surface.

### Existing icon duplication

At handoff time, `site/public/favicon.ico` and `desktop/src-tauri/icons/icon.ico` are the same old binary
asset. Preserve that consistency with the new icon instead of independently exporting two subtly
different ICO files.

---

## 7. Target asset structure

Add a small, explicit source-of-truth directory at repository root:

```text
branding/
  sky-auto-player-app-icon.svg
  sky-auto-player-app-icon-small.svg       # only if optical small master is actually required
  sky-auto-player-mark-mono.svg            # optional but recommended
  README.md                                 # concise durable spec after rollout, not this handoff
  exports/
    windows/
      sky-auto-player.ico
    web/
      apple-touch-icon.png
```

Do **not** turn this into a large design-system framework. The repository guide explicitly discourages
extra machinery without evidence. A handful of source assets and one concise README is enough.

### Required copies / consumers

```text
branding/sky-auto-player-app-icon.svg
  -> source of truth

branding/exports/windows/sky-auto-player.ico
  -> desktop/src-tauri/icons/icon.ico
  -> site/public/favicon.ico
  -> PyInstaller EXE icon input

branding/sky-auto-player-app-icon.svg
  -> site/public/assets/sky-auto-player-mark.svg

small optical SVG (if needed)
  -> site/public/favicon.svg

branding/exports/web/apple-touch-icon.png
  -> site/public/apple-touch-icon.png
```

Copies are acceptable because build systems have different path expectations. Keep them byte-identical
where they represent the same export, and verify that explicitly.

---

## 8. Exporting the Windows ICO

Prefer the repository's existing Tauri CLI rather than adding a new image-processing dependency.
Tauri v2's `icon` command accepts a square PNG or SVG source and generates a Windows `icon.ico`.

From `desktop/`, a representative flow is:

```powershell
bun install --frozen-lockfile
bun run tauri icon ../branding/sky-auto-player-app-icon.svg --output .brand-icon-export
```

Check the actual CLI help if argument placement differs for the pinned version:

```powershell
bun run tauri icon --help
```

Then inspect the generated `icon.ico` and copy the approved output to:

```text
branding/exports/windows/sky-auto-player.ico
desktop/src-tauri/icons/icon.ico
site/public/favicon.ico
```

Tauri documents that a manually produced Windows ICO should contain 16, 24, 32, 48, 64, and 256 px
layers, with 32 px first for optimal development display. If the CLI output already satisfies this,
do not add a second ICO toolchain.

### Important small-size caveat

The Tauri CLI resizes one source automatically. If the 16/24 px dashed edge is not acceptable, create
small-size raster layers from the approved small optical master and assemble the final ICO with a
locally available tool. Do not add a runtime application dependency just to build icons. Record the
exact local command in the PR description if a non-repository tool was needed.

---

## 9. Website implementation

### 9.1 Replace the website mark

Replace:

```text
site/public/assets/sky-auto-player-mark.svg
```

with the canonical approved SVG.

Do not rename this path unless there is a concrete reason. Header and footer already consume it, so
keeping the path minimizes unrelated churn.

### 9.2 Replace browser favicons

Replace:

```text
site/public/favicon.svg
site/public/favicon.ico
```

The SVG favicon may use the small optical master if that is cleaner at tiny sizes.

### 9.3 Add proper Apple touch icon

Add:

```text
site/public/apple-touch-icon.png
```

Recommended size: 180×180 PNG. It should use the application-icon plate, not a transparent free-floating
triangle.

Update `site/src/layouts/BaseLayout.astro` so `rel="apple-touch-icon"` points to
`/apple-touch-icon.png` through the existing `withBase(...)` helper instead of pointing to an SVG.
Preserve GitHub Pages base-path awareness.

### 9.4 OG banner

Regenerate:

```text
site/public/assets/og-banner.jpg
```

Target remains 1200×630 because `BaseLayout.astro` currently publishes those OG dimensions.

Approved banner direction:

- Night background (`#07090D` or the current page background token value).
- New icon on the left or left-center.
- `Sky Auto Player` as the dominant text.
- Existing tagline: `Play the sheet. Not the keyboard.`
- restrained ivory / gold / sky-grey palette
- no generated fantasy scene, starscape, character art, glow field, or stock background
- must remain legible as a small social-card preview

A deterministic browser-rendered composition using existing website typography is preferable to an
AI-generated image. Playwright is already a site dev dependency, so a one-off local render is fine.
Do not add a permanent rendering framework unless useful beyond this asset.

### 9.5 Header and footer

Because `SiteHeader.astro` and `SiteFooter.astro` already reference the stable mark path, asset
replacement should update them automatically.

Do not redesign header/footer spacing unless the new mark visibly requires a tiny optical adjustment.
If adjustment is necessary, keep it minimal and test 24 px / 32 px rendering.

### 9.6 Site tests

Update `site/tests/e2e/site-contracts.spec.ts` only as needed:

- keep assertions for `/favicon.ico`, `/favicon.svg`, and `/assets/sky-auto-player-mark.svg`
- add `/apple-touch-icon.png`
- keep `/assets/og-banner.jpg`

Do not weaken existing route, SEO, or asset checks.

---

## 10. README / GitHub repository presentation

The current README title begins with a generic music emoji. Integrate the actual project mark so the
repository landing surface uses the new identity.

Preferred approach:

```html
<div align="center">
  <img src="site/public/assets/sky-auto-player-mark.svg" alt="Sky Auto Player logo" width="96">

# Sky Auto Player
...
</div>
```

Remove the `🎵` prefix from the H1 once the actual logo is present. Do not turn the README hero into a
large marketing poster; the product screenshot should remain the main product evidence.

Check GitHub rendering after the change.

---

## 11. Tauri desktop implementation

### 11.1 Replace icon

Replace:

```text
desktop/src-tauri/icons/icon.ico
```

with the canonical new Windows ICO.

Current `tauri.conf.json` already references `icons/icon.ico`, so do not change config merely for the
sake of changing it.

### 11.2 In-app logo audit

Search the desktop React source for any explicit project-logo asset or hard-coded old constellation
mark. If none exists, do **not** add decorative logo placements just to satisfy the rollout.

The operating-system app icon, taskbar icon, shortcut icon, window representation, and About/update
surfaces are the priority. Existing functional iconography from Lucide is not brand artwork and must
not be replaced.

### 11.3 Theme independence

The GUI has several user themes. The brand icon does not change with `aurora`, `minimalist`, `slate`,
`cyberpunk`, or `classic`. Do not theme the gold/sky-grey nodes dynamically.

---

## 12. Shipping PyInstaller executable

The root `Sky-Auto-Player.spec` currently omits an explicit icon. If this remains the supported Windows
release executable, add the new ICO to the `EXE(...)` definition using the canonical exported ICO.

The exact PyInstaller syntax should follow the currently pinned PyInstaller version. Expected shape:

```python
exe = EXE(
    ...,
    icon=str(ROOT / 'branding' / 'exports' / 'windows' / 'sky-auto-player.ico'),
    ...,
)
```

Confirm from the pinned build that the path is accepted and the frozen EXE contains the correct icon.
Do not change console/window mode, version metadata, native loading, or packaging layout as part of
this branding task.

If `Sky-Auto-Player.spec` cannot access the root branding path in the current build environment, use
the smallest path/config adjustment. Do not duplicate design logic in Python.

---

## 13. Updater and other Windows executables

Audit user-visible Windows executables produced by the repository, including the native updater and
calibration helper.

Classification rule:

- **Primary/user-visible process** that can appear independently in Explorer/taskbar/dialogs: apply
  the brand icon if the current build system already has a clean resource mechanism or adding one is
  low-risk and well-contained.
- **Short-lived internal helper** with no meaningful user-facing identity: do not introduce a new Rust
  resource dependency solely for cosmetic consistency unless the owner asks for it.

This task must not perturb native timing, updater integrity, signatures/checksums, or release
provenance. Any new binary resource embedding that affects release artifacts must be covered by the
existing release verification path.

Document any intentionally unbranded helper as a residual scope note in the PR.

---

## 14. Monochrome variant

Create a simple monochrome variant for cases where color is unavailable.

Rules:

- Night plate remains `#07090D` for dark-on-dark app-icon usage, or produce a transparent mark variant
  if a documented surface requires it.
- Diamond and both circle strokes become `#F4EFE3`.
- All three connection relationships remain: two solid, one dashed.
- Do not simplify the dashed edge away.

This variant does not need to be wired into the application unless an actual surface uses it. It is a
brand asset, not a reason to add more UI.

---

## 15. Asset quality checks

### Vector checks

- valid SVG XML
- 128×128 viewBox
- no raster `<image>` payload
- no filters / blur / gradients
- no font dependency inside the mark
- no clipping at the 128×128 bounds
- geometry remains centered with reasonable optical margins

### Raster checks

At each required size inspect on both:

- dark desktop background
- light desktop background where the icon plate is visible

Confirm:

- triangle reads right-facing
- diamond reads as a diamond
- both circles remain open rings
- dashed edge is obviously different from the solid edges
- three large-master dashes are balanced and centered
- 16/24 px output does not become an accidental solid edge
- no muddy antialiasing halo
- no pixel clipping at rounded corners

### ICO checks

Confirm the ICO contains the expected Windows layers. The desktop icon and site favicon ICO should be
byte-identical copies of the canonical exported ICO.

### Branding consistency checks

Search the repository for old brand asset names / old SVG geometry and confirm no active public
surface still displays the constellation mark.

Do not replace unrelated “constellation” words in prose; only visual assets/references are in scope.

---

## 16. Tests and verification

Follow `AGENTS.md`: run narrow checks first, then the applicable repository verification.

### Website

From `site/`:

```powershell
bun install --frozen-lockfile
bun run check
bun run lint
bun run format:check
bun run build
bun run verify:dist
bun run verify:seo
bun run test:functional
```

At minimum the site contract test must prove the new critical assets publish correctly under the
GitHub Pages base path.

### Desktop React/Tauri frontend

From `desktop/`:

```powershell
bun install --frozen-lockfile
bun run check
```

If Tauri packaging is available on the Windows runner/local environment, also run the narrowest build
that proves the new ICO is accepted.

### Root repository

Run the repository-owned verification entry point appropriate to the changed files:

```powershell
uv run python scripts/check.py static
```

Then run broader checks if required by source changes. If the PyInstaller spec changes, use the
packaging/release verification path that the current repository documents rather than inventing a
new smoke process.

### Visual evidence required in PR

The PR description must include or link screenshots/contact-sheet evidence for:

- canonical 128 px icon
- 48 px
- 32 px
- 24 px
- 16 px
- website header
- website footer
- browser favicon if practical
- Windows Explorer/taskbar/shortcut or equivalent Tauri/PyInstaller icon evidence when practical

Do not judge tiny icons only from a zoomed 800% screenshot; include actual-size samples.

---

## 17. Expected file changes

This is a guide, not a forced exact diff. The expected minimal implementation is approximately:

### Add

```text
branding/sky-auto-player-app-icon.svg
branding/sky-auto-player-mark-mono.svg
branding/README.md
branding/exports/windows/sky-auto-player.ico
branding/exports/web/apple-touch-icon.png
site/public/apple-touch-icon.png
```

Potentially add only if needed:

```text
branding/sky-auto-player-app-icon-small.svg
```

### Replace/update

```text
site/public/assets/sky-auto-player-mark.svg
site/public/favicon.svg
site/public/favicon.ico
site/public/assets/og-banner.jpg
desktop/src-tauri/icons/icon.ico
site/src/layouts/BaseLayout.astro
site/tests/e2e/site-contracts.spec.ts
README.md
Sky-Auto-Player.spec
```

Potentially update only if current-source audit proves necessary:

```text
user-visible updater/native Windows resource configuration
```

Avoid unrelated changes to application code or design tokens.

---

## 18. Acceptance criteria — implementation is NOT done until all pass

### Design

- [ ] Node centers form an equilateral right-facing triangle.
- [ ] Top-left is a gold/ivory diamond.
- [ ] Bottom-left is a gold outlined circle.
- [ ] Right is a sky-grey outlined circle.
- [ ] A→B edge is solid.
- [ ] A→C edge is solid.
- [ ] B→C edge is dashed.
- [ ] Dashed edge is made from deliberately positioned segments, not visually accidental dash phase.
- [ ] Primary production mark contains no glow, gradient, texture, or decorative effects.
- [ ] 16/24 px versions remain legible.

### Website

- [ ] Header shows new mark.
- [ ] Footer shows new mark.
- [ ] SVG favicon shows new mark.
- [ ] ICO favicon shows new mark.
- [ ] Apple touch icon points to a real PNG and shows new mark.
- [ ] OG social banner uses new mark.
- [ ] GitHub Pages base-path behavior remains correct.
- [ ] Site E2E critical asset contracts pass.

### Repository presentation

- [ ] README uses the actual new project mark and no longer relies on the generic music emoji as the
      primary identity.

### Desktop / Windows

- [ ] Tauri icon ICO is new mark.
- [ ] Tauri configuration still resolves the icon.
- [ ] Shipping PyInstaller EXE uses the new icon if it remains a supported release surface.
- [ ] Site `favicon.ico`, Tauri `icon.ico`, and canonical Windows ICO are identical exports.
- [ ] Any intentionally unbranded helper executable is documented in the PR.

### Engineering

- [ ] No playback/native/input/update semantics changed.
- [ ] No new runtime dependency added solely for image generation.
- [ ] No unrelated formatting churn.
- [ ] Relevant site/desktop/root checks pass.
- [ ] PR includes visual evidence at actual small sizes.

---

## 19. Non-goals / prohibited scope expansion

Do not use this task to:

- redesign the website
- redesign desktop themes
- change typography globally
- rename the product
- change the tagline
- alter playback UX
- add animation to the brand mark
- add a splash screen just because a new logo exists
- add a new design-system package
- add a custom agent framework or brand build framework
- refactor unrelated asset loading
- touch security/input behavior

Keep the diff reviewable and brand-focused.

---

## 20. PR structure recommendation

Suggested branch:

```text
feat/logo-rollout
```

Suggested commits (do not force if a smaller clean history is better):

1. `brand: add approved logo source and exports`
2. `site: roll out new Sky Auto Player identity`
3. `desktop: apply new Windows application icon`
4. `build: embed brand icon in packaged executable`

Suggested PR title:

```text
brand: roll out the new Sky Auto Player logo
```

Suggested PR body sections:

- Summary
- Approved design invariants
- Surfaces updated
- Small-size icon evidence
- Tests / verification
- Packaging evidence
- Residual scope (if any helper executables intentionally remain unbranded)

Do not merge automatically. The project owner wants a separate visual/implementation acceptance pass.

---

## 21. Handoff to acceptance reviewer

After Codex finishes, provide the resulting PR number/link to the reviewer in ChatGPT.

The reviewer should inspect:

1. changed-file list and diff
2. canonical SVG geometry
3. dashed-segment coordinates / output
4. favicon and desktop icon consistency
5. PyInstaller/Tauri configuration
6. website asset contract tests
7. CI/check status
8. actual-size visual evidence, especially 16/24/32 px
9. absence of unrelated behavior changes

The reviewer should reject the rollout if the dashed connection is uneven, if the play triangle has
lost its equilateral structure, if small icons become muddy, or if the implementation reintroduces
AI-slop visual effects.

---

## 22. Final implementation principle

Treat the approved logo as a **small piece of engineered geometry**, not as an illustration.

The correct implementation is the one where:

- the right-facing triangle is instantly readable,
- the three nodes stay distinct,
- the two solid edges and one rhythmic dashed edge feel intentional,
- the mark survives 16 px,
- every public project surface tells the same visual story,
- and no extra decoration is needed to make it feel finished.
