# 03 — P0: Overdue / Stall Fidelity Policy

## 1. Problem

The current production loop can dispatch work that is already due, commit it, loop, discover the next authored packet is also already due, and immediately dispatch that one too.

After a scheduler stall, suspend/resume event, long system interruption, or unexpectedly expensive physical operation, this can turn authored spacing into a near-zero-spacing burst:

```text
authored:  A ---- 8 ms ---- B ---- 8 ms ---- C
stall:                          40 ms
current:   send A, send B, send C as fast as loop/SendInput allows
```

That is not recovery. It is a different rhythm.

Production `strict_timing=false` currently means the 20 ms hard-late diagnostic guard does not prevent this.

## 2. Decision

Do **not** add an adaptive rebase or catch-up controller.

Instead enforce one simple production invariant:

> After a physical send completes, the worker may not perform a second distinct physical send until it has observed a future physical target and crossed that target through the wait/deadline path.

This guarantees the engine never drains an overdue physical backlog as a burst.

## 3. Runtime state

Add one explicit worker-local state flag with a semantic name, for example:

```rust
awaiting_future_physical_boundary: bool
```

Initial value:

```text
false
```

After every successful authored or pending-release physical `SendInput`:

```text
true
```

After the worker selects a physical target that is **strictly in the future**, enters the wait path for that target, and receives `WaitBoundary::Due` for the same frozen target:

```text
false
```

A metadata-only authored boundary does not count as a physical send and does not by itself clear the flag.

A command/focus/lease interrupt invalidates the plan and does not clear the flag unless a later future physical wait is actually completed.

## 4. Backlog fault

When a new physical plan is selected and:

```text
awaiting_future_physical_boundary == true
AND
physical_target_qpc <= current_qpc
```

then the worker must **not** call `SendInput`.

Terminate with a structural fidelity error and cleanup.

Suggested reason:

```text
overdue_physical_backlog
```

Record at least:

- current target QPC;
- current QPC sample used for classification;
- signed/unsigned lateness ticks;
- path/masks if already frozen;
- previous successful physical completion QPC when available.

The error is fatal independent of `strict_timing`.

## 5. Why allow the first overdue physical operation?

Windows is not a hard real-time OS. A single operation can wake late because of ordinary scheduling jitter. Treating every positive residual as fatal would make normal playback unusable and would turn the project into a threshold-tuning exercise.

The structural error is not “one note was late”. It is:

> the worker is now in a state where it would need to issue another distinct physical boundary without any future timing boundary separating it from the previous send.

Allowing at most one overdue operation preserves existing best-effort behavior for isolated lateness while eliminating burst catch-up.

Very long stalls are additionally bounded by the existing supervisor lease timeout; QPC includes sleep/standby time, so a sufficiently old heartbeat remains a separate fail-closed mechanism.

## 6. Why not use a fixed 20 ms production abort threshold?

Do not promote the current strict diagnostic `HARD_LATE_ABORT_THRESHOLD_US = 20_000` into the core production recovery rule in this refactor.

Reasons:

- 20 ms is a policy number, not a structural boundary;
- acceptable isolated lateness is machine/workload dependent;
- the requested fix is preventing wrong catch-up timing, which can be stated without a new arbitrary threshold;
- real-host benchmark data can later justify an optional explicit late-abort product policy.

Keep strict diagnostic lateness thresholds separate from the structural backlog invariant.

## 7. Why not rebase the playback epoch?

`PlaybackClockState` contains `rebase_epoch()`, but active runtime state also contains timing values derived from the old timeline, especially:

```text
active.release_not_before_ticks
pending release due ticks
scheduled generation metadata
```

A correct rebase would need to transform all outstanding temporal state atomically and decide which already-authored events are dropped versus shifted. That is a new playback semantics design, not a local latency fix.

This overhaul therefore does not use active-playback rebasing for stalls.

## 8. Interaction with the new per-key release scheduler

The rule applies to all **physical** transactions:

- DownOnly;
- Mixed;
- UpOnly pending release;
- coalesced pending releases + authored frame.

It does not apply to:

- stale metadata commit;
- authored frame that only registers deferred releases and makes no physical call;
- observer processing.

Same-QPC-target work must be coalesced by the planner when semantics allow. The worker must not intentionally create two physical sends at the same authored target and then claim the second is a backlog fault.

## 9. Interaction with close legitimate deadlines

Example:

```text
A target = T
B target = T + 100 µs
```

If A's physical send/commit takes longer than 100 µs and B is already due when the loop returns, the correct fidelity-first behavior is to abort before B rather than send B at an invented spacing.

This is intentional.

The remedy for this scenario is to reduce dispatch occupancy or author feasible spacing, not to disable the backlog guard.

## 10. Where to classify

Classification must occur after a new immutable physical plan exists but before final physical admission and before `SendInput`.

Do not put it in observer/telemetry code.

Use one QPC sample already needed by the scheduler/plan-due classification where possible. Do not add repeated QPC reads solely for diagnostics.

The plan target itself remains unchanged; the classifier decides send vs terminal.

## 11. State transitions

Conceptual state machine:

```text
START
  awaiting_future = false

future plan selected
  └─ wait reaches exact plan target
       awaiting_future = false
       send
       awaiting_future = true

next plan already due
  └─ awaiting_future == true
       FATAL overdue_physical_backlog

next plan future
  └─ wait reaches target
       awaiting_future = false
       send
       awaiting_future = true
```

Metadata-only work may occur between these states but cannot authorize a second overdue physical send.

## 12. Required tests

### Test A — two overdue authored Down chords

After first physical send, make the next distinct target already past.

Expected:

- first send may succeed;
- second `SendInput` is never called;
- terminal reason is backlog fidelity fault;
- cleanup runs.

### Test B — three overdue chords

Expected exactly the same: never drain B/C.

### Test C — next target is 1 tick in the future

Expected:

- worker enters wait/spin;
- deadline result clears the guard;
- second send is allowed.

### Test D — next target becomes overdue because first SendInput was slow

Expected abort before second send.

### Test E — metadata-only boundary between sends

```text
physical A success
stale/deferred metadata boundary
physical B already overdue
```

Metadata does not clear guard; B is rejected.

### Test F — pending Up release is next and already overdue after previous physical send

Expected no immediate Up catch-up send; terminal cleanup handles release state safely.

### Test G — interrupt/replan

A command/focus/lease wake before a future target does not falsely clear the backlog guard. Only reaching a future physical deadline through the waiter clears it.

### Test H — ordinary isolated late first dispatch

With `awaiting_future=false`, one already-due operation remains allowed and records signed lateness.

## 13. Telemetry

Add structural counters only if they are useful and cheap outside the precision region, for example:

```text
overdue_backlog_abort_count
```

Do not add a controller based on this counter.

The final error/report should make a backlog abort distinguishable from:

- partial SendInput;
- lease expiry;
- focus loss;
- QPC failure.

## 14. Acceptance

This phase is accepted only when a deterministic test can inject an arbitrary stall and prove:

```text
number of physical SendInput calls after stall before a new future deadline wait <= 1
```

for DownOnly, UpOnly pending release, and Mixed/coalesced paths.
