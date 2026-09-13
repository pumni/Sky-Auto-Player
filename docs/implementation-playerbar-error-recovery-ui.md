# Player Bar error-recovery UI — blocking follow-up for PR #235

Status: **REQUEST CHANGES / blocking recovery UX work order**.

PR: #235  
Branch: `docs/playerbar-lifecycle-spotify-handoff-2026-09`  
Reviewed implementation head before this handoff: `75edbd21d78a5bd3e5dc6e451764c39c915dc45a`.

This work order is specifically about **recoverability and error-state UX** after the Player Bar layout recovery. The current transport alignment is much better, but the error state still leaves the user with a disabled primary control and no obvious way to recover.

Do not merge PR #235 until this work order is satisfied.

## 1. Preserve the accepted work

Do not regress:

- explicit Now Playing identity;
- duplicate-start protection;
- native retirement barrier;
- authoritative playback status query;
- `prepared_id` correlation;
- natural finish auto-next;
- Previous/Next semantics;
- Stop preemption;
- confirmation ownership;
- bounded retired-session bookkeeping;
- stale-session event rejection;
- late-start/orphan cleanup;
- current Player Bar controls-row + separate timeline-row layout.

This round should be a focused recovery UX + state reconciliation improvement.

## 2. Confirmed failure in the actual Tauri UI

The actual Tauri screenshot at the reviewed head shows:

- a playback status timeout;
- an error banner that is visually misaligned/cropped;
- the primary control rendered as a disabled pending spinner;
- Previous/Next/Stop visible but the recovery path is not discoverable;
- the banner text says controls are available, but it does not present a clear Retry or Stop action.

The user experience is effectively:

```
Playback unavailable
Native playback status query timed out...
[disabled spinner]
```

The user cannot confidently tell how to recover or start playback again.

This is a release-blocking UX failure even if the underlying Stop button is technically callable.

## 3. Error-state design principle

There must be **no valid playback state** where the only obvious primary affordance is a disabled spinner and the user has no explicit recovery action.

Every recoverable error must present a clear next action.

The UI must distinguish:

1. recoverable status/reconciliation warning;
2. real playback failure;
3. target/environment failure;
4. native state unknown.

Do not label every timeout as “Playback unavailable”.

## 4. Required recovery matrix

Implement UI/state behavior equivalent to the following matrix.

### Case A — no native session exists

Condition:

```
status.active == null
and no known session remains owned
```

Required UI:

```
Playback could not start
No active playback session was created.

[ Try again ]
```

Requirements:

- `startRequestId = null`;
- transport operation cleared;
- state is retryable;
- primary Play is enabled if a valid song/current song exists;
- no disabled spinner remains;
- Try again may call the same authoritative prepare/start flow as Play.

### Case B — native session is Playing

Required UI:

- normal Playing transport;
- Pause available;
- Stop available;
- no error panel saying “Playback unavailable”.

If the session was restored through reconciliation, a small neutral notice is allowed:

```
Playback state recovered.
```

Do not store successful recovery in `playback.error`.

### Case C — native session is Paused

Required UI:

- Resume available;
- Stop available;
- no blocking error panel.

### Case D — native session exists but state is Starting / Stopping / temporarily unknown

Required error/recovery surface:

```
Playback status needs attention
The app could not confirm the current session state.

[ Retry status ]    [ Stop playback ]
```

Requirements:

- `Retry status` calls the authoritative status query;
- `Stop playback` is a visible text action and calls the existing safe Stop path;
- no requirement for the user to infer that a small square icon means recovery;
- primary pending control may remain non-interactive while ownership is unknown, but the banner must provide explicit recovery actions.

### Case E — session already terminal

Status reconciliation must retire/reset local ownership and return to:

```
Play / Try again
```

No stale spinner.

### Case F — status query itself is unreachable

If frontend still knows `sessionId`:

```
Playback status is unavailable
Sky Auto Player cannot confirm the current session.

[ Retry status ]    [ Stop playback ]
```

If frontend has **no authoritative session ID**, do not create a fake frontend-only reset that could overlap an unknown native physical session.

Use the existing late-response/orphan cleanup guarantees and return to retryable idle only when the store can prove it no longer owns a native session.

If that proof cannot be made safely with the current contract, introduce a narrowly scoped native recovery command rather than lying to the UI.

Any new native recovery primitive must:

1. discover current active playback authoritatively;
2. request safe stop if one exists;
3. ensure physical key/session cleanup;
4. wait for retirement;
5. return authoritative clean status.

Do not implement arbitrary sleeps or blind retries.

## 5. Add an explicit Retry Status store action

Add a first-class store action, naming may vary:

```ts
retryPlaybackStatus(): Promise<void>
```

or:

```ts
reconcilePlaybackNow(): Promise<void>
```

Requirements:

- calls the existing authoritative `bridge.getPlaybackStatus()`;
- uses the same identity/reconciliation logic as watchdog recovery;
- does not duplicate a second divergent reconciliation implementation;
- concurrency-safe: repeated clicks cannot run conflicting reconciliation;
- stale result cannot overwrite a newer playback session;
- clears warning/error when recovery succeeds;
- preserves actionable failure details if terminal failure is recovered.

Do not make `PlaybackAdmission` call bridge methods directly.

## 6. Error state must carry severity/category

The current `playback.error: string | null` is too coarse for presentation.

Introduce the smallest model needed to distinguish at least:

```ts
playback.issue = {
  kind: 'recoverable_status' | 'playback_failure' | 'target_failure' | 'conflict'
  message: string
} | null
```

Exact representation may differ.

If changing the state shape broadly is too invasive, derive an equivalent typed presentation state from existing error codes and ownership.

Important rule:

- successful reconciliation != error;
- status timeout with recovery actions != hard unavailable;
- `target_not_found`, integrity mismatch, focus failure, etc. may be real actionable failures.

Do not lose native `failure_code` and `failure_message`.

## 7. PlaybackAdmission / recovery surface

The error/recovery surface must stop being a passive text-only alert.

For recoverable status issues, render:

```
[ concise title ]
[ concise explanation ]             [ Retry status ] [ Stop playback ]
```

or a compact equivalent.

Requirements:

- actions are keyboard accessible;
- Retry and Stop have visible text labels;
- Stop only renders when a known session exists and Stop is valid;
- Try again renders when no session remains and playback is retryable;
- actions reuse store methods;
- loading state on Retry must not cause geometry reflow;
- retry action itself can show a small spinner inside the button if necessary.

For hard target failures:

```
Sky window was not found.
Open Sky and make sure its window is visible, then try again.

[ Try again ]
```

No generic “Native playback worker failed” when a typed message is available.

## 8. Fix the banner positioning

The current error panel is misaligned/cropped in the actual Tauri screenshot.

Do not rely on a fragile:

```css
left: 50%;
transform: translateX(-50%);
```

inside a parent whose containing block behaves differently in WebView2.

Use a predictable positioning scheme.

Preferred:

```css
.player-admission-error {
  left: 12px;
  right: 12px;
  width: auto;
  transform: none;
}
```

with a nested inner content container if a maximum readable line length is desired.

Alternative:

- create a full-width admission overlay row;
- center an inner panel via normal flex/grid layout.

Acceptance:

- left and right edges stay inside the client area;
- no text is clipped at 1200×760, 1280×720, 800×560;
- banner is visually aligned with the Player Bar/workbench margins;
- no horizontal scrollbar;
- long messages wrap or truncate deliberately;
- action buttons remain visible.

## 9. Avoid duplicate error messaging

Current UI can show:

- large error banner;
- Player Track subtitle “Playback error”.

For a recoverable reconciliation warning, do not show both with equal severity.

Recommended:

- PlayerTrackInfo continues showing song metadata/state;
- recovery banner owns the detailed warning and actions.

For a hard failure, PlayerTrackInfo may show a compact state such as `Playback failed`, while the banner gives the actionable detail.

Avoid:

```
Playback unavailable
...
Playback error
```

as two redundant high-severity messages.

## 10. Primary transport behavior during recovery

### Known active Playing

Primary = Pause.

### Known active Paused

Primary = Resume.

### Known active Starting/Stopping

Primary may remain disabled/pending, but explicit Retry Status / Stop Playback must be available in the recovery panel where valid.

### No session / retryable idle

Primary = Play.

### Hard target failure

Primary can be Play/Try again if native ownership is clean.

There must be no terminal/recoverable state where:

- `sessionId == null`;
- `startRequestId == null`;
- no operation is active;

but the primary remains a disabled spinner.

## 11. Stop discoverability

Keep the square Stop icon in the normal transport for experienced users.

But during a recoverable error with an owned session, the recovery surface must also contain:

```
Stop playback
```

as a text button.

This is intentional duplication for safety/discoverability.

Do not remove the normal transport Stop icon.

## 12. Retry semantics

`Retry status` means:

> ask native for authoritative playback state and synchronize the frontend.

It does **not** mean start a new song.

`Try again` means:

> retry prepare/start because the previous attempt is confirmed not to own a native session.

Do not use the same label for both actions.

## 13. Required unit/store tests

Add tests for at least:

1. status timeout + no active session -> Play/Try again becomes available;
2. status timeout + active Playing -> restore Playing, no hard error;
3. status timeout + active Paused -> restore Paused;
4. status timeout + active Starting -> recovery issue exposes Retry + Stop;
5. missing terminal event + terminal status -> clear ownership and return retryable;
6. status query failure with known session -> issue remains recoverable and Stop remains callable;
7. repeated Retry Status clicks do not create conflicting reconciliations;
8. stale Retry Status result cannot overwrite a newer session;
9. successful Retry clears recoverable issue;
10. hard target failure preserves typed message;
11. replay-same-song stale terminal regression remains green;
12. late-start/orphan cleanup remains green.

## 14. Required component tests

`PlaybackAdmission` / recovery UI:

- recoverable warning renders `Retry status`;
- renders `Stop playback` when a session exists;
- renders `Try again` when ownership is clean;
- hard target failure does not render misleading Stop if no session exists;
- successful recovery removes the alert;
- action buttons have accessible names;
- no passive “controls are available” text without actions.

`PlayerTransport`:

- no-session retryable state renders Play;
- known playing state renders Pause;
- known paused state renders Resume;
- unknown-owned-session state does not pretend Play is safe;
- Stop remains available where allowed.

## 15. Required E2E

Add browser E2E using deterministic mock seams.

### Recoverable status timeout with known session

```
start A
force status timeout/lost lifecycle event
assert recovery banner visible
assert Retry status visible
assert Stop playback visible
click Retry status
mock reports Playing
assert banner disappears
assert Pause is visible
```

### No-session recovery

```
start attempt never creates session
status says no active session
assert Play/Try again becomes enabled
assert no disabled spinner remains
```

### Stop from recovery panel

```
known session + status unavailable
click Stop playback
assert native/mock Stop invoked
assert terminal retirement returns UI to Play
```

### Hard target failure

```
target_not_found
assert actionable Sky-window message
assert Try again visible
assert no misleading generic unavailable/retry-status wording
```

### Layout

At 1200×760, 1280×720, 800×560:

- recovery banner fully inside viewport;
- no clipping;
- action buttons visible;
- transport row/timeline geometry unchanged when banner appears;
- footer outer height unchanged.

## 16. Actual Tauri acceptance

Mandatory.

On the exact implementation head, capture actual Tauri screenshots for:

- normal idle;
- normal playing/dry-run playing;
- recoverable status timeout with known session;
- target-not-found or equivalent hard failure;
- retryable no-session state.

At minimum inspect 1200×760 and 800×560.

The reviewed failure screenshot showed clipping in Tauri that browser E2E did not catch. Therefore actual Tauri visual verification is a hard gate.

## 17. Expected file scope

Likely:

- `desktop/src/state/store.ts`
- `desktop/src/state/store.test.ts`
- `desktop/src/components/player/PlaybackAdmission.tsx`
- `desktop/src/components/player/PlayerTransport.tsx`
- `desktop/src/components/player/PlayerTrackInfo.tsx` only if duplicate error text needs cleanup
- `desktop/src/styles/player.css`
- `desktop/tests/e2e/library.spec.ts`
- mock bridge test seams if necessary

Native changes only if current authoritative status contract cannot safely support unknown ownership recovery.

If native changes are required, keep them narrowly scoped to playback recovery/status and add direct native tests.

Do not touch scheduler/SendInput/timing semantics.

## 18. Validation

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

Actual Tauri screenshots remain mandatory in addition to CI.

## 19. Definition of Done

PR #235 is not ready until:

- [ ] recovery banner is visually aligned and not clipped;
- [ ] recoverable timeout does not say “Playback unavailable” by default;
- [ ] known session recovery exposes explicit `Retry status` + `Stop playback`;
- [ ] no-session recovery exposes Play/`Try again`;
- [ ] successful reconciliation clears the warning;
- [ ] hard failures keep actionable typed messages;
- [ ] no state leaves the user with only a disabled spinner and no obvious escape;
- [ ] Player Bar transport geometry remains stable when the recovery panel appears;
- [ ] actual Tauri screenshots confirm the result;
- [ ] lifecycle anti-stuck tests remain green;
- [ ] exact-head CI is green.

Keep PR #235 Draft and unmerged until independent acceptance.
