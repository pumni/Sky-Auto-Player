# Real-Time Dispatch Architecture

This is the normative implementation contract for the Rust playback worker.
The scheduler remains pure; the platform boundary is the only place that
touches Windows. All physical input uses `SendInput`.

## 1. Ownership and thread split

The worker dispatch thread owns the real-time sequence and the coordinator
owns schedule/generation state. The dispatch thread may read or commit
coordinator state only at the defined planning and ownership boundaries. It
must not perform telemetry formatting, unbounded allocation, observer health
updates, or learned timing estimation on the physical path.

The observer is a separate consumer thread. A bounded
`crossbeam_queue::ArrayQueue<DispatchObservation>` (production capacity 64) is
shared between them:

```text
dispatch thread --nonblocking push--> ArrayQueue --single consumer--> observer thread
                                      full: drop new observation
```

The producer records a drop and continues. It never waits for slack and never
evicts an older observation. The consumer owns health windows, telemetry
materialization, shared observer metrics, and its own local timing state. It
cannot authorize, reorder, retry, or commit physical input. Shutdown signals
the consumer, joins it, merges its local metrics, and then publishes the final
report.

## 2. Immutable plan

One outer worker epoch builds one typed plan from the coordinator. The plan
contains the next authored or pending deadline, physical path, polyphony,
health budget, and absolute QPC target. The plan is reused by waiting, due
selection, and physical dispatch. Commands, focus/pause transitions, target
changes, lease-only wakes, recovery changes, and interrupts invalidate it and
cause a replan.

Production planning has no adaptive dispatch-cost estimator and no lead
subtraction. Authored/effective timestamps are used as authored. Pending
release deadlines include only the completion-anchored hold floor and retry or
recovery floors. Compatibility lead arguments may remain at APIs used by old
tests/callers, but production ignores them and reports zero applied lead.

The physical target is derived once from the playback epoch:

```text
physical_target_qpc = playback_epoch_qpc + effective_deadline_ticks
```

Waitable-timer guard and bounded spin are wake mechanics only. Neither is an
applied dispatch lead and neither changes the coordinator deadline. The target
is carried through the final gate and sender; it is not replaced by the wake
sample or reconstructed after `SendInput`.

Stale-Up compiler metadata with an empty packet is handled as coordinator
metadata. It has no physical path, no sender boundary, and does not consume a
startup or normal physical target. Physical packets are packetized from the
canonical masks.

## 3. Final physical sequence

The Down/Mixed and Up-only paths share the same authoritative transport order.
Down adds the target/focus checks; Up-only never uses the focus gate.

```text
frozen plan
  -> fresh command/target/lease checks
  -> Down target + foreground validation when required
  -> one QPC started sample
  -> lease admission using that exact started sample
  -> one packetized SendInput call
  -> completion QPC and transport-mask validation
  -> coordinator ownership commit/recovery
  -> bounded observation enqueue
```

The supplied `started` sample is the physical start/admission boundary. The
sender reports `completed`; production does not subtract a learned send cost
from the target. Completion is used for diagnostics and release ownership.

Packet construction validates scan-code masks and sends Up entries before Down
entries in one call. A zero, partial, skipped, mixed, or otherwise inconsistent
transaction is handled fail-closed. A partial Down/Mixed result is not blindly
retried. A pending release may requeue only unconfirmed ownership after a
validated transport result; coordinator disagreement forces full-instrument
cleanup and termination.

## 4. Completion-anchored hold model

Python materializes the fixed floor before native startup:

```text
frame_us = ceil(1_000_000 / game_fps)
effective_min_hold_us = max(requested_min_hold_us, frame_us + 500)
```

Native checked tick arithmetic enforces:

```text
release_floor = down_completed + effective_min_hold
effective_release = max(authored_release, release_floor, retry_not_before)
```

This is a sender-side visibility floor, not game-observed timing. A slow Down
can defer its own Up; it does not move an unrelated future authored action.
Recovery and retry state remain coordinator correctness state, not observer
statistics.

## 5. Wait and interrupt ordering

The worker uses a high-resolution waitable timer, event interruption, and a
bounded QPC spin. A timer guard may wake early; the final QPC deadline gate
decides whether to wait again or enter the physical path. A wake that is only
for lease, command, focus, pause, or interrupt replans and cannot dispatch the
old plan.

The spin path may read an interrupt generation hint with `Relaxed` ordering
every bounded group (currently 32 iterations). The final generation/deadline
decision uses the authoritative `Acquire` path. This optimization is only a
wake hint; it cannot bypass the final gate.

MMCSS Games/High and process power-throttling opt-out are scoped to the worker.
TimeCritical is not the default and priority setup failure is reported rather
than silently changing the timing contract.

## 6. Startup and stale work

Startup establishes the playback epoch and waits for the first physical target
using the same target formula as steady state. There is no adaptive startup
lead and no startup estimator sample. The first physical call still performs
the complete final control/lease/target gate. Stale metadata may be committed
before physical work, but it cannot run after the final precision handoff on
the physical call stack.

The worker may project a pre-epoch deadline for control-loop wake/replan
bookkeeping, but it never calls `SendInput` before the authoritative physical
QPC target/epoch gate. This projection distinction must not be used to create
an early physical send.

## 7. Failure and publication boundaries

Every QPC query used for a correctness decision is terminal on failure.
Coordinator commit follows confirmed transport evidence. Cleanup releases
active/possibly-active keys and verifies the resulting state before successful
completion. The ready boundary is published only after startup gates and the
required physical/recovery state are complete.

Observer failure, telemetry overflow, or metric conversion failure cannot
rewrite physical ownership. The worker terminates through the normal cleanup
path and preserves the primary and secondary errors.

The live snapshot is a small projection of authoritative playback state. The
final report is materialized after worker and observer shutdown. The deprecated
`estimator_state_json` field and historical lead fields exist only for Python
compatibility; they are ignored by planning and contain no learned state.

## 8. Verification matrix

- `sky_dispatch_core`: controlled-clock deadline, hold-floor, ownership,
  recovery, and property tests.
- `sky_dispatch_win32`: packet ordering, strict mask validation, QPC/wait,
  focus, and `SendInput` seam tests.
- `sky_player_rs`: final-gate ordering, completion evidence, observer queue
  overflow, startup/stale handling, cleanup, and no-allocation dispatch tests.
- Security audit: only the platform crate contains Win32 bindings and
  `SendInput`; forbidden hook/injection/process-tampering mechanisms remain
  absent.
