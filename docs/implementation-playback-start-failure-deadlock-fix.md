# Blocking fix: physical startup failure deadlock in PR #235

Status: **P0 REQUEST CHANGES**.

Reviewed implementation head: `bb2558ff822e983c58d37f5986e2db76d8b9fecb`.

The actual Tauri run without a visible Sky process exposed a real native deadlock. This is not merely missing physical-input qualification and must be fixed before PR #235 can leave Draft.

## Confirmed root cause

`NativePlaybackService::start()` acquires and retains:

```rust
let mut active_slot = self.active.lock()?;
```

near the beginning of the method.

On the physical startup failure path, `create_native_player(...)` returns an error such as:

```
no admissible visible Sky window was found
```

The error branch creates `failed_active`, publishes `starting`, publishes the typed failed state, and then calls:

```rust
publish_retirement_barrier(
    &events,
    &self.active,
    &self.last_terminal,
    &failed_active,
    &publication,
)
```

`publish_retirement_barrier()` calls `release_terminal_ownership()`, which attempts to lock `self.active` again.

The original `active_slot` guard is still live.

The mutex is not re-entrant, so the start command deadlocks after the failure-state publication.

This exactly matches the actual Tauri observation:

1. UI briefly receives `target_not_found`.
2. `start_playback` never returns.
3. the start thread keeps `self.active` locked.
4. `get_playback_status` blocks waiting for the same mutex.
5. frontend status watchdog times out.
6. `playback.stop` also blocks on the same mutex.
7. Retry Status and Stop cannot recover the UI.

## Required fix

Do not fix this with longer frontend timeouts, retries, sleeps, or by hiding the recovery banner.

The native startup-failure path must never attempt to reacquire a mutex already held by `start()`.

Refactor lock ownership so that the failure path has a clear pre-activation terminal contract.

Acceptable designs include either:

### Option A — explicit pre-activation failure retirement

Create a narrowly scoped helper for a session that failed **before it was installed into `self.active`**.

It must:

- release the reserved playback activity lease;
- record `last_terminal` with:
  - session id;
  - prepared id;
  - song id;
  - failed state;
  - classified failure code/message;
- mark the synthetic failed session done;
- publish the final `playback.failed` event only after the retirement state is authoritative;
- never lock `self.active`, because no active-slot ownership was published.

This is the preferred semantic model if the failed session was never observable through `playback.get_status().active`.

### Option B — install then retire

Install the synthetic failed session into `self.active` while the guard is held, then release the guard before calling the ordinary retirement barrier.

If this model is chosen:

- `done` must not claim completion before the barrier;
- status/Stop behavior during the brief failed ownership window must be coherent;
- no second playback may steal ownership;
- terminal event remains a true retirement barrier.

Whichever design is chosen, explain why it preserves single-start linearization.

## Lock-scope requirements

Audit the entire `NativePlaybackService::start()` method for long-held mutex guards.

At minimum verify:

- `self.active` is never recursively acquired;
- failure publication cannot occur while holding a guard that publication/retirement helpers also acquire;
- `self.prepared` is not unnecessarily held across event publication or blocking worker cleanup;
- lock ordering between `active`, `last_terminal`, per-session state, and activity lease is consistent.

Do not weaken the current one-active-session invariant.

## Required native regression test

Add a test that exercises the actual start-failure orchestration, not only helper functions such as `physical_startup_failure()`.

Use an existing test seam or introduce a narrow test-only startup-failure seam.

The test must prove:

1. physical `playback.start` enters a deterministic target-not-found/startup-failure path;
2. the start call returns an error within a bounded deadline rather than hanging;
3. immediately afterward `service.status()` returns within a bounded deadline;
4. `status.active == None`;
5. `status.last_terminal` contains the failed session;
6. terminal `prepared_id` matches the failed start request;
7. failure code/message remain `target_not_found` and actionable;
8. the playback activity reservation has been released;
9. a new reservation/start can be admitted immediately;
10. an idempotent Stop for the retired session cannot hang.

Use `recv_timeout` / bounded join or another deterministic timeout so a regression fails the test instead of hanging CI forever.

## Required command-level/Tauri test

Where practical, add a command-layer regression around:

```
start_playback -> target failure
get_playback_status
stop_playback (if terminal-idempotent path is exercised)
```

The important assertion is that the command lane remains responsive after the failed start.

## Required actual-Tauri acceptance without Sky

This test does **not** need SendInput and is mandatory because the bug reproduces specifically when Sky is absent.

On the exact implementation head:

1. ensure no admissible Sky window is running;
2. choose a valid song;
3. invoke physical Play;
4. verify an actionable target-not-found message appears;
5. verify the start command settles rather than hanging;
6. verify the UI reaches a clean no-active-session failure state;
7. verify **Try again / Play** is available;
8. verify Retry Status is not required just to escape the failed start;
9. invoke authoritative status and verify it returns promptly;
10. retry the start and verify the same failure is repeatable without restarting the app.

Expected stable UI after target-not-found:

```
Sky window was not found
Open Sky and make sure its window is visible, then try again.

[ Try again ]
```

There should be no owned session requiring Stop after the pre-activation target failure.

## Preserve the accepted recovery UI

The error-recovery UI added at `bb2558ff...` is directionally accepted:

- explicit Retry Status;
- explicit Stop playback when a known session exists;
- Try again when ownership is clean;
- full-width in-bounds recovery panel;
- typed target messages.

Do not remove these controls to mask the native bug.

## Validation

Run at minimum:

```powershell
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo xtask check rust
cargo xtask check desktop

cd desktop
bun run check
bun run test:e2e
bun run build
bun run test:e2e:bundle
```

Then run exact-head GitHub Actions.

## Definition of Done

- [ ] No re-entrant `self.active` lock on physical startup failure.
- [ ] Target-not-found start returns within a bounded time.
- [ ] `playback.get_status` remains responsive immediately afterward.
- [ ] Failed pre-activation session is fully retired.
- [ ] Playback activity lease is released.
- [ ] Next/retry start can be admitted.
- [ ] Typed target failure survives.
- [ ] Actual Tauri without Sky returns to a retryable UI without app restart.
- [ ] No frontend timeout increase is used as the fix.
- [ ] Exact-head CI is green.
- [ ] PR #235 remains Draft until independent acceptance.
