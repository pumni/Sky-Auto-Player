# 2026-07-26 Pages UI Spacing & Brand Mark Polish Plan

> **Status:** PROPOSED — executable implementation plan for AI coding agents.  
> **Scope:** `site/` marketing website only (Astro static).  
> **Out of scope:** Python app (`src/`), installer, updater, SendInput, release packaging, `docs/` technical Markdown content (except this plan + INDEX entry).  
> **Branch context:** `refactor/pages-architecture` (or equivalent site branch).  
> **Hierarchy:** `AGENTS.md` (P0–P2) wins over this plan. This plan is **proposal / working notes**, not normative. When conflict arises with `docs/architecture.md` or security mandates, stop and report — do not improvise.

---

## 0. Purpose (read once, then execute phases in order)

This plan fixes **visual polish defects** found in the Astro `site/` after the pages architecture refactor. Defects were confirmed by:

1. Source audit of `site/src/**` against the pages refactor guide (`04-UI-UX-REFACTOR-SPEC`, `05-DESIGN-SYSTEM-AND-CSS`, `12-ACCEPTANCE-CHECKLIST`).
2. Runtime measurement at viewport 1280×900: header height ≈ 73px, hero `padding-top` = 128px, gap header-bottom → hero kicker ≈ **253px**.
3. Full-page screenshot of `http://127.0.0.1:4321/Sky-Auto-Player/` (owner-supplied, 2026-07-26).
4. SVG geometry analysis of `site/public/assets/sky-auto-player-mark.svg` (content center X ≈ 49.5 vs viewBox center 64 → **−14.5px left bias**).

### 0.1 Problems this plan MUST fix (complete inventory)

| ID | Severity | Symptom | Root cause (do not re-diagnose) |
|----|----------|---------|----------------------------------|
| **P1** | Critical | Hero (and FAQ hero) sits too far below topbar | `.section-shell--feature` applies symmetric `padding-block: var(--section-space-feature)` to first section under header |
| **P2** | Critical | Brand mark looks left-shifted; path does not seat on node centers | SVG nodes/path not centered; mid node `cx=78` vs path point `73`; uneven padding L=14 / R=43 |
| **P3** | High | Large empty bands between sections; rhythm feels uniform and sparse | Adjacent sections each pad top+bottom → stacked gaps; no first/last modifiers |
| **P4** | High | FAQ Preview section is nearly empty (heading + link only) | `FaqPreview.astro` never renders teaser questions; heading still has `margin-bottom: space-6` |
| **P5** | Medium–High | Metadata strip (`JSON SKYSHEET TXT…`) reads as one glued line | Only flex `gap`; no in-item separator/dot; guide requires clearer item separation without slash-at-wrap |
| **P6** | Medium | Button look/target size inconsistent across Header / FinalCta / global | `.button` rules re-declared in scoped component styles, overriding global `min-height: 44px` |
| **P7** | Low–Medium | Steps hotkey note feels detached | `.hotkey-note { margin-top: var(--space-8) }` after list already spaced |
| **P8** | Low–Medium | Technical ledger `dt`/`dd` can feel cramped when wrapping | `justify-content: space-between` + wrap without row gap / min widths |
| **P9** | Low | Duplicate brand SVG files can drift | `public/favicon.svg` and `public/assets/sky-auto-player-mark.svg` are identical copies with no single source rule |
| **P10** | Low | FaqPreview heading margin wastes space when empty (fixed by P4) | Same as P4 residual |

### 0.2 Non-goals (FORBIDDEN)

Executing agent MUST NOT:

1. Change any file under `src/sky_music/`, `tests/` (Python), `installer/`, `updater.bat`, `Sky-Auto-Player.spec`, `pyproject.toml` version, or Python CI workflows **except** if a workflow path filter already lists `site/` (do not expand Python jobs).
2. Delete legacy `docs/*.html` marketing files in this plan (that is PR-2 migration; out of scope).
3. Add React/Vue/Svelte or any client framework.
4. Reintroduce `body { overflow-x: hidden }` to hide layout bugs.
5. Change product claims, ToS risk wording meaning, or invent new marketing claims.
6. Hardcode `/Sky-Auto-Player` string inside components (keep `withBase` / `SITE` helpers).
7. Restyle the entire palette or switch fonts away from Inter Variable + Playfair Display Variable.
8. “Improve” unrelated sections (e.g. rewrite TimingConsole event copy) unless a phase explicitly lists the file.
9. Commit unless the human explicitly asks to commit.
10. Skip verification commands at the end of each phase.

### 0.3 Definition of done (whole plan)

All of the following are true:

- [ ] P1–P9 addressed exactly as specified in phases below.
- [ ] `cd site && bun run check && bun run lint && bun run format:check && bun run build && bun run verify:dist` all exit 0.
- [ ] Manual visual checklist in Phase 7 pass at 320 / 390 / 768 / 1024 / 1280 / 1440.
- [ ] Brand mark optical center appears centered in header at 28×28 and in favicon.
- [ ] Hero kicker gap from header bottom is **≤ 96px** at 1280px wide (target **64–88px**).
- [ ] FAQ Preview shows **exactly 3** teaser questions (EN and VI) plus the full-FAQ link.
- [ ] No Python app tests required; do not run full `uv run pytest` unless agent also touched Python (should not).
- [ ] This plan’s status line remains PROPOSED until human marks implemented; agent may append an “As-built notes” section only if human requests.

### 0.4 Package manager rule for `site/`

This repo’s `site/package.json` declares `"packageManager": "bun@1.3.14"`.

- Use **`bun`** for install/run inside `site/` (`bun install`, `bun run <script>`).
- Do **not** switch to npm/yarn/pnpm or rewrite lockfiles unless a phase explicitly says so.
- If `bun` is missing on the machine, stop and report; do not silently fall back to npm.

### 0.5 Execution protocol (mandatory for every phase)

1. Read the phase **entirely** before editing.
2. Touch **only** files listed in that phase’s “Files allowed”.
3. Apply edits **in the order** listed (Step 1, Step 2, …).
4. Run the phase “Verification” commands; if any fail, fix **only** the regression caused by this phase — do not start the next phase.
5. Do not combine phases into one mega-commit unless human asks; prefer one logical change per phase.
6. If a listed file path does not exist, **stop** and report the path — do not invent replacements.
7. Do not “simplify” tokens or rename CSS variables unless a step says the exact new name.
8. Keep EN/VI parity: any user-visible string change on home must update both `home.en.ts` and `home.vi.ts` as specified.

---

## 1. Current file map (do not rediscover)

Use these paths as absolute-from-repo-root references:

```text
site/
  package.json
  public/
    favicon.svg                          # P2, P9
    favicon.ico                          # regenerate only if Phase 2 says so
    assets/
      sky-auto-player-mark.svg           # P2, P9 (canonical mark)
  src/
    styles/
      tokens.css                         # section space tokens (Phase 3 may adjust)
      utilities.css                      # .section-shell* (Phase 1, 3)
      global.css                         # .button global (Phase 5)
      reset.css                          # DO NOT CHANGE unless a step says so
    components/
      layout/
        SiteHeader.astro                 # brand img, button scoped styles (P2, P6)
        SiteFooter.astro                 # DO NOT CHANGE
      home/
        Hero.astro                       # P1, P5
        FaqPreview.astro                 # P4
        FinalCta.astro                   # P6
        Steps.astro                      # P7
        TechnicalTrust.astro             # P8
        HomePage.astro                   # only if FaqPreview props change
        PlaybackProof.astro              # DO NOT CHANGE unless phase lists it
        ComparisonTable.astro            # DO NOT CHANGE
        ProductView.astro                # DO NOT CHANGE
        Formats.astro                    # DO NOT CHANGE
        TimingConsole.astro              # DO NOT CHANGE
      faq/
        FaqPage.astro                    # P1 (faq-hero padding)
    data/
      home.en.ts                         # P4, P5 if strings
      home.vi.ts                         # P4, P5 if strings
      home.types.ts                      # P4 types
    layouts/
      BaseLayout.astro                   # favicon links only if Phase 2 requires
```

---

## 2. Design targets (numbers — do not freestyle)

### 2.1 Spacing tokens (after Phase 3)

Keep token **names**. Only change values if Phase 3 Step says so. Final intended values:

```css
--section-space-compact: clamp(3rem, 5vw, 4.25rem);
--section-space-default: clamp(4rem, 6.5vw, 5.75rem);
--section-space-feature: clamp(5rem, 8vw, 7.5rem);
```

Rationale: current tokens (`3.5/4.75/6` → `5/7/9` rem max) stack too large between sections. New caps reduce stacked gap without collapsing hierarchy.

### 2.2 First-section (hero) padding — asymmetric

```css
/* Home hero + FAQ hero only */
padding-top: clamp(2.25rem, 3.5vw, 3.25rem);   /* 36–52px typical */
padding-bottom: var(--section-space-feature);
```

**Acceptance at 1280px width:**

| Metric | Current (broken) | Target |
|--------|------------------|--------|
| Hero `padding-top` | 128px | **36–52px** |
| Gap header bottom → `.hero .kicker` top | ~253px | **64–88px** |
| Hero `padding-bottom` | 128px | keep feature token (after Phase 3: ≤ 120px @1280) |

### 2.3 Brand mark geometry (Phase 2)

Final SVG requirements (all must hold):

- `viewBox="0 0 128 128"`.
- Rounded rect background: `width=128 height=128 rx=28` fill `#07090d` (unchanged brand plate).
- Exactly **3** filled circles (nodes) + **1** open polyline/path with `fill="none"`, `stroke-linecap="round"`, `stroke-linejoin="round"`, `stroke-width="5"`, stroke `#ded8cc`.
- Node colors (keep brand): top `#f7dda2`, mid `#b8ccd6`, bottom `#dcae55` (or `#efca78` family already used — use exact values specified in Phase 2 steps).
- **Optical box:** content bbox center X must be in **[62, 66]**; center Y in **[62, 66]**.
- **Padding:** left and right empty margin inside viewBox each ≥ **20px**; top and bottom each ≥ **18px**.
- Path endpoints must equal node centers (same `x,y` as circle `cx,cy`).
- Circle radii: top `r=8`, mid `r=6.5`, bottom `r=6.5` (uniform family; not 9/7/7 unbalanced).

### 2.4 FAQ teaser count

- Exactly **3** items on home FAQ preview.
- Items are static copy in `home.en.ts` / `home.vi.ts` (not live content-collection fetch) to avoid async complexity in `FaqPreview`.
- Each item: `{ question: string; href: string }` where `href` is a **path without base** like `/faq/#download` or `/vi/faq/#download` — component applies `withBase`.

### 2.5 Metadata strip

- Keep the same six labels (EN): `JSON`, `SKYSHEET`, `TXT`, `OPEN SOURCE`, `PORTABLE`, `NO INSTALLER`.
- VI: keep existing translated labels from `home.vi.ts` (do not invent new Vietnamese unless already present).
- Visual: each `<li>` contains a leading dot (CSS `::before` or `<span aria-hidden="true">`) so items never read as one word-soup; **no** `li + li::before { content: "/" }` slash pattern (forbidden by guide).

---

## 3. Phase 0 — Preflight (read-only, no product edits)

### Goal
Prove the workspace matches this plan’s assumptions before changing CSS/SVG.

### Files allowed
- **Read any file under `site/`.**
- **Write:** none (except optional local notes outside git — do not create unsolicited files in the repo).

### Steps

1. Confirm `site/package.json` exists and contains `"packageManager": "bun@1.3.14"`.
2. Confirm these files exist (fail if any missing):
   - `site/src/styles/utilities.css`
   - `site/src/styles/tokens.css`
   - `site/src/styles/global.css`
   - `site/src/components/home/Hero.astro`
   - `site/src/components/home/FaqPreview.astro`
   - `site/src/components/faq/FaqPage.astro`
   - `site/src/components/layout/SiteHeader.astro`
   - `site/src/components/home/FinalCta.astro`
   - `site/src/components/home/Steps.astro`
   - `site/src/components/home/TechnicalTrust.astro`
   - `site/src/data/home.en.ts`
   - `site/src/data/home.vi.ts`
   - `site/src/data/home.types.ts`
   - `site/public/assets/sky-auto-player-mark.svg`
   - `site/public/favicon.svg`
3. Open `site/src/components/home/Hero.astro` and confirm the root section class list includes both `section-shell` and `section-shell--feature`.
4. Open `site/src/styles/utilities.css` and confirm `.section-shell--feature` sets `padding-block: var(--section-space-feature)`.
5. Open `site/public/assets/sky-auto-player-mark.svg` and confirm the path `d` attribute is currently `M25 36 73 58 47 96` (or equivalent three-point path) and circles near `(23,32)`, `(78,58)`, `(47,99)`. If the SVG was already rewritten by a partial earlier attempt, **still apply Phase 2 fully** using the target coordinates in Phase 2 (overwrite to the specified final SVG).
6. Run (must pass before Phase 1):

```powershell
cd site
bun install
bun run build
```

### Verification
- Build exits 0.
- No files modified in git for this phase (`git status` shows clean for site sources, or only unrelated pre-existing dirty files reported to human).

### Stop conditions
- Missing files → stop, list missing paths.
- Build fails before any of our edits → stop, do not “fix forward” by editing unrelated config.

---

## 4. Phase 1 — Fix first-section gap under topbar (P1)

### Goal
Reduce the empty region between the sticky/static header and the first content (home hero + FAQ hero) without shrinking the bottom feature breathing room of those sections.

### Files allowed
1. `site/src/styles/utilities.css`
2. `site/src/components/home/Hero.astro`
3. `site/src/components/faq/FaqPage.astro`

### Do NOT touch
- `tokens.css` in this phase (token value changes are Phase 3).
- Any other home section components.

### Design decision (locked)
Introduce a **layout utility** for first sections under the header:

```css
.section-shell--after-header {
  padding-top: clamp(2.25rem, 3.5vw, 3.25rem);
}
```

Rules:

- This class **only overrides `padding-top`**.
- `padding-bottom` continues to come from `.section-shell` / `--compact` / `--feature` as already applied on the element.
- Because both `.section-shell--feature` and `.section-shell--after-header` set padding properties, **source order in `utilities.css` must place `.section-shell--after-header` AFTER `.section-shell--feature`** so `padding-top` wins. Do not use `!important`.

### Step 1 — Add utility in `utilities.css`

Open `site/src/styles/utilities.css`.

Inside `@layer layout { ... }`, **after** the existing `.section-shell--feature` block, append exactly:

```css
  /* First section under site header: pull content up without killing feature bottom space. */
  .section-shell--after-header {
    padding-top: clamp(2.25rem, 3.5vw, 3.25rem);
  }
```

Do not delete or rename existing `.section-shell`, `--compact`, or `--feature` rules in this phase.

### Step 2 — Home hero class list

Open `site/src/components/home/Hero.astro`.

Find:

```astro
<section class="hero section-shell section-shell--feature" aria-labelledby="hero-title">
```

Replace with:

```astro
<section
  class="hero section-shell section-shell--feature section-shell--after-header"
  aria-labelledby="hero-title"
>
```

Keep the existing scoped `<style>` block. In that block, `.hero { border-top: 0; }` must remain. **Do not** add a second `padding-top` override inside Hero scoped CSS (single source = utility class).

### Step 3 — FAQ hero class list

Open `site/src/components/faq/FaqPage.astro`.

Find the FAQ hero section (currently similar to):

```astro
<section class="faq-hero section-shell section-shell--feature" aria-labelledby="faq-title">
```

Replace with:

```astro
<section
  class="faq-hero section-shell section-shell--feature section-shell--after-header"
  aria-labelledby="faq-title"
>
```

Do not change FAQ content, structured data, or final CTA in this phase.

### Step 4 — Optional defensive scoped note (only if specificity fights)

If after build/preview the hero still shows feature-sized top padding, check compiled order. Astro scoped styles should not set padding on `.hero`. If someone previously added padding on `.hero`, **remove that padding property only** — do not add `!important` on the utility.

### Verification (Phase 1)

```powershell
cd site
bun run build
```

Manual (required):

1. `bun run preview -- --host 127.0.0.1 --port 4321`
2. Open `http://127.0.0.1:4321/Sky-Auto-Player/`
3. In DevTools, select `section.hero` and confirm computed `padding-top` is between **36px and 52px** at viewport width 1280 (approximately; fluid clamp may land ~42–52px).
4. Confirm computed `padding-bottom` is still the feature size (still large; will shrink slightly only in Phase 3).
5. Open `http://127.0.0.1:4321/Sky-Auto-Player/faq/` and confirm FAQ H1 is similarly closer to the header.
6. Confirm header border and layout still one row on desktop.

### Acceptance
- [ ] Home hero no longer has a large black void under the topbar.
- [ ] FAQ hero matches the same top treatment.
- [ ] No horizontal scroll introduced.
- [ ] No other sections changed.

---

## 5. Phase 2 — Rebuild brand mark SVG (P2, P9)

### Goal
Replace the left-biased constellation mark with a geometrically centered mark whose path endpoints coincide with node centers. Keep both public SVG paths in sync (single content, two files — no new build pipeline).

### Files allowed
1. `site/public/assets/sky-auto-player-mark.svg` — **canonical content**
2. `site/public/favicon.svg` — **must be byte-identical** to the mark after this phase (same SVG markup)
3. `site/src/components/layout/SiteHeader.astro` — only if `width`/`height` attributes need to stay 28 (they already are; usually **no edit**)
4. `site/src/layouts/BaseLayout.astro` — only if icon `link` rel paths are wrong (usually **no edit**)

### Do NOT touch
- `favicon.ico` binary in this phase unless the human explicitly asks. SVG favicon is primary (`rel=icon type=image/svg+xml`). Leaving `.ico` stale is acceptable for this plan; note it in as-built if unchanged.
- Do not convert the header `<img>` to inline SVG in this phase (keeps cache + simple).
- Do not change brand wordmark text “Sky Auto Player”.

### Locked final SVG markup

Overwrite **both** `sky-auto-player-mark.svg` and `favicon.svg` with **exactly** the following file contents (including newline at end of file):

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" role="img" aria-labelledby="title">
  <title id="title">Sky Auto Player</title>
  <rect width="128" height="128" rx="28" fill="#07090d"/>
  <!--
    Centered constellation (optical center ~64,64).
    Path endpoints == node centers.
    Content bbox approx x:34–94 (pad L/R 34/34), y:26–100 (pad T/B 26/28).
  -->
  <path
    d="M40 34 L88 58 L52 94"
    fill="none"
    stroke="#ded8cc"
    stroke-width="5"
    stroke-linecap="round"
    stroke-linejoin="round"
  />
  <circle cx="40" cy="34" r="8" fill="#f7dda2"/>
  <circle cx="88" cy="58" r="6.5" fill="#b8ccd6"/>
  <circle cx="52" cy="94" r="6.5" fill="#dcae55"/>
</svg>
```

### Geometry checklist (must verify mathematically before closing phase)

Compute:

- Nodes: `(40,34) r=8`, `(88,58) r=6.5`, `(52,94) r=6.5`
- BBox left = `min(40-8, 88-6.5, 52-6.5)` = `min(32, 81.5, 45.5)` = **32**
- BBox right = `max(40+8, 88+6.5, 52+6.5)` = `max(48, 94.5, 58.5)` = **94.5**
- BBox top = `min(34-8, 58-6.5, 94-6.5)` = `min(26, 51.5, 87.5)` = **26**
- BBox bottom = `max(34+8, 58+6.5, 94+6.5)` = `max(42, 64.5, 100.5)` = **100.5**
- Center X = `(32+94.5)/2` = **63.25** (within [62,66] ✓)
- Center Y = `(26+100.5)/2` = **63.25** (within [62,66] ✓)
- Pad L = 32 ≥ 20 ✓, Pad R = 128-94.5 = 33.5 ≥ 20 ✓
- Path points equal node centers ✓

If an agent “improves” coordinates, they **violate this plan**. Use the locked markup only.

### Step 1 — Write mark file

Write the locked SVG to `site/public/assets/sky-auto-player-mark.svg` (full replace).

### Step 2 — Sync favicon.svg

Copy the same markup to `site/public/favicon.svg` (full replace).  
PowerShell:

```powershell
Copy-Item -Force site/public/assets/sky-auto-player-mark.svg site/public/favicon.svg
```

### Step 3 — Confirm header still references mark

In `SiteHeader.astro`, brand image must remain:

```astro
src={withBase('/assets/sky-auto-player-mark.svg')}
width="28"
height="28"
```

Do not change `class="brand__mark"` sizing CSS (`width: 28px; height: 28px`).

### Step 4 — Confirm BaseLayout icon links

`BaseLayout.astro` should still include:

- `rel="icon" type="image/svg+xml"` → `withBase('/assets/sky-auto-player-mark.svg')` **or** favicon.svg — **keep existing hrefs**; do not churn URLs.
- `rel="apple-touch-icon"` → existing favicon.svg path.

No edit required if paths already resolve under `public/`.

### Step 5 — Visual check

1. Hard-refresh browser (cache may pin old SVG).
2. At 28×28 in header: constellation must not sit in the left half of the rounded square.
3. Open mark URL directly: `/Sky-Auto-Player/assets/sky-auto-player-mark.svg`.

### Verification (Phase 2)

```powershell
cd site
bun run build
# Optional: prove the two SVGs match
fc /b public\assets\sky-auto-player-mark.svg public\favicon.svg
# On Unix-like: cmp public/assets/sky-auto-player-mark.svg public/favicon.svg
```

### Acceptance
- [ ] Mark no longer optically left-heavy.
- [ ] Path meets all three node centers.
- [ ] Both SVG files identical.
- [ ] Header brand alignment with wordmark unchanged (flex gap still `var(--space-2)`).

### Explicit anti-patterns
- Do not add filters, glows, gradients, or fourth nodes.
- Do not use `currentColor` in this phase (mark sits on dark plate with fixed fills for favicon contrast).
- Do not change `rx` from 28.

---

## 6. Phase 3 — Section rhythm tokens + stacked-gap reduction (P3)

### Goal
Reduce oversized empty bands between sections while preserving compact/default/feature hierarchy. Do **not** invent a new layout system; only retune tokens and keep Phase 1 first-section utility.

### Files allowed
1. `site/src/styles/tokens.css`
2. `site/src/styles/utilities.css` — only if a comment update is needed; **prefer no structural change** beyond what Phase 1 already added

### Do NOT touch
- Component-level `margin-top: var(--space-8)` fixes that belong to Phase 5 (Steps hotkey).
- Do not remove `border-top` from `.section-shell` (section separators stay).

### Step 1 — Replace section space token values

Open `site/src/styles/tokens.css`.

Find the three declarations (current values may match):

```css
    --section-space-compact: clamp(3.5rem, 6vw, 5rem);
    --section-space-default: clamp(4.75rem, 8vw, 7rem);
    --section-space-feature: clamp(6rem, 10vw, 9rem);
```

Replace **only those three lines** with:

```css
    --section-space-compact: clamp(3rem, 5vw, 4.25rem);
    --section-space-default: clamp(4rem, 6.5vw, 5.75rem);
    --section-space-feature: clamp(5rem, 8vw, 7.5rem);
```

Do not change color tokens, font tokens, `--space-1`…`--space-9`, or motion tokens in this phase.

### Step 2 — Confirm shell modifiers still map correctly

In `utilities.css`, leave:

```css
  .section-shell {
    padding-block: var(--section-space-default);
    border-top: 1px solid var(--border-subtle);
  }

  .section-shell--compact {
    padding-block: var(--section-space-compact);
  }

  .section-shell--feature {
    padding-block: var(--section-space-feature);
  }

  .section-shell--after-header {
    padding-top: clamp(2.25rem, 3.5vw, 3.25rem);
  }
```

If Phase 1 was done correctly, no edit needed here.

### Step 3 — Sanity: which sections use which shell (do not change classes in this phase)

| Section component | Expected classes after Phases 1–3 |
|-------------------|-----------------------------------|
| Hero | `section-shell section-shell--feature section-shell--after-header` |
| PlaybackProof | `section-shell` (default) |
| ComparisonTable | `section-shell section-shell--compact` |
| ProductView | `section-shell section-shell--feature` |
| Steps | `section-shell` (default) |
| TechnicalTrust | `section-shell` (default) |
| Formats | `section-shell section-shell--compact` |
| FaqPreview | `section-shell section-shell--compact` (content filled in Phase 4) |
| FinalCta | `section-shell section-shell--feature` |
| FAQ page hero | `section-shell section-shell--feature section-shell--after-header` |
| FAQ page final CTA | `section-shell section-shell--feature` |

If any home section is missing `section-shell` entirely, **report** — do not silently reclassify in this phase.

### Verification (Phase 3)

```powershell
cd site
bun run build
```

Manual at 1280 width:

1. Scroll home: gaps between section **content blocks** should feel tighter than pre-change but still show clear separation + top borders.
2. Hero top gap still within Phase 1 target (after-header wins on padding-top).
3. Final CTA still has substantial presence (feature padding).

### Acceptance
- [ ] Token values updated exactly as specified.
- [ ] No component class churn in this phase.
- [ ] Hierarchy compact < default < feature still true at max clamp (4.25 < 5.75 < 7.5 rem).

---

## 7. Phase 4 — Metadata strip separators + FAQ Preview content (P5, P4, P10)

### Goal
1. Make hero metadata items visually separable without slash-wrap bugs.
2. Fill FAQ Preview with three teaser questions + keep “read full FAQ” link.
3. Remove dead bottom margin when the section has real content (structure spacing correctly).

### Files allowed
1. `site/src/data/home.types.ts`
2. `site/src/data/home.en.ts`
3. `site/src/data/home.vi.ts`
4. `site/src/components/home/Hero.astro`
5. `site/src/components/home/FaqPreview.astro`
6. `site/src/components/home/HomePage.astro` — **only if** prop wiring must change (prefer FaqPreview keep `data={content.faqPreview}`)

### Do NOT touch
- FAQ content collection Markdown under `site/src/content/faq/**` (teasers are home locale data only).
- Comparison / playback copy.

---

### Part A — Types

Open `site/src/data/home.types.ts`.

Locate the `HomeContent` (or equivalent) interface fields for `hero` and `faqPreview`.

#### A1. Hero metadata

Keep `metadata: string[]` **unchanged** (labels stay strings). Visual dots are pure CSS in Hero — no type change required for metadata.

#### A2. FAQ preview shape

Replace the existing `faqPreview` type (whatever it currently is — typically `{ kicker, title, readMoreLink }`) with:

```ts
faqPreview: {
  kicker: string;
  title: string;
  readMoreLink: string;
  items: ReadonlyArray<{
    question: string;
    /** Locale path without origin; may include hash. Example: "/faq/#download" */
    href: string;
  }>;
};
```

If `HomeContent` is defined as a single interface, edit that field only. Do not restructure unrelated fields.

---

### Part B — English locale data (`home.en.ts`)

#### B1. Hero metadata

Keep the existing six strings. Do not reorder:

```ts
metadata: ['JSON', 'SKYSHEET', 'TXT', 'OPEN SOURCE', 'PORTABLE', 'NO INSTALLER'],
```

#### B2. FAQ preview items

Update `faqPreview` to:

```ts
  faqPreview: {
    kicker: 'Before you download',
    title: 'A few useful answers first.',
    readMoreLink: 'Read the full FAQ',
    items: [
      {
        question: 'Is Sky Auto Player free and open source?',
        href: '/faq/#free',
      },
      {
        question: 'Which sheet formats are supported?',
        href: '/faq/#formats',
      },
      {
        question: 'Can this affect my Sky account?',
        href: '/faq/#account-safety',
      },
    ],
  },
```

**Hash IDs must match FAQ entry `key` values** already used as `id={faq.data.key}` in `FaqItem.astro`. Before locking hashes, open `site/src/content/faq/en/*.md` frontmatter and confirm keys:

| Expected key | File (typical) |
|--------------|----------------|
| `free` | `free.md` |
| `formats` | `formats.md` |
| `account-safety` | `account-safety.md` |

If a key differs (e.g. `account_safety`), use the **actual** `key:` from frontmatter. Do not create new FAQ markdown in this phase.

If current `faqPreview.kicker` / `title` / `readMoreLink` already match the strings above, keep them and only add `items`.

---

### Part C — Vietnamese locale data (`home.vi.ts`)

Mirror structure. Use these strings (locked):

```ts
  faqPreview: {
    kicker: 'Trước khi tải',
    title: 'Một vài câu trả lời hữu ích.',
    readMoreLink: 'Đọc toàn bộ FAQ',
    items: [
      {
        question: 'Sky Auto Player có miễn phí và mã nguồn mở không?',
        href: '/vi/faq/#free',
      },
      {
        question: 'Hỗ trợ những định dạng sheet nào?',
        href: '/vi/faq/#formats',
      },
      {
        question: 'Việc dùng tool có ảnh hưởng tài khoản Sky không?',
        href: '/vi/faq/#account-safety',
      },
    ],
  },
```

If existing VI kicker/title/readMoreLink are already good and only missing `items`, keep existing kicker/title/readMoreLink text **as already in file** and only add `items` with the three questions above (or the file’s existing tone if questions already exist — prefer locked strings above for consistency).

Hashes must use the same keys as VI FAQ frontmatter (`site/src/content/faq/vi/*.md`).

---

### Part D — Hero metadata CSS (P5)

Open `site/src/components/home/Hero.astro`.

#### D1. Markup

Current:

```astro
<ul class="metadata-strip" aria-label="Supported packaging and sheet metadata">
  {data.metadata.map((item) => <li>{item}</li>)}
</ul>
```

Replace with:

```astro
<ul class="metadata-strip" aria-label="Supported packaging and sheet metadata">
  {
    data.metadata.map((item) => (
      <li>
        <span class="metadata-strip__dot" aria-hidden="true" />
        <span>{item}</span>
      </li>
    ))
  }
</ul>
```

For FAQ/i18n: if `locale === 'vi'`, change `aria-label` to a Vietnamese equivalent:

```ts
aria-label={locale === 'vi' ? 'Định dạng sheet và thông tin đóng gói' : 'Supported packaging and sheet metadata'}
```

(`locale` is already a prop on Hero.)

#### D2. CSS — replace metadata rules

Find:

```css
  .metadata-strip {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    margin-bottom: var(--space-4);
  }
  .metadata-strip li {
    display: flex;
    align-items: center;
  }
```

Replace with:

```css
  .metadata-strip {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-5);
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    margin-bottom: var(--space-4);
    list-style: none;
    padding: 0;
    margin-left: 0;
  }
  .metadata-strip li {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 1.25rem;
  }
  .metadata-strip__dot {
    width: 0.35rem;
    height: 0.35rem;
    border-radius: 50%;
    background: var(--color-accent);
    flex: 0 0 auto;
    opacity: 0.85;
  }
```

**Forbidden:** `li + li::before { content: "/"; }` or any slash separator.

Note: `reset.css` may not zero `ul` margins for non-`role=list` lists. Setting `list-style/padding/margin` on `.metadata-strip` is required.

---

### Part E — Rewrite `FaqPreview.astro` (P4)

Replace the component implementation with the following behavior (structure locked; class names locked).

#### E1. Frontmatter / script

```astro
---
import type { HomeContent } from '../../data/home.types';
import { withBase } from '../../utils/urls';

interface Props {
  data: HomeContent['faqPreview'];
  locale: 'en' | 'vi';
}
const { data, locale } = Astro.props;
const isVi = locale === 'vi';
const faqUrl = withBase(isVi ? '/vi/faq/' : '/faq/');
---
```

#### E2. Body

```astro
<section
  id="faq-preview"
  class="section-shell section-shell--compact"
  aria-labelledby="faq-preview-title"
>
  <div class="container">
    <div class="section-heading section-heading--split">
      <div>
        <p class="kicker">{data.kicker}</p>
        <h2 id="faq-preview-title" class="section-title">{data.title}</h2>
      </div>
      <a class="text-link" href={faqUrl}>{data.readMoreLink}</a>
    </div>

    <ul class="faq-teasers">
      {
        data.items.map((item) => (
          <li>
            <a class="faq-teasers__link" href={withBase(item.href)}>
              <span class="faq-teasers__q">{item.question}</span>
              <span class="faq-teasers__go" aria-hidden="true">
                →
              </span>
            </a>
          </li>
        ))
      }
    </ul>
  </div>
</section>
```

#### E3. Styles (replace entire `<style>` block)

```css
  .section-heading--split {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: flex-end;
    gap: var(--space-4);
    margin-bottom: var(--space-6);
  }
  .kicker {
    font-weight: 700;
    color: var(--color-text-muted);
    text-transform: uppercase;
    font-size: var(--font-size-xs);
    letter-spacing: 0.1em;
    margin-bottom: var(--space-2);
  }
  .section-title {
    margin: 0;
  }
  .text-link {
    color: var(--color-accent);
    text-decoration: underline;
    font-weight: 600;
  }
  .faq-teasers {
    list-style: none;
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--border-default);
    max-width: 52rem;
  }
  .faq-teasers li {
    border-bottom: 1px solid var(--border-subtle);
  }
  .faq-teasers__link {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding-block: var(--space-4);
    color: var(--color-text);
    text-decoration: none;
    min-height: 44px;
  }
  .faq-teasers__link:hover .faq-teasers__q {
    color: var(--color-accent-strong);
  }
  .faq-teasers__q {
    min-width: 0;
    font-weight: 600;
    font-size: var(--font-size-body);
    line-height: 1.35;
    transition: color var(--motion-fast) var(--ease-out);
  }
  .faq-teasers__go {
    flex: 0 0 auto;
    color: var(--color-accent);
    font-family: var(--font-mono);
  }
```

#### E4. HomePage wiring

Confirm `HomePage.astro` still has:

```astro
<FaqPreview data={content.faqPreview} locale={content.locale} />
```

No change if already correct.

### Verification (Phase 4)

```powershell
cd site
bun run check
bun run build
```

Manual:

1. Home EN: metadata shows six items each with a gold/accent dot; wrapping does not produce a leading slash.
2. Home EN: FAQ preview lists three questions; each link navigates to FAQ with correct hash (spot-check one).
3. Home VI: three Vietnamese questions; hrefs under `/vi/faq/`.
4. Section no longer looks empty.

### Acceptance
- [ ] Types compile (`bun run check`).
- [ ] EN + VI both have `items.length === 3`.
- [ ] Metadata dots present; no slash pseudo.
- [ ] Teaser links use `withBase`.

---

## 8. Phase 5 — Button unification + micro spacing polish (P6, P7, P8)

### Goal
1. Stop re-declaring `.button` in component scoped CSS; use global button styles from `global.css`.
2. Tighten Steps hotkey note attachment.
3. Improve Technical ledger wrap gap.

### Files allowed
1. `site/src/styles/global.css` — only if a small gap exists vs requirements below; prefer **read + confirm** then minimal edit
2. `site/src/components/layout/SiteHeader.astro`
3. `site/src/components/home/FinalCta.astro`
4. `site/src/components/faq/FaqPage.astro` — only the Final CTA button CSS duplicate if present
5. `site/src/components/home/Steps.astro`
6. `site/src/components/home/TechnicalTrust.astro`

### Do NOT touch
- Hero CTAs markup (already uses `button button--primary` / `button--secondary` without local redefinition — leave as is).
- Menu toggle styles in header (not `.button`).

---

### Part A — Confirm global button contract

Open `site/src/styles/global.css` `@layer components` and ensure `.button` includes **all** of:

- `display: inline-flex; align-items: center; justify-content: center;`
- `min-height: 44px;`
- `padding: 0.7rem 1.15rem;`
- `border: 1px solid transparent; border-radius: var(--radius-control);`
- `font-weight: 650;` (or 600 — **keep existing global value**, do not bikeshed)
- `font-size: var(--font-size-sm);`
- primary / secondary / small variants as already present

If `min-height: 44px` is missing, add it. Do not rename classes.

`.button--small` must keep `min-height: 40px` (header density) as already defined.

---

### Part B — Strip duplicate button CSS from SiteHeader

Open `site/src/components/layout/SiteHeader.astro` `<style>` block.

**Delete** the entire local rule blocks for:

- `.button`
- `.button--primary` (+ hover)
- `.button--secondary` (+ hover)

**Keep**:

- `.site-header`, `.site-header__inner`, `.brand`, `.brand__mark`, `.site-nav`, `.site-actions`
- `.button--small` **only if** header needs a local override — prefer relying on global `.button--small`. If global already has `.button--small`, delete local too.
- `.header-download` usage on the anchor: class list must remain  
  `class="button button--primary button--small header-download"`
- menu toggle rules, locale links, media queries

Markup for Download CTA must still include global classes: `button button--primary button--small`.

---

### Part C — Strip duplicate button CSS from FinalCta

Open `site/src/components/home/FinalCta.astro`.

**Delete** local `.button`, `.button--primary`, `.button--secondary` (and hovers) from `<style>`.

**Keep** layout rules:

- `.final-cta`, `.final-cta__layout`, `h2`, `p`, `.final-cta__actions`

Markup already uses `class="button button--primary"` / `button--secondary` — leave markup.

---

### Part D — FAQ page final CTA buttons

Open `site/src/components/faq/FaqPage.astro`.

If its scoped CSS redefines `.button` / variants (some copies paste FinalCta styles), **delete those button rules** the same way. Keep `.final-cta` layout rules.

If FAQ page relies on global buttons only and has no `.button` CSS, skip.

---

### Part E — Steps hotkey note (P7)

Open `site/src/components/home/Steps.astro`.

Find:

```css
  .hotkey-note {
    max-width: 58rem;
    margin-top: var(--space-8);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border-subtle);
    ...
  }
```

Change **only** `margin-top` from `var(--space-8)` to `var(--space-5)`:

```css
    margin-top: var(--space-5);
```

Also check `.steps` rule: if it has `margin: 0 0 var(--space-6);` leave it. Do not change step row padding.

---

### Part F — Technical ledger wrap (P8)

Open `site/src/components/home/TechnicalTrust.astro`.

Find `.ledger-row`:

```css
  .ledger-row {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    padding-block: var(--space-3);
    border-bottom: 1px solid var(--border-subtle);
  }
```

Replace with:

```css
  .ledger-row {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--space-2) var(--space-6);
    padding-block: var(--space-3);
    border-bottom: 1px solid var(--border-subtle);
  }
  .ledger-row dt {
    font-weight: 600;
    min-width: 8rem;
  }
  .ledger-row dd {
    color: var(--color-text-subtle);
    margin: 0;
    text-align: right;
    flex: 1 1 auto;
    min-width: 10rem;
  }
```

If `.ledger-row dt` / `dd` rules already exist below, **merge** into the above (do not leave duplicate selectors with conflicting values). Ensure `dd { margin: 0 }` because reset may not clear `dd` margin in all browsers.

### Verification (Phase 5)

```powershell
cd site
bun run check
bun run lint
bun run build
```

Manual:

1. Header Download button still gold, readable, ≥40px tall (`button--small`).
2. Hero primary button ≥44px tall.
3. Final CTA buttons match hero styling (same padding language).
4. Steps hotkey line closer to step list.
5. Technical table: on a narrow desktop width (~900px), dt/dd wrap with visible gap, not colliding.

### Acceptance
- [ ] No scoped redefinition of `.button` base in Header/FinalCta/FaqPage.
- [ ] Global `.button` remains single source of truth.
- [ ] P7/P8 CSS values applied exactly.

---

## 9. Phase 6 — Automated quality gate (must pass)

### Goal
Prove the site still builds cleanly after all polish phases.

### Files allowed
None (read-only / commands only).

### Steps — run exactly

```powershell
cd site
bun install
bun run check
bun run lint
bun run format:check
bun run build
bun run verify:dist
```

If `format:check` fails only on files this plan touched, run:

```powershell
bun run format
bun run format:check
```

Do **not** format the entire monorepo outside `site/`.

### Optional e2e (run if Playwright browsers already installed and previous suite was green on branch)

```powershell
cd site
bun run test:e2e
```

If e2e fails due to environment (missing browser) and not due to selectors, report environment failure separately. If e2e fails because FAQ preview or header selectors broke, **fix the regression** in the allowed files from Phases 1–5 only.

### Acceptance
- [ ] `check`, `lint`, `format:check`, `build`, `verify:dist` all exit 0.
- [ ] Dist still contains EN/VI home + FAQ routes under base path.

---

## 10. Phase 7 — Manual visual acceptance matrix (human or agent with browser)

### Goal
Close P1–P9 with eyes, not only build codes.

### Preview

```powershell
cd site
bun run build
bun run preview -- --host 127.0.0.1 --port 4321
```

Base URL: `http://127.0.0.1:4321/Sky-Auto-Player/`

### Checklist by viewport

For each width in `{320, 390, 768, 1024, 1280, 1440}`:

| # | Check | Pass criteria |
|---|--------|----------------|
| V1 | No horizontal page scroll | Document width ≤ viewport |
| V2 | Header one intentional row | Brand + actions visible; at ≤420px Download may hide per existing CSS; menu toggle present below 64rem |
| V3 | Hero top gap | At 1280: kicker not floating mid-void; `padding-top` on `.hero` ≈ 36–52px |
| V4 | Brand mark | Constellation centered in rounded square at 28px |
| V5 | Metadata | Dots visible; items wrap cleanly; no leading `/` |
| V6 | Section rhythm | No double-desert gaps; borders still separate sections |
| V7 | FAQ preview | Exactly 3 teasers + full FAQ link; links work |
| V8 | Buttons | Primary/secondary consistent; targets comfortable |
| V9 | Technical ledger | dt/dd readable when wrapped |
| V10 | Steps hotkey | Note near list, not stranded |
| V11 | FAQ page hero | Same tightened top gap as home |
| V12 | VI home | Same layout; Vietnamese strings; teaser hrefs under `/vi/faq/` |

### DevTools measurement script (optional, paste in console at 1280×900)

```js
(() => {
  const header = document.querySelector('.site-header')?.getBoundingClientRect();
  const kicker = document.querySelector('.hero .kicker')?.getBoundingClientRect();
  const hero = document.querySelector('.hero');
  const cs = hero ? getComputedStyle(hero) : null;
  return {
    headerH: header && Math.round(header.height),
    heroPadTop: cs && cs.paddingTop,
    heroPadBottom: cs && cs.paddingBottom,
    gapHeaderToKicker: header && kicker ? Math.round(kicker.top - header.bottom) : null,
  };
})();
```

**Pass numbers at width 1280:**

- `heroPadTop` ∈ [36px, 52px] (approx; allow ±4px for subpixel)
- `gapHeaderToKicker` ∈ [64, 96]

### Screenshot recommendation (for PR description)

Capture:

1. Home top (header + hero) @1280
2. Brand mark crop
3. FAQ preview section
4. Full page @1280 optional

Do not commit screenshot binaries into the repo unless human asks.

---

## 11. Phase order summary (execute strictly)

| Order | Phase | Primary IDs |
|------:|-------|-------------|
| 0 | Preflight | — |
| 1 | After-header spacing utility + hero/FAQ classes | P1 |
| 2 | Brand mark SVG rebuild + favicon sync | P2, P9 |
| 3 | Section space token retune | P3 |
| 4 | Metadata dots + FAQ teasers data/UI | P4, P5, P10 |
| 5 | Button de-dupe + steps/ledger micro polish | P6, P7, P8 |
| 6 | Automated gates | — |
| 7 | Manual visual matrix | all |

**Do not skip Phase 0.**  
**Do not start Phase 4 before Phase 1** (spacing regressions harder to judge).  
Phase 2 may run before or after Phase 1, but **preferred order is 1 → 2 → 3 → 4 → 5**.

---

## 12. File touch inventory (final)

| File | Phases |
|------|--------|
| `site/src/styles/utilities.css` | 1 (required), 3 (usually no-op) |
| `site/src/components/home/Hero.astro` | 1, 4 |
| `site/src/components/faq/FaqPage.astro` | 1, 5 |
| `site/public/assets/sky-auto-player-mark.svg` | 2 |
| `site/public/favicon.svg` | 2 |
| `site/src/styles/tokens.css` | 3 |
| `site/src/data/home.types.ts` | 4 |
| `site/src/data/home.en.ts` | 4 |
| `site/src/data/home.vi.ts` | 4 |
| `site/src/components/home/FaqPreview.astro` | 4 |
| `site/src/styles/global.css` | 5 (only if min-height missing) |
| `site/src/components/layout/SiteHeader.astro` | 5 |
| `site/src/components/home/FinalCta.astro` | 5 |
| `site/src/components/home/Steps.astro` | 5 |
| `site/src/components/home/TechnicalTrust.astro` | 5 |
| `docs/plan/2026-07-26_pages-ui-spacing-and-brand-mark-polish-plan.md` | this document (human/plan author) |
| `docs/INDEX.md` | plan author adds Active References entry |

**Explicitly forbidden files for implementing agents:** anything under `src/sky_music/`, `installer/`, Python tests, `docs/index.html` legacy, workflow permission broadening.

---

## 13. Commit guidance (only if human requests commits)

Use conventional commits, one phase per commit preferred:

1. `fix(site): tighten hero and FAQ top spacing under header`
2. `fix(site): center brand mark constellation geometry`
3. `refactor(site): retune section spacing tokens`
4. `feat(site): add FAQ teasers and metadata item dots`
5. `refactor(site): unify button styles and micro spacing polish`

Do not mix Python and site commits.

---

## 14. Rollback

If a phase fails review:

1. `git checkout -- <files of that phase only>`
2. Re-run `bun run build`
3. Do not roll back unrelated phases

SVG rollback: restore previous mark from git history for both SVG paths together.

---

## 15. As-built notes (agent fills only if human asks)

```text
Date:
Agent:
Phases completed:
Deviations (must be empty or human-approved):
verify:dist result:
Manual matrix result:
Remaining known issues:
```

---

## 16. Quick “no thinking” command block (copy/paste after all edits)

```powershell
cd site
bun install
bun run check
bun run lint
bun run format
bun run format:check
bun run build
bun run verify:dist
bun run preview -- --host 127.0.0.1 --port 4321
```

Then complete Phase 7 checklist in the browser.

---
