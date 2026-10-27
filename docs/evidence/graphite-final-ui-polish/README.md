# Graphite hierarchy-first UI evidence

This evidence package follows the corrected issue #549 direction: primary actions stay filled and
prominent, selected states persist, and secondary actions remain quiet until hover or focus.

## Provenance

- Before images were retained from source SHA
  `cf97261887047c626b9222c118adab32a27e39df`.
- After images were captured from implementation SHA
  `d6f2136bfe72074bc7855bc550fbd3ca33477f31`.
- Both sets use the Windows Playwright Chromium UI with the mock playback bridge. These are browser
  captures, not native Tauri screenshots. No physical input was sent.
- The three before images were reused unchanged. The optional capture test produces only the three
  after images listed below and does not write computed-style dumps.

## Control hierarchy

| Area                 | Role and state                                                                                                                                                                                           |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Titlebar             | Minimize and maximize are ghost controls; Close shows the danger fill on hover.                                                                                                                          |
| Library navigation   | Active source and playlist retain a selected tile and leading accent marker.                                                                                                                             |
| Track browser        | Inspection selection keeps its selected row surface; the playing row uses a separate accent treatment. Row actions stay ghost.                                                                           |
| Inspector            | Details and Runtime tabs keep a selected fill and underline. The overlay behavior remains covered at 640×448 without a redundant screenshot.                                                             |
| Player Dock          | Play, Pause, and Resume are round filled primary actions. Shuffle, Auto Play, and the open Inspector trigger use persistent selected fills when active. Other transport and utility controls stay ghost. |
| Settings and dialogs | Active Settings category stays filled. Primary Create/Save actions use the dark accent fill; destructive confirmation uses the danger fill. Inputs and switches retain their control surfaces.           |
| Menus                | Routine items are ghost actions; destructive menu items keep danger text and a visible danger hover surface.                                                                                             |

## Matched images

The full player view shows a selected Blue Bird row while Candle Run is playing, making inspection
selection and playback status easy to distinguish. Settings shows the active Playback category.
The caption image is a 46×40 Close-control crop from the 1366×768 viewport.

| State                                                 | Before                                                      | After                                                     |
| ----------------------------------------------------- | ----------------------------------------------------------- | --------------------------------------------------------- |
| Playing track with a different selected row, 1366×768 | [Before](before/player-playing-selected-track-1366x768.png) | [After](after/player-playing-selected-track-1366x768.png) |
| Settings category selection, 1366×768                 | [Before](before/settings-1366x768.png)                      | [After](after/settings-1366x768.png)                      |
| Caption Close danger hover, 46×40 crop                | [Before](before/titlebar-close-hover-1366x768.png)          | [After](after/titlebar-close-hover-1366x768.png)          |

## Contrast measurements

Ratios use WCAG relative luminance on the Graphite tokens. The E2E assertions also read rendered
computed colors for primary, danger, and focused states.

| Foreground / background                              | Contrast |
| ---------------------------------------------------- | -------: |
| White on primary rest fill `#705eae`                 |   5.38:1 |
| White on primary hover fill `#7564b4`                |   4.95:1 |
| White on primary pressed fill `#6553a0`              |   6.37:1 |
| White on danger rest fill `#c42b36`                  |   5.61:1 |
| White on danger hover fill `#d13a45`                 |   4.77:1 |
| Danger text `#f27a80` on menu danger hover `#452d32` |   4.70:1 |
| Accent ink `#b2a5e9` on selected control `#302d38`   |   6.07:1 |
| Accent ink `#b2a5e9` on navigation `#191a1b`         |   7.84:1 |

## Focused verification

All core state assertions run by default. Only image capture is opt-in.

- `cd desktop && bun run typecheck` — passed.
- `cd desktop && bun run lint` — passed.
- `cd desktop && bun run format:check` — passed.
- `cd desktop && bun run test:e2e -- tests/e2e/graphite-final-ui-polish.spec.ts` — 5 passed; the opt-in capture test was skipped.
- `GRAPHITE_FINAL_UI_CAPTURE=after bun run test:e2e -- tests/e2e/graphite-final-ui-polish.spec.ts --grep "captures the three matched Graphite hierarchy evidence views"` — 1 passed; wrote only the three after images above.
- `cd desktop && bun run test:e2e -- tests/e2e/workbench.spec.ts --grep "compact Inspector supports focus restoration"` — 1 passed.
- `git diff --check` — passed.

The full `cargo xtask check all` gate and hosted CI remain final merge checks; they were not repeated in
this focused review iteration. No native playback, input, or security code changed.
