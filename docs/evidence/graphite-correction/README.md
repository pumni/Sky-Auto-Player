# Graphite Workbench — final V2 evidence

**Review state at preparation:** Checkpoint G's art direction was accepted for continuation. This
package was prepared for Lead technical review while repository owner Visual Approval Gate #2 was
pending and PR #503 was Draft. PR #503 has since merged. This archive records the request and state
at preparation; it does not infer a separate owner signoff from the merge or change the screenshot
source-SHA provenance recorded below.

The current renders are genuine Playwright/Chromium screenshots of the application using its mock bridge and repeatable catalog/playback states. They are not synthetic UI mockups. The correction-H before images are the existing curated renders at reviewed HEAD [`62c5312`](https://github.com/pumni/Sky-Auto-Player/tree/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e); the after images replace those same curated screenshot slots. The F references remain the renders from reviewed commit [`6389920`](https://github.com/pumni/Sky-Auto-Player/tree/638992059a34a5040462e87019893a6f6daf5cdf). These screenshots do not demonstrate native game playback or physical input.

The two populated and compact-overlay slots now contain the **Correction I candidate**. Their before links below refer to the reviewed H commit at `5e5e01293c2d918401599104cf18fa84f2509554`; the other three curated slots still contain H renders.

## Curated screenshots

| View                                                                                                                  | Before H                                                                                                                                                                                         | After H                                                                   |
| --------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------- |
| 1366×768 populated workbench: 500 songs, Dry-run A playing while B is selected, Update available, docked Song Details | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-populated-1366x768.png)           | [Corrected workbench](checkpoint-g/g-populated-1366x768.png)              |
| 800×560 compact workbench with collapsed Navigator and Player Dock                                                    | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-workbench-800x560.png)            | [Corrected compact](checkpoint-g/g-workbench-800x560.png)                 |
| 640×448 compact Inspector overlay with explicit Dry-run state                                                         | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-overlay-640x448.png)              | [Corrected overlay](checkpoint-g/g-overlay-640x448.png)                   |
| 1366×768 Runtime Diagnostics for that Dry-run session                                                                 | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-runtime-diagnostics-1366x768.png) | [Corrected Runtime](checkpoint-g/g-runtime-diagnostics-1366x768.png)      |
| 640×448 long playlist with Add/Import menu open                                                                       | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-playlist-add-import-640x448.png)  | [Corrected playlist menu](checkpoint-g/g-playlist-add-import-640x448.png) |

The six PNGs added by this PR remain the five corrected captures above and one matched F playlist-menu reference. Older evidence already merged into `main` was not changed. No additional PNGs or JSON measurements are committed; Playwright writes generated captures and measurements only under ignored `desktop/test-results/graphite-workspace-checkpoint/`.

## Correction H: measured before and after

| Check                                                 | Reviewed G at `62c5312`                                     | Corrected candidate                                                                                                                                                                                            | Evidence                                                                    |
| ----------------------------------------------------- | ----------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| Browser canvas and column header                      | `rgb(24, 25, 27)` (`#18191B`)                               | `rgb(21, 22, 24)` (`#151618`); chrome remains `rgb(25, 26, 27)` (`#191A1B`)                                                                                                                                    | Computed styles at all captured sizes                                       |
| Library collapse control at rest                      | Border `rgb(114, 119, 125)`, fill `rgb(36, 37, 38)`         | Transparent border and fill, secondary icon `rgb(166, 166, 166)`; hover `rgb(34, 35, 36)`; keyboard focus is one 2px `#B2A5E9` outline offset 1px; forced colors use ButtonText/ButtonFace and Highlight focus | Graphite geometry and forced-colors regressions                             |
| Inspector at 640×448                                  | 320px overlay; saved dock width 320px                       | 288px overlay; saved dock width remains 320px                                                                                                                                                                  | Computed bounds plus 280/320/380px dock preference round-trips              |
| Search center within available region, Update visible | 1366px: 111px error; 800px: 109px; 640px: 15px              | 0px at the three captured widths; ≤1px required at 640/800/1024/1366/1920 with Update available/unavailable and DPR 1/1.25/1.5                                                                                 | `titlebar.spec.ts`                                                          |
| Song row fill and table columns                       | 6px fill inset on both sides; row/header cell edges aligned | Same 6px inset; every measured index/title/liked/notes/duration edge delta is 0px                                                                                                                              | `graphite-row-layout.spec.ts`; 15px classic scrollbar reservation simulated |

The correction does not add row padding or alter the 46px row height. With a simulated 15px classic scrollbar, the scrollport is 788px wide, its client/frame width is 773px, and each selected/playing/hovered row remains inset 6px from the frame on both sides. The measured raw right pane inset is 21px (6px row inset plus 15px reserved scrollbar); subtracting that reservation gives the matching 6px pane inset. Index, Title, Liked, Notes, and Duration row/header edge deltas remain 0px. The source header remains at y=41px; the first song begins at y=113px at 1366px and y=105px at 800px/640px.

## Final composition measurements

The accepted G composition structure remains unchanged: one continuous Track Browser canvas (including the table header), `#191A1B` workbench chrome, a localized `#242526` source label, and one `#2A2B2C` table divider. Correction H changes the canvas and column header to `#151618`. The source heading remains a semantic `h1` with its count inline; no tablist role is added. The source header is 40px at 1366px and 32px at 800px/640px. The table header is 32px and each song row is 46px.

| Viewport | Source header | Table header | First song row | Document/body width |
| -------- | ------------: | -----------: | -------------: | ------------------: |
| 1366×768 |    y=41, h=40 |   y=81, h=32 |    y=113, h=46 |         1366 / 1366 |
| 800×560  |    y=41, h=32 |   y=73, h=32 |    y=105, h=46 |           800 / 800 |
| 640×448  |    y=41, h=32 |   y=73, h=32 |    y=105, h=46 |           640 / 640 |

Runtime capture shows `Playback status: Playing`, `Now Playing: Candle Run`, and `Mode: Dry run · no physical input sent`. Sender status, sample count, backend and timing values correctly say **Not measured** for dry-run playback; the capture does not fabricate physical sender metrics.

## 640px Add songs popover inspection

The menu can cover nearby explanatory copy while it is open, as a popover normally does. Browser checks confirm its full bounds remain inside 640×448, all three actions receive pointer hit tests, keyboard opening focuses the first action, Escape closes the menu and restores focus to Add songs, and Axe reports no serious or critical violations. After dismissal there is no persistent menu overlay; the playlist header remains collision-free and Add, Import, Browse, and Back continue to work. Import and add-mode transitions are exercised, so no menu redesign was needed.

## Screenshot and regression-test audit

One opt-in generator remains: [`visual-evidence.spec.ts`](../../../desktop/tests/e2e/visual-evidence.spec.ts). From `desktop/`, run:

```powershell
$env:GRAPHITE_WORKSPACE_CAPTURE = 'candidate'
bun run test:e2e tests/e2e/visual-evidence.spec.ts --reporter=line
```

It writes five current application captures and a measurements JSON into ignored test-results. Ordinary E2E and canonical verification do not write to tracked evidence. Of the former capture/checkpoint specs, five protect functional behavior and one owns capture generation. The maintained regressions cover row/header geometry, playlist actions, virtualized selection, keyboard, accessibility, responsive behavior, forced-colors behavior and all five legacy palettes. The popover keyboard-focus/dismissal checks are in `library.spec.ts`.

## Verification and scope

Final verification outcomes, exact HEAD and hosted required CI run are recorded in the PR description after the final source commit. A native Tauri screenshot was not captured: `bun run tauri:dev` could not start because port 1420 was already occupied, and the existing window was occluded, so I did not bring it forward or alter the running app. Browser screenshots use a mock bridge. No native display/drag-region, display-DPI, physical-input or real-game timing acceptance is claimed.

**Decision requested at preparation:** Lead to review the final source and test evidence; repository owner to explicitly approve or request changes to the five current G screenshots for Visual Approval Gate #2. This report is not an approval record for that gate.

## Owner Correction I — one Browser surface and left-only row inset

The two matched comparisons use the same 500-song mock catalog, playback, selection, Inspector and viewport state. The H screenshots are immutable references at the reviewed pre-correction commit; the I candidate screenshots reuse and overwrite the existing curated slots above. No additional PNG was added.

| View                                                                                                                   | Before H                                                                                                                                                                                | Correction I candidate                               |
| ---------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| 1366×768, 500 scrollable songs, Candle Run playing while Blue Bird is selected, Aurora Landing hovered, docked Details | [H workbench](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/5e5e01293c2d918401599104cf18fa84f2509554/docs/evidence/graphite-correction/checkpoint-g/g-populated-1366x768.png) | [I workbench](checkpoint-g/g-populated-1366x768.png) |
| 640×448 compact workbench, temporary Inspector overlay, explicit Dry run                                               | [H overlay](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/5e5e01293c2d918401599104cf18fa84f2509554/docs/evidence/graphite-correction/checkpoint-g/g-overlay-640x448.png)      | [I overlay](checkpoint-g/g-overlay-640x448.png)      |

### Computed colors and geometry

| Measurement                                                                   | 1366×768                                                              | 640×448                                                               |
| ----------------------------------------------------------------------------- | --------------------------------------------------------------------- | --------------------------------------------------------------------- |
| Browser pane / section / source header / source label / table / column header | `rgb(21, 22, 24)` (`#151618`) for all six                             | `rgb(21, 22, 24)` (`#151618`) for all six                             |
| Source header box                                                             | x=249, y=41, 788×40px                                                 | x=57, y=41, 578×32px                                                  |
| Column header / first song row                                                | header y=81, h=32px; first row y=113, h=46px                          | header y=73, h=32px; first row y=105, h=46px                          |
| Normal Playwright scrollport                                                  | offset/client width 788/788px; reservation 0px; scroll height 23032px | offset/client width 578/578px; reservation 0px; scroll height 23032px |
| Hovered/selected/playing row fill relative to frame                           | left 6px, right 0px; row h=46px                                       | left 6px, right 0px; row h=46px                                       |
| Header-to-row cell edge delta                                                 | index, title, liked, notes and duration: 0px                          | index, title, liked, notes and duration: 0px                          |

The workbench keeps its existing Graphite chrome `rgb(25, 26, 27)` (`#191A1B`) outside the Browser. The table retains one `rgb(42, 43, 44)` (`#2A2B2C`) divider. Hover `#222324`, selected `#25242C`, text hierarchy, heading typography and Search are unchanged. At Browser container widths 441–520px the regression checks the owner-specified 6px/0px fill and 12px/6px text padding; at <=440px it checks 4px/0px fill and 8px/4px header padding. The empty-search state removes the scrollport and rows without introducing another Browser surface or horizontal overflow.

At 1366px, the normal Chromium screenshot has no reserved scrollbar width (offset/client 788/788px). The geometry test also simulates a classic scrollbar: offset/client 788/773px, 15px reserved on the right. In that case row fill ends at the scrollport frame's right edge, while the frame is 15px short of the pane's inner right edge; after subtracting the reservation, the exterior row inset remains exactly 0px. All five header/row column edges stay aligned at 0px. The 640px compact capture likewise measures no native scrollbar reservation; the same live geometry assertion accounts for any reservation reported by the browser.

The optional capture workflow remains the existing `visual-evidence.spec.ts`. For exactly the two views above, run from `desktop/`:

```powershell
$env:GRAPHITE_WORKSPACE_CAPTURE = 'candidate'
$env:GRAPHITE_WORKSPACE_CAPTURE_VIEWS = 'populated,overlay'
bun run test:e2e tests/e2e/visual-evidence.spec.ts --reporter=line
```

Only those two `page.screenshot` captures are emitted; their temporary images and measurements remain under ignored `desktop/test-results/graphite-workspace-checkpoint/`. The focused row geometry regression and desktop TypeScript check pass. This correction checkpoint intentionally did not run `cargo xtask check all`, production smoke, or hosted CI, per the shortened visual-review workflow. It is not final owner approval or native Tauri verification.
