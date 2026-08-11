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
| `scheduled` | Authored/effective playback timestamp, including explicit recovery offset. |
| `physical_target` | Absolute QPC target derived from the playback epoch and `scheduled`. |
| `started` | One authoritative QPC sample taken immediately before final admission/SendInput. |
| `completed` | QPC sample returned after the packetized SendInput call. |
| `effective_min_hold` | Fixed materialized hold floor passed into the native worker. |
| `effective_release` | Release deadline after hold and retry/recovery floors. |

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

For every successful Down packet:

```text
release_floor = completed + effective_min_hold
effective_release = max(authored_release, release_floor, retry_not_before)
```

The release floor is a sender-side contract. A completion sample does not prove
kernel delivery or game observation. A slow Down can defer its own release and
the corresponding recovery state, but cannot shift unrelated future authored
actions.

## 3. Planning and physical target

Each worker epoch freezes one typed plan. It contains the current authored or
pending deadline, physical path, polyphony, health budget, and absolute QPC
target. The same deadline is used for waiting, due selection, final target
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

## 4. Authoritative send ordering

The final physical path is ordered and fail-closed:

1. Recheck command, target generation, focus (Down only), and the prepared
   packet against the current coordinator state.
2. Take one authoritative QPC `started` sample.
3. Evaluate the lease using that same sample.
4. Call the packetized `SendInput` transport with that supplied start sample.
5. Read/validate the transport's `completed` QPC boundary and masks.
6. Commit coordinator ownership using the confirmed transport result.
7. Enqueue one bounded raw observation and return to orchestration.

The transport sends Up entries before Down entries in one call. Partial Down or
mixed integrity loss is never blindly retried. A skipped key that the
coordinator still owns is state disagreement and requires full cleanup and
termination. Up recovery may requeue only unconfirmed ownership and must finish
before the worker advertises a ready boundary.

Up-only traffic uses command and lease admission but not the Down focus gate.
Down traffic compares the stamped HWND with the current foreground window at
the final gate. Focus hints and early loop gates are wake hints, not physical
authorization.

## 5. Wait, wake, and spin

The production wait path is a high-resolution waitable timer plus bounded QPC
spin. Timer/wake guard is kept separate from the absolute physical target.
Interrupts, lease-only wakes, focus changes, and command transitions invalidate
the frozen plan and replan; they never dispatch a stale plan.

The spin loop may use an interrupt-generation hint with `Relaxed` ordering and
poll it every bounded group of iterations. The final interrupt/deadline gate is
authoritative and uses `Acquire` ordering. Spin is bounded and cannot replace
the QPC deadline check.

MMCSS Games/High and power-throttling opt-out remain scoped scheduling aids.
TimeCritical is not the default. No wait or priority choice changes the
SendInput-only security boundary.

## 6. Deferred observation

After ownership commit, the dispatch thread pushes a raw observation into a
fixed-capacity `crossbeam_queue::ArrayQueue` using a nonblocking operation.
Capacity is bounded at 64 for the production observer. If full, the new
observation is dropped and a counter is incremented; older observations remain
queued. The producer does not drain, allocate, format strings, update health,
or mutate the coordinator.

A single observer consumer thread owns the queue drain, health windows,
telemetry record materialization, shared metric publication, and observer
timing counters. It uses its own local health state and metrics. On shutdown,
the worker signals the consumer, joins it, merges its metrics, and only then
publishes the final report. Observer output cannot authorize or reorder input.

## 7. Observability and compatibility

The authoritative sender-side metrics are:

- `sender_started_qpc` and `sender_completed_qpc`;
- signed `dispatch_start_error_ticks = sender_started_qpc - physical_target_qpc`
  as the primary dispatch timing metric;
- `send_duration = sender_completed_qpc - sender_started_qpc`;
- completion residual/error as diagnostic evidence only;
- requested/confirmed/skipped packet masks;
- release-floor/defer/recovery evidence; and
- bounded observer queue/drop and telemetry counters.

`actual_us`, completion lateness, and observed hold are sender-side proxies.
They must not be described as game-observed timing. The compatibility report
field `estimator_state_json` is deprecated and returns a non-operative marker;
old lead fields remain zero. No production decision may depend on them.

The signed start residual may be early or late and is retained without taking
an absolute value. It is observation/benchmark output, not controller
feedback: no EMA, PID, adaptive lead, or start-error compensation is allowed.

## 8. Validation obligations

Scheduler and coordinator code stays Windows- and I/O-free. Only the Win32
platform crate may contain `ctypes`-equivalent Win32 bindings and `SendInput`.
Timing edges are tested with controlled clocks. Any QPC arithmetic,
coordinator ownership, transport mask, target/focus, or cleanup inconsistency
fails closed rather than guessing.
