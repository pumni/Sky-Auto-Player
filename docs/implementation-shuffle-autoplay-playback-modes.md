# Shuffle + Auto Play playback modes handoff

Status: **stacked draft implementation work order**.

This branch is intentionally stacked on PR #235. Do not merge it before #235 is accepted and merged.

Base when created: `b98c2adc124c5670362632de527d350a1b4ce786`.

The goal is to add two user-facing playback modes with real semantics:

1. **Shuffle** — a Spotify-like transport toggle that changes traversal order for Previous / Next / natural advance.
2. **Auto Play** — a persisted user preference that controls whether a successful natural finish automatically advances to the next item.

Do not add decorative icons without queue semantics.

## 1. Scope and invariants

Preserve all lifecycle correctness from PR #235:

- Now Playing identity is separate from Library selection.
- Native retirement barrier remains authoritative.
- No next/restart before old session retirement.
- Stop never auto-advances.
- Failure never auto-advances.
- Stale session events remain rejected.
- No frontend-only seek.
- No fake Shuffle.
- No unbounded row snapshot of the Library.
- No timing/SendInput/scheduler changes.

## 2. Shuffle UI

Add a Shuffle button to the main Player Bar transport, matching the familiar media-player hierarchy.

Target order:

```
Shuffle   Previous   Play/Pause   Next   Stop
```

Requirements:

- use the Lucide Shuffle icon;
- Shuffle is a secondary borderless control like Previous/Next/Stop;
- default background transparent;
- inactive color = normal secondary;
- active color = accent;
- `aria-label="Shuffle"`;
- `aria-pressed="true|false"`;
- tooltip/title reflects state if useful;
- toggling Shuffle must not move the centered Play/Pause control;
- Play/Pause remains the visual center anchor.

Do not add Repeat unless separately requested.

## 3. Shuffle state model

Shuffle is a **playback-mode/session preference**, not a timing setting.

Add store state equivalent to:

```ts
playback.shuffleEnabled: boolean
```

Default: `false`.

It may be toggled before playback starts or while a song is Playing/Paused.

Do not persist Shuffle in the native settings file for this work unless there is a compelling existing product convention. The default on application launch remains Off.

When no playback context exists, the button may still toggle the desired mode for the next explicit start if a song is selected. If no song/context can ever be traversed, it may be disabled, but do not make the behavior inconsistent across idle vs playing without a reason.

## 4. Shuffle traversal semantics

The Shuffle button must alter actual playback traversal.

When Shuffle is OFF:

```
Next -> currentIndex + 1
Previous near beginning -> currentIndex - 1
natural auto-advance -> currentIndex + 1
```

When Shuffle is ON:

- current song remains current when Shuffle is enabled;
- future traversal follows a deterministic shuffled permutation of the captured playback context;
- no song repeats until all items in the context have been visited;
- the current song is the first element / position 0 of the newly created shuffled traversal;
- Previous walks backward through the actual shuffled traversal;
- Next walks forward through it;
- natural Auto Play uses the same shuffled Next resolution;
- explicit Stop does not advance;
- end of shuffled traversal stops unless a separately implemented repeat mode says otherwise.

### Previous threshold

Preserve existing semantics:

- if current playback position > approximately 3 seconds, Previous restarts the current song;
- otherwise it moves to the previous traversal position.

In Shuffle mode, “previous” means previous shuffled traversal item, not numerical library index - 1.

## 5. Do not materialize an unbounded queue

Do not copy all `SongRow` objects into playback state.

A preferred O(1)-memory traversal design is a deterministic permutation over context indices.

One acceptable model:

```ts
type ShuffleTraversal = {
  originIndex: number
  step: number
  position: number
}
```

For a context of `total = N`:

- choose a seed-derived `step` in `[1, N-1]`;
- ensure `gcd(step, N) == 1`;
- `originIndex` is the current song index when Shuffle is initialized;
- target index for traversal position `p` is:

```
(originIndex + p * step) mod N
```

Because `step` is coprime with `N`, every index is visited exactly once before repetition.

The exact permutation implementation may differ if it is deterministic, testable, bounded, and produces no repeats.

Do not use `Math.random()` at every Next call because that allows repeats and makes Previous/history semantics incoherent.

A random/opaque seed may be generated once when Shuffle is enabled/new context is created.

## 6. Shuffle toggle behavior during a context

### OFF -> ON while A is playing

- A remains current.
- Build a new shuffled traversal anchored at A.
- traversal position = 0.
- next action uses shuffled position 1.

### ON -> OFF while A is playing

- A remains current.
- clear shuffle traversal metadata;
- sequential Next starts from A's actual `currentIndex + 1`.

### New explicit song start

If Shuffle is enabled before the explicit start:

- capture the new playback context;
- initialize shuffled traversal anchored on that selected song.

### Context mutation

Existing membership revision invalidation rules remain authoritative.

If Liked Songs / playlist membership invalidates the current playback context:

- shuffled traversal becomes invalid with the context;
- Previous/Next must not use stale shuffled indices;
- the next explicit start creates a fresh context/traversal.

Do not silently regenerate a shuffle against changed membership while a session still believes it owns the old context.

## 7. Auto Play semantics

Add a persisted setting labeled **Auto Play**.

Default: **ON**.

Default ON is required to preserve the current PR #235 natural auto-next behavior for existing users/configurations.

Auto Play controls only:

> whether a successful natural completion automatically starts the next item from the current playback context.

It does **not** control:

- explicit Play;
- explicit Next;
- Previous;
- Stop;
- retry after failure;
- target-not-found recovery.

### Auto Play ON

On natural successful finish:

- wait for the authoritative retirement barrier;
- if context is valid and a next traversal item exists:
  - sequential next when Shuffle OFF;
  - shuffled next when Shuffle ON;
- prepare and start next song.

### Auto Play OFF

On natural successful finish:

- retire the old session;
- stay on the just-finished current song;
- reset to stable idle/replay state;
- Play means replay current song;
- do not move Library selection automatically.

### End of context

Even with Auto Play ON:

- if no next traversal position exists, stop in the stable replayable state;
- do not wrap.

### Stop / failure

Never auto-advance.

### Dry-run / Test playback

The quick-profile **Test playback (no input)** is a one-shot diagnostic action.

Do not chain Auto Play across dry-run test sessions.

A context created for dry-run may retain traversal metadata for manual Next if the current architecture supports it, but natural dry-run completion must not auto-start additional songs.

## 8. Persist Auto Play as a behavior setting, not timing identity

Auto Play does not change the authored schedule or native dispatch plan.

Do not put it into:

- `PlaybackConfigDto`;
- plan fingerprint;
- scheduler cache identity;
- physical timing policy.

Prefer a native-owned settings model such as:

```rust
pub struct PlaybackBehaviorSettings {
    pub auto_play: bool,
}
```

inside `ApplicationSettings`, with a corresponding patch structure/DTO.

Alternative naming is acceptable, but keep the behavior distinct from schedule-affecting timing defaults.

### Schema migration

The durable settings schema changes.

Requirements:

- increment settings schema version appropriately;
- old configs missing Auto Play migrate/default to `true`;
- normalized/persisted JSON contains the value;
- Wave 2 migration fixtures/tests are updated surgically;
- do not rewrite unrelated numeric representation in fixtures.

### Patch semantics

Toggling Auto Play must **not invalidate an already prepared native schedule** because Auto Play does not affect that schedule.

Current settings patching invalidates prepared playback broadly. Refine invalidation so a pure Auto Play behavior patch:

- persists authoritatively;
- updates frontend settings;
- does not clear/recompile an existing prepared plan;
- does not alter settings/plan fingerprint;
- does not invalidate timing analysis cache unless there is another schedule-affecting field in the same patch.

If a patch also changes Hold/Tempo/FPS/Timing Margin/etc., retain normal schedule invalidation.

## 9. Quick Playback Profile placement

The user specifically wants Auto Play next to the Hold control.

In the compact Playback Profile popover, make the first control row conceptually:

```
Base Hold     [1f]       Auto Play   [switch ON]
```

Exact spacing may adapt to the 260–320 px popover, but requirements are:

- Base Hold remains immediately editable;
- Auto Play is on the same logical row / visually adjacent;
- switch does not widen the popover beyond its accepted compact width;
- text remains readable at the minimum supported viewport;
- no horizontal overflow.

Use an accessible switch:

- native checkbox styled as a switch, or
- button with `role="switch"` and `aria-checked`.

Do not use only color to indicate ON/OFF.

Recommended visible label: `Auto Play`.

## 10. Full Settings placement

Mirror Auto Play in Settings > Playback.

It should be visible near Base Hold / playback defaults, not hidden in Advanced.

Suggested layout:

```
Base Hold       Auto Play
Tempo           FPS
```

or another compact arrangement that preserves the current Settings hierarchy.

The quick profile and Settings must edit the same authoritative persisted value.

## 11. Auto Play live behavior

Toggling Auto Play while a song is currently Playing/Paused should take effect for the **next natural terminal event**.

Do not freeze Auto Play into the current native session because it is a frontend queue behavior, not an authored timing value.

Examples:

- song playing, Auto Play ON -> user toggles OFF -> natural finish stops.
- song playing, Auto Play OFF -> user toggles ON -> natural finish advances if a valid next item exists.

The settings mutation itself must not interrupt the active session.

## 12. Player Bar interaction between Shuffle and Auto Play

Examples to cover:

### Shuffle OFF, Auto Play ON

```
A -> B -> C
```

### Shuffle ON, Auto Play ON

```
A -> shuffled item X -> shuffled item Y
```

with no repeats before context exhaustion.

### Shuffle ON, Auto Play OFF

A finishes -> idle on A.

Then explicit Next -> shuffled next item.

### Toggle Shuffle while playing

A remains current; only future Next/auto-next order changes.

### Toggle Auto Play while playing

No native playback state changes immediately.

## 13. UI details for Shuffle

Place Shuffle before Previous, Spotify-style:

```
Shuffle   Previous   Play/Pause   Next   Stop
```

Do not disturb current accepted transport baseline.

Acceptance:

- all visible secondary controls share the same center-Y;
- primary Play/Pause remains within 2 px of viewport center;
- Shuffle active state uses accent color;
- Shuffle remains borderless/transparent like other secondary controls;
- active state has `aria-pressed="true"`.

If adding Shuffle on the left would shift the primary control, refactor positioning so the core primary remains independently centered. Do not solve it with fragile dummy slots.

## 14. Required unit/store tests — Shuffle

At minimum:

1. Shuffle defaults Off.
2. Toggle sets `aria-pressed`/store mode.
3. Enabling Shuffle while current A plays does not change current song.
4. Shuffled Next never immediately repeats A when context size >1.
5. A shuffled traversal visits every context index exactly once.
6. No duplicate before exhaustion.
7. Previous walks the shuffled history/order.
8. >3 s Previous restarts the current shuffled item.
9. Toggling Shuffle Off returns future traversal to sequential from current index.
10. New explicit start while Shuffle is On initializes a fresh traversal.
11. Membership invalidation prevents stale shuffled Next.
12. Stale session events still cannot alter new traversal ownership.

Test multiple context sizes, including composite totals such as:

- 2;
- 3;
- 4;
- 5;
- 10;
- 12;

to prove any coprime/permutation algorithm works for non-prime totals.

## 15. Required tests — Auto Play

At minimum:

1. old/missing setting defaults to true.
2. patch persists false/true.
3. pure Auto Play patch does not invalidate prepared playback.
4. pure Auto Play patch does not change plan/settings fingerprint used for schedule identity.
5. Auto Play ON natural finish advances sequentially when Shuffle Off.
6. Auto Play OFF natural finish returns idle/replayable.
7. Auto Play ON + Shuffle On advances through shuffled traversal.
8. explicit Next works with Auto Play Off.
9. Stop never auto-advances.
10. failure never auto-advances.
11. dry-run natural finish never auto-advances.
12. toggle OFF while playing takes effect at finish.
13. toggle ON while playing takes effect at finish.

## 16. Component/E2E acceptance

### Player Bar

- Shuffle button appears before Previous.
- `aria-pressed` reflects active state.
- active accent is visible.
- no border/box background.
- baseline remains stable.
- Play remains centered.

### Quick profile

At 1200×760, 1280×720, 800×560:

- Base Hold + Auto Play fit without expanding accepted 260–320 px popover;
- no clipping;
- toggle is obvious;
- state persists after closing/reopening.

### Settings

- Auto Play mirrors the same value;
- toggling in Settings updates quick profile;
- toggling in quick profile updates Settings.

### E2E behavioral flows

Add:

```
Shuffle ON
Play A
Next several times
assert order is non-sequential and no repeats
```

Use deterministic mock seed/test seam so tests are stable.

Add:

```
Auto Play OFF
Play A to natural finish
assert A remains current, state idle, no B start

Auto Play ON
Replay A to natural finish
assert next traversal item starts
```

And the same with Shuffle ON.

## 17. Actual Tauri visual acceptance

Required on exact-head implementation.

Capture at minimum:

- Player Bar idle with Shuffle Off;
- Player Bar playing with Shuffle On;
- quick Playback Profile with Auto Play On;
- quick Playback Profile with Auto Play Off;
- 800×560 compact state.

Verify:

- Shuffle does not shift Play center;
- no control overlap;
- Auto Play switch fits beside Base Hold;
- profile popover remains within viewport.

## 18. Validation

Run:

```powershell
cargo xtask check all

cd desktop
bun run check
bun run build
bun run test:e2e
bun run test:e2e:bundle
```

Then exact-head GitHub Actions must be green.

## 19. Dependency on PR #235

This is a stacked PR.

Do not merge this PR before #235.

After #235 is accepted and merged:

1. rebase this branch onto current `main`;
2. retarget PR base to `main`;
3. resolve any PlayerTransport/store/settings conflicts;
4. rerun all validation and Tauri visual acceptance on the new exact head.

Do not carry the temporary handoff/deadlock documentation as a reason to skip rebase.

## Definition of Done

- [ ] Shuffle button is real, not decorative.
- [ ] Shuffle changes Next/Previous/natural traversal.
- [ ] No repeats before shuffled context exhaustion.
- [ ] Play/Pause remains centered.
- [ ] Auto Play defaults ON and is persisted.
- [ ] Auto Play appears beside Base Hold in the quick profile.
- [ ] Auto Play is mirrored in full Playback Settings.
- [ ] Auto Play OFF stops on natural finish.
- [ ] Auto Play ON advances after retirement.
- [ ] Dry-run test remains one-shot.
- [ ] Pure Auto Play toggle does not invalidate schedules.
- [ ] Existing PR #235 anti-stuck/lifecycle guarantees remain intact.
- [ ] Actual Tauri visual evidence passes.
- [ ] Exact-head CI is green.
