# Hold-Frame Timing Model

Sky Auto Player accepts exactly three explicit hold selections: `1.0`,
`1.25`, and `1.5` frames. The default is `1.0` at the user-selected game
FPS. FPS is never detected, inferred, or changed from runtime observations.

For a selected ratio and FPS, Python first materializes the requested hold:

```text
frame_us = ceil(1_000_000 / fps)
measured_margin_us = max(0, calibrated_or_default_margin_us)
down_late_grace_us = policy.down_late_grace_us
effective_margin_us = max(measured_margin_us, down_late_grace_us)
effective_min_hold_us = round(hold_frames * frame_us) + effective_margin_us
```

The default hold margin is `500 µs`; calibration may provide a validated
sender-side completion-hold correction. Production calibration uses one pair
metric per Down/Up SendInput packet, based on `T_D/P_D/C_D` and `T_U/P_U/C_U`;
Raw Input receipt timing is not part of qualification. It uses exactly the six
`1/5/15 × hot/cold` buckets, at least 100 clean pairs per bucket, and at most
200 attempts per bucket. Its candidate is the maximum positive p99 of
`sender_hold_shrink` plus `100 µs`. A candidate at or below `2,000 µs` is
valid and applies `max(300 µs, candidate)`; a candidate above `2,000 µs` is
out of the trusted correction envelope and applies no calibrated margin.
Protocol 9/cache v5/v6 evidence is incompatible with protocol 10/cache v7 and
falls back to the unchanged `500 µs` default. Completed out-of-envelope
protocol-10 evidence is retained as unhealthy cache evidence.

The sender evidence is computed in raw QPC ticks before conversion:

```text
sender_hold_shrink = (T_U - T_D) - (C_U - C_D)
sender_hold_shrink = ((P_D - T_D) - (P_U - T_U))
                    + ((C_D - P_D) - (C_U - P_U))
```

The identity is checked with checked arithmetic. It describes completion
interval compression in the Rust/SendInput sender only; it is not a claim
about game-observed timing.
The native worker receives only the materialized `effective_min_hold_us` and
uses it as a fixed duration. PyO3 does not add another frame-relative floor;
Rust only range-checks and validates this value in QPC ticks. It does not learn
or subtract SendInput cost. `FrameTimingPolicy.min_hold_margin_us` is the
post-policy effective static margin, not necessarily the raw calibration
measurement; `min_hold_margin_source` still records measurement provenance.
The independent fixed `down_late_grace_us` sender policy is `500 µs` and is
converted once to QPC ticks. The policy coupling enforces:

```text
effective_margin_us >= down_late_grace_us
```

Therefore an authorized Down accepted at the latest cutoff cannot reduce the
sender pre-call hold below the selected frame-base hold. Grace is never added
to an authored target or adapted during playback. Equality at the cutoff is allowed;
the first QPC tick beyond it is a missed Down. Up-only releases remain exempt.

At 60 FPS with the default margin:

| Hold | Requested | Effective |
|---:|---:|---:|
| 1.0 frame | 17,167 µs | 17,167 µs |
| 1.25 frames | 21,334 µs | 21,334 µs |
| 1.5 frames | 25,500 µs | 25,500 µs |

## Authored minimum-hold validation

For an authored Down at timestamp `A`, the authored same-key Up target must
already satisfy:

```text
authored_up >= authored_down + effective_min_hold_us
```

The static margin is materialized once while building the authored schedule.
The native boundary validates this interval in checked QPC ticks before the
worker starts. An invalid schedule is rejected; the runtime never delays or
replaces the authored Up target to repair it.

## Feasibility and diagnostics

Authored validation rejects a same-key interval below the selected hold floor.
The native-boundary validator performs this check before the worker can send
anything, including exact same-timestamp retriggers and timestamp overflow
cases. Runtime completion is evidence for sender-side telemetry and ownership
accounting only; it does not create a completion-relative hold floor or a new
deadline. Runtime deadline/overdue policy handles a late boundary without
rewriting authored timestamps or emitting a catch-up send.

A transport zero/partial result is terminal and is handled by fail-closed
cleanup; it is not retried in production. Strict timing evaluates completion
residuals, while normal playback preserves the schedule when transport
integrity remains valid.

Diagnostic mode may report sender-side start, completion, lateness, duration,
and release-floor evidence. Production retains only bounded scalar timing
counters. These values are not game-onset or audio-onset measurements. The
old estimator and adaptive dispatch lead are not part of this model; historical
lead fields are compatibility-only zeros.
