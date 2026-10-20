# Graphite row and Library checkpoint F

These are real Playwright captures of the same 500-song mock catalog, playback state, and CSS viewport sizes at the reviewed baseline and the correction candidate. Song A (`Candle Run`) is playing in Dry run while Song B (`Blue Bird`) is selected; the 1366px docked and 640px overlay states also show B in Song Details. The 1366px and 800px states keep a third row hovered. No screenshot is a synthetic mockup.

The `before/` captures were made from reviewed HEAD `fea610129a07acaab422347972a98194794422db`. The `candidate/` captures are generated from the correction source on this Draft PR; the exact commit is recorded in the PR body after push.

## Matched screenshots

| Viewport and state                                                                                            | Before                                                                                                                                                                                               | Candidate                                                                                                                                                                                                        |
| ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1366×768, expanded Navigator, docked Inspector, A playing / B selected, hovered row                           | [Full workbench](before/populated-1366x768.png) · [hovered row](before/hovered-row-1366x768.png) · [selected row](before/selected-row-1366x768.png) · [playing row](before/playing-row-1366x768.png) | [Full workbench](candidate/populated-1366x768.png) · [hovered row](candidate/hovered-row-1366x768.png) · [selected row](candidate/selected-row-1366x768.png) · [playing row](candidate/playing-row-1366x768.png) |
| 800×560, collapsed Navigator, Inspector closed, Player Dock visible                                           | [Full workbench](before/collapsed-800x560.png) · [Navigator](before/collapsed-navigator-800x560.png) · [Player Dock](before/player-dock-800x560.png)                                                 | [Full workbench](candidate/collapsed-800x560.png) · [Navigator](candidate/collapsed-navigator-800x560.png) · [Player Dock](candidate/player-dock-800x560.png)                                                    |
| 640×448, saved Navigator preference expanded, responsive collapse, compact Inspector overlay, Dry run visible | [Full workbench](before/compact-overlay-640x448.png) · [Navigator](before/compact-navigator-640x448.png) · [Player Dock](before/compact-player-dock-640x448.png)                                     | [Full workbench](candidate/compact-overlay-640x448.png) · [Navigator](candidate/compact-navigator-640x448.png) · [Player Dock](candidate/compact-player-dock-640x448.png)                                        |

## Locked correction and measured result

- All Songs body stays `#18191B`; its title/header is `#1D1E20`, the column header is `#1B1C1E`, and the single column-header rule stays `#2A2B2C`.
- Hover stays `#222324`; selection stays `#25242C`; the playing row keeps its existing neutral `#191A1B` surface and speaker mark. No zebra rows, new outlines or gradients were introduced.
- The Graphite collapsed rail is 48 CSS px with centered 40×40 px navigation, expand and add hit areas. Existing palettes keep 56px geometry and their original 44px tiles.
- At 640px the effective rail collapses to 48px while the saved `navigatorPreference` remains `expanded`; the accessible Expand control can manually open it within that viewport. Resizing to 800px restores the saved expanded state. The compact Inspector remains an overlay.
- Selected, playing and hovered rows are 46px high with 6px horizontal inset and 4px radius at the requested screenshots. A 438px pane-width stress case verifies the <=440px container rule uses a 4px inset. The test compares visible row/header cell X positions and widths within 2px.
- Measured candidate pane widths: 1366px: Navigator 240px / Browser 790px / docked Inspector 320px; 800px: collapsed Navigator 48px / Browser 740px / Inspector closed; 640px: collapsed Navigator 48px / Browser 580px / overlay Inspector 320px. Player Dock remains 92px; no document or Track Browser horizontal overflow was measured.
- On the baseline, the 640px Navigator occupied 208px; collapsed rail width was 56px; row fills reached the Browser frame edges with 0px inset and square corners. At 800px the saved collapsed rail was 56px. The baseline and candidate JSON include the actual rendered rectangles and computed styles.

Computed measurements: [baseline](before/computed-geometry.json) · [candidate](candidate/computed-geometry.json).

## Interaction and accessibility evidence

The candidate geometry and screenshot test runs by default with
`bun run test:e2e tests/e2e/graphite-row-layout.spec.ts`; it writes the current captures and
computed measurements to `candidate/` while checking them. The before captures are the recorded
Playwright run from the reviewed baseline above.

The E2E coverage checks that a Like target receives pointer hit-testing and click input without changing row selection, More Actions opens from the hovered row, keyboard focus reaches both actions, A's native playing identity remains separate from selected B, the compact dock retains the explicit Dry run label, the 640px collapse does not write a changed preference, and expanded width returns at 800px. Existing virtualized keyboard navigation and uncertainty/status tests also pass.

Focused verification commands and results for this checkpoint are listed in the PR description. Final canonical repository verification and hosted CI remain for the completed V2 package after Lead review.
