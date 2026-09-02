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
| `final_policy_qpc` | Worker-owned QPC sample taken after final control/target/focus checks and used directly for lease evaluation; those predicates are not evaluated at the timestamp itself. |
| `pre_call_qpc` | True sender-owned QPC sample taken after payload resolution and immediately before `SendInput`; only the Down late-grace cutoff comparison follows it. |
| `sendinput_completion_qpc` | QPC sample returned after the prepared SendInput call. |
| `pre_call_to_completion` | The interval from `pre_call_qpc` to `sendinput_completion_qpc`; compatibility field `send_duration_us` retains this value. |
| `effective_min_hold` | Fixed materialized hold floor passed into the native worker. |
| `down_late_grace` | Independent fixed sender correctness grace for authorized Down admission; production is `500 µs`. |
| `transport_margin` | Calibrated sender transport component; default/fallback is `300 µs` and calibration never changes the Down grace. |
| `min_release_gap` | One frame period plus the same static sender headroom reserved for the hold floor, between a same-key Up and the next same-key Down. |
| `authored_hold_valid` | Pre-start proof that authored Down→Up spacing meets the materialized hold. |

The worker never applies a learned dispatch-cost lead to `scheduled` or
`physical_target`. Historical `dispatch_lead_us`, estimator state, and lead
saturation fields are accepted only for compatibility and are non-operative.

The timing evidence has four distinct boundaries: the authored target, the
sender pre-call QPC, the SendInput completion QPC, and game observation. Only
the first three are available to this application. `require_focus=true` is a
safety profile with a final foreground-verification cost; it must not be
described as having the same latency as `require_focus=false`.

## 2. Hold and release contract

The native application validates `hold_frames` as one of `1.0`, `1.25`, or `1.5`, computes the
requested hold, and passes:

```text
frame_us = ceil(1_000_000 / game_fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
down_late_grace_us = policy.down_late_grace_us
transport_margin_us = max(0, calibrated_or_default_transport_margin_us)
effective_min_hold = (
    frame_base_hold_us + down_late_grace_us + transport_margin_us
)
sender_headroom_us = down_late_grace_us + transport_margin_us
min_release_gap_us = frame_us + sender_headroom_us
```

For every authored same-key Down→Up pair:

```text
authored_up >= authored_down + effective_min_hold
next_same_key_down - previous_same_key_up >= min_release_gap_us
```

The static margin is applied once while materializing the authored schedule.
The native desktop adapter receives that materialized value verbatim; the native admission validator
checks this relationship before worker start in
the same QPC tick domain used by dispatch. If the interval is invalid, native
admission fails before any musical packet can be sent; the worker never
reschedules the Up target.

The release gap is authored statically and uses the same bounded sender-side
headroom as the hold floor. Runtime never delays the next Down to repair a
boundary. Sender completion evidence can verify the sender-side interval, but
it does not prove that the game sampled either transition.

`FrameTimingPolicy.min_hold_margin_us` remains a compatibility aggregate of the
Down grace and transport component. The explicit policy fields are
`frame_base_hold_us`, `down_late_grace_us`, `transport_margin_us`, and
`min_release_gap_us`; `min_hold_margin_source` preserves transport provenance.
The policy enforces:

```text
transport_margin_us >= 0
effective_min_hold_us = frame_base_hold_us + down_late_grace_us + transport_margin_us
```

The effective margin affects only the authored effective minimum hold. It is
never added to the authored target, never used as dispatch lead, and never
adapted during playback. Independently, production uses the fixed
`down_late_grace_us = 500` sender policy, materialized once as
`down_late_grace_ticks` at admission. For the tightest valid authored hold:

```text
authored_up - authored_down = effective_min_hold
actual_down_pre_call <= authored_down + down_late_grace

therefore:
authored_up - actual_down_pre_call
    >= effective_min_hold - down_late_grace
```

Because the effective static margin is at least the Down grace, every
production-accepted Down preserves at least the selected frame-base hold at
the authoritative pre-call boundary.

Equality at the cutoff is permitted; a pre-call QPC one tick beyond it makes
zero Down `SendInput` syscalls and follows the existing missed-Down recovery
path. Calibration never changes this grace.

Before a native session starts, the boundary validator rejects every authored
same-key Down→Up interval below `effective_min_hold_us`, including intervals
that share one authored timestamp. Runtime completion lateness is evidence for
sender-side telemetry and ownership accounting only. Runtime deadline/overdue
policy never invents a completion-relative hold deadline, moves an authored
timestamp, or emits a catch-up burst. Before the first successful musical Down
commit, a missed Down remains a startup failure. After startup, an unapproved
or late-grace-exceeding musical Down is recorded and committed as missed; the worker
continues with the next authored boundary while required safety Ups are still
released.

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
candidate_transport_margin_us = transport_worst_positive + 100

candidate <= 2,000 µs -> VALID, applied = max(300 µs, candidate)
candidate > 2,000 µs  -> OUT_OF_ENVELOPE, applied = none,
                         playback transport margin = 300 µs
```

The correction is applied exactly once to the authored minimum hold. It does
not change Note-On timestamps, physical Down targets, `down_late_grace_us`
(`500 µs`), or runtime scheduling. Protocol 10, native schema 15, artifact
schema 11, cache version 8, source formula version 6, and evidence kind
`sender_completion_hold_shrink` are mutually incompatible with protocol-9 /
cache-v5/v6/v7 Raw Input or old sender-formula evidence. A failed or invalid
measurement preserves the previous compatible cache; an old cache falls back
to the explicit `300 µs` transport margin and is not timing-qualified.

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

## 2.2 Historical protocol-9 Raw Input observer (non-normative)

The remainder of this subsection records the retired observer protocol for
forensics only. It is not a production qualification contract and must not be
used to interpret or migrate a protocol-9 cache.

Calibration is a separate native, app-owned Raw Input proxy. It measures a
target-to-receipt total hold proxy, not game polling, rendering, audio, or
network latency. The sender boundary is measured as:

```text
validate/build/tag packet -> arm sequence -> prepare fixed INPUT array
-> wait to `T - 700 µs` with zero waiter spin
-> fused sender target_crossing/pre_call_qpc -> SendInput -> sendinput_completion_qpc
-> validate receipt
```

The calibration precision boundary follows production dispatch: tagged INPUT
materialization is complete before the handoff, the wait layer owns only the
low-occupancy wait to `T - 700 µs`, and the shared fused sender owns the final
QPC crossing, authoritative `P`, and the single `SendInput` call. No INPUT
construction or allocation is permitted after the handoff.

The Raw Input QPC timestamp is taken at entry to the `WM_INPUT` handler. The
same handler preserves the raw uint32-millisecond `GetMessageTime()` queue
timestamp as diagnostic evidence. Queue-time differences use modular uint32
subtraction across wraparound; queue time is not converted to the QPC epoch and
does not participate in qualification. Down and Up packets must both match
their direction, exact scan code, and extended flags. Publishable calibration
requires the injection sequence tag to decode
and match on every receipt; Windows does not document preservation of
`KEYBDINPUT.dwExtraInfo` in `RAWKEYBOARD.ExtraInformation`, so a missing tag is
terminal rather than silently correlated. The pump-thread barrier handler
explicitly removes already-queued `WM_INPUT`
messages with an input-range filter before the next active packet is armed, so
missing tags cannot silently alias stale receipts. If a packet is incomplete or
times out, the correlation boundary is lost and the session cannot arm another
packet; finding stale-generation evidence during the drain or normal dispatch
likewise prevents further packet arming. An active receipt with an incompatible
identity or direction, a duplicate, or a pending-receipt overflow is likewise
boundary-losing evidence; a scheduling class mismatch remains a rejected
sample and the only retryable measurement rejection. Parser/read failures,
reordered receipts, and chronology violations terminate the observer/session;
they are never converted into bounded retry noise. A clean
pair computes signed per-key values:

```text
D = down_receipt_qpc - down_completion_qpc
U = up_receipt_qpc - up_completion_qpc
T = target_qpc
P = pre_call_qpc
C = sendinput_completion_qpc
R = first_receipt_qpc

scheduler_shrink = (P_D - T_D) - (P_U - T_U)
sendinput_shrink = (C_D - P_D) - (C_U - P_U)
delivery_shrink = (R_D - C_D) - (R_U - C_U)
total_proxy_shrink = (T_U - T_D) - (R_U - R_D)

total_proxy_shrink = scheduler_shrink + sendinput_shrink + delivery_shrink
```

The five-key startup correlation probe verifies physical All-Up read-only after
its balanced Down/Up sequence; it must not send an untagged All-Up packet while
the exact-tag observer is active. A publishable bucket performs one final
pump-thread-owned queue/trust seal: the pump enters `Sealing`, drains pending
`WM_INPUT`, restores its Raw Input registration, drains once more, and reaches
`Sealed` before posting its exit and allowing physical cleanup. A failure before
or during that seal terminates the bucket and cannot be published.

`R - C` is intentionally signed. The pump thread may observe a foreground
`WM_INPUT` while the measurement thread is still inside `SendInput`, so
`R < C` is valid and must not be converted into a pairing anomaly. The
collector closes only after every expected `(sequence, scan code, direction)`
identity has arrived; duplicates and unexpected receipts remain bounded
diagnostic evidence rather than satisfying completion early.

Each balanced pair anchors its Down target to the preceding packet's exact
`SendInput` completion plus the requested class gap. After that Down completes,
the Up target is derived from that exact Down completion plus the same gap.
Classification uses the observed completion-to-entry idle interval, not the
requested target spacing. This keeps a late or long Down syscall from silently
turning a requested cold Up into a hot sample; the pair is either observed in
the requested class or rejected with its diagnostic counters.

The publishable matrix has six buckets (`1/5/15 × hot/cold`) with at least
100 clean pairs in each. The configured `100` is a clean-pair target, not an
attempt cap: the native runner may make bounded additional attempts and
serializes the actual attempt count. If the target is not reached, the result
remains diagnostic/non-publishable and no empty quantile is accepted as
calibration evidence. Qualification uses:

```text
candidate = positive global p99 + 100 µs

candidate <= 2,000 µs
    => calibrated, applied margin = max(300 µs, candidate)
candidate > 2,000 µs
    => out of the trusted correction envelope, no calibrated correction
```

An out-of-envelope measurement is complete evidence, is written as unhealthy
cache v5, and playback falls back to the explicit 500 µs hold margin. A
measurement/integrity failure preserves the previous cache. The correction is
applied only to the hold floor; it never leads the playback target, changes
`down_late_grace`, or claims game-observed timing. Full-calibration checkpoints use plain-text SHA256
sidecars and a stable common provenance manifest. Resume and finalization
reject any bucket whose source/build identity, native source fingerprint,
Rust version, QPC frequency, Windows build, CPU identity, topology,
efficiency-class histogram, or scheduling-aid acquisition labels differs from
the other five; only the observation timestamp may differ. Each native result
records the acquired MMCSS label, PowerThrottling/HighQoS guard state, and
waiter mode. Runtime cache loading also requires the stable host fingerprint to
match the current native host. The final artifact and cache are published only
after the complete matrix and cache have both passed validation.

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

After a successful musical Down dispatch, the worker arms a Down-only boundary
state. A later Down-bearing boundary is authorized only when that exact frozen
boundary was observed with `physical_target_qpc > now` after the previous Down
commit. The authorization is an identity stamp, not a QPC-only proof, and it
survives waiter-entry latency and a same-boundary control replan. It is cleared
by a changed plan/target, pause or focus rebase, or completion of the stamped
boundary.

An overdue Down without that exact authorization is a missed musical boundary:
Production sends no Down and commits the authored frame as missed. A
HardLate-authorized Down follows the same recovery path after the trusted
sender's authoritative pre-call late-grace cutoff rejects the syscall. Strict timing diagnostics
may keep these misses terminal for qualification. Up-only safety release is
not part of the musical backlog rule and is sent even when its authored target
is late. Thus no overdue Down burst is emitted, but ordinary jitter does not
terminate a started production session.

## 4. Authoritative send ordering

The final physical path is ordered and fail-closed:

1. Prepare and validate the immutable packet before the target wait.
2. One interruptible lease-bounded hybrid waiter crosses the absolute authored
   physical target with the frozen bounded QPC spin. It returns the crossing
   sample; it does not change the target.
3. Recheck command/control, the stamped target, and foreground focus (Down
   only) after target crossing. A rejection records a bounded final-gate
   diagnostic and performs no packet syscall.
4. Recheck the program-owned control, target stamp, and focus atomics after
   the foreground query. This is a cheap revalidation and does not issue a
   second foreground query.
5. Take `final_policy_qpc` after those checks and use it for the final lease
   admission.
6. Enter the trusted prepared sender. It resets Win32 last-error state, takes
   the true `pre_call_qpc` after payload resolution, checks the Down-only
   late-grace cutoff against that sample, and immediately performs one
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
`DeadlineMissedBeforeSend` result is the one timing exception: when the
session has already committed its first musical Down, it performs the bounded
missed-frame recovery path and keeps `SendInput` uncalled.

Up-only traffic uses command and lease admission but not the Down focus gate.
Down traffic compares the stamped HWND with the current foreground window at
the final gate. Focus hints and early loop gates are wake hints, not physical
authorization.

## 5. Wait, wake, and spin

The production wait path uses one high-resolution waitable timer and event
interruption directly to the absolute physical target. There is no per-note
`T - 2,000 µs` admission wake and no second precision wait. The waiter sleeps
until the remaining target interval reaches the frozen startup-calibrated
threshold, then performs the bounded QPC spin to the target. Interrupts,
lease-only wakes, focus changes, and command transitions invalidate the frozen
plan and replan; they never dispatch a stale plan.

Production startup calibration uses six wake samples when at least 20 ms remains
before the startup readiness deadline. It derives
`clamp(max(p99, robust) + 50 µs, 250 µs, 1,000 µs)`. Probe failure or insufficient
startup budget uses the 1,000 µs fallback. This value is frozen for the session;
it changes waiting cost only, never authored timestamps and never dispatch lead.

The final precision loop performs the QPC target comparison and bounded
interrupt-generation polling. It does not inspect lease state, focus, or
commands. The worker then performs the final control/target/focus proof, runs
one cheap atomic revalidation of the program-owned state, and records
`final_policy_qpc` for lease admission. The trusted sender samples the true
`pre_call_qpc` after payload resolution and immediately before the
cutoff/`SendInput` pair, closing the worker-to-syscall preemption window. It
uses the same materialized session margin; it does not compute a new
threshold.

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
reorder input. The observer also owns fixed `[Option<HoldAnchor>; 15]` sender
packet-boundary hold forensics. Successful packets are processed in canonical
UP-then-DOWN order; each paired sample compares authored target, pre-call, and
completion QPC boundaries. These are sender-side hold proxies only, never game
observations. Incomplete transport does not mutate pairing state, and the
forensics scalars do not alter the native telemetry schema or dispatch path.

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
- release-floor/defer evidence; and
- diagnostic-only observer queue/drop and telemetry counters;
- diagnostic-only sender packet-boundary hold pairs, including minimum
  pre-call/completion hold, positive shrink, grace violations, ownership
  anomalies, and same-call retrigger counts.

The producer-side maximum SendInput pre-call lateness is retained in raw QPC
ticks. The `2 ms`, `5 ms`, and `10 ms` bucket cutoffs are converted once during
worker admission; the compatibility/public microsecond maximum is derived only
when a metrics snapshot is published. This preserves the public schema and
bucket semantics without a QPC frequency conversion on every physical send.

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
