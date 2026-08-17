# Timing Principles

This document is normative for the production dispatch loop. It describes
sender-side timing only; it makes no claim about game polling, rendering, or
audio onset.

## 1. Domains and vocabulary

Authored schedule times are immutable `TimelineTicks`. Runtime QPC values are
used for waiting and physical boundaries. Conversion between the domains is
performed at the configuration/epoch boundary and with checked typed
arithmetic; the worker does not route control decisions through repeated
microsecond conversions.

| Term | Meaning |
| --- | --- |
| `scheduled` | Immutable authored playback timestamp. |
| `physical_target` | Absolute QPC target derived from the playback epoch and `scheduled`. |
| `final_admission_qpc` | One authoritative QPC sample taken immediately before final admission and supplied to the packetized SendInput call. |
| `sendinput_completion_qpc` | QPC sample returned after the packetized SendInput call. |
| `admission_to_completion` | The interval from `final_admission_qpc` to `sendinput_completion_qpc`; compatibility field `send_duration_us` retains this value. |
| `effective_min_hold` | Fixed materialized hold floor passed into the native worker. |
| `authored_hold_valid` | Pre-start proof that authored Down→Up spacing meets the materialized hold. |

The worker never applies a learned dispatch-cost lead to `scheduled` or
`physical_target`. Historical `dispatch_lead_us`, estimator state, and lead
saturation fields are accepted only for compatibility and are non-operative.

## 2. Hold and release contract

Python validates `hold_frames` as one of `1.0`, `1.25`, or `1.5`, computes the
requested hold, and passes:

```text
frame_us = ceil(1_000_000 / game_fps)
effective_min_hold = max(requested_min_hold_us, frame_us + 500)
```

For every authored same-key Down→Up pair:

```text
authored_up >= authored_down + effective_min_hold
```

The static margin is applied once while materializing the authored schedule.
The native admission validator checks this relationship before worker start in
the same QPC tick domain used by dispatch. If the interval is invalid, native
admission fails before any musical packet can be sent; the worker never
reschedules the Up target.

Before a native session starts, the boundary validator rejects every authored
same-key Down→Up interval below `effective_min_hold_us`, including intervals
that share one authored timestamp. Runtime completion lateness is evidence for
sender-side telemetry and ownership accounting only. Runtime deadline/overdue
policy may terminate a missed boundary, but it never invents a
completion-relative hold deadline, moves an authored timestamp, or emits a
catch-up burst.

## 2.1 Host delivery calibration

Calibration is a separate native, app-owned Raw Input proxy. It measures
`SendInput` completion to `WM_INPUT` receipt, not game polling, rendering,
audio, or network latency. The sender boundary is measured as:

```text
validate/build/tag packet -> arm sequence -> SetLastError(0)
-> start_qpc -> SendInput -> completion_qpc -> validate receipt
```

The Raw Input timestamp is taken at entry to the `WM_INPUT` handler. Down and
Up packets must both match their direction, sequence, and scan code. A clean
pair computes signed per-key values:

```text
D = down_receipt_qpc - down_completion_qpc
U = up_receipt_qpc - up_completion_qpc
shrink = D - U
```

The publishable matrix has six buckets (`1/5/15 × hot/cold`) with at least
100 clean pairs in each. The selected static margin is the positive global
bucket p99 plus a 100 µs guard, clamped to 300–2,000 µs. This margin is
applied only to the hold floor; it never leads the playback target or claims
game-observed timing. Full-calibration checkpoints use plain-text SHA256
sidecars and a stable common provenance manifest. Resume and finalization
reject any bucket whose source/build identity, native source fingerprint,
Rust version, QPC frequency, or Windows build differs from the other five;
only the observation timestamp may differ. The final artifact and cache are
published only after the complete matrix and cache have both passed validation.

## 3. Planning and physical target

Each worker epoch freezes one typed plan: `NoWork`, a metadata boundary, or a
physical boundary. A physical plan contains one prepared packet view (authored
Up/Down or pending Up), its commit proof, and one absolute QPC target. Health
budgets are observer diagnostics and are not part of the physical admission
plan. The same frozen target is used for waiting, due selection, final target
validation, and observation. The coordinator remains the sole owner of
schedule and key-generation state.

The physical target is normally:

```text
epoch_qpc + effective_scheduled_ticks
```

It is never reconstructed from the wake timestamp and never advanced by a
sender-duration estimate. Wait guard time is a wake mechanism only; it is not a
dispatch target offset and is not reported as applied lead. The startup path
uses the same epoch/target rule and reserves no special adaptive startup lead.
Stale-Up metadata with an empty physical packet is committed as metadata and
does not consume a physical target.

After a successful physical dispatch, a later physical target that is already
overdue without an authoritative deadline-wait handoff is a missed schedule,
not permission to emit a catch-up burst. The worker terminates fail-closed and
does not replay overdue boundaries.

## 4. Authoritative send ordering

The final physical path is ordered and fail-closed:

1. Recheck command, target generation, focus (Down only), and the prepared
   packet against the current coordinator state.
2. Take one authoritative QPC `final_admission_qpc` sample.
3. Evaluate the lease using that same sample.
4. Call the packetized `SendInput` transport with that supplied start sample.
5. Read/validate the transport's `sendinput_completion_qpc` boundary and masks.
6. Commit coordinator ownership using the confirmed transport result.
7. Enqueue one bounded raw observation and return to orchestration.

The transport sends Up entries before Down entries in one call. Partial Down or
mixed integrity loss is never blindly retried. A skipped key that the
coordinator still owns is state disagreement and requires full cleanup and
termination. Any zero/partial transport result is terminal for the playback
worker; cleanup is a separate fail-closed release-all operation.

Up-only traffic uses command and lease admission but not the Down focus gate.
Down traffic compares the stamped HWND with the current foreground window at
the final gate. Focus hints and early loop gates are wake hints, not physical
authorization.

## 5. Wait, wake, and spin

The production wait path is a high-resolution waitable timer plus bounded QPC
spin. The production spin threshold is fixed at `700 µs`; no adaptive probe,
EMA, PID, or runtime threshold controller is allowed. Timer/wake guard is kept
separate from the absolute physical target.
Interrupts, lease-only wakes, focus changes, and command transitions invalidate
the frozen plan and replan; they never dispatch a stale plan.

The spin loop may use an interrupt-generation hint with `Relaxed` ordering and
poll it every bounded group of iterations. The final interrupt/deadline gate is
authoritative and uses `Acquire` ordering. Spin is bounded and cannot replace
the QPC deadline check.

At the wait layer, an interrupt can invalidate a plan only while the physical
target remains in the future. Once `QPC_now >= physical_target`, the waiter
returns `Deadline`, including when an interrupt wake and the target race. The
final command, target, focus, and lease admission remains authoritative and can
still reject the physical send. Production admission requires both the
high-resolution waitable timer and event wait; a missing timer or any runtime
wait failure is terminal rather than a silent sleep-based fallback.

MMCSS Games/High and power-throttling opt-out remain scoped scheduling aids.
TimeCritical is not the default. No wait or priority choice changes the
SendInput-only security boundary.

## 6. Deferred observation

Production does not enqueue observations or run an observer thread. After
ownership commit it records only bounded scalar counters. Strict diagnostic
mode pushes a raw observation into a fixed-capacity
`crossbeam_queue::ArrayQueue` using a nonblocking operation. If full, the new
observation is dropped and a counter is incremented; older observations remain
queued. The producer does not drain, allocate, format strings, update health,
sample `queue.len()`, or mutate the coordinator. Queue high-watermark and
health-window calculations belong to the observer-side diagnostic path.

In diagnostic mode a single observer consumer thread owns the queue drain,
health windows, telemetry record materialization, shared metric publication,
and observer timing counters. It uses its own local health state and metrics.
On shutdown, the worker signals the consumer, joins it, merges its metrics,
and only then publishes the final report. Observer output cannot authorize or
reorder input.

## 7. Observability and compatibility

The authoritative sender-side metrics are:

- `final_admission_qpc` and `sendinput_completion_qpc`;
- signed `dispatch_start_error_ticks = final_admission_qpc - physical_target_qpc`
  as the primary dispatch timing metric;
- `admission_to_completion = sendinput_completion_qpc - final_admission_qpc`;
- completion residual/error as diagnostic evidence only;
- requested/confirmed/skipped packet masks;
- release-floor/defer evidence; and
- diagnostic-only observer queue/drop and telemetry counters.

`actual_us`, completion lateness, and observed hold are sender-side proxies.
They must not be described as game-observed timing. The compatibility report
field `estimator_state_json` is deprecated and returns a non-operative marker;
old lead fields remain zero. No production decision may depend on them.

The signed start residual may be early or late and is retained without taking
an absolute value. It is observation/benchmark output, not controller
feedback: no EMA, PID, adaptive lead, or start-error compensation is allowed.

The serialized `send_started_ticks`, `send_completed_ticks`, and
`send_duration_us` names remain compatibility aliases for older Python/API
callers. They map to the same admission and completion boundaries above;
production does not take a second QPC sample to preserve the old names.
The optional `dispatch_ready_qpc` and `core_post_send_duration_us` fields are
diagnostic-only and are not sampled by the production sender.  The dispatch
worker CPU metric is likewise captured during worker finalization, never by
the deferred observer thread.

## 8. Validation obligations

Scheduler and coordinator code stays Windows- and I/O-free. Only the Win32
platform crate may contain `ctypes`-equivalent Win32 bindings and `SendInput`.
Timing edges are tested with controlled clocks. Any QPC arithmetic,
coordinator ownership, transport mask, target/focus, or cleanup inconsistency
fails closed rather than guessing.
