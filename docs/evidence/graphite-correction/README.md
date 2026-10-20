# Graphite Workbench — final V2 evidence

**Review state:** Checkpoint G's art direction was accepted for continuation. This package is the final V2 implementation evidence for Lead technical review; repository owner Visual Approval Gate #2 is still pending. PR #503 remains Draft and must not be merged before that approval.

The current renders are genuine Playwright/Chromium screenshots of the application using its mock bridge and repeatable catalog/playback states. They are not synthetic UI mockups. The correction-H before images are the existing curated renders at reviewed HEAD [`62c5312`](https://github.com/pumni/Sky-Auto-Player/tree/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e); the after images replace those same curated screenshot slots. The F references remain the renders from reviewed commit [`6389920`](https://github.com/pumni/Sky-Auto-Player/tree/638992059a34a5040462e87019893a6f6daf5cdf). These screenshots do not demonstrate native game playback or physical input.

## Curated screenshots

| View | Before H | After H |
| --- | --- | --- |
| 1366×768 populated workbench: 500 songs, Dry-run A playing while B is selected, Update available, docked Song Details | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-populated-1366x768.png) | [Corrected workbench](checkpoint-g/g-populated-1366x768.png) |
| 800×560 compact workbench with collapsed Navigator and Player Dock | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-workbench-800x560.png) | [Corrected compact](checkpoint-g/g-workbench-800x560.png) |
| 640×448 compact Inspector overlay with explicit Dry-run state | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-overlay-640x448.png) | [Corrected overlay](checkpoint-g/g-overlay-640x448.png) |
| 1366×768 Runtime Diagnostics for that Dry-run session | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-runtime-diagnostics-1366x768.png) | [Corrected Runtime](checkpoint-g/g-runtime-diagnostics-1366x768.png) |
| 640×448 long playlist with Add/Import menu open | [Reviewed G](https://raw.githubusercontent.com/pumni/Sky-Auto-Player/62c5312bf1c9f90818cabfb1427bee3eb1c68d4e/docs/evidence/graphite-correction/checkpoint-g/g-playlist-add-import-640x448.png) | [Corrected playlist menu](checkpoint-g/g-playlist-add-import-640x448.png) |

The six PNGs added by this PR remain the five corrected captures above and one matched F playlist-menu reference. Older evidence already merged into `main` was not changed. No additional PNGs or JSON measurements are committed; Playwright writes generated captures and measurements only under ignored `desktop/test-results/graphite-workspace-checkpoint/`.

## Correction H: measured before and after

| Check | Reviewed G at `62c5312` | Corrected candidate | Evidence |
| --- | --- | --- | --- |
| Browser canvas and column header | `rgb(24, 25, 27)` (`#18191B`) | `rgb(21, 22, 24)` (`#151618`); chrome remains `rgb(25, 26, 27)` (`#191A1B`) | Computed styles at all captured sizes |
| Library collapse control at rest | Border `rgb(114, 119, 125)`, fill `rgb(36, 37, 38)` | Transparent border and fill, secondary icon `rgb(166, 166, 166)`; hover `rgb(34, 35, 36)`; keyboard focus is one 2px `#B2A5E9` outline offset 1px; forced colors use ButtonText/ButtonFace and Highlight focus | Graphite geometry and forced-colors regressions |
| Inspector at 640×448 | 320px overlay; saved dock width 320px | 288px overlay; saved dock width remains 320px | Computed bounds plus 280/320/380px dock preference round-trips |
| Search center within available region, Update visible | 1366px: 111px error; 800px: 109px; 640px: 15px | 0px at the three captured widths; ≤1px required at 640/800/1024/1366/1920 with Update available/unavailable and DPR 1/1.25/1.5 | `titlebar.spec.ts` |
| Song row fill and table columns | 6px fill inset on both sides; row/header cell edges aligned | Same 6px inset; every measured index/title/liked/notes/duration edge delta is 0px | `graphite-row-layout.spec.ts`; 15px classic scrollbar reservation simulated |

The correction does not add row padding or alter the 46px row height. With a simulated 15px classic scrollbar, the scrollport is 788px wide, its client/frame width is 773px, and each selected/playing/hovered row remains inset 6px from the frame on both sides. The measured raw right pane inset is 21px (6px row inset plus 15px reserved scrollbar); subtracting that reservation gives the matching 6px pane inset. Index, Title, Liked, Notes, and Duration row/header edge deltas remain 0px. The source header remains at y=41px; the first song begins at y=113px at 1366px and y=105px at 800px/640px.

## Final composition measurements

The accepted G composition structure remains unchanged: one continuous Track Browser canvas (including the table header), `#191A1B` workbench chrome, a localized `#242526` source label, and one `#2A2B2C` table divider. Correction H changes the canvas and column header to `#151618`. The source heading remains a semantic `h1` with its count inline; no tablist role is added. The source header is 40px at 1366px and 32px at 800px/640px. The table header is 32px and each song row is 46px.

| Viewport | Source header | Table header | First song row | Document/body width |
| --- | ---: | ---: | ---: | ---: |
| 1366×768 | y=41, h=40 | y=81, h=32 | y=113, h=46 | 1366 / 1366 |
| 800×560 | y=41, h=32 | y=73, h=32 | y=105, h=46 | 800 / 800 |
| 640×448 | y=41, h=32 | y=73, h=32 | y=105, h=46 | 640 / 640 |

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

**Requested decision:** Lead to review the final source and test evidence; repository owner to explicitly approve or request changes to the five current G screenshots for Visual Approval Gate #2. This report does not approve that gate.
