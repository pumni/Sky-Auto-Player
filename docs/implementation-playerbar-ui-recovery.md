# Player Bar UI recovery — blocking follow-up for PR #235

Status: **REQUEST CHANGES / blocking UI recovery work order**.

PR: #235  
Branch: `docs/playerbar-lifecycle-spotify-handoff-2026-09`  
Implementation baseline reviewed before this handoff: `305be0a4a1c2d808cf24fb4a606103855783db70`.

This document supersedes the visual-layout portions of the earlier Player Bar work order where they conflict with this recovery plan.

The current lifecycle/state-machine work is directionally accepted. The current Player Bar presentation is **not accepted**. The actual Tauri UI shows a broken transport arrangement: Previous is vertically separated, the primary pending/play control drops into the timeline region, Next/Stop sit on another baseline, and the transport/timeline hierarchy collapses in the error state.

Do not merge PR #235 until this work order is satisfied.

## 1. Preserve the lifecycle work

Do **not** revert or redesign the following unless a test proves a regression:

- explicit Now Playing identity separate from Library selection;
- transport-operation ownership and duplicate-start protection;
- native retirement barrier;
- authoritative `getPlaybackStatus` reconciliation;
- `prepared_id` correlation for active/terminal status;
- actionable terminal failure code/message;
- natural finish auto-next;
- Previous/Next semantics;
- Stop preemption;
- playback-context membership invalidation;
- confirmation ownership and Cancel;
- bounded retired-session bookkeeping;
- late-start/orphan cleanup;
- stale-session event rejection.

The purpose of this round is to repair the **presentation layer** around the already-hardened lifecycle.

## 2. Confirmed visual failure

The actual Tauri UI at the reviewed head exhibits all of the following:

- Previous appears above the other transport actions.
- The primary circular pending/Play-Pause control is visually lower and overlaps the timeline area.
- Next and Stop are not aligned with Previous/primary.
- The progress labels/rail compete for the same vertical space as the controls.
- The error state leaves the transport looking collapsed and fragmented.
- The visual hierarchy no longer resembles a mature media transport.

Treat this as a real release-blocking regression even if browser E2E geometry is green.

## 3. Root design problem to remove

The current CSS uses a 7-column balancing grid:

```css
.transport-actions {
  grid-template-columns:
    minmax(0, 1fr)
    34px
    34px
    40px
    34px
    34px
    minmax(0, 1fr);
}
```

with artificial balance and fixed slot assignments.

Do not continue incrementally patching this structure.

The current footer also leaves effectively no vertical tolerance:

```
--player-height = 80px
player-bar vertical padding = 16px total
player-bar border = 1px
remaining content budget ~= 63px

player-transport min-height = 63px
rows = 40px + 18px
gap = 5px
```

That exact-fit budget is too fragile for Windows WebView2/font/DPI variation.

The recovery implementation must remove both sources of fragility.

## 4. Required transport structure

Use a simple DOM/CSS structure with two explicit vertical rows:

```
transport
├── controls-row
│   ├── previous
│   ├── primary Play/Pause/loading
│   ├── next
│   └── stop
└── timeline-row
    ├── current time
    ├── progress rail
    └── total time
```

The exact markup may differ, but the geometry must behave like this.

### Controls row

The following four controls must share **one visual centerline**:

```
Previous    Play/Pause    Next    Stop
```

Requirements:

- Previous, primary, Next, and Stop have the same vertical center within **1 px** in the actual Tauri screenshot at 100% display scaling.
- Play/Pause is the primary visual anchor.
- Previous and Next are immediately adjacent secondary controls.
- Stop is a safety action after Next, with a slightly larger semantic gap allowed, but it must stay on the same baseline.
- No control may be positioned with an arbitrary vertical offset to compensate for another state.
- No empty “balance slot” may be used solely to fake centering.

### Primary control

Play and Pause are two states of one primary control.

Required:

- circular;
- filled primary treatment;
- same width/height for Play, Pause, loading/transition state;
- center of primary control remains fixed across idle, starting, playing, paused, stopping, error-recovery states;
- loader icon may rotate, but the button box must not move.

Recommended size: **40–44 px**.

### Secondary controls

Previous, Next, and Stop:

- no visible CSS border;
- transparent background by default;
- no raised/card appearance;
- same nominal hit target size, recommended **32–36 px**;
- visible focus ring remains;
- hover may change icon color and may use a subtle circular hover surface if desired;
- Stop uses a plain stop/square glyph, not a circled glyph.

Do not add fake Shuffle/Repeat controls to balance the row.

## 5. Centering rule

The center of Play/Pause must align with the application/titlebar center axis.

At standard desktop widths:

```
abs(play_button_center_x - viewport_center_x) <= 2 px
```

This must remain true whether Stop is visible or not.

Do not use a fake empty control on the opposite side solely to satisfy this equation.

Preferred solutions:

- center the core `Previous / Primary / Next` group absolutely or via a dedicated centered inner wrapper;
- position Stop relative to the centered core group without participating in the primary centering calculation;
- or use another simple layout whose geometry is obvious from CSS.

The implementation should be understandable without solving a seven-column grid mentally.

## 6. Timeline must be a separate row

The timeline must never occupy the controls row.

Required vertical separation:

- controls row has its own height;
- timeline row begins below the controls row;
- minimum visible gap between the primary control bounding box and timeline labels/rail: **6 px**;
- the primary button may never overlap the time labels or progress rail;
- status text such as “Starting playback” replaces the time-label region or appears above the rail without changing the controls-row Y position.

Timeline requirements:

- current time left;
- total time right;
- progress rail spans the useful center width;
- stale snapshots remain rejected by lifecycle logic;
- no seek behavior is added.

## 7. Player Bar height

Do not preserve the current 80 px token if doing so requires exact-fit math.

Choose a height that remains compact but leaves real tolerance for WebView2.

Suggested evaluation range: **88–96 px**.

The final value is acceptable if:

- default 1200×760 Tauri window still leaves the Library comfortable;
- controls and timeline never overlap;
- the footer does not look oversized;
- there is at least a few pixels of vertical breathing room rather than an exact 63/63 content fit.

If the token changes, update the app-shell row deliberately and test all minimum-size behavior.

## 8. Left and right zones

### Left — Now Playing

Preserve the accepted identity model.

Required:

- current song title;
- secondary playback state/metadata;
- Heart acts on current song;
- text truncates, never pushes the center transport away;
- width is bounded by the three-column Player Bar shell.

In error state, do not replace useful identity with a large red block inside the left zone.

### Right — Sky-specific tools

Keep:

- playback profile button/popover from PR #234;
- utility panel button.

The right zone must not push the centered primary transport off-axis.

Do not add Spotify volume/device controls.

## 9. Error presentation

The current screenshot shows a wide “Playback unavailable” banner above the Player Bar while the footer subtitle also says “Playback error”.

This double-emphasis is visually heavy.

For recoverable transport/reconciliation errors:

- keep the transport usable;
- do not change Player Bar geometry;
- prefer one concise status/error surface;
- do not allow a recoverable warning to visually dominate the entire bottom of the app;
- provide clear action text only when an action is needed.

A recoverable status-query timeout must not make the UI look like the player is permanently unavailable if Play/Stop/retry remains possible.

For hard failures such as target-not-found, the actionable message may remain prominent, but the Player Bar layout must stay unchanged.

## 10. Admission/confirmation overlay

The confirmation panel may appear above the Player Bar, but:

- it must not change footer height;
- it must not move the controls row;
- it must not alter timeline Y;
- transport controls that are intentionally locked may visually disable, but remain in the same positions;
- Cancel must remain available according to lifecycle ownership rules.

## 11. Required state geometry

For the exact same window size and selected song, capture the primary button center and transport row Y for:

1. idle;
2. preparing;
3. starting;
4. playing;
5. paused;
6. stopping;
7. recoverable error;
8. confirmation-required.

Across all states:

- primary button center X drift: <= 2 px;
- primary button center Y drift: <= 1 px;
- Previous/Next/Stop center Y relative to primary: <= 1 px when visible;
- timeline rail Y drift: <= 1 px;
- Player Bar outer height drift: 0 px.

No state is allowed to reflow the footer vertically.

## 12. Required viewport acceptance

Manually inspect the actual Tauri window at:

### 1200 × 760

This is the configured default window size and is the most important acceptance viewport.

Verify:

- no clipping;
- primary centered;
- controls one baseline;
- timeline separate;
- left and right zones fit;
- profile popover remains usable.

### 1280 × 720

Verify the same and ensure vertical compression does not collapse the footer.

### 800 × 560

This is the configured minimum.

Verify:

- no horizontal scrollbar;
- title truncates before transport moves;
- right tools remain reachable;
- no overlapping controls;
- admission/error surface remains readable;
- profile popover remains inside viewport.

Do not claim responsive acceptance from browser-only screenshots.

## 13. Actual Tauri screenshot gate

This is mandatory.

Before declaring the work complete, produce screenshots from the **actual Tauri app**, not the web shell, on the exact implementation head.

At minimum capture:

- `1200x760-idle.png`
- `1200x760-playing.png` or dry-run playing
- `1200x760-paused.png`
- `1200x760-starting.png`
- `1200x760-error.png`
- `800x560-idle.png`
- `800x560-playing.png`
- `1200x760-profile-popover.png`

If the environment cannot produce a real Tauri screenshot, the PR remains **NOT READY FOR VISUAL ACCEPTANCE**.

Do not commit these binaries unless repository policy calls for it; they may remain local evidence, but report their paths and exact head SHA.

## 14. Automated geometry tests

Keep E2E useful, but make it test the real failure mode.

Add assertions for:

### Same-baseline controls

When Previous/Primary/Next/Stop are visible:

```ts
abs(centerY(previous) - centerY(primary)) <= 1
abs(centerY(next) - centerY(primary)) <= 1
abs(centerY(stop) - centerY(primary)) <= 1
```

### Timeline separation

```ts
timelineTop - primaryBottom >= 6
```

or the equivalent using the actual label/rail bounding boxes.

### Stable states

Measure the primary center and timeline rail before/after:

- idle -> starting;
- starting -> playing;
- playing -> paused;
- paused -> playing;
- playing -> error/terminal recovery where applicable.

Do not only test document overflow.

### Secondary style

Continue asserting:

- border width = 0;
- default background transparent;
- Play/Pause retains primary treatment.

## 15. Recommended implementation strategy

A good implementation sequence is:

1. **Do not touch store/native logic.**
2. Save screenshots of the current broken Tauri Player Bar for comparison.
3. Replace only the transport markup/CSS with a simple two-row structure.
4. Make core Previous/Primary/Next group centered independently.
5. Attach Stop as a same-baseline safety action.
6. Separate timeline row.
7. Increase Player Bar height only if needed for real vertical tolerance.
8. Verify default 1200×760 Tauri first.
9. Verify 1280×720 and 800×560.
10. Only then update E2E geometry assertions.
11. Run all lifecycle tests to prove the UI refactor did not change behavior.

Do not start by adding more media-query exceptions to the current seven-column grid.

## 16. Files expected to change

Primary scope:

- `desktop/src/components/player/PlayerTransport.tsx`
- `desktop/src/styles/player.css`
- `desktop/tests/e2e/library.spec.ts`
- `desktop/src/components/player/PlayerTransport.test.tsx` if markup/roles change

Possibly:

- `desktop/src/styles/tokens.css` if `--player-height` changes;
- `desktop/src/components/player/PlaybackAdmission.tsx` only for error/admission presentation;
- small Player Bar presentation tests.

Do **not** change:

- `desktop/src-tauri/src/native_runtime.rs`;
- playback DTOs;
- store lifecycle semantics;
- scheduler/SendInput;
- timing policy;

unless an independent correctness test fails.

## 17. Validation

Run:

```powershell
cd desktop
bun run check
bun run build
bun run test:e2e
bun run test:e2e:bundle

cd ..
cargo xtask check desktop
cargo xtask check rust
```

Then exact-head GitHub Actions must be green.

However:

> CI green is necessary but not sufficient for this UI recovery.

Actual Tauri screenshot acceptance is mandatory.

## 18. Final Codex report

The final report must contain:

- exact head SHA;
- exact files changed in the UI recovery round;
- explanation of the old layout failure;
- explanation of the new transport layout;
- Player Bar height before/after;
- measured primary center offset from viewport center;
- measured control center-Y deltas;
- measured primary-to-timeline vertical gap;
- local validation commands/results;
- exact-head GitHub Actions run;
- actual Tauri screenshot paths for all required states/viewports;
- explicit confirmation that lifecycle/native code was not changed in this recovery round, or justification if it had to be changed.

## Definition of Done

The UI recovery is accepted only when:

- [ ] Previous / Play-Pause / Next / Stop share one visual baseline.
- [ ] Primary Play/Pause remains exactly centered.
- [ ] Stop does not distort primary centering.
- [ ] Secondary controls are visually unboxed.
- [ ] Timeline is clearly separated below controls.
- [ ] No overlap exists at 1200×760, 1280×720, or 800×560.
- [ ] Idle/starting/playing/paused/stopping/error/confirmation keep stable footer geometry.
- [ ] Player Bar remains compact and visually balanced.
- [ ] Profile/utility controls from PR #234 remain intact.
- [ ] Lifecycle behavior and anti-stuck fixes remain intact.
- [ ] Actual Tauri screenshots pass manual visual review.
- [ ] Exact-head CI is green.

Until every item is satisfied, keep PR #235 Draft and unmerged.
