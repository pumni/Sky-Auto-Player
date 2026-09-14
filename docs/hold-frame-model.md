# Hold-Frame Timing Model

Sky Auto Player accepts exactly three explicit hold selections: `1.0`,
`1.25`, and `1.5` frames. The default is `1.0` at the user-selected game
FPS. FPS is never detected, inferred, or changed from runtime observations.

Timing evidence has four boundaries: the authored target, sender pre-call QPC,
SendInput completion QPC, and game observation. This application can verify
the first three sender-side boundaries only; completion evidence does not prove
that the game sampled the transition. `require_focus=true` is a safety profile
with a final foreground-verification cost, so the two focus modes are not
promised identical latency.

For a physical boundary, native preparation materializes and validates
one immutable packet before the interruptible wait. The worker's single
hybrid wait and bounded QPC spin cross the later of the authored target and
relevant physical floors. If a Down floor is outside its latest-start window,
the worker waits only to the authored target, consumes the exact authorization,
and classifies the Down chord as missed. A mixed packet's Up prefix then waits
independently for its musical hold floor before recovery. It runs the final
command/control, target, and focus gates, repeats the program-owned atomic
checks, and evaluates the lease. The worker records `final_policy_qpc` for
lease admission. The prepared sender then takes the true `pre_call_qpc`
immediately before the latest-start check and one `SendInput` call.
Up entries precede Down entries;
an overlapping Up/Down mask is rejected during preparation. A partial Up is
reported with partial-progress evidence but is never silently retried by this
single-send primitive.

For a selected ratio and FPS, the native planner first materializes the requested hold:

```text
frame_us = ceil(1_000_000 / fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
timing_margin_us = persisted_user_value
effective_min_hold_us = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

The user-owned Timing Margin defaults to `500 µs`, ranges from `0` through
`3,000 µs` in `100 µs` steps, and applies equally to Hold and Release Gap.
It is also the Down latest-start headroom: a Down may begin no later than its
authored target plus this margin. Calibration never supplies part of the
authored timing equation. Qualified calibration may produce an advisory
recommendation from measured transport reserve plus a fixed `100 µs` guard.
Without qualified evidence, the informational fallback recommendation is the
`500 µs` default Timing Margin. Calibration is never shown as qualified when
fallback is used.
Production calibration uses one pair metric per Down/Up SendInput
packet, based on `T_D/P_D/C_D` and `T_U/P_U/C_U`; Raw Input receipt timing is
not part of qualification. It uses exactly the six `1/5/15 × hot/cold`
buckets, at least 100 clean pairs per bucket, and at most 200 attempts per
bucket. Its transport-reserve candidate is the maximum positive
`sendinput_shrink_us.max` across required buckets plus a `100 µs` guard. A
candidate at or below `2,000 µs` qualifies and reserves at least `300 µs`; a
candidate above `2,000 µs` is out of the trusted envelope and uses the
unqualified transport reserve. A qualified recommendation is the transport
reserve plus the guard, rounded up to the next `100 µs` step.
An unqualified recommendation falls back to the `500 µs` default Timing
Margin. Recommendations are informational only and never write a setting.
Users adjust Timing Margin with its bounded stepper; a change affects only the
next prepared session.

The release gap is one base game frame plus the exact Timing Margin. The
immutable schedule remains the source of authored targets. In addition, the
worker-owned fixed-size `PhysicalTimingGuard` tracks per-key completion floors
using sender QPC evidence:

```text
musical_up_not_before = last_successful_down_completion + frame_base_hold
down_not_before = last_successful_up_completion + frame
latest_down_start = authored_down_target + timing_margin
```

The worker waits until the authored target and relevant physical floor are
both reached. If a Down floor exceeds its latest-start window, the worker
classifies the whole Down chord as missed at the authored target. Any Up-prefix
recovery then waits for its independent hold floor. A true pre-call QPC check closes the remaining race immediately
before `SendInput`. Authored Up and mixed Up-prefix recovery respect the
Down-completion hold floor. Emergency, focus-loss, and cleanup Ups bypass
musical floors so safety release stays immediate. Completion evidence never
proves that Sky sampled the transition.

Production hold forensics checks the fixed physical floors with trusted sender
timestamps. For each successful paired key, it records
`up_pre_call - prior_down_completion` against `frame_base_hold_ticks` and
`down_pre_call - prior_up_completion` against `frame_ticks`. It publishes
sample counts, minimum observed intervals, and violation counts separately
from authored schedule validation. These are sender-side checks, not proof
that Sky sampled, rendered, or produced audio for the transition.
The native worker receives the materialized `effective_min_hold_us` and
`min_release_gap_us` values and uses them as fixed authored durations. It also
converts `frame_base_hold_us`, `frame_us`, and `timing_margin_us` once during
worker admission to initialize the physical guard. Actual completion QPC
remains in the player worker and never enters `sky_dispatch_core`. Checked
arithmetic and masks fail closed; invalidated evidence requires lifecycle
reset before further physical queries. The authored policy enforces:

```text
effective_min_hold_us = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

Timing Margin is the only authored Down execution headroom. Zero is a valid
strict/no-headroom choice. Equality at the latest-start QPC is allowed; the
first tick beyond it is a missed Down. A Down chord is never split or retried.
Up-only safety releases remain exempt from musical floors.

At 60 FPS with the default margin:

| Hold | Requested | Effective hold | Release gap |
|---:|---:|---:|---:|
| 1.0 frame | 16,667 µs | 17,167 µs | 17,167 µs |
| 1.25 frames | 20,834 µs | 21,334 µs | 17,167 µs |
| 1.5 frames | 25,001 µs | 25,501 µs | 17,167 µs |

At 60 FPS with the default 1.0-frame selection, the exact same-key minimum
cycle is `17,167 + 17,167 = 34,334 µs` (about 29.13 repeated presses per
second per key). This is a deliberate sender-side reliability tradeoff; it is
not a claim about game frame registration.

## Authored minimum-hold validation

For an authored Down at timestamp `A`, the authored same-key Up target must
already satisfy:

```text
authored_up >= authored_down + effective_min_hold_us
```

The static margin is materialized once while building the authored schedule.
The native boundary validates this interval in checked QPC ticks before the
worker starts. It also requires `min_release_gap_us` between a same-key Up and
the next same-key Down. An invalid schedule is rejected; the runtime never
delays or replaces authored targets to repair it.

## Feasibility and diagnostics

Authored validation rejects a same-key interval below the selected hold floor.
The native-boundary validator performs this check before the worker can send
anything, including exact same-timestamp retriggers and timestamp overflow
cases. It also requires the next same-key Down to meet the materialized
`min_release_gap_us` after the previous same-key Up. Equal release-gap
boundaries are valid; same-timestamp same-key overlaps are rejected, while
disjoint masks may still coalesce. Runtime may delay a physical packet until
its floor is reached, but it never changes an authored target or retries a
missed Down.
Successful completion also updates the worker's physical floors, so a later
Up or same-key Down cannot violate a hold or release interval after transport
latency. Miss classification uses mutually exclusive reasons:
`unobserved_backlog`, `physical_window_expired`, or
`final_sender_window_expired`. Their counters sum to `missed_down_boundaries`;
a sender latest-start rejection is counted at that boundary once. Hold- and
release-floor delay counts and maxima are separate evidence, with the latest
contributing authored and physical QPC targets and masks retained in the
bounded trace. Each miss preserves authored timestamps and prevents a catch-up
send.

A transport zero/partial result is terminal and is handled by fail-closed
cleanup; it is not retried in production. Strict timing evaluates completion
residuals, while normal playback preserves the schedule when transport
integrity remains valid.

Production retains bounded worker-local scalars and a fixed anomaly ring:
hold/release floor sample counts, minimum observed intervals, floor violations,
same-call retriggers, anchor overwrites, unmatched Ups, and ring overwrites.
Structural anomalies and timing diagnostics have separate counters. Hold- and
release-floor delay counts and maxima are published independently from the
exclusive Down miss taxonomy. The production forensics block exposes an
availability/version marker and never allocates, locks, samples QPC, or
consults the diagnostic observer.
These values are not game-onset or audio-onset measurements. The old estimator
and adaptive dispatch lead are not part of this model; historical lead fields
are compatibility-only zeros.
