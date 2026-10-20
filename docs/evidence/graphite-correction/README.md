# Graphite workspace checkpoint G

**Status:** awaiting Lead visual review. This is a workspace-composition correction only; it does not approve Visual Gate #2 or authorize merging PR #503.

The **F** images show reviewed source `638992059a34a5040462e87019893a6f6daf5cdf`. The **G** images are genuine Playwright renders of the React application with the same mock catalog, viewport and playback state: 500 songs, Candle Run playing in Dry run while Blue Bird is selected, Update available, and Song Details open except in the playlist menu example. The browser mock does not send physical input. Captures are Chromium at the CSS viewport dimensions shown, not native Tauri screenshots.

## Matched application renders

| State                                                              | F checkpoint                                                                                                                                                                                      | G candidate                                                |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| 1366×768 populated workbench, expanded Navigator, docked Inspector | [F render](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/638992059a34a5040462e87019893a6f6daf5cdf/docs/evidence/graphite-correction/checkpoint-f/candidate/populated-1366x768.png)      | [G render](checkpoint-g/g-populated-1366x768.png)          |
| 800×560 everyday workbench, collapsed Navigator, Player Dock       | [F render](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/638992059a34a5040462e87019893a6f6daf5cdf/docs/evidence/graphite-correction/checkpoint-f/candidate/collapsed-800x560.png)       | [G render](checkpoint-g/g-workbench-800x560.png)           |
| 640×448 compact Inspector overlay and Dry run                      | [F render](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/638992059a34a5040462e87019893a6f6daf5cdf/docs/evidence/graphite-correction/checkpoint-f/candidate/compact-overlay-640x448.png) | [G render](checkpoint-g/g-overlay-640x448.png)             |
| 640×448 long playlist source and Add/Import menu                   | [F render](checkpoint-g/f-playlist-add-import-640x448.png)                                                                                                                                        | [G render](checkpoint-g/g-playlist-add-import-640x448.png) |

## Composition and measurements

G uses one continuous All Songs canvas (`#18191B`) and a single compact source header. The surrounding header chrome stays `#191A1B`; the local active-source label is `#242526`. The semantic `h1` remains in place, its count shares the same row, and no tablist role is introduced. The column header matches the canvas and retains one `1px #2A2B2C` divider. No new colors or decorative outlines were added.

| Viewport | Source header |  Column header | First song row | Vertical gain vs F |
| -------- | ------------: | -------------: | -------------: | -----------------: |
| 1366×768 | 40px; y=41–81 | 32px; y=81–113 |    y=113, h=46 |       33px earlier |
| 800×560  | 32px; y=41–73 | 32px; y=73–105 |    y=105, h=46 |       41px earlier |
| 640×448  | 32px; y=41–73 | 32px; y=73–105 |    y=105, h=46 |       41px earlier |

For comparison, F's wrapped/stacked source header measured 42.984px, the column header began at y=109.984px, and the first song row began at y=145.984px at these widths. G's outer pane begins at y=40px and its inner content at y=41px. No horizontal overflow was measured at any viewport. The long 640px playlist heading is `14px/600` at spacious widths and `13px/600` at compact widths; its count is `12px/400` and `11px/400` respectively. Both share one row. The complete source name remains available as the heading text and `title` value when constrained.

Computed G surfaces: header `rgb(25, 26, 27)` (`#191A1B`); active source label `rgb(36, 37, 38)` (`#242526`); canvas and table header `rgb(24, 25, 27)` (`#18191B`); column rule `rgb(42, 43, 44)` (`#2A2B2C`). The heading is an `H1`, and the Track Browser header contains zero `[role=tablist]` elements. At 1366, h1/count are `14px/600` and `12px/400`; at 800/640 they are `13px/600` and `11px/400`.

The compact playlist test creates the long title, checks the source/count/Add geometry, hit-tests Browse All Songs, Import files and Import folder, imports a real test fixture, enters the add-songs view, and returns with Back. The rendered capture shows the Add menu while those actions remain usable.

## Functional regression and capture workflow

The six former screenshot/checkpoint specs were audited: five now run as functional regression specs, retaining 327 Playwright `expect` call sites across geometry, interaction, keyboard, palette, accessibility and truthful playback checks. Screenshot and JSON writing has been removed from those regressions and from ordinary library tests. The only screenshot-writing Playwright spec is the optional `desktop/tests/e2e/visual-evidence.spec.ts`. To regenerate the candidate images and computed geometry, run from `desktop/`:

```powershell
$env:GRAPHITE_WORKSPACE_CAPTURE = 'candidate'
bun run test:e2e tests/e2e/visual-evidence.spec.ts --reporter=line
```

Output is written to ignored `desktop/test-results/graphite-workspace-checkpoint/`, never into the evidence folder. The checked-in G package retains five curated PNGs. The PR's previous 110 generated PNGs, 10 generated JSON files and obsolete palette swatch were removed; evidence already merged into `main` was not deleted. F's populated-workbench images are linked to the immutable reviewed commit above.

## Visual decision requested

Please review the single source-header row over one continuous song canvas, the 40px/32px responsive header heights, the measured first-row position and density gain, and the compact long-playlist Add/Import menu. Request changes by naming the viewport and element. Visual Gate #2 and final owner approval remain pending.
