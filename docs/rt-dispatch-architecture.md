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

Strict-timing diagnostics use a separate consumer thread. A bounded
`crossbeam_queue::ArrayQueue<DispatchObservation>` (capacity 64) is shared
between the diagnostic dispatch path and that consumer:

```text
dispatch thread --nonblocking push--> ArrayQueue --single consumer--> observer thread
                                      full: drop new observation
```

The diagnostic producer records a drop and continues. It never waits for slack
and never evicts an older observation. Production allocates neither this queue
nor an observer thread; it retains only bounded scalar worker counters. The
diagnostic consumer owns health windows, telemetry materialization, shared
observer metrics, and its own local timing state. It cannot authorize,
reorder, retry, or commit physical input. Diagnostic shutdown signals the
consumer, joins it, merges its local metrics, and then publishes the final
report.

## 2. Immutable plan

One outer worker epoch builds one typed plan from the coordinator. The plan is
`NoWork`, a metadata boundary, or a physical boundary. A physical plan carries
one prepared authored/pending view, its commit proof, and one absolute QPC
target. Observer health budgets are not physical-plan inputs. The plan is
reused by waiting, due selection, and physical dispatch. Commands,
focus/pause transitions, target changes, lease-only wakes, and interrupts
invalidate it and cause a replan.

Production planning has no adaptive dispatch-cost estimator and no lead
subtraction. Authored timestamps are used as authored. Release deadlines
include only the authored minimum-hold policy. Any remaining lead-shaped
arguments are test-only
compatibility seams; production coordinator APIs do not accept a dispatch lead
and the publication adapter reports the historical applied-lead field as zero.

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
  -> final_proof_qpc target/focus/control proof
  -> lease admission using that proof sample
  -> QPC spin to the authored target and one pre_call_qpc sample
  -> one packetized SendInput call
  -> completion QPC and transport-mask validation
  -> coordinator ownership commit on clean success
  -> terminal fail-closed cleanup on transport anomaly
  -> essential scalar state commit
  -> diagnostic-only bounded observation enqueue
```

The supplied `pre_call_qpc` sample is the physical pre-call boundary, not a
Windows syscall-entry or game-receipt timestamp.
The transport reports `sendinput_completion_qpc`; production does not subtract
a learned send cost from the target. Completion is used for diagnostics and
ownership evidence only; it does not create a completion-relative hold floor.

The primary sender-side timing evidence is the signed start residual:

```text
dispatch_start_error_ticks = pre_call_qpc - physical_target_qpc
pre_call_to_completion = sendinput_completion_qpc - pre_call_qpc
completion_error = sendinput_completion_qpc - physical_target_qpc  # diagnostic only
```

The start residual is benchmark/observability output only. It is never fed
back into scheduling or used as adaptive compensation.

Packet construction validates scan-code masks and sends Up entries before Down
entries in one call. A zero, partial, skipped, mixed, or otherwise inconsistent
transaction is handled fail-closed. Production never retries `SendInput` and
never queues a release for a later transport attempt. Any transport anomaly
forces full-instrument cleanup and termination.

Cleanup verification keeps transport and physical evidence separate. A
`VerifiedAllUp` result is possible only when the target HWND is still the
foreground window, the physical probe observes no held instrument key, and the
entire requested Up mask is confirmed by the transport. A zero- or partial-
progress transport result combined with physical all-up is therefore
inconclusive, never cleanup success. Focus loss also makes the physical probe
inconclusive; it must not be interpreted as all keys being up.

## Focus pause and recovery

The supervisor's focus hint is a coarse wake/gate signal. The worker's outer
focus-loss transition is a pause edge, not a cleanup edge:

```text
focus invalid
  -> clear verified target and restore marker
  -> increment focus_lost once on entry
  -> enter focus pause and publish progress
  -> perform no physical release or preflight while unfocused
```

This keeps an unfocused physical probe `Inconclusive` rather than treating it
as evidence that all instrument keys are up. No Down can be admitted while the
focus pause is active, and the fresh foreground HWND check remains the final
physical Down authority.

After focus is observed again, the worker waits for the configured restore
grace and validates the current foreground/target identity. If a manual pause
is already active, it clears only the focus pause; it does not release or
preflight physical input a second time. The manual-resume path owns the one
preflight required before playback continues. Without an active manual pause,
the worker performs the safety sequence below:

```text
load current target stamp
  -> clear verified target
  -> suspend_live_input
  -> physical preflight
  -> fresh foreground HWND and target-stamp checks
  -> exit focus pause
```

Cleanup or preflight failure is terminal and fail-closed. If focus or target
identity changes after cleanup, the worker clears the restore marker and stays
paused for another attempt. A manual pause remains independent; restoring
focus never clears it.

When focus is required, the Python supervisor makes one minimal
`ShowWindow(SW_RESTORE)` plus `SetForegroundWindow` attempt at playback
startup, before the native worker starts. It refreshes the cached target and
actual foreground state afterward and treats an OS refusal as an ordinary
non-terminal outcome. After startup, focus polling only publishes observed
state; it never reclaims foreground focus automatically. Explicit manual
refocus remains the user-controlled retry path.

## 4. Authored timestamp and minimum-hold model

Python materializes the fixed floor before native startup:

```text
frame_us = ceil(1_000_000 / game_fps)
effective_min_hold_us = materialize_hold(selected_hold_frames, frame_us, calibrated_margin_us)
```

Native admission checked tick arithmetic enforces before worker start:

```text
authored_up >= authored_down + effective_min_hold
```

The static margin is materialized once into the authored schedule. An invalid
interval fails native admission before any musical SendInput. The worker never
combines Down completion with the authored hold to create a second floor, never
creates a completion-derived minimum-hold terminal state, and never rewrites an
authored Up target. Runtime
deadline/overdue policy handles a late boundary; recovery-only pending
releases are stored in a fixed `[Option; 15]` per-key table with mask and
generation ownership. There is no transport retry state.

## 5. Wait and interrupt ordering

The worker uses a high-resolution waitable timer and event interruption to the
`T - 2,000 µs` admission boundary with zero waiter spin. The final authored
precision stage has a bounded QPC spin fixed at `700 µs` in production. A timer guard may wake early;
the final QPC deadline gate
decides whether to wait again or enter the physical path. A wake that is only
for lease, command, focus, pause, or interrupt replans and cannot dispatch the
old plan. The timer is first in the Windows multi-wait handle array so a
simultaneous timer/event wake enters the QPC classification path. Before the
physical target, an interrupt returns `Interrupted`; once QPC reaches the
target, the waiter returns `Deadline`. Final command, target, focus, and lease
admission remains authoritative after that result and may still reject
`SendInput`.

The spin path may read an interrupt generation hint with `Relaxed` ordering
every bounded group (currently 32 iterations). The final generation/deadline
decision uses the authoritative `Acquire` path. This optimization is only a
wake hint; the QPC deadline check runs first and cannot be bypassed by an event.
Production admission requires the high-resolution waitable timer and event wait
and terminates on startup or runtime wait failure; it does not degrade to sleep
timing. `WaitBoundary::Due` carries the authoritative wake QPC into dispatch;
the dispatch path does not take a redundant QPC sample or reconstruct the
physical target from wake time.

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

After a successful physical boundary, an overdue replan without a deadline-wait
handoff is a missed schedule. The worker terminates rather than emitting an
overdue catch-up burst.

## 7. Failure and publication boundaries

Every QPC query used for a correctness decision is terminal on failure.
Coordinator commit follows confirmed transport evidence. Cleanup releases
active/possibly-active keys and verifies the resulting state before successful
completion. The ready boundary is published only after startup gates and the
required physical ownership and cleanup state are complete.

Production has no observer failure or queue-overflow path. Diagnostic observer
failure, telemetry overflow, or metric conversion failure cannot
rewrite physical ownership. The worker terminates through the normal cleanup
path and preserves the primary and secondary errors.

The live snapshot is a small projection of authoritative playback state. The
The diagnostic final report is materialized after worker and observer shutdown;
the production report uses worker scalar state. Historical estimator plumbing
is not part of the active production session contract and cannot affect timing.

## 8. Verification matrix

- `sky_dispatch_core`: controlled-clock deadline, hold-floor, authored
  ownership, stale/retrigger/mixed-packet, and soak tests.
- `sky_dispatch_win32`: packet ordering, strict mask validation, QPC/wait,
  focus, and `SendInput` seam tests.
- `sky_player_rs`: final-gate ordering, completion evidence, diagnostic observer queue
  overflow, startup/stale handling, cleanup, and no-allocation dispatch tests.
- Security audit: only the platform crate contains Win32 bindings and
  `SendInput`; forbidden hook/injection/process-tampering mechanisms remain
  absent.
