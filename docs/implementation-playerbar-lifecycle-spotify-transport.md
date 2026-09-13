# Player Bar lifecycle correctness and Spotify-like transport handoff

Status: **implementation work order for Codex**.

Base commit: `3925074b205b8fb9dd19515286158bca67f46c2c` (`main` after PR #233).

This is a new, independent draft PR. PR #234 is still open and is intentionally not a dependency. Rebase cleanly if #234 lands before this work is accepted; do not copy its unrelated Timing Margin UI changes into this branch.

The goal is not to cosmetically imitate Spotify. The goal is to make the bottom Player Bar behave like a mature media transport: stable Now Playing identity, coherent lifecycle transitions, reliable natural end-of-song cleanup, correct Previous/Next semantics, and controls whose affordances match what they actually do.

## 0. Priority and scope discipline

Correctness comes before visual parity.

Mandatory for this PR:

1. fix playback/session lifecycle races;
2. separate Library selection from Now Playing identity;
3. make end-of-song transition deterministic and replay/next-safe;
4. prevent double-start and conflicting transport commands;
5. make Play/Pause/Previous/Next semantics coherent;
6. add an explicit playback context sufficient for Previous/Next;
7. add repeat-one/off only if it is implemented end-to-end and tested;
8. add natural-finish tests at store, native-contract, and E2E/mock levels;
9. update Player Bar hierarchy to resemble a mature transport without removing Sky-specific safety controls.

Do **not** add fake or disabled controls merely to look like Spotify.

Do **not** implement seek/scrubbing in this PR. The current timeline is read-only and seeking a SendInput schedule requires a separate safety design for active-key release and schedule reconstruction.

Do **not** add audio volume, device routing, lyrics, or other Spotify-specific controls that have no meaning for Sky Auto Player.

Do **not** change authored timing, SendInput scheduling, Timing Margin, Late Down tolerance, hold/release policy, packet/chord semantics, or the physical dispatch algorithm except where a lifecycle cleanup barrier requires a narrowly scoped orchestration change.

## 1. Confirmed findings on the current code

The following are not speculative design preferences. They are concrete lifecycle or affordance problems in the current implementation.

### F1 — Library selection and Now Playing identity are conflated

Current code deliberately allows the user to browse another song while one is active:

`desktop/src/state/store.ts` currently keeps `playback.songTitle` for the active song while changing `library.selectedSongId`.

At the same time `PlayerTrackInfo.tsx` renders the title from playback state but the Heart action from `library.selectedSongId`.

This creates a real identity bug:

```
A is playing
user selects B in the library
Player Bar title/progress = A
Player Bar Heart action = B
```

A later start can also retain a stale title because the current start path prefers an existing `playback.songTitle` over the newly prepared title.

### F2 — Play has no preparing/start-operation lock

The Play handler performs:

```
prepare
await
start
```

but the UI remains logically idle while the prepare IPC is in flight. A fast double-click can issue overlapping prepare/start flows. Native correctly rejects a second active playback, but the frontend can then surface an error while the first session is actually running.

Risk confirmation buttons have the same class of problem if clicked repeatedly.

### F3 — `starting` renders Pause although native rejects Pause

`PlayerTransport.tsx` treats `starting`, `playing`, `paused`, and `stopping` as one active bucket. For every active state except paused/stopping it renders Pause.

Native only accepts pause from `PlaybackSessionState::Playing`.

Therefore Pause is currently actionable in a state where the backend contract guarantees rejection.

### F4 — terminal UI events are published before the native active slot is fully retired

The native monitor currently publishes terminal state / `playback.finished` while the monitor still owns cleanup work. The active slot is cleared later.

Native start rejects while the active slot is still occupied:

```
another playback session is active
```

This is a critical race for Spotify-like auto-next. A frontend that starts the next song immediately from `playback.finished` can race the old session cleanup.

For this PR, the final terminal event consumed by queue advancement must become a **real retirement barrier**: after the event is observable, an immediate next start must be valid without retrying around a stale active-session owner.

### F5 — natural completion leaves stale terminal session state in the frontend

On `playback.finished`, the current store sets `state = finished` but retains the old `sessionId`, snapshot, and playback identity through object spreading.

That makes terminal state long-lived and forces the next user action to operate through stale session data.

### F6 — normal lifecycle messages are stored as playback errors

`playback.state_changed` currently assigns `event.payload.message` to `playback.error` for every state.

A normal terminal transition carrying `"Playback finished"` can therefore briefly render as a playback error until the following `playback.finished` event clears it.

Message/status and error are different channels and must not be conflated.

### F7 — current SkipForward icon does not mean Next

The current UI uses a forward-skip icon, but native `playback.skip` only terminates/skips the current session. There is no queue/context advancement.

A control that visually means “Next track” must actually start the next track. If the backend primitive remains “skip current session”, keep it internal and compose a real frontend `nextPlayback()` operation around it.

### F8 — Stop/Skip do not have the same pending-operation discipline as Pause/Resume

Pause/Resume expose `pendingCommand`; Stop/Skip can be clicked repeatedly before a lifecycle event makes the transition visible.

All transport commands need one coherent operation gate.

### F9 — mock/E2E playback does not naturally finish by duration

The mock bridge transitions to playing but does not automatically reach natural end-of-song. Existing E2E therefore proves manual pause/resume/stop but not:

```
play -> natural finish -> cleanup -> replay/next
```

This test gap must be closed.

### F10 — retired session tracking is unbounded

`retiredSessionIds` is a process-lifetime Set. It must remain bounded or be replaced by a session-generation/ownership scheme.

## 2. Required playback model

Do not continue using `library.selectedSongId` as the implicit playback owner.

Introduce an explicit playback identity and context. Exact type names may differ, but the model must provide the following concepts.

### 2.1 Browser selection

```ts
library.selectedSongId
```

Meaning only:

> the song the user is currently browsing/inspecting in the Library.

Changing it while another song is playing must not mutate Now Playing identity, controls, progress, like state, or queue position.

### 2.2 Now Playing identity

Playback needs an explicit identity, for example:

```ts
playback.currentSong: {
  songId: string
  title: string
} | null
```

The Player Bar title, Heart action, duration/progress ownership, risk indicator, and transport operations must use `playback.currentSong`, not Library selection.

When A is playing and B is selected:

```
library.selectedSongId = B
playback.currentSong.songId = A
Player Bar = A
Heart in Player Bar = A
Song Details pane may show B
```

### 2.3 Playback context

Capture enough information when a song is started to support Previous/Next without depending on whichever row is currently selected later.

A context should be equivalent to:

```ts
type PlaybackContext = {
  source: LibrarySource
  query: string
  generation: number
  currentIndex: number
  currentSongId: string
}
```

Do not store full `SongRow[]` for an arbitrarily large library. Reuse the existing paged library/search infrastructure to resolve adjacent indices lazily.

The context is frozen for the session/queue traversal. Browsing to another source or typing a new search must not silently rewrite the context of the currently playing song.

If the captured catalog generation becomes stale, fail the Next/Previous operation cleanly and create a new context from an explicit user start rather than guessing.

## 3. Required transport operation state

The UI needs to distinguish playback state from an in-flight user command.

Use one coherent operation field rather than unrelated booleans, e.g.:

```ts
type TransportOperation =
  | null
  | 'preparing'
  | 'starting'
  | 'pausing'
  | 'resuming'
  | 'stopping'
  | 'advancing'
  | 'restarting'
```

The exact representation may differ, but these invariants are mandatory:

- one user transport operation owns the control surface at a time;
- double-click Play cannot create two prepare/start flows;
- confirmation buttons cannot start the same prepared plan twice;
- Stop and Next cannot run concurrently;
- Pause cannot race Resume;
- operation state is cleared on authoritative lifecycle completion or command failure;
- stale promise continuations from an older operation cannot overwrite a newer session.

Use epochs/tokens where necessary. Do not rely only on disabling a React button; the store action itself must be idempotent/race-safe.

## 4. Required state-machine behavior

### 4.1 Idle / selected song

A selected Library row with no current playback shows a Play action.

Click Play:

```
idle
-> preparing
-> [confirmation_required | starting]
-> playing
```

During `preparing` and `starting`:

- primary Play/Pause control is disabled or shows a progress affordance;
- Pause is **not** available;
- a second Play is ignored/rejected locally;
- Stop may be available only after native has created a session that can actually be stopped.

### 4.2 Playing

Controls:

- Previous
- Pause
- Next
- Stop safety control
- Repeat state if implemented

Browsing another song must not affect Now Playing.

### 4.3 Pause / Resume

```
playing -> pausing -> paused
paused  -> resuming -> playing
```

The UI may optimistically indicate the operation is pending, but the authoritative playback state still comes from native lifecycle events.

A missing matching lifecycle event must not leave the UI disabled forever. Add bounded reconciliation appropriate to the existing architecture, or prove that the event channel fails closed into an explicit fatal/error state.

### 4.4 Stop

Stop means:

> terminate the current playback and do not automatically advance.

Required:

```
playing/paused/starting
-> stopping
-> terminal retired
-> stable ready/idle UI
```

Stop must not be presented as Previous or Next.

### 4.5 Natural finish without an enabled next item

After successful natural end:

- old session is retired;
- `sessionId = null`;
- `pendingCommand/transportOperation = null`;
- old snapshot is cleared or converted to a deliberate stable completed presentation;
- playback error is null;
- progress returns to a stable non-playing value;
- Player Bar may retain the just-finished `currentSong` so Play means “play this song again”;
- it must not remain logically `playing`, `starting`, `stopping`, or bound to the retired session ID.

Choose one stable terminal UI contract and test it. Recommended:

```
playback.phase = 'idle'
playback.currentSong = last played song
sessionId = null
snapshot = null
```

### 4.6 Natural finish with a valid next item

Do not auto-start from a non-retired terminal signal.

Sequence must be equivalent to:

```
A terminal event that guarantees A is retired
-> resolve next item from frozen playback context
-> prepare B
-> start B
-> B becomes currentSong
```

There must be no observable error `another playback session is active` in the valid path.

### 4.7 Failure

Failure must:

- retire the failed session;
- clear transport pending state;
- preserve a useful current-song identity for diagnosis/retry;
- display the actual failure as an error;
- reject late events from the failed session after a new session starts;
- allow an explicit retry after retirement.

Normal informational messages must not populate `playback.error`.

## 5. Native terminal retirement contract

This is the most important backend acceptance item.

Define and document one lifecycle event as the frontend **retirement barrier**. Prefer using the existing terminal events:

- `playback.finished`
- `playback.failed`

After one of these events is delivered for session A, a new valid playback start must not fail because session A still owns:

- `NativePlaybackService.active`;
- the activity coordinator playback reservation;
- a live player worker ownership that blocks the next session;
- pending control state.

It is acceptable to publish `playback.state_changed { state: finished/failed }` earlier for display/diagnostics, but queue advancement must use the final retirement-barrier event.

Do not paper over this with arbitrary sleeps in React.

Do not implement a blind retry loop around `another playback session is active`.

Add a native regression test that makes the terminal event observable and immediately attempts the next start/reservation. The test must prove the barrier contract.

Preserve:

- physical key cleanup;
- sender trace finalization;
- bounded shutdown;
- event ordering guarantees;
- diagnostics ownership.

If making the terminal event a true barrier requires moving event publication after cleanup, update the native event-order tests accordingly and explain the new contract.

## 6. Previous / Next semantics

### 6.1 Next

The visible Next control must mean:

> advance to the next song in the frozen playback context and start it.

If currently active:

1. initiate a safe skip/stop of the current native session;
2. wait for the retirement barrier;
3. resolve context index + 1;
4. lazily ensure/fetch the target row using the existing paged library infrastructure;
5. prepare the target with the current user settings for the new session;
6. start it;
7. update `playback.currentSong` only when ownership is valid.

If there is no next item:

- finish current playback normally;
- land in stable idle/ready state;
- do not wrap unless a repeat-context mode is explicitly implemented.

### 6.2 Previous

Use familiar media-player behavior:

- if the current playback position is greater than approximately **3 seconds**, Previous restarts the current song from the beginning;
- otherwise it moves to the previous item in the playback context.

The 3-second threshold may be a named constant and should be covered by tests.

Restart must be a real safe playback transition:

```
retire current session
-> prepare current context item
-> start new session
```

Do not reset the progress bar locally while the old native session continues.

At context index 0 with no repeat-context mode, Previous should restart the current song rather than underflow.

## 7. Repeat behavior

At minimum, implement **Repeat Off** and **Repeat One** if the control is shown.

Repeat One means:

- a natural successful finish restarts the same context item after the retirement barrier;
- explicit Stop does **not** repeat;
- explicit Next advances and does **not** get trapped by repeat-one;
- failure does not automatically repeat;
- restart uses a newly prepared session and current settings.

Do not add Repeat Context/All unless its end-of-context semantics are implemented and tested.

If Repeat is not implemented end-to-end, omit the button entirely. Do not render a nonfunctional Spotify-looking icon.

## 8. Shuffle

Shuffle is **not mandatory for this PR**.

Do not implement shuffle by materializing an unbounded library into memory or by randomizing only the currently loaded virtualized rows.

If a bounded, deterministic, context-correct shuffle design naturally falls out of this work, document it separately for follow-up. Do not expand this PR just to match Spotify visually.

## 9. Player Bar UX hierarchy

Use the Spotify screenshot only as an interaction-hierarchy reference, not a pixel-copy target.

### Left zone — Now Playing

Must display data from `playback.currentSong` while a session/current playback identity exists.

Recommended content:

- song icon/artwork placeholder;
- current song title;
- useful secondary metadata/status;
- Heart button for the **current song**, never the merely selected Library row.

When no current song exists, show the selected-song affordance or the existing “No song selected” state consistently.

### Center zone — transport

Target hierarchy:

```
Previous   Play/Pause   Next
          progress
```

Repeat One may be added adjacent if implemented.

Keep **Stop** as a Sky-specific safety action. It may be visually secondary and placed beside the main controls, but it must remain discoverable during an active physical session.

State-specific behavior:

- preparing/starting: disabled primary control or spinner/progress affordance; no invalid Pause;
- playing: Pause;
- paused: Resume/Play;
- stopping/advancing: disable conflicting controls;
- idle with a current/selected song: Play.

Next and Previous must expose correct disabled states at context boundaries.

### Right zone

Keep Sky-specific tools/profile/diagnostics behavior. Do not add Spotify volume/device controls.

## 10. Timeline behavior

Keep the timeline read-only in this PR.

Requirements:

- progress belongs to `playback.currentSong` / active session, not Library selection;
- stale snapshots from retired sessions cannot move the progress of a new session;
- on pause the progress remains stable;
- on resume it continues;
- on natural finish/reset it follows the chosen stable terminal UI contract;
- starting a new song clears old progress before the new authoritative snapshot arrives.

Do not convert `<progress>` into a seek slider.

## 11. Event ownership and stale-event rejection

Current retired-session protection is directionally correct but must be bounded.

Required:

- a late state/snapshot/terminal event from session A cannot mutate session B;
- a terminal event from an already retired session is idempotently ignored;
- pending start events that arrive before the start promise resolves remain supported;
- replacing the unbounded `retiredSessionIds` Set with a bounded generation/last-retired structure is preferred;
- if a small bounded LRU/set is used, document its capacity and why it is sufficient.

Tests must exercise event-before-promise and late-event-after-new-session cases.

## 12. Mock bridge requirements

Upgrade `desktop/src/bridge/mockBridge.ts` so tests can model real lifecycle, not just manual controls.

Provide a deterministic short-duration playback mode or test seam that can emit:

```
starting
playing
snapshots with advancing current_us
finished state
playback.finished terminal event
```

The terminal event must respect the same retirement-barrier semantics expected from native.

Avoid long real-time waits in tests. Use deterministic/fake timers or a bounded short test duration.

Mock Next/Previous tests must not rely on all 500 rows already being loaded.

## 13. Required tests

### 13.1 Store/unit tests

Add focused tests for at least:

1. browsing B while A plays keeps `currentSong = A`;
2. Player Bar like action targets A, not B;
3. starting B after A finishes cannot retain A's title;
4. double Play creates one prepare/start operation;
5. double confirmation creates one start;
6. Pause is not issued while `starting`;
7. Pause/Resume pending state clears from matching lifecycle events;
8. normal `Playback finished` message is not stored as an error;
9. natural finish clears stale session/snapshot state;
10. natural finish followed immediately by replay succeeds;
11. stale A events cannot mutate B;
12. Next advances context;
13. Previous after >3 s restarts current item;
14. Previous near 0 moves to prior context item;
15. explicit Stop never auto-advances;
16. Repeat One restarts only after natural successful finish if Repeat One is implemented;
17. retired-session bookkeeping remains bounded.

### 13.2 Native tests

Add/modify tests proving:

- terminal state event ordering;
- final `playback.finished` is a real retirement barrier;
- final `playback.failed` is a real retirement barrier;
- immediate next start/reservation after the barrier succeeds;
- Stop remains idempotent;
- Skip primitive safely terminates the current session;
- no active physical key/session ownership leaks across terminal cleanup.

Do not weaken existing sender/dispatch tests.

### 13.3 Component tests

PlayerTransport:

- starting does not render an enabled Pause;
- playing renders Pause;
- paused renders Resume;
- operation pending disables conflicting commands;
- Next/Previous accessible names and disabled boundaries are correct;
- Stop remains available in allowed active states.

PlayerTrackInfo:

- uses current playback identity for title/Heart while browsing another row;
- falls back coherently when no current song exists.

### 13.4 E2E

Add real user-flow E2E using the improved mock bridge:

#### Natural finish

```
select A
Play
wait for Playing
let A naturally finish
assert no Playing/Starting/Stopping remains
assert session ownership is reset
assert progress/state is stable
Play again
assert a new session starts
```

#### Browse during playback

```
play A
select B in Library
assert Player Bar still shows A
assert Player Bar Heart affects A
```

#### Immediate next

```
play A
Next
assert A retires
assert B becomes Now Playing
assert no "another playback session is active" error
```

#### Natural auto-next if enabled by the implementation

```
play A with a next context item
A finishes naturally
B starts only after A retirement barrier
```

#### Double interaction

Use rapid double click on Play and confirmation and prove only one session becomes active.

#### Starting state

Hold the mock in `starting`; assert Pause cannot be invoked.

#### Narrow viewport

The added controls must not create horizontal overflow at the currently supported narrow desktop viewport.

## 14. Native/Tauri manual acceptance

This work cannot be accepted only from the web mock.

Before final handoff, run the actual Tauri app on Windows and verify:

- natural physical or dry-run finish transitions back cleanly;
- immediate replay after finish works;
- selecting another Library song while playing does not alter Now Playing;
- rapid Play clicks do not create duplicate starts/errors;
- Pause is unavailable during pre-roll/starting;
- Pause/Resume does not get stuck;
- Stop returns to a clean state;
- Next changes to the next context item without stale-session errors;
- no stuck physical key remains after Stop/Next/failure.

Capture concise before/after evidence or screenshots if practical, but do not commit large binary artifacts unless the repository already has a place for them.

## 15. Validation commands

Run at minimum:

```powershell
cargo xtask check desktop
cd desktop
bun run check
bun run build
bun test
bun run test:e2e
bun run test:e2e:bundle
```

Also run any native/Tauri playback tests touched by the lifecycle changes and the canonical GitHub Actions validation path.

A Vite bundle-size warning alone is not a failure unless this PR materially regresses bundle size.

## 16. Scope guardrails

Do not:

- change timing defaults (Timing Margin 500 us, Late Down tolerance 2000 us);
- change authored hold/release equations;
- change SendInput packet construction;
- change chord grouping or scan codes;
- add arbitrary sleep/retry delays to hide lifecycle races;
- add a frontend-only fake seek;
- auto-change Library selection every time playback advances unless product behavior explicitly requires it;
- make Next depend on whatever search text happens to be on screen after the playback context was created;
- implement an unbounded queue snapshot;
- remove Stop as a safety control;
- merge this PR.

If a lifecycle fix requires a native contract change, keep it tightly scoped and test the contract directly.

## 17. Suggested implementation order

Codex should implement in this order and keep each stage testable:

1. add failing tests for F1/F2/F3/F5/F6;
2. introduce explicit current-song identity and transport-operation state;
3. harden prepare/start against duplicate/stale operations;
4. fix PlayerTransport state mapping;
5. refactor native terminal publication into a proven retirement barrier;
6. reset frontend terminal session ownership;
7. make retired-session handling bounded;
8. introduce frozen playback context;
9. implement real Previous/Next composition;
10. implement Repeat One only if fully end-to-end;
11. upgrade mock natural-finish lifecycle;
12. add E2E and actual-Tauri manual acceptance;
13. polish Player Bar layout only after state semantics are green.

Do not begin with icons/CSS.

## 18. Definition of Done

The PR is ready for independent acceptance only when all of the following are true:

- [ ] Library selection and Now Playing are separate identities.
- [ ] Player Bar Heart always acts on Now Playing.
- [ ] No stale title survives into a newly started song.
- [ ] Double Play/confirmation cannot create duplicate sessions.
- [ ] Starting does not expose an invalid Pause action.
- [ ] Pause/Resume cannot remain permanently pending in the normal event path.
- [ ] Stop/Next/Previous use one coherent transport-operation gate.
- [ ] `playback.finished` / `playback.failed` terminal contract is a real retirement barrier.
- [ ] Natural finish removes stale session/snapshot ownership.
- [ ] Normal finish messages are not shown as errors.
- [ ] Next really starts the next context item.
- [ ] Previous follows the restart-vs-previous threshold semantics.
- [ ] Repeat One, if shown, works end-to-end and never repeats explicit Stop/failure.
- [ ] No fake Shuffle/Seek controls are present.
- [ ] Stale old-session events cannot mutate the new session.
- [ ] Retired-session bookkeeping is bounded.
- [ ] Mock tests include natural end-of-song.
- [ ] E2E covers natural finish, replay, browse-during-playback, double Play, and Next.
- [ ] Actual Tauri manual playback lifecycle has been checked on Windows.
- [ ] Canonical exact-head CI is green.
- [ ] Final Codex report lists changed files, exact head SHA, tests, CI run, and any intentionally deferred Spotify-like features.

Leave the PR draft and unmerged for independent review.
