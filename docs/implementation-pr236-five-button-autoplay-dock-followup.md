# PR #236 transport UI follow-up — fixed five-button dock + Auto Play tool

Status: **REQUEST CHANGES / superseding UI placement contract for PR #236**.

Reviewed implementation head before this follow-up:
`fe8d882b52698197db1c9b5065addc16f939a580`.

The Shuffle and Auto Play behavior work is directionally accepted. This follow-up changes the required **presentation**.

It supersedes the earlier requirement that Auto Play be placed beside Base Hold in the compact Playback Profile.

## 1. Product decision

The bottom dock must use a stable media-player hierarchy.

### Center transport

Exactly five transport controls must always be visible:

```
Shuffle   Previous   Play/Pause   Next   Stop
```

“Always visible” means the button boxes remain rendered and occupy the same positions in every supported playback state.

A control may be disabled when its action is not valid, but it must not disappear and cause the row to reflow.

### Right tools

Auto Play is no longer a control inside the quick Playback Profile.

The bottom-right tool group must be:

```
Auto Play   Playback Profile   Utility
```

Auto Play is its own persistent toggle button and must be the **leftmost button of the right-side tool group**.

Keep Auto Play mirrored in Settings > Playback as the durable settings surface.

Remove Auto Play from the compact Playback Profile popover.

## 2. Current implementation problems

At reviewed head `fe8d882b...`:

1. Stop is conditionally rendered only when `hasSession`, so idle has only four visible transport controls.
2. Before a native session ID exists, Stop disappears.
3. When `playback.prepared` exists but there is no active session/start request, the primary slot can render no button at all. This is especially visible around confirmation/blocked states.
4. Auto Play is still rendered in `ProfileFields` beside Base Hold.
5. The current hover treatment is only a basic color swap; improve it into a deliberate Spotify-like secondary-control interaction while preserving the borderless design.

These are UI acceptance blockers even though exact-head CI is green.

## 3. Five-button invariant

`PlayerTransport` must always render five visible button hit targets.

The identity/order is fixed:

1. Shuffle
2. Previous
3. Primary Play/Pause/pending control
4. Next
5. Stop

Do not conditionally unmount any of the five based on playback state.

### Shuffle

Always visible.

Possible states:

- enabled, Off;
- enabled, On;
- disabled while a conflicting transport operation/confirmation owns the player.

When disabled it remains visible but visually muted.

### Previous

Always visible.

Disable when:

- there is no valid context;
- context position has no previous move and restart semantics are not applicable;
- transport operation/confirmation owns navigation.

It must remain in place.

### Primary

Exactly one primary button must always exist in the center slot.

Use state mapping equivalent to:

- idle/retryable -> Play;
- playing -> Pause;
- paused -> Resume/Play icon;
- preparing/starting/advancing/restarting/stopping -> same circular button with pending spinner;
- confirmation-required -> disabled Play or a stable confirmation-pending primary presentation;
- blocked -> disabled Play presentation;
- recoverable unknown-owned-session -> pending/disabled primary while explicit recovery actions remain available.

Do not leave `.player-primary-slot` empty.

The primary button's width, height, and center coordinates must remain identical across all states.

### Next

Always visible.

Disable at the end of the traversal or while navigation is owned by another operation.

### Stop

Always visible.

- no native/known session -> visible but disabled;
- starting/playing/paused/stopping-known-session -> enabled according to the existing safety contract;
- `stopping` operation already in flight -> visible but disabled;
- terminal/no ownership -> visible but disabled.

Do not remove Stop when there is no session.

This gives the user a fixed mental model and prevents dock geometry changes.

## 4. Spotify-like secondary hover behavior

Shuffle, Previous, Next, and Stop are secondary controls.

Default inactive visual:

- no border;
- transparent background;
- muted/secondary icon color;
- full-size hit target.

Enabled hover/focus-visible:

- icon becomes clearly brighter, targeting `var(--text-primary)` or the theme's equivalent high-contrast foreground;
- transition should be quick and subtle, approximately 100–150 ms;
- no card/box border may appear;
- no layout movement;
- a very small opacity/brightness emphasis is acceptable;
- avoid a filled hover pill unless it remains visually lighter than Play/Pause.

Recommended behavior:

```css
.player-secondary-action {
  color: var(--text-tertiary-or-secondary);
  background: transparent;
  border: 0;
  transition:
    color 120ms ease,
    opacity 120ms ease,
    filter 120ms ease;
}

.player-secondary-action:hover:not(:disabled),
.player-secondary-action:focus-visible:not(:disabled) {
  color: var(--text-primary);
}
```

If a slight scale is used, it must not alter layout/bounding geometry. Prefer color/brightness only.

Disabled:

- still visible;
- muted;
- no misleading hover brightening;
- cursor semantics should not imply action.

### Active Shuffle

When Shuffle is On:

- use accent color;
- keep `aria-pressed="true"`;
- hover may brighten the accent but must not turn it white and visually lose the active-state meaning.

## 5. Stable geometry

Across idle, preparing, confirmation, starting, playing, paused, advancing, stopping, failed/retryable states:

- exactly five visible transport button boxes;
- primary center X drift <= 2 px from viewport center;
- primary center X/Y drift between states <= 1 px;
- all five control center-Y values differ from primary by <= 1 px;
- controls row does not reflow when Stop changes enabled state;
- timeline geometry remains unchanged.

At 1200×760, 1280×720, 800×560, and the existing 640×448 logical-DPI E2E viewport:

- no clipping;
- no horizontal overflow;
- all five buttons remain visible;
- right-side Auto Play/Profile/Utility tools remain visible and do not move the primary off center.

## 6. Auto Play dock button

Move Auto Play out of `ProfileFields`.

Add an always-visible button to `PlayerTools` **before** the Playback Profile button.

DOM order:

```
[player-auto-play-button]
[profile-summary-button]
[player-utility-button]
```

The button updates the same authoritative persisted `settings.auto_play` value already implemented by PR #236.

Requirements:

- icon-only dock control;
- `aria-label="Auto Play"`;
- `aria-pressed={settings.auto_play}`;
- title/tooltip indicates `Auto Play on` / `Auto Play off`;
- Off = muted;
- On = accent;
- enabled hover follows the same brighten behavior as other bottom-dock tools;
- no border;
- transparent background;
- 32×32-ish hit target consistent with Profile/Utility;
- if settings mutation is pending, prevent conflicting repeated writes without making the entire dock reflow.

Do not use a Repeat icon if it could imply repeat-one/repeat-all semantics. Choose an available Lucide icon that communicates continuous/automatic queue playback; accessible label remains authoritative.

## 7. Auto Play persistence and settings

Keep the current accepted semantics:

- persisted setting;
- default On;
- natural finish auto-advances only when On;
- Off leaves current song replayable;
- explicit Next works when Off;
- Stop/failure do not auto-advance;
- dry-run natural finish remains one-shot;
- pure Auto Play patch does not invalidate prepared native schedules;
- Auto Play does not participate in timing/schedule fingerprint.

Keep Auto Play in Settings > Playback.

The dock button and Settings control must mirror one authoritative value.

## 8. Remove Auto Play from quick Playback Profile

The compact profile returns to timing/profile controls only.

Remove:

- `AutoPlaySwitch` from `ProfileFields`;
- `autoPlay` prop from `ProfileFields`;
- `.profile-base-row` layout whose only purpose was Base Hold + Auto Play;
- compact-profile tests/evidence that require the Auto Play switch.

Base Hold should again use the normal compact profile field layout.

The quick profile must remain 260–320 px and should become shorter/simpler after this removal.

## 9. Keep AutoPlaySwitch only where useful

If `AutoPlaySwitch.tsx` remains useful in full Settings, keep it.

Do not duplicate persistence logic between the dock button and Settings.

The dock button should call the same `patchSettings({ autoPlay: ... })` path or a small shared store action that delegates to it.

## 10. Required component tests

### PlayerTransport

For each representative state:

- idle;
- confirmation_required;
- starting with no session response yet;
- playing;
- paused;
- stopping;
- failed/no session;

assert exactly one visible button for each accessible control role:

- Shuffle;
- Previous;
- central Play/Pause/pending control;
- Next;
- Stop.

Assert Stop is present-but-disabled when no session rather than absent.

Assert the primary is never missing when a prepared plan exists.

### PlayerTools

Assert order:

1. Auto Play
2. Configure playback profile
3. Utility panel

Assert:

- Auto Play default `aria-pressed=true`;
- click persists Off;
- `aria-pressed=false` after authoritative update;
- Settings reflects the same value;
- toggling back On works;
- quick Playback Profile no longer contains an Auto Play switch.

## 11. Required E2E visual/interaction tests

### Five controls always visible

At idle before selection and after selecting a song:

- Shuffle visible;
- Previous visible;
- Play visible;
- Next visible;
- Stop visible.

Stop may be disabled but must have a bounding box.

During confirmation:

- five boxes still present;
- center primary still present and stable.

During starting:

- five boxes present;
- primary pending;
- Stop box present even before it becomes actionable.

During playing:

- Shuffle / Previous / Pause / Next / Stop all visible.

After Stop/natural finish:

- returns to Shuffle / Previous / Play / Next / Stop without geometry shift.

### Hover brightness

For enabled secondary buttons, use Playwright hover and computed style to verify:

- default color differs from hover color;
- hover resolves to the intended high-contrast foreground;
- background remains transparent;
- border remains 0;
- bounding box does not move.

For active Shuffle:

- active default = accent;
- hover remains recognizably active/accented.

### Auto Play right dock

Verify bounding-box order:

```
autoPlay.x < profile.x < utility.x
```

and all three share a stable vertical center.

At 800×560 and 640×448 logical viewport:

- all three remain visible;
- no overlap with center transport/timeline;
- Play/Pause remains centered.

## 12. Actual Tauri visual evidence

Replace/update the previous evidence set because the product layout has changed.

Capture on the final exact implementation head:

- 1200×760 idle, showing all five transport buttons + right-side Auto Play/Profile/Utility;
- 1200×760 playing, Shuffle Off;
- 1200×760 playing, Shuffle On;
- 1200×760 Auto Play Off;
- 1200×760 Auto Play On;
- 800×560 idle;
- 800×560 playing;
- Playback Profile open, showing that Auto Play is no longer inside the popover;
- Settings > Playback with Auto Play.

Actual Tauri/WebView2 evidence remains required; browser-shell geometry alone is insufficient.

## 13. Acceptance summary

PR #236 remains Draft until all are true:

- [ ] five transport controls are always visible;
- [ ] Stop is visible-but-disabled rather than unmounted when unavailable;
- [ ] primary center slot is never empty;
- [ ] secondary enabled controls visibly brighten on hover/focus;
- [ ] active Shuffle retains accent state;
- [ ] no secondary border/box returns;
- [ ] Auto Play is a separate bottom-right dock button;
- [ ] Auto Play is left of Profile and Utility;
- [ ] Auto Play is removed from the quick Playback Profile;
- [ ] Settings still mirrors Auto Play;
- [ ] all prior Shuffle/Auto Play behavior tests remain green;
- [ ] exact-head CI is green;
- [ ] updated actual-Tauri screenshots pass manual review.

Do not merge as part of implementation.
