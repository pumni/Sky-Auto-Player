# WP1-R1 Coverage Migration Ledger

This ledger maps the independent review's deleted Graphite/UI oracles to tests that exercise the
current Tailwind v4, Base UI, and shared-component architecture. It records the reviewed source
identity `8bb6789589062fa5bf86595427a307fddb6a8217`; old selectors and screenshot-only checks were
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

The ordinary Add songs menu uses the shared Base UI 1.9.0 Menu root with its supported
`modal={false}` setting. The keyboard regression covers ArrowDown/ArrowUp, Space, Enter,
Tab/Shift+Tab exit, Escape restoration, outside interaction, compact hit targets, and focus leaving
Base UI's guards in `library.spec.ts`. The compact regression opens the menu at 640×448 with the
keyboard, confirms the first item is focused, and runs the unfiltered Axe scan while the popup is
open. That scan reproducibly reports serious Axe 4.14 `aria-hidden-focus` violations on the
library's mounted `span[data-base-ui-focus-guard][aria-hidden="true"][tabindex="0"]` elements.
The pinned library implements these guards in `node_modules/@base-ui/react/utils/FocusGuard.mjs`
and its menu trigger/focus manager; the guards are intentionally focusable so their focus handlers
can move keyboard focus out of the popup. Axe's full finding is preserved in
`apps/desktop/test-results/library-compact-playlist-s-8c55d-name-access-and-hit-targets/error-context.md`
for this run. The same focused reproduction is:

```powershell
$env:GRAPHITE_WORKSPACE_CAPTURE = 'candidate'
bun run test:e2e -- tests/e2e/library.spec.ts --grep "compact playlist source keeps Add, Import, Back, long-name access, and hit targets" --reporter=line --workers=1
```

The test's accessible-role queries and focus assertions expose the menu and its focused menu item
to the browser accessibility tree; the broader Add songs keyboard test validates the tab exit and
focus restoration sequence. This is browser keyboard/accessibility-tree evidence, not a claim of a
manual NVDA or Narrator session. Axe remains enabled with no rule/selector exclusions, and no
Base UI focus guard is changed. A previous full run also found the guard during a populated
playlist scan; the final full run passed that scan. The final canonical gate reports 148 passed,
1 skipped, and this 1 failed E2E test. R5 therefore remains unresolved pending the required
leader decision on the exact guard-only finding; it is not counted as passing.
