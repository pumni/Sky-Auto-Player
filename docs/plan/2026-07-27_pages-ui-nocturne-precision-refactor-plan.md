# 2026-07-27 — Pages UI Nocturne Precision Refactor Plan

> **Status:** PROPOSED — executable implementation plan for an AI coding agent.
> **Scope:** Astro static marketing website under `site/` only.
> **Created from:** UI review of HEAD `2e40d2b459fa8572bb4d85bc18b2e989b052e52b` against the UI Refactor Playbook baseline `aca9c5417f25c8c4756e0a8c4767da3b20e0d2f8`.
> **Design direction:** Nocturne Precision — *Celestial score meets timing instrument*.
> **Execution style:** One phase at a time. Each phase is independently verifiable and should be committed separately only when the human explicitly asks for commits.

## 0. How to use this plan

This is a proposal, not a new source of truth. `AGENTS.md`, the normative architecture documents, current code contracts, and existing tests win whenever this plan conflicts with them. The older proposal [`2026-07-26_pages-ui-spacing-and-brand-mark-polish-plan.md`](2026-07-26_pages-ui-spacing-and-brand-mark-polish-plan.md) remains historical/parallel context; do not silently combine its edits with this plan. If both plans touch the same file, this plan must first re-check the current file and explicitly record which behavior is retained.

The implementing agent must follow this loop for every phase:

1. Read the entire phase, including its stop conditions, before editing.
2. Confirm the listed files exist and inspect the nearby current code.
3. Change only the files allowed by that phase.
4. Run that phase's verification commands immediately.
5. If verification fails, fix only the regression introduced by that phase; do not continue forward.
6. Record the result in the phase's checklist and keep the plan status `PROPOSED` unless the human asks for an as-built update.
7. Do not create or commit screenshot binaries unless the human explicitly requests them.

### Security boundary — immutable

This plan is UI-only and must never expand into the Python playback application. The agent must not modify game files, read game memory, attach/debug another process, add hooks or injection, bypass anti-cheat, or introduce any input mechanism other than Windows `SendInput` in the existing platform boundary. Do not touch `src/sky_music/`, `installer/`, `updater.bat`, release packaging, or security-audit rules for this website refactor.

## 1. Baseline and observed acceptance failures

### 1.1 Current repository facts

- Current HEAD: `2e40d2b459fa8572bb4d85bc18b2e989b052e52b`.
- Playbook baseline: `aca9c5417f25c8c4756e0a8c4767da3b20e0d2f8`.
- Site source: `site/`.
- Stack: Astro static output, Bun `1.3.14`, Playwright, Axe.
- Required routes: `/`, `/vi/`, `/faq/`, `/vi/faq/`, plus legacy FAQ redirects.
- Existing source already contains semantic tokens, a timing console, actual picker screenshots, a product showcase, and shared presentation CSS.

### 1.2 Review evidence that drives this plan

The following are confirmed defects, not optional polish:

| ID | Severity | Evidence | Required outcome |
|---|---|---|---|
| R1 | Blocker | Playwright reports horizontal overflow at 320, 360, 390, 768, 912, 1024, 1280 and 1440px. | `body.scrollWidth` and `documentElement.scrollWidth` must be no more than `viewport + 1px` at every required width. |
| R2 | Blocker | Mobile visual snapshots do not exist; desktop screenshot is not stable and differs by about 5%. | Stable, intentional visual baselines for EN/VI and the required viewport matrix. |
| R3 | High | Hero copy, console, and rail are sibling blocks; the rail does not visibly connect headline → event → key state. | Hero must read as one timing composition without text overlap. |
| R4 | High | Most sections use the same border + intro + two-column utility pattern. | Preserve information order while varying density, alignment, surface and visual weight. |
| R5 | Medium | `--type-micro` starts at 11.52px and is used for several readable metadata labels. | Readable labels must meet the 12px floor; only truly decorative metadata may remain smaller. |
| R6 | Medium | VI visual proof is missing from the committed screenshot contract. | EN/VI typography, wrapping, links and controls must be verified at 320/390px. |

### 1.3 What is already acceptable and must be preserved

- Warm gold remains the primary action/timing accent.
- Dark blue-black nocturnal palette remains the page foundation.
- Timing vocabulary remains: timestamp, chord, note, hold, playhead and frame.
- Actual picker screenshot remains the primary product proof; do not replace it with fabricated HTML UI.
- Existing route, i18n, canonical, hreflang, sitemap, structured data, base-path and ToS warning contracts remain unchanged.
- The illustrative timing console may remain a designed demo because the playbook explicitly specifies it; it must stay clearly labeled as an illustration and must not pretend to be a live product screenshot.

## 2. Scope boundaries and forbidden shortcuts

### In scope

- `site/src/components/` home, layout and UI presentation components.
- `site/src/styles/` tokens, layout, patterns, global and motion-related CSS where needed.
- `site/src/data/home.en.ts`, `site/src/data/home.vi.ts`, and types only when layout needs existing content represented explicitly.
- `site/tests/e2e/` visual, responsive, keyboard and accessibility coverage for the site.
- This proposal file and, if needed, one `docs/INDEX.md` active-reference entry.

### Out of scope

- Any Python application code, scheduler, platform backend, SendInput seam, installer or updater.
- Route changes, framework migration, new UI framework, Tailwind/Bootstrap, runtime animation dependency, or package-manager migration.
- Rewriting product claims, weakening ToS/account-safety language, inventing testimonials/metrics, or adding unverified technical claims.
- Deleting or overwriting the older UI plan.
- Committing `site/dist/`, `site/playwright-report/`, `site/test-results/`, or ad-hoc screenshot artifacts.

### Forbidden visual shortcuts

- No gradient blobs, glassmorphism everywhere, bento/card wall, neon cyberpunk glow, generic starfield, particles, parallax, autoplay sound, infinite timing loop, marquee, gradient text or fake browser chrome.
- Do not solve overflow by adding a broad `body { overflow-x: hidden; }` band-aid. Contain intentional bleed at the owning component and prove the document width contract.
- Do not hide semantic table headers with `display: none`; visually hide them while keeping them in the accessibility tree.
- Do not animate layout properties when `transform`/`opacity`/`clip-path`/custom properties are sufficient.

## 3. Locked design and engineering targets

### 3.1 Page-level composition

Keep this exact narrative order unless a screenshot comparison proves a better order and the change is recorded before implementation:

```text
Header
Hero / positioning + timing instrument
Proof strip / capability signals
Timing mechanism / causal explanation
Product showcase / actual picker screenshot
Comparison / generic macro vs music scheduler
How it works / three steps
Technical trust / boundaries and ToS notice
Supported formats
FAQ preview
Final performance CTA
Footer
```

The page must have visible density variation:

- **Opening:** hero has moderate top space, larger bottom space, no ordinary section separator.
- **Proof:** compact high-density transition strip.
- **Explanation:** open negative space around a causal diagram.
- **Product peak:** largest tonal shift and near-full-bleed screenshot stage.
- **Utility:** comparison, steps, technical, formats and FAQ become progressively denser without all becoming cards.
- **Closure:** final CTA is its own composition, not another ordinary section.

### 3.2 Responsive contract

The implementation must be checked at:

```text
320x800, 390x844, 768x1024, 1024x768,
1280x800, 1440x900, 1920x1080
```

Required behavior:

- 320–479px: one-column narrative; buttons full width; timing events use two tiers; key grid shows only the useful five-key/active excerpt; no horizontal scroll.
- 480–767px: one-column product stage; screenshot before annotations; no overlay annotation text on the screenshot.
- 768–1023px: split only where content remains readable; hero may remain one column if the instrument would otherwise become narrow.
- 1024px+: 12-column composition may begin; hero instrument must retain at least 30rem where the layout uses the desktop stage.
- 1280px+: intentional bleed is allowed only when clipped by the owning section and verified not to expand document scroll width.
- 200% zoom: readable content and primary CTA remain visible; no content relies on hover-only disclosure.

### 3.3 Typography contract

- Keep `Playfair Display Variable` for H1 and important narrative H2 only.
- Keep `Inter Variable` for body, navigation, buttons, H3 and functional UI headings.
- Keep system mono for timestamps, status, format extensions, shortcuts and telemetry.
- Keep heading accessible text as one correctly spaced text stream; visual line spans must not remove whitespace.
- Use tabular numerals for telemetry/timestamps.
- Raise any readable micro label below 12px to the next readable token. A label may remain below 12px only when it is decorative and not necessary to understand the interface.
- Add/retain a VI proof fixture or Playwright assertion containing diacritics: `ă â ê ô ơ ư đ á ả ã ạ`.

### 3.4 Color, surface and motion contract

- Continue using semantic tokens (`--color-page`, `--color-surface-*`, `--color-text-*`, `--color-action`, `--color-info`, `--color-success`, `--color-warning`) rather than raw component colors.
- Preserve the three depth levels: page/editorial, instrument surface and active timing state.
- Gold is reserved for primary CTA, active node, key timestamp, selected locale and focus; it must not become the color of every heading/icon.
- Hero timing sequence runs once, stops on the final active event and can replay only through an explicit control.
- Reduced motion immediately displays the complete final state, disables smooth scrolling and does not lose status information.
- Hover effects are guarded by fine pointer capability and never carry the only meaning of an interaction.

### 3.5 Concrete overflow target

The fix must be structural:

1. Identify every intentional bleed in the hero (`negative margin`, pseudo-element inset, constellation position, rail connector).
2. Define the hero section as the ownership boundary for decorative bleed so the decoration cannot increase page scroll width.
3. Preserve the visible desktop bleed inside that boundary; do not simply remove the composition.
4. Re-run the browser measurement at all nine responsive widths after each layout change.

The agent must report which element caused the original overflow and which owning boundary now contains it.

## 4. File ownership map

The following map is the default ownership boundary. A phase may touch fewer files, but must not touch more without recording why in the plan's as-built notes.

| Concern | Primary files | Allowed responsibility |
|---|---|---|
| Global tokens/layers | `site/src/styles/tokens.css`, `global.css`, `layout.css`, `patterns.css`, `utilities.css` | Semantic tokens, page/grid primitives, shared surfaces, focus and motion defaults. |
| Header/footer | `site/src/components/layout/SiteHeader.astro`, `SiteFooter.astro` | Header density, nav state, locale target, mobile menu semantics; no unrelated page layout. |
| Hero/timing | `site/src/components/home/Hero.astro`, `TimingConsole.astro` | Unified composition, containment, event/key relationship, final/reduced-motion state. |
| Page rhythm | `HomePage.astro` plus each home section component | Section role, density, offset and surface variation; preserve content meaning. |
| Product proof | `ProductView.astro` and existing screenshot assets | Actual screenshot stage, annotation rail and mobile image order/crop. |
| Utility sections | `PlaybackProof.astro`, `ComparisonTable.astro`, `Steps.astro`, `TechnicalTrust.astro`, `Formats.astro`, `FaqPreview.astro`, `FinalCta.astro` | Distinct composition per role; no generic mega-component. |
| Content parity | `site/src/data/home.en.ts`, `home.vi.ts`, `home.types.ts` | Only existing content structure or exact translations required by UI; no new marketing claims. |
| Visual QA | `site/tests/e2e/accessibility.spec.ts`, `visual.spec.ts`, `navigation.spec.ts`, new focused spec only if needed | Stable screenshot states, overflow, locale, keyboard, reduced motion and route contracts. |

### 4.1 Shared primitive rule

Prefer the existing shared CSS patterns and small UI components. If a new primitive is necessary, it must have one narrow responsibility and no more than the minimum props needed by current consumers. Do not create a generic `Section` component with many layout props. Candidate primitives are `SectionIntro`, `Kicker`, `InstrumentPanel`, `TimingRail`, `Metric`, `ActionLink`, `DataRow` and `ConstellationMark`; add only those that remove real duplication.

### 4.2 Content and semantic rule

The UI may reorganize markup but must preserve:

- one page H1;
- meaningful heading order;
- event lists as semantic ordered lists where applicable;
- comparison table headers and cell associations;
- FAQ links and hashes;
- visible ToS/account-safety warning;
- keyboard focus order, `aria-current`, focus restoration and Escape behavior.

## 5. Phase 0 — Preflight and baseline capture

### Goal

Create a reproducible before-state without changing the product. This phase makes the later visual decisions measurable and prevents the agent from accepting a prettier but regressed page.

### Files allowed

- Read any file under `site/` and the playbook folder.
- Write no tracked source files.
- Build/test output may be generated only in ignored locations and must be removed if untracked artifacts appear.

### Steps

1. Confirm the branch, `git rev-parse HEAD`, working-tree status and the baseline revision used by the playbook.
2. Read `site/AGENTS.md`, `site/package.json`, `site/playwright.config.ts`, current visual/accessibility tests and all files named by the file ownership map.
3. Confirm Bun is the package manager and do not run `npm`, `yarn`, `pnpm` or `pip`.
4. Run the existing static gates from `site/`:

   ```powershell
   bun run check
   bun run lint
   bun run format:check
   bun run build
   bun run verify:dist
   ```

5. Run the current E2E suite once. Classify failures into pre-existing failures, environment failures and failures relevant to this plan. Do not update snapshots in this phase.
6. Capture or inspect baseline screenshots at minimum for `/` EN desktop 1440px, `/` EN mobile 390px and `/vi/` mobile 390px. If a baseline is missing, record `MISSING BASELINE`; do not silently create an acceptance snapshot.
7. Measure `document.body.scrollWidth`, `document.documentElement.scrollWidth` and `window.innerWidth` at 320, 390, 768, 1024, 1280 and 1440px. List the top offending element(s) by bounding rectangle.
8. Measure hero header-to-kicker gap, hero computed padding, product image bounds and the number of teaser rows. Record the values in the final review notes, not in production code.

### Stop conditions

- A missing source file, missing dependency or pre-existing build failure must be reported before any implementation phase.
- If the current branch has unrelated dirty changes, preserve them and do not reformat or overwrite them.
- If a visual baseline is missing, the plan may add a dedicated QA phase later, but the agent must not call a newly generated image a regression baseline without human review.

### Acceptance

- Baseline command results are recorded.
- Existing failures are classified.
- At least one concrete overflow owner is identified.
- No tracked file changes are made.

## 6. Phase 1 — Fix document overflow at the hero ownership boundary

### Goal

Retain the intentional hero bleed while making the document width equal to the viewport. The fix must address the actual owner of the decoration, not mask the entire document.

### Files allowed

1. `site/src/components/home/Hero.astro`
2. `site/src/styles/layout.css` only if a shared containment utility is genuinely needed
3. `site/tests/e2e/accessibility.spec.ts` only to improve diagnostics or add a targeted assertion; do not weaken the existing assertion

### Do not touch

- `body` overflow policy in `global.css` as a workaround.
- Product, timing data, route metadata, Python code or the external updater.
- Any `overflow-x: hidden` added solely to make the test green.

### Implementation steps

1. Re-read the current hero markup and identify all decorative nodes: `hero__score-lines`, `hero__constellation`, `hero__bridge`, `hero__visual::before` and `hero__rail::after`.
2. Add containment to the smallest semantic owner that encloses the bleed. Prefer `overflow: clip` on the hero section or an equivalent logical containment that preserves the visible inner stage. Use `overflow: hidden` only if `clip` is not supported by the project target and document the fallback.
3. Verify that clipping does not clip focus rings, buttons, the product image, or the mobile menu. Decorative elements may be clipped; interactive content may not.
4. If the stage still expands the document because the grid item itself has a negative margin, move the bleed into an absolutely positioned decorative layer inside the hero instead of broadening the grid item. Keep the content box within the container.
5. Preserve desktop optical alignment: stage may visually reach the right edge, but the document must not become wider than the viewport.
6. Add a diagnostic helper to the responsive test only if useful: report viewport, document width and the first offending selector on failure. Keep the pass criterion `<= viewport + 1`.
7. Run the overflow test at every required width before touching any other design issue.

### Verification

```powershell
cd site
bun run check
bun run lint
bun run build
bun run verify:dist
bun run test:e2e -- --grep "horizontal overflow"
```

Manual checks:

- At 320px, the hero console is readable and no right edge is cut from content.
- At 390px, full-width CTA buttons remain inside the viewport.
- At 1024px, the hero does not create a phantom scrollbar.
- At 1440px, intentional bleed remains visually present without changing document width.
- At 1920px, the max-width composition remains centered and does not become over-constrained.

### Acceptance

- All responsive overflow assertions pass at 320, 360, 390, 768, 912, 1024, 1280, 1440 and 1920px.
- No broad document-level overflow hack is introduced.
- The phase notes name the original offending selector and the new containment owner.

## 7. Phase 2 — Foundation tokens and shared presentation primitives

### Goal

Make later visual variation safe and consistent before changing the page silhouette. This phase should improve the source of truth without redesigning every section in one edit.

### Files allowed

1. `site/src/styles/tokens.css`
2. `site/src/styles/global.css`
3. `site/src/styles/layout.css`
4. `site/src/styles/patterns.css`
5. `site/src/styles/utilities.css`
6. `site/src/components/ui/SectionIntro.astro`
7. `site/src/components/ui/Kicker.astro`
8. New small UI primitive only when the current consumers prove duplication

### Locked decisions

- Keep the existing raw hex fallback and OKLCH progressive enhancement; do not remove fallbacks.
- Keep token names used by current components unless renaming is required to remove a proven conflict. If a token is renamed, update every consumer in the same phase and record the mapping.
- Introduce/retain semantic section spacing with distinct compact, default, open, feature and peak roles.
- Keep cascade layers ordered as `reset, tokens, theme, base, layout, patterns, components, utilities`.
- Do not add a utility framework or a monolithic CSS file.

### Implementation steps

1. Inventory raw colors, repeated border/background/shadow declarations and all uses of `.kicker`, `.section-heading`, `.section-title`, `.section-copy`, `.ui-instrument-*` and `.ui-timing-rail`.
2. Keep raw palette values in `tokens.css`; expose semantic aliases for page, quiet surface, instrument surface, active surface, primary/secondary/tertiary text, action/info/success/warning and line levels.
3. Add any missing motion tokens from the playbook, but do not attach an animation to a component by default.
4. Ensure `.container`, `.grid-12`, section spacing and logical properties live in layout/pattern styles rather than being duplicated in every component.
5. Normalize readable label size. Keep a separate decorative metadata token if necessary; do not silently make all metadata larger and destroy the timing console density.
6. Make shared button/focus/target rules a single source of truth. All interactive targets should be at least 44×44px unless the existing compact header button is explicitly documented as a 40px density exception.
7. Remove only dead duplicate rules made obsolete by this phase. Do not keep a `v1` and `v2` style system in parallel.

### Verification

```powershell
cd site
bun run check
bun run lint
bun run format:check
bun run build
```

Manual:

- Render a small EN/VI type proof with long H2, Vietnamese diacritics, timestamps and long button text.
- Check focus ring on page background, instrument surface, active state and warning callout.
- Confirm no contrast regression and no gold overuse.

### Acceptance

- Components consume semantic tokens rather than newly hardcoded colors.
- No CSS minified-size increase above 25% without a written reason.
- Existing page layout remains recognizable; major composition changes wait for later phases.

## 8. Phase 3 — Recompose the hero as one timing instrument

### Goal

Make the hero communicate the product in five seconds: sheet playback, timing precision, Windows/open source and a real product behind the concept. The visual must feel like one instrument stage, not a headline beside an unrelated card.

### Files allowed

1. `site/src/components/home/Hero.astro`
2. `site/src/components/home/TimingConsole.astro`
3. `site/src/components/ui/ConstellationMark.astro` only if the existing decorative mark cannot be reused without changing semantics
4. `site/src/styles/patterns.css` only for a shared instrument/rail primitive
5. `site/tests/e2e/accessibility.spec.ts`
6. `site/tests/e2e/visual.spec.ts`

### Composition contract

Desktop target:

```text
copy / 5 columns        instrument stage / 7–8 columns
headline                 timing engine + playhead
description              event rows connected to active state
primary + secondary      key excerpt/grid
risk note                rail/constellation relationship
```

The stage may begin around grid column 6 and bleed right at large widths, but its content box remains inside the hero ownership boundary established in Phase 1. The copy and stage may share a tonal background; text must never physically overlap.

### Implementation steps

1. Preserve the existing H1 accessible text and EN/VI strings. If visual line spans remain, ensure the text node contains an explicit space between line groups.
2. Keep the kicker as a concise system preface. Do not uppercase a long Vietnamese sentence if that harms scanning; use sentence case or a locale-specific tracking override.
3. Keep the primary and secondary CTA semantics, links, target size and release/repository behavior unchanged.
4. Move the timing rail relationship into the composition: the visual rail should have a clear start/end relationship with the stage or headline, and decorative nodes must remain `aria-hidden` while event text stays semantic.
5. In `TimingConsole`, preserve a semantic `<ol>` for events. The active event must have a text equivalent; active color must not be the only signal.
6. Make the playhead timestamp, active event and key excerpt visibly related through one accent system. The key grid remains secondary; do not let fifteen equal boxes dominate the headline.
7. On mobile, switch event rows to timestamp/type on the first line and keys on the second line. Hide or condense only decorative/nonessential key cells; do not hide the event list.
8. Keep the demo sequence bounded to one run of 4–5 seconds. It must stop on the final event. Replay is user-triggered only.
9. On reduced motion, skip the sequence and render the final active event/key state immediately. Also remove smooth scroll and transform hover motion through the global reduced-motion rule.
10. Ensure the timing script is scoped to the current stage, disconnects its `IntersectionObserver` after the first run, and does not create duplicate listeners if Astro renders more than one page section.
11. Keep the illustration transparent in its caption/accessible label. Never label the demo as a screenshot of the actual picker.

### Verification

```powershell
cd site
bun run check
bun run lint
bun run format:check
bun run build
bun run test:e2e -- --grep "timing|reduced motion|visual"
```

Manual states:

- initial hero before intersection;
- hero after one sequence reaches its final state;
- explicit replay;
- reduced motion;
- keyboard focus on both CTAs and replay control;
- 320px and 390px where no event row or button touches the viewport edge;
- 1440px where the stage feels unified and not like a floating card beside copy.

### Acceptance

- Hero communicates sheet + timing + Windows/open source without reading the rest of the page.
- No text overlap, hidden CTA, clipped focus ring or horizontal overflow.
- Timing sequence is finite and its final state is stable.
- Reduced-motion output contains the same meaningful information as animated output.

## 9. Phase 4 — Page rhythm and utility-section differentiation

### Goal

Resolve the playbook's main visual criticism: the page currently repeats a documentation template. Keep the information architecture but give each section a distinct role, density and alignment strategy.

### Files allowed

1. `site/src/components/home/HomePage.astro` only if section wrapper/order wiring must change
2. `site/src/styles/layout.css`
3. `site/src/components/home/ProofStrip.astro`
4. `site/src/components/home/PlaybackProof.astro`
5. `site/src/components/home/ComparisonTable.astro`
6. `site/src/components/home/Steps.astro`
7. `site/src/components/home/TechnicalTrust.astro`
8. `site/src/components/home/Formats.astro`
9. `site/src/components/home/FaqPreview.astro`
10. `site/src/components/home/FinalCta.astro`

### Locked section roles

| Section | Required visual behavior |
|---|---|
| Proof strip | Compact transition strip; high information density; not a full card section. |
| Playback proof | Open 5/7 composition; causal diagram is the visual explanation, not a second hero console. |
| Product showcase | First major visual peak; near-full-bleed actual screenshot; annotation rail points to real features. |
| Comparison | Semantic table with left column visually quieter and right column active; no generic two-card treatment. |
| Steps | Editorial sequence with vertical number rail; hotkey strip attached to the sequence. |
| Technical trust | Ledger/data density; warning callout visible but quieter than primary CTA. |
| Formats | Dense baseline-aligned rows; extensions carry hierarchy; no three-card grid. |
| FAQ preview | Lightweight rows with full-FAQ action aligned to heading on desktop and below it on mobile. |
| Final CTA | Separate closure composition with 7–8 column headline, action group and faint rail/constellation. |

### Implementation steps

1. Replace the assumption that every section needs a top border. Keep separators where they clarify a transition; remove or soften them where a tonal surface already separates the content.
2. Use the existing section spacing roles deliberately: proof `xs`, utility `sm`, explanation `lg`, product peak `xl`/`lg`, final CTA `lg`. Do not stack equal top and bottom deserts around every adjacent section.
3. Introduce intentional one- or two-column optical offsets only where they strengthen reading order. Every offset must remain inside the page grid and be tested for overflow.
4. Keep one shared intro type rhythm, but do not force all content into the same `4fr / 8fr` split. Select the layout from the section role: 5/7 causal proof, 8/4 product annotation, 10-column comparison, offset editorial steps, dense ledger.
5. Keep the product showcase's real screenshot unchanged in color and crop behavior. Use a neutral product frame, not fake browser controls. Desktop annotations may connect across the gap; mobile must place screenshot first and annotations afterward.
6. Ensure the comparison remains a real table. On mobile, visually hide the `<thead>` without removing it from the accessibility tree; use `data-label` only as a visual aid.
7. Keep technical boundaries explicit: `Windows SendInput`, process/application boundary, no game memory inspection, no code injection and update behavior. Do not weaken or rewrite the warning.
8. Treat format tags as metadata, not pill badges. Use extension, name, description and tags as aligned rows.
9. Make FAQ rows and final CTA actions keyboard/focus friendly. Do not add hover-only information.
10. Preserve the existing EN/VI content meaning. If a section needs a new label for semantic structure, add an exact counterpart in both locale data files.

### Visual acceptance questions

At a full-page screenshot, an independent reviewer must be able to point to:

- one hero instrument peak;
- one product screenshot peak larger than the utility surfaces;
- a compact proof transition;
- an open causal explanation;
- a dense technical/trust block;
- a distinct closing CTA.

If three adjacent sections look interchangeable after the change, the phase is not complete.

### Verification

```powershell
cd site
bun run check
bun run lint
bun run format:check
bun run build
bun run verify:dist
```

Manual at 390, 768, 1024 and 1440px:

- no doubled vertical desert between adjacent sections;
- no card wall or repeated rounded surface;
- product screenshot remains readable;
- comparison/ledger rows do not collide;
- final CTA is visually separate from FAQ and footer;
- no route/content/ToS changes.

### Acceptance

- The page no longer reads as nine copies of the same section template.
- Product screenshot remains the clearest actual-product proof.
- Utility sections have distinct density and hierarchy without introducing decorative gimmicks.

## 10. Phase 5 — Product proof, typography proof and locale parity

### Goal

Make the actual product screenshot the strongest proof and verify that the same composition survives English and Vietnamese content lengths.

### Files allowed

1. `site/src/components/home/ProductView.astro`
2. `site/src/components/home/FinalCta.astro`
3. `site/src/components/layout/SiteHeader.astro`
4. `site/src/components/layout/SiteFooter.astro` only if a proven locale wrap issue is found
5. `site/src/data/home.en.ts`
6. `site/src/data/home.vi.ts`
7. `site/src/data/home.types.ts` only for existing content structure
8. `site/src/styles/tokens.css` only for locale/type token overrides
9. `site/tests/e2e/visual.spec.ts`
10. A focused type-proof test/fixture under `site/tests/e2e/` only if necessary

### Product stage steps

1. Keep the real `picker.webp` and `picker-mobile.webp` assets. Do not create a fake product screenshot or redraw the picker in HTML.
2. Preserve explicit image dimensions and `height: auto` on the mobile source. Do not apply a second `object-fit: cover` crop to an already-cropped mobile asset.
3. Keep screenshot order before annotations on mobile.
4. Keep the product frame labels neutral (`SKY AUTO PLAYER / PICKER`, build/portable metadata) and do not add macOS traffic-light controls.
5. Verify every annotation points to a feature actually visible in the screenshot: search/picker, timing profile, controls or song selection. Remove any annotation that cannot be visually located; do not invent features.
6. Keep full-size image activation a normal keyboard-accessible link. Only add a lightbox if focus trap, Escape, restore focus and reduced motion can be proved without a new heavy dependency.

### Typography and locale steps

1. Render EN and VI at 320px and 390px with the same DOM structure.
2. Check H1/H2 line wrapping, especially the hero title and final CTA title. No H1 may exceed seven lines or create an orphaned single word when a small width adjustment can avoid it.
3. Check all uppercase kickers and mono labels. For long Vietnamese labels, use sentence case or a lower tracking override rather than forcing wide uppercase text.
4. Check buttons with long VI text; buttons may stack full width on mobile but text must remain fully visible and target size must remain usable.
5. Check timing event rows and technical ledger with Vietnamese terms. If a term/value can wrap, allow it to wrap; do not add `white-space: nowrap` to the whole row.
6. Check locale switch semantics and visible active state. Keep `aria-current` and keyboard activation.
7. Do not change route paths, content hashes, canonical/hreflang or structured data while fixing wrapping.

### Verification

```powershell
cd site
bun run check
bun run lint
bun run format:check
bun run build
bun run verify:dist
```

Manual type proof:

- EN + VI H1/H2;
- `ă â ê ô ơ ư đ á ả ã ạ`;
- `00:13.066`, `A1 + B2 + C3`, hold labels and format extensions;
- long CTA and navigation labels;
- screenshot frame and annotation labels;
- 200% zoom at both locales.

### Acceptance

- Product screenshot is the first unmistakable proof of the actual application.
- No double crop, layout shift, image collapse or annotation overlay on mobile.
- EN/VI layouts pass the same visual and overflow contracts.
- No visible text that is required for understanding falls below 12px.

## 11. Phase 6 — Interaction, accessibility and motion hardening

### Goal

Prove that the visual refactor remains usable without pointer hover, animation, or English-only assumptions.

### Files allowed

1. `site/src/components/layout/SiteHeader.astro`
2. `site/src/components/home/Hero.astro`
3. `site/src/components/home/TimingConsole.astro`
4. `site/src/components/home/FaqPreview.astro`
5. `site/src/components/home/FinalCta.astro`
6. `site/src/styles/global.css`
7. `site/src/styles/patterns.css`
8. `site/tests/e2e/accessibility.spec.ts`
9. `site/tests/e2e/navigation.spec.ts`
10. `site/tests/e2e/locale-switch-contrast.spec.ts`

### Header/menu contract

1. Desktop header remains one row at 1024px and above where the design allows it.
2. Mobile menu opens as an opaque sheet below the header, not a floating glass card.
3. Opening moves focus to the first link.
4. Escape closes and restores focus to the toggle.
5. Tab/Shift+Tab stays within the open menu if the menu is a modal-like full viewport state.
6. Outside pointer closes the menu where existing behavior promises it.
7. Locale links retain correct current-page paths on `/`, `/vi/`, `/faq/` and `/vi/faq/`.
8. Body scroll locking is required only if the menu becomes full viewport; otherwise do not add unnecessary global lock logic.

### Motion contract

1. Timing sequence uses only compositor-friendly properties and bounded timers.
2. Intersection observer disconnects after the first activation.
3. Replay increments/cancels run identity so old timers cannot mutate a newer run.
4. Reduced motion does not merely hide animation; it exposes final timing/event/key state immediately.
5. Hover media guard is used for pointer-only row effects.
6. Focus styling remains visible in reduced motion and on every surface.

### Accessibility contract

1. Run Axe on `/`, `/faq/`, `/vi/` and `/vi/faq/`.
2. Check one H1 per page and logical H2/H3 order.
3. Check `aria-current` on nav/locale and `aria-labelledby` on instrument/sections.
4. Confirm decorative score lines, constellation, rail and key grid are hidden from the accessibility tree where the event list already supplies the meaning.
5. Confirm event list, comparison table, technical definition list and FAQ links remain semantic.
6. Check visible focus contrast on page, panel, warning and CTA backgrounds.
7. Check keyboard-only navigation from header through hero actions, replay, product image and footer.

### Verification

```powershell
cd site
bun run check
bun run lint
bun run build
bun run test:e2e -- --grep "axe|keyboard|menu|locale|reduced motion|timing"
```

### Acceptance

- Axe has no serious/critical violations on all four routes.
- Keyboard-only header/menu flow works and focus is restored correctly.
- Reduced motion has complete static information, not a blank or half-rendered demo.
- No interaction depends solely on hover, color or animation.

## 12. Phase 7 — Visual regression matrix and final QA

### Goal

Replace the current incomplete/unstable visual contract with a deliberate, reviewable matrix. Never update snapshots just to make a failing test green.

### Files allowed

1. `site/tests/e2e/visual.spec.ts`
2. `site/tests/e2e/accessibility.spec.ts`
3. `site/tests/e2e/navigation.spec.ts`
4. `site/tests/e2e/site-contracts.spec.ts`
5. New test fixture/helper under `site/tests/e2e/` only if it reduces duplication and remains deterministic
6. No production file changes in this phase unless a test exposes a regression introduced in Phase 1–6

### Required visual matrix

At minimum, cover:

| Route | 320 | 390 | 768 | 1024 | 1440 |
|---|---:|---:|---:|---:|---:|
| `/` | yes | yes | yes | yes | yes |
| `/vi/` | yes | yes | yes | yes | yes |
| `/faq/` | yes | no | yes | no | yes |
| `/vi/faq/` | yes | no | yes | no | yes |

Also run a non-snapshot overflow measurement at 912, 1280 and 1920px.

### Stable screenshot-state rules

1. Set `prefers-reduced-motion: reduce` for ordinary full-page snapshots.
2. Add a deterministic `data-visual-test="true"` or equivalent state only if the production component already supports a truthful final state; do not add a test-only fake UI.
3. Wait for `document.fonts.ready` and image completion through Playwright assertions, not arbitrary sleeps.
4. Ensure the timing stage is in its final state before capture. The final state must be the same state a reduced-motion user sees.
5. Do not create screenshots at one viewport and label them as another viewport.
6. Treat snapshot generation as a review artifact: generate only after the implementation phase passes functional checks, inspect all new images, then ask the human before committing binary snapshots if repo policy requires them.

### Test scenarios

- Full-page EN home desktop and mobile.
- Full-page VI home desktop and mobile.
- FAQ/VI FAQ key route snapshots.
- Mobile menu closed/open, first-link focus, Escape restore focus.
- Hero timing final state and reduced-motion state.
- Product image keyboard focus and mobile source selection.
- FAQ hash navigation and expanded detail.
- 200% zoom primary content visibility.
- No horizontal overflow at all required widths.
- Route canonical/hreflang/legacy redirect contracts.

### Final command gate

Run from `site/` in this order:

```powershell
bun install
bun run check
bun run lint
bun run format:check
bun run build
bun run verify:dist
bun run test:e2e
```

If a command fails:

- classify it as implementation, stale baseline, missing browser, network/dependency or unrelated pre-existing failure;
- do not skip the command;
- do not update snapshots before proving the DOM/state is intentional;
- fix only the phase that introduced the failure;
- rerun the full gate after the local fix.

### Acceptance

- Required routes render under `/Sky-Auto-Player` base path.
- All functional E2E and Axe tests pass.
- Visual matrix exists for all required route/viewport combinations or each missing case is explicitly approved and recorded.
- No screenshot test is unstable across two consecutive captures.
- All overflow measurements pass.

## 13. Definition of done

The refactor is complete only when every item below is true:

### Design and hierarchy

- [ ] Page clearly reads as Nocturne Precision, not a generic SaaS dark template.
- [ ] Hero is one coherent timing composition with no text overlap.
- [ ] Product screenshot is the strongest actual-product proof and has intentional near-full-bleed weight.
- [ ] Page density varies across proof, explanation, product, utility and closure.
- [ ] No card wall, decorative star field, gradient blob, fake browser chrome or infinite animation exists.

### Layout and responsive behavior

- [ ] `body.scrollWidth` and `documentElement.scrollWidth` are at most viewport + 1px at 320, 360, 390, 768, 912, 1024, 1280, 1440 and 1920px.
- [ ] No interactive content is clipped by the hero containment boundary.
- [ ] No horizontal scrolling appears at 200% zoom.
- [ ] Hero, console, table, ledger, product screenshot and CTA reflow at 320/390px.
- [ ] Header remains intentional and usable through 1024px.

### Typography and localization

- [ ] EN and VI glyphs render correctly with no accent collision.
- [ ] H1/H2 wrapping is deliberate at 320 and 390px.
- [ ] Readable labels are at least 12px; smaller text is decorative only.
- [ ] Display serif is not used for functional H3/UI labels.
- [ ] Mono telemetry uses tabular numerals where alignment matters.
- [ ] EN/VI links, hashes, locale switch and visible meaning remain equivalent.

### Accessibility and interaction

- [ ] Axe has no serious/critical issue on all four routes.
- [ ] Keyboard-only navigation works for header, menu, locale, CTAs, replay, product image and footer.
- [ ] Escape closes mobile menu and restores toggle focus.
- [ ] Comparison headers remain in the accessibility tree.
- [ ] Decorative visuals are hidden correctly.
- [ ] Reduced motion presents complete static information.

### Engineering and delivery

- [ ] No new UI framework or runtime animation dependency.
- [ ] Shared token/pattern ownership is clear; no obsolete v1/v2 CSS remains.
- [ ] `bun run check`, `lint`, `format:check`, `build`, `verify:dist` and `test:e2e` pass.
- [ ] `site/dist/`, test reports and temporary screenshots are not committed.
- [ ] Working tree contains only intentional proposal/code/test changes.

## 14. Rollback and failure handling

### Phase rollback

Each phase must remain independently revertible. If a phase fails review:

1. Stop before starting the next phase.
2. Identify files changed by that phase using `git diff --name-only`.
3. Revert only the phase's files/commit after confirming no user changes overlap. Never use `git reset --hard` or broad checkout on an ambiguous dirty tree.
4. Re-run the phase's verification and the baseline overflow check.
5. Record the failed hypothesis and new evidence in the plan's as-built notes.

### Specific rollback rules

- **Overflow regression:** remove the new bleed/offset or restore the previous component boundary; do not add global overflow hiding.
- **Visual regression:** retain the functional fix if correct, isolate the visual mismatch and adjust only the owning composition; do not accept a snapshot update without inspection.
- **Typography regression:** revert font/token changes before changing content strings; keep EN/VI structure equal.
- **Accessibility regression:** revert markup change or restore semantic roles before styling around the failure.
- **Route/SEO regression:** stop immediately and restore route/metadata behavior; this plan does not authorize route contract changes.

## 15. Commit and handoff protocol

Commits are optional and require explicit human instruction. If requested, prefer one logical commit per phase:

1. `fix(site): contain hero bleed and remove responsive overflow`
2. `refactor(site): establish semantic UI tokens and shared patterns`
3. `style(site): unify hero timing instrument composition`
4. `style(site): vary homepage rhythm and product peak`
5. `fix(site): verify product proof and EN VI typography parity`
6. `fix(site): harden site accessibility and reduced motion`
7. `test(site): add deterministic visual QA matrix`

Do not mix Python/security changes with site commits. Do not push or open a PR unless asked.

At handoff, report:

- files changed per phase;
- original overflow owner and containment solution;
- before/after screenshot routes and viewports;
- test command results;
- known tradeoffs or intentionally deferred items;
- confirmation that P0 security surfaces were untouched.

## 16. As-built notes template

Keep this section unchanged until implementation begins. The coding agent may fill it only with observed facts after the human asks for execution.

```text
Date:
Agent:
Starting HEAD:
Playbook baseline:
Phases completed:
Files changed by phase:
Original overflow owner:
Final containment owner:
Visual matrix result:
EN/VI type-proof result:
Accessibility result:
Build/check/lint/format/verify-dist result:
Known deviations (must be empty or human-approved):
Deferred work:
```
