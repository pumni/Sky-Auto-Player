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
| `final_policy_qpc` | Worker-owned QPC evidence sampled after final control/target/focus checks; it is not a musical deadline. |
| `pre_call_qpc` | True sender-owned QPC sample taken after payload resolution and immediately before `SendInput`; the latest-start comparison follows it. |
| `sendinput_completion_qpc` | QPC sample returned after the prepared SendInput call. |
| `pre_call_to_completion` | The interval from `pre_call_qpc` to `sendinput_completion_qpc`; compatibility field `send_duration_us` retains this value. |
| `timing_margin` | User-owned persisted value, from `0` through `3,000 µs` in `100 µs` steps; frozen into each prepared session. |
| `min_hold` | Fixed materialized floor equal to the selected frame-based hold plus the exact user Timing Margin. |
| `physical_latest_down_start` | `authored_target + Timing Margin`; the physical/musical feasibility boundary for a Down-bearing packet. |
| `sender_cutoff` | Strict/diagnostic mode uses `physical_latest_down_start`; a paired normal prepared Down uses its frozen static authored hold-validity cutoff. |
| `prepared_sender_cutoff` | Paired Down only: `physical_target + hold_slack`, where `hold_slack = authored_up - authored_down - effective_min_hold`; an unpaired Down has no invented finite cutoff. |
| `musical_up_not_before` | Per-key floor from the last successful Down completion plus `frame_base_hold_ticks`. |
| `down_not_before` | Per-key floor from the last successful Up completion plus `frame_ticks`. |
| `min_release_gap` | One frame period plus the same exact user Timing Margin between a same-key Up and the next same-key Down. |
| `timing_margin_recommendation` | Informational value: qualified calibration may suggest a supported value; unqualified evidence falls back to `500 µs`, while insufficient headroom produces no recommendation. It never writes settings. |
| `authored_hold_valid` | Pre-start proof that authored Down→Up spacing meets the materialized hold. |

The worker never applies a learned dispatch-cost lead to `scheduled` or
`physical_target`. Historical `dispatch_lead_us`, estimator state, and lead
saturation fields are accepted only for compatibility and are non-operative.

The timing evidence has four distinct boundaries: the authored target, the
sender pre-call QPC, the SendInput completion QPC, and game observation. Only
the first three are available to this application. For a `require_focus=true`
Down, the accepted final contract performs exactly one fresh synchronous
foreground proof, then rechecks the target stamp, published focus, and late
control before the sender. UpOnly traffic and every `require_focus=false`
path perform zero fresh foreground queries. The only remaining focus race is
the interval from that query to `SendInput`.

## 2. Hold and release contract

The native application validates `hold_frames` as one of `1.0`, `1.25`, or `1.5`, computes the
frame-based hold and applies the persisted user margin symmetrically:

```text
frame_us = ceil(1_000_000 / game_fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
timing_margin_us = persisted_user_value
min_hold_us = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

For every authored same-key Down→Up pair:

```text
authored_up >= authored_down + min_hold_us
next_same_key_down - previous_same_key_up >= min_release_gap_us
```

The user margin is applied once while materializing the authored schedule.
The native desktop adapter receives that materialized value verbatim; the native admission
validator checks this relationship before worker start in the same QPC tick domain used by
dispatch. If the interval is invalid, native admission fails before any musical packet can be
sent; the worker never reschedules the Up target.

The release gap is authored statically and uses the same user-owned margin as
the hold floor. A fixed-size worker-owned `PhysicalTimingGuard` remains
available for strict/diagnostic physical forensics. Normal prepared playback
uses authored targets only; completion-relative floors do not move a normal
target or rebase the timeline.

Strict/diagnostic Down-bearing packets evaluate physical feasibility against the
Timing Margin boundary. Normal prepared packets use the immutable authored
target. A paired Down also carries a static authored hold-validity cutoff;
this is not dynamic physical-window policy or scheduler-lateness grace. An
unpaired Down has no finite cutoff, but still requires causal future
authorization:

```text
physical_latest_down_start_qpc = authored_target_qpc + timing_margin_ticks
packet_not_before_qpc = max(authored_target_qpc, relevant physical floors)
packet_not_before_qpc <= physical_latest_down_start_qpc
strict_sender_cutoff_qpc = physical_latest_down_start_qpc
paired prepared_sender_cutoff_qpc = physical_target_qpc + authored_hold_slack_ticks
unpaired prepared_sender_cutoff_qpc = none; future authorization is required
normal pre_call_qpc = authoritative sender evidence for the applicable cutoff
strict pre_call_qpc <= strict_sender_cutoff_qpc
```

If the physical floor is later than `physical_latest_down_start_qpc`, strict or
diagnostic dispatch classifies the complete Down chord as
`PhysicalWindowExpired`. Normal prepared playback has no dynamic physical
floor, but a paired sender that crosses its static authored validity cutoff is
classified as `DownExpiredBeforeSend`; an unobserved due Down is
`UnobservedBacklog`. Emergency and cleanup Ups bypass musical floors. Partial, uncertain, or post-send clock failures
invalidate guard evidence and fail closed through cleanup.

Timing Margin remains the only user-owned authored headroom: it extends the
authored hold and release targets and defines `physical_latest_down_start` for
strict/diagnostic qualification. Prepared paired Down hold slack is static
authored validity, not a configurable late-note timing policy.

Before a native session starts, the boundary validator rejects every authored
same-key Down→Up interval below `min_hold_us`, including intervals
that share one authored timestamp. Runtime completion QPC stays in the player
worker and never enters `sky_dispatch_core`. An unobserved Down or a Down whose
physical window has expired is committed as missed; the worker continues with
the next authored boundary while required safety Ups are still released. No
stale Down is sent as a startup or recovery exception, and no missed target is
retried or caught up.

## 2.1 Host sender hold-margin calibration (protocol 10)

Production calibration qualifies only sender-side completion-hold shrink. It
does not estimate game consumption, physical switch state, audio onset, Raw
Input delivery, or `WM_INPUT` message-pump latency. Publishable calibration
does not register, wait for, retry on, or qualify from Raw Input.

For each balanced packet pair, all keys share one set of authoritative QPC
boundaries. In raw QPC ticks:

```text
T_D = scheduled Down target       P_D = Down SendInput pre-call
C_D = Down SendInput completion   T_U = scheduled Up target
P_U = Up SendInput pre-call       C_U = Up SendInput completion

target_hold = T_U - T_D
completion_hold = C_U - C_D
scheduler_shrink = (P_D - T_D) - (P_U - T_U)
sendinput_shrink = (C_D - P_D) - (C_U - P_U)
sender_hold_shrink = target_hold - completion_hold

sender_hold_shrink = scheduler_shrink + sendinput_shrink
```

The identity is checked before converting to microseconds. All tick
subtractions and conversions are checked; failure is terminal. The signed
pair metric remains diagnostic evidence, but qualification uses only the
positive `sendinput_shrink_us.max` from each required bucket. It is not a
per-key worst value and not a sum of scheduler and SendInput quantiles.
Polyphony remains important because 1-, 5-, and 15-key packets have different
SendInput call durations.

The required matrix is exactly `1/hot`, `1/cold`, `5/hot`, `5/cold`, `15/hot`,
and `15/cold`. Each bucket requires 100 clean pairs and permits at most 200
attempts. Only hot/cold class mismatch is a retryable rejection. Down targets
are anchored to the previous completion plus the requested gap; Up targets
are anchored to the exact Down completion plus the requested gap. After `C_D`,
the runner waits for `T_U` directly and sends Up without waiting for a receipt.

For each bucket, qualification uses the maximum positive SendInput shrink:

```text
transport_worst_positive = max(0, maximum required-bucket sendinput_shrink_us.max)
candidate_transport_reserve_us = transport_worst_positive + 100

candidate <= 2,000 µs -> VALID, reserve = max(300 µs, candidate)
candidate > 2,000 µs  -> OUT_OF_ENVELOPE, use fallback reserve = 300 µs
```

With qualified calibration, the user-facing recommendation is
`ceil_to_100us(measured_transport_reserve + 100us_guard)`. When
calibration is missing, invalid, or out of envelope, the recommendation is the
`500 µs` default Timing Margin. It never changes a saved user setting or an
active/prepared session. Settings and the quick profile present it as
informational text; users adjust Timing Margin with the bounded stepper.
If the rounded result exceeds the public `3,000 µs` maximum, the recommendation
is unavailable with source `insufficient_headroom`; the UI does not clip it.
Qualification status and source remain visible. Calibration does not change
Note-On timestamps, physical Down targets, Timing Margin, or runtime
scheduling. Protocol 10, native schema 16, artifact
schema 11, cache version 8, source formula version 6, and evidence kind
`sender_completion_hold_shrink` are mutually incompatible with protocol-9 /
cache-v5/v6/v7 Raw Input or old sender-formula evidence. A failed or invalid
measurement preserves the previous compatible cache; an old cache does not
qualify a recommendation and uses the default Timing Margin recommendation.

Before warm-up, sender calibration performs a sender-only preflight: it proves
physical All-Up, sends one prepared full All-Up packet through the production
SendInput primitive, records that packet's real completion as the first
completion anchor, and proves All-Up again. This setup packet is not a warm-up
sample and cannot enter any quantile. The preflight does not register, wait
for, or inspect Raw Input.

Raw Input may exist in a separate engineering observer diagnostic. Its receipt
timestamps, queue timestamps, and observer failures must never affect sender
quantiles, clean-pair counts, retry decisions, candidate margin, or cache
trust, and must never be written as production margin evidence.

Historical protocol-9 Raw Input observer mechanics have been retired from active
production contracts and remain preserved in Git history.

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

The worker's Down authorization state has only `AwaitingFuture` and
`FutureAuthorized(PhysicalBoundaryStamp)`. Every Down-bearing boundary,
including first preroll, must be observed while its exact frozen target is in
the future. The identity stamp survives waiter-entry latency and a same-plan
control replan, and is consumed before the boundary is admitted. A changed
plan, pause, focus/epoch reset, or completed boundary clears it.

In normal prepared playback, an authorized Down is sent once while its static
authored hold-validity cutoff permits it. A due Down without the exact future
proof is `unobserved_backlog`; a paired Down rejected by the sender cutoff is
`final_sender_window_expired`/`DownExpiredBeforeSend`. Normal prepared
recovery never constructs a dynamic `physical_window_expired` policy. All
missed Down targets remain immutable and are never retried, rebased, or
emitted as a catch-up burst.

## 4. Authoritative send ordering

The final physical path is ordered and fail-closed:

1. Prepare and validate the immutable packet before the target wait.
2. One interruptible high-resolution hybrid waiter crosses the immutable
   authored target. Strict/diagnostic mode may include its physical floors.
3. Apply the initial command/control and Down target/published-focus checks
   after target crossing. An eligible `require_focus=true` Down then performs
   exactly one fresh synchronous foreground query; UpOnly and
   `require_focus=false` perform zero fresh queries. A rejection performs no
   packet syscall.
4. After that query, recheck the target stamp, then the published focus, then
   late control. These rechecks are atomic and do not issue another foreground
   query.
5. Take `final_policy_qpc` as evidence after those checks.
6. Enter the trusted prepared sender. It resets Win32 last-error state, takes
   the true `pre_call_qpc` after payload resolution, applies only the strict
   physical check when enabled, and immediately performs one
   packetized `SendInput` call. It does not wait, spin, or redo
   control/focus/target admission.
7. Read/validate the transport's `sendinput_completion_qpc` boundary and masks.
8. Commit coordinator ownership using the confirmed transport result.
9. Enqueue one bounded raw observation and return to orchestration.

The transport sends Up entries before Down entries in one call. Partial Down or
mixed integrity loss is never blindly retried. A skipped key that the
coordinator still owns is state disagreement and requires full cleanup and
termination. Any zero/partial transport result is terminal for the playback
worker; cleanup is a separate fail-closed release-all operation. The typed
`DownExpiredBeforeSend` is also the normal prepared paired-sender expiry result
when the static authored validity cutoff is crossed. Its recovery uses only
the immutable bounded Up prefix and frozen missed-boundary commit; normal
recovery never enters `PhysicalTimingWindow` or `PhysicalTimingGuard` policy.

Up-only traffic uses command admission but not the Down focus gate and issues
zero fresh foreground queries. Down traffic first uses the stamped HWND and
published target/focus atomics; an eligible `require_focus=true` Down then
gets exactly one fresh synchronous foreground proof, followed by target-stamp,
published-focus, and late-control rechecks. `require_focus=false` Down traffic
also issues zero fresh queries. Only the query-to-`SendInput` race remains;
focus hints and early loop gates are wake hints, not physical authorization.

## 5. Wait, wake, and spin

The production wait path uses one high-resolution waitable timer and event
interruption directly to the absolute authored target. There is no per-note
early admission wake and no second precision wait. The waiter sleeps until the
remaining target interval reaches the frozen startup-calibrated threshold, then
performs the bounded QPC spin to the target. Interrupts, focus changes, and
command transitions invalidate the frozen plan; the supervisor watchdog is a
control-plane interrupt, not a musical deadline.

Production startup calibration uses six wake samples when at least 20 ms remains
before the startup readiness deadline. It derives
`clamp(max(p99, robust) + 50 µs, 250 µs, 1,000 µs)`. Probe failure or insufficient
startup budget uses the 1,000 µs fallback. This value is frozen for the session;
it changes waiting cost only, never authored timestamps and never dispatch lead.

The final precision loop performs the QPC target comparison and bounded
interrupt-generation polling. It does not inspect lease state, focus, or
commands. The worker then performs initial control/target/published-focus
admission; an eligible `require_focus=true` Down receives exactly one fresh
synchronous foreground proof, followed by target-stamp, published-focus, and
late-control rechecks. UpOnly and `require_focus=false` perform zero fresh
queries. The trusted sender samples the true `pre_call_qpc` after payload
resolution and immediately before the sender-cutoff/`SendInput` pair; only the
query-to-`SendInput` race remains.
Physical feasibility still uses the materialized session margin; normal
playback applies the fixed internal total tolerance through the effective
cutoff, while strict mode uses the physical boundary unchanged.

The handoff benchmark reports `target_crossing_to_final_policy_us` for the
worker-owned target crossing through final policy admission and
`final_policy_to_true_pre_call_us` for the remaining worker-to-sender gap.
That second interval is expected to be small but non-zero on a preempted
worker, and is now measured rather than hidden by timestamp aliasing.
`WaitResult.spin_ticks` records the bounded spin in the single direct target
wait.

At the wait layer, an interrupt can invalidate a plan only while the physical
target remains in the future. Once `QPC_now >= physical_target`, the precision
waiter returns `Deadline`, including when an interrupt wake and the target race.
The final command, target, focus, and lease admission remains authoritative and
can still reject the physical send after that crossing. A same-boundary
`Continue` or replan does not erase an exact Down authorization; a different
boundary or epoch does. Kernel `wait_result.is_some()` is timing transport
evidence, not the authority for musical authorization. Production admission
requires both the high-resolution waitable timer and event wait; a missing
timer or any runtime wait failure is terminal rather than a silent sleep-based
fallback.

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

Production forensics remains in the worker as fixed-size state. Successful
packets update hold and release anchors only from trusted completion QPC values;
observed next-direction pre-call starts are compared with the fixed base
hold/frame floors. It retains sample counts, minima, violations, structural
anomalies, and a bounded anomaly ring. The sender never samples QPC for this
forensics, allocates, locks, or consults the diagnostic observer.

## 7. Observability and compatibility

The authoritative sender-side metrics are:

- `final_policy_qpc`, true `pre_call_qpc`, and `sendinput_completion_qpc`;
  - diagnostic-only `precision_handoff` evidence carrying the direct target
  wait wake (when present), target-crossing, and final-policy QPC boundaries;
- signed `dispatch_start_error_ticks = pre_call_qpc - physical_target_qpc`
  as the primary pre-call timing metric; it is not a syscall-entry or game-
  receipt timestamp;
- `pre_call_to_completion = sendinput_completion_qpc - pre_call_qpc`;
- completion residual/error as diagnostic evidence only;
- requested/confirmed/skipped packet masks;
- release-floor/defer evidence;
- mutually exclusive Down miss reasons and separate hold/release-floor delay
  counts, maximum delay, QPC boundaries, and contributing masks; and
- diagnostic-only observer queue/drop and telemetry counters;
- fixed-size production hold/release floor samples, minima, violations,
  ownership anomalies, and same-key overlap corruption evidence.

The producer-side maximum SendInput pre-call lateness is retained in raw QPC
ticks. The `2 ms`, `5 ms`, and `10 ms` bucket cutoffs are converted once during
worker admission; the compatibility/public microsecond maximum is derived only
when a metrics snapshot is published. Native telemetry schema 16 carries the
floor and latest-start trace boundaries without a QPC frequency conversion on
every physical send.

Fixed internal scalar telemetry retains transport status, strict/test-support
miss evidence, and bounded physical forensics. The retired normal late-rescue
counters are not claims about game observation and are no longer published.
`physical_window_expired` and `unobserved_backlog` remain available for strict
or diagnostic qualification, and the public/native telemetry schema remains
backward-compatible apart from the retired dead-policy fields.

`actual_us`, completion lateness, and observed hold are sender-side proxies.
They must not be described as game-observed timing. The compatibility report
field `estimator_state_json` is deprecated and returns a non-operative marker;
old lead fields remain zero. No production decision may depend on them.

The signed start residual may be early or late and is retained without taking
an absolute value. It is observation/benchmark output, not controller
feedback: no EMA, PID, adaptive lead, or start-error compensation is allowed.

The serialized `send_started_ticks`, `send_completed_ticks`, and
`send_duration_us` names remain compatibility aliases for older API
callers. They map to the true sender `pre_call_qpc`,
`sendinput_completion_qpc`, and `pre_call_to_completion`; the separate
`final_policy_qpc` remains policy evidence and is not used as the sender start.
The optional `dispatch_ready_qpc`, `precision_handoff`, and
`core_post_send_duration_us` fields are diagnostic-only and are not sampled by
the production sender. `precision_handoff` reuses the precision waiter's raw
deadline wake evidence and does not add a QPC read. The dispatch worker CPU
metric is likewise captured during worker finalization, never by the deferred
observer thread.

## 8. Validation obligations

Scheduler and coordinator code stays Windows- and I/O-free. Only the Win32
platform crate may contain `ctypes`-equivalent Win32 bindings and `SendInput`.
Timing edges are tested with controlled clocks. Any QPC arithmetic,
coordinator ownership, transport mask, target/focus, or cleanup inconsistency
fails closed rather than guessing.
