# 01 — Real-Time Invariants and Failure Policy

This document defines what the implementation must prove. Tests should be written against these invariants rather than against incidental function structure.

## 1. Time domains

The core uses three distinct concepts:

```text
TimelineTicks  = authored/logical playback position
QpcTicks       = absolute host monotonic physical time
DurationTicks  = non-negative interval in the same QPC frequency domain
```

Rules:

1. Authored schedule data is immutable after compile.
2. A physical target is created exactly once by mapping the selected logical deadline to the current playback epoch.
3. Once a physical plan is frozen, every waiter/admission/observer reference to its target uses the exact same `QpcTicks` value.
4. No conversion to integer microseconds is required to authorize a healthy physical send.
5. Microsecond conversion belongs to configuration boundaries, Python/report output, diagnostics, or observer work.

## 2. Authoritative timing boundaries

For one physical transaction:

```text
physical_target_qpc
        │
        ├── wait/timer/spin reaches target
        │
final_admission_qpc
        │
        ├── final lease classification / minimal transport call setup
        │
SendInput
        │
sendinput_completion_qpc
```

Required semantics:

- `physical_target_qpc`: exact intended sender-side dispatch boundary.
- `final_admission_qpc`: the one production QPC sample taken after final target/focus/control prechecks and used by final lease admission.
- `sendinput_completion_qpc`: QPC sample immediately after the `SendInput` return path.

`final_admission_qpc` is the primary start-error boundary:

```text
dispatch_start_error = final_admission_qpc - physical_target_qpc
```

It is **not** proof of kernel delivery, Raw Input receipt, game polling, render frame, network state, or audible onset.

## 3. Chord invariant

For every authored Down action with mask `D`:

```text
all keys in D are presented in one SendInput array
```

The implementation must never:

- split `D` into multiple physical sends,
- silently remove a blocked key and send the rest,
- stagger the chord in native runtime,
- retry a partially inserted Down prefix.

A partial Down-bearing `SendInput` result permanently loses chord integrity and is terminal.

## 4. Generation ownership invariant

For each physical key slot, at most one generation may own the key physically.

Runtime state must satisfy:

```text
active slot => exactly one active generation
pending release => same generation is still active
released slot => no active owner and no pending release
```

A new Down on a slot is legal only if either:

1. the slot is already physically Up, or
2. the same physical transaction contains the valid release of the old generation before the new Down.

There is no third state where two generations are accepted as simultaneously active on one keyboard key.

## 5. Per-key release invariant

For generation `G`:

```text
release_not_before(G) = down_sendinput_completion(G) + effective_min_hold
release_due(G) = max(authored_up(G), release_not_before(G))
```

No physical Up for `G` may start before `release_due(G)`.

A late release affects only that generation unless another authored Down explicitly depends on the same physical key.

## 6. No unrelated head-of-line blocking

For current authored frame at target `T`, let:

```text
U = owned Up mask
D = authored atomic Down chord mask
F = Up keys whose release_due > T
```

If:

```text
F & D == 0
```

then `D` MUST remain dispatchable at `T` even though members of `F` are deferred.

The current implementation violates this invariant by assigning the maximum Up release floor to the whole compiled packet. Phase P0 removes that behavior.

## 7. Same-key deadline feasibility

If:

```text
F & D != 0
```

then the current Down chord cannot be physically represented at target `T` without either violating minimum hold or moving/splitting the chord.

Required result:

```text
terminal fidelity fault -> cleanup -> no Down send
```

Suggested structured code/reason:

```text
physical_deadline_infeasible
```

Include at least:

- target timeline ticks,
- blocked key mask,
- generation ID(s) if available,
- release due ticks.

Do not make the error text part of the control logic.

## 8. Native admission feasibility invariant

Before the worker starts, the compiled authored schedule must be checked against `effective_min_hold`.

For every owned generation with authored Down `Td` and authored Up `Tu`:

```text
Tu >= Td
Tu - Td >= effective_min_hold
```

If not, reject session construction with a deterministic validation error.

This is a defense-in-depth native contract even if the Python scheduler normally prevents the same input.

The validator must also preserve existing overflow/range checks.

## 9. No catch-up burst invariant

A real-time player is not a batch processor.

If the worker wakes after a large stall and multiple independent physical boundaries are already overdue, it must not execute:

```text
send overdue note A
loop
send overdue note B
loop
send overdue note C
```

with near-zero spacing.

That output is neither the authored rhythm nor a valid recovery.

The exact backlog classifier is specified in `03_P0_OVERDUE_STALL_POLICY.md`.

## 10. Single-target invariant

A `PhysicalDispatchPlan` owns exactly one `physical_target_qpc`.

The following are forbidden after the plan is built:

- rebuilding target from `epoch + deadline`,
- converting target to µs and back,
- replacing an overdue target with `now`,
- subtracting a learned lead,
- changing target because wake error changed.

A plan invalidated by command/focus/target/lease state is discarded and replanned; it is not mutated into a new target.

## 11. Precision-region work invariant

After the deadline handoff, healthy production code may perform only bounded correctness work needed to authorize/perform the current send:

```text
reuse frozen plan
final command precheck
final target/focus proof if Down-bearing
one final-admission QPC sample
lease classification
SendInput
completion QPC sample
bounded touched-slot state transition
one nonblocking observation push
return to outer scheduler
```

Explicitly forbidden in this region:

- heap allocation,
- mutex/RwLock acquisition,
- string formatting,
- logging,
- Python/PyO3 calls,
- packet building,
- schedule traversal unrelated to touched intents,
- full 15-slot invariant scan,
- health-window calculation,
- telemetry serialization,
- queue draining,
- `ArrayQueue::len()` diagnostic sampling,
- repeated tick↔microsecond conversions.

## 12. SendInput result invariant

For an authored physical send:

### Full insertion

Commit exactly the state represented by the frozen commit token.

### Zero insertion

Do not retry in production authored playback. Terminal + cleanup.

### Partial insertion

Physical state is uncertain for the affected packet. Do not infer a safe prefix for Down-bearing traffic. Terminal + cleanup.

### QPC failure before send

No physical commit. Terminal + cleanup as required by current session policy.

### QPC failure after send

Treat affected physical state as uncertain. Terminal + full cleanup.

Cleanup/release FSM retries remain a separate recovery mechanism and must not be reused as authored playback retry logic.

## 13. Observer invariant

Observer data can describe what happened; it cannot decide what happens next on the healthy path.

The producer may drop a new observation when the fixed queue is full. Dropping telemetry is preferable to delaying a note.

Observer failure handling must not cause the producer to block waiting for observer progress.

## 14. Focus invariant

When focus protection is disabled, no foreground-window query belongs on the send path.

When focus protection is enabled:

- cached Python focus is only a hint,
- final Down-bearing authorization uses a fresh native target/foreground proof,
- the foreground query occurs before `final_admission_qpc`, so it does not contaminate the admission-to-completion metric.

Up-only release traffic does not acquire a Down focus dependency.

## 15. Command/lease invariant

A command or supervisor lease can invalidate a frozen plan up to the final admission point.

The final control gate must remain bounded and nonblocking. Do not replace the current atomics with a lock.

A command racing exactly with the physical boundary is resolved by the documented final gate ordering; there is no attempt to “undo” a fully successful `SendInput` transaction after the fact.

## 16. Wait invariant

Production requires:

```text
QPC clock
+ high-resolution waitable timer
+ interrupt event
+ bounded final QPC spin
```

Timer guard/spin timing changes wake behavior only. They never change the physical target.

Any runtime wait backend failure is terminal rather than falling back to a lower-fidelity sleep path.

## 17. Scheduling-priority invariant

Priority is an aid to get CPU time, not permission to starve Windows.

The dispatch thread should be runnable briefly around deadlines and blocked otherwise. It must not become an unbounded busy loop or a realtime-class process.

## 18. Failure classes

### Class A — structural fidelity fault (always fatal)

Examples:

- partial Down/Mixed insertion,
- same-key deadline infeasible,
- overdue backlog/catch-up condition,
- coordinator ownership disagreement,
- prepared packet mismatch,
- QPC ordering/failure,
- required wait backend failure.

Action: stop authored playback, perform the appropriate full cleanup, preserve primary error.

### Class B — operator/control transition

Examples:

- pause,
- quit,
- skip,
- panic release,
- focus loss when focus gating is enabled.

Action: follow the existing explicit state machine; cleanup/suspend where required.

### Class C — diagnostic degradation

Examples:

- observer sample dropped,
- SendInput duration warning,
- wait overshoot warning,
- approximate queue high-watermark unavailable.

Action: record/degrade telemetry only. Do not move a deadline.

### Class D — ordinary bounded lateness

One physical boundary may be slightly late due to Windows scheduling/SendInput cost while no second physical boundary has also become overdue.

Action: execute the single still-coherent operation and record signed lateness. Do not compensate future timestamps.

## 19. Cleanup invariant

Every Class A terminal fault that might leave physical state uncertain must reach one bounded cleanup path.

Do not chain multiple independent cleanup FSMs for the same fault. The existing “choose cleanup scope once” design should remain.

After terminal cleanup:

```text
backend.active_mask == 0
backend.possibly_active_mask == 0
backend.failed_release_mask == 0
no coordinator generation remains physically owned
```

If physical verification is inconclusive, report failure; do not label the session clean.
