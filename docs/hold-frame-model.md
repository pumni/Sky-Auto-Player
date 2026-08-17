# Hold-Frame Timing Model

Sky Auto Player accepts exactly three explicit hold selections: `1.0`,
`1.25`, and `1.5` frames. The default is `1.0` at the user-selected game
FPS. FPS is never detected, inferred, or changed from runtime observations.

For a selected ratio and FPS, Python first materializes the requested hold:

```text
frame_us = ceil(1_000_000 / fps)
requested_min_hold_us = round(hold_frames * frame_us) + margin_us
effective_min_hold_us = max(requested_min_hold_us, frame_us + 500)
```

The default margin is `500 µs`; calibration may provide a validated margin.
The validated host-delivery calibration uses only paired Down/Up evidence from
the six required `1/5/15 × hot/cold` buckets, with at least 100 clean pairs in
each bucket. Its recommended margin is the global positive p99 hold shrink plus
`100 µs`, clamped to `300–2,000 µs`; cache v1 or incomplete/dirty evidence
falls back to the unchanged `500 µs` default.
The native worker receives only the materialized `effective_min_hold_us` and
uses it as a fixed duration. It does not learn or subtract SendInput cost.

At 60 FPS with the default margin:

| Hold | Requested | Effective |
|---:|---:|---:|
| 1.0 frame | 17,167 µs | 17,167 µs |
| 1.25 frames | 21,334 µs | 21,334 µs |
| 1.5 frames | 25,500 µs | 25,500 µs |

## Authored minimum-hold floor

For an authored Down at timestamp `A`, a same-key release cannot be committed
before:

```text
release_floor = A + effective_min_hold_us
effective_release = max(authored_release, release_floor)
```

The floor is checked in QPC ticks with checked arithmetic. It is an authored
sender-side contract and does not claim to measure when Sky or the game render
loop observed the input. The native worker records the actual Down completion
only as feasibility evidence; if a late completion makes the authored Up
impossible, the session fails closed and never moves that Up target.

## Feasibility and diagnostics

Authored validation rejects a same-key interval below the selected hold floor.
The native-boundary validator performs this check before the worker can send
anything, including exact same-timestamp retriggers and timestamp overflow
cases. Runtime completion is checked against the same authored floor. A
late-completion release that is still feasible may use the coordinator's
per-key recovery state, but it does not move the authored release or block an
unrelated Down chord. A same-key Down whose floor is infeasible fails closed
before transport.

A transport zero/partial result is terminal and is handled by fail-closed
cleanup; it is not retried in production. Strict timing evaluates completion
residuals, while normal playback preserves the schedule when transport
integrity remains valid.

Diagnostic mode may report sender-side start, completion, lateness, duration,
and release-floor evidence. Production retains only bounded scalar timing
counters. These values are not game-onset or audio-onset measurements. The
old estimator and adaptive dispatch lead are not part of this model; historical
lead fields are compatibility-only zeros.
