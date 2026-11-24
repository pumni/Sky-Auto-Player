# WP1-R1 Coverage Migration Ledger

This ledger maps the independent review's deleted Graphite/UI oracles to tests that exercise the
current Tailwind v4, Base UI, and shared-component architecture. It records the reviewed source
identity `862741b106b9d68f05d4738871b41cb214c6a9db`; old selectors and screenshot-only checks were
not carried forward as assertions.

| Removed suite or test | Meaningful oracle | Current replacement coverage |
| --- | --- | --- |
| `graphite-appearance-regression.spec.ts` | Semantic text, action, status and focus contrast; usable hit targets and active controls | `theme-accessibility.spec.ts`: all six dark palettes, Light, actual PlayerTransport rest/hover/pressed colors, selected/playing row surfaces, row action visibility, forced colors and reduced motion. `playback.spec.ts`: five transport controls and viewport geometry. `library-layout.spec.ts`: compact navigation targets. |
| `graphite-chrome-regression.spec.ts` | Titlebar controls do not collide; Search remains centered in the free titlebar span and tracks long playlist scope labels | `titlebar.spec.ts`: collision and pointer matrix across viewport, update state, long label and scale. `workbench-layout.spec.ts`: Search center axis and Player controls center. `GlobalSearch.test.tsx` and `library.spec.ts`: source scope, debounced query and clear behavior. |
| `graphite-final-ui-polish.spec.ts` | Play, Pause and Resume share the filled primary role; active navigation/toggles persist; quiet secondary controls expose hover/focus and danger; text and icon actions use the correct roles; forced-colors and reduced-motion feedback | `theme-accessibility.spec.ts`: actual Play color states in all dark palettes and Light, Pause/Resume filled dry-run states, destructive confirmation rest/hover/pressed colors, and forced-colors/reduced-motion behavior. `playback.spec.ts`: selected transport toggles. `library.spec.ts`: row focus/selection and visible row actions. `library-layout.spec.ts`: active navigation rails. `graphiteThemeTokens.test.ts`: required Tailwind action/scrim token mappings. |
| `graphite-geometry-regression.spec.ts` | Pane surfaces, compact Inspector overlay, Player Dock, and Runtime hierarchy remain within approved geometry | `workbench.spec.ts`, `workbench-layout.spec.ts`, and `workbench-resize.spec.ts`: dock/overlay layout, pane bounds, resizing, persistence and Runtime view. `player-dock-density.spec.ts`: 800×560 and 640×448 title/status/Test/progress and center-control geometry. `visual-evidence.spec.ts`: rendered workspace scenes at 640, 800, 1200, 1366 and 1920 widths. |
| `graphite-package-c.spec.ts` | Graphite preview variants consume semantic tokens and preserve contrast, input states and geometry | The retired A/Soft Graphite preview variants are not product states in the accepted design. Their shared production guarantees are now tested against the single locked Graphite system and all six dark palette IDs by `graphiteThemeTokens.test.ts` and `theme-accessibility.spec.ts`; rendered rest/hover/pressed controls and current visual evidence use the production theme. |
| `graphite-row-layout.spec.ts` | Row surfaces/actions, collapsed navigation, and Player Dock geometry stay aligned | `library-layout.spec.ts`: shared geometry, headings, rows, actions, dividers and collapsed rail. `theme-accessibility.spec.ts`: selected/playing surface and indicator oracles in Dark and Light. `player-dock-density.spec.ts` and `workbench-layout.spec.ts`: dock bounds and center alignment. |
| `light-appearance.spec.ts` | Light tokens stay independent of saved dark palette; rendered contrast across library, Settings, Diagnostics, recovery, update and empty/unavailable/error states; focus and control states; desktop and compact geometry | `graphiteThemeTokens.test.ts`, `system-appearance.spec.ts`, and `theme-accessibility.spec.ts`: Light action/token contrast, palette independence, text contrast over rendered Light scenes, six desktop/compact widths, selected/playing rows, and Axe checks on the surfaced application states. `settings-diagnostics.spec.ts`, `titlebar.spec.ts`, `playback-confirmation-layout.spec.ts`, and `update-modal-geometry.spec.ts` retain component-specific interaction and layout oracles. |
| `palette-appearance.spec.ts` | All palette tokens/contrast; Graphite default ID; Settings geometry/focus; consumers and interactions; forced colors | `graphiteThemeTokens.test.ts`: exact Graphite/Light action tokens, six palette definitions, swatches, and contrast. `settings-diagnostics.spec.ts`: all six palette IDs round-trip through Settings. `system-appearance.spec.ts`: saved dark palette stays independent from active mode. `theme-accessibility.spec.ts`: rendered computed colors, focus, row state and forced-colors behavior. |
| `AppSelect.test.tsx` | Hold/FPS formatting, Default tempo, accessible listbox, disabled option/Escape, and Tab dismissal without a focus trap | Formatting assertions moved to `playbackOptionFormatting.test.ts`; listbox selection, disabled option, Escape and focus restoration live in `select.test.tsx`; `player-dock-density.spec.ts` now exercises Tab dismissal against a rendered Playback Profile Select at each covered dock viewport. |

## Current Axe limitation

The open Add songs menu is covered for ArrowDown/ArrowUp, Space, Enter, Tab/Shift+Tab exit,
Escape restoration, outside interaction, compact hit targets, and focus leaving Base UI's guards by
`library.spec.ts`. The final full Playwright run reports Axe 4.14 `aria-hidden-focus` in
`compact playlist source keeps Add, Import, Back, long-name access, and hit targets`, which scans
with the Add songs menu open. Axe points to Base UI 1.9.0's own
`span[data-base-ui-focus-guard][aria-hidden="true"][tabindex="0"]`. The issue reproduces with the
required shared `modal={false}` Menu root. Axe remains enabled for both checks, and no
Base UI focus guard is changed. One earlier full run reported the same finding during the populated
playlist post-import scan; the final full run passed that scan. This is an unresolved R5 blocker
pending an upstream-safe resolution and leader decision; the final Axe failure is not counted as
passing.
