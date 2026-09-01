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

For a physical boundary, Python/native preparation materializes and validates
one immutable packet before the direct target wait. The worker's single
interruptible hybrid wait and bounded QPC spin cross the absolute authored
target, then run the final
command/control, target, and focus gates, repeats the program-owned atomic
checks, and evaluates the lease. The worker records `final_policy_qpc` for
lease admission. The prepared sender then takes the true `pre_call_qpc`
immediately before the cutoff and one `SendInput` call.
Up entries precede Down entries;
an overlapping Up/Down mask is rejected during preparation. A partial Up is
reported with partial-progress evidence but is never silently retried by this
single-send primitive.

For a selected ratio and FPS, Python first materializes the requested hold:

```text
frame_us = ceil(1_000_000 / fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
down_late_grace_us = policy.down_late_grace_us
transport_margin_us = max(0, calibrated_or_default_transport_margin_us)
effective_min_hold_us = (
    frame_base_hold_us + down_late_grace_us + transport_margin_us
)
sender_headroom_us = down_late_grace_us + transport_margin_us
min_release_gap_us = frame_us + sender_headroom_us
```

The independent Down late-discovery grace is `500 µs`. The default and every
fallback transport margin is `300 µs`; a valid calibration may replace only
that transport component. Thus the default effective additive margin is
`800 µs`, and calibration is never reported as qualified when fallback is
used. Production calibration uses one pair metric per Down/Up SendInput
packet, based on `T_D/P_D/C_D` and `T_U/P_U/C_U`; Raw Input receipt timing is
not part of qualification. It uses exactly the six `1/5/15 × hot/cold`
buckets, at least 100 clean pairs per bucket, and at most 200 attempts per
bucket. Its transport candidate is the maximum positive
`sendinput_shrink_us.max` across required buckets plus a `100 µs` guard. A
candidate at or below `2,000 µs` is valid and applies at least the `300 µs`
floor; a candidate above `2,000 µs` is out of the trusted correction envelope,
keeps the evidence unhealthy, and falls back to the `300 µs` transport floor.
Protocol 9/cache v5/v6/v7 evidence is incompatible with protocol 10/cache v8
and falls back to the explicit transport floor.

The release gap reserves the same static sender headroom as the hold floor:
one base game frame plus `down_late_grace_us + transport_margin_us`. It is an
authored schedule value, not a runtime delay or a guarantee that the game
sampled Up before the next Down.

Production hold forensics keeps these two contracts separate. Static schedule
validation still requires the authored target gap to be at least
`min_release_gap_us` (`frame_us + sender_headroom_us`). The conservative
sender observation `next_down_pre_call - previous_up_completion` is instead
compared with the base frame visibility floor `frame_us`: transport completion
may consume the sender headroom that was intentionally reserved by the
authored policy. The forensics block reports that consumed headroom separately
and retains a hard violation only when the observed interval falls below the
base frame floor or has negative timestamp ordering. This is sender evidence,
not proof that Sky sampled, rendered, or produced audio for the transition.

The sender evidence is computed in raw QPC ticks before conversion:

```text
sender_hold_shrink = (T_U - T_D) - (C_U - C_D)
sender_hold_shrink = ((P_D - T_D) - (P_U - T_U))
                    + ((C_D - P_D) - (C_U - P_U))
```

The identity is checked with checked arithmetic. It describes completion
interval compression in the Rust/SendInput sender only; it is not a claim
about game-observed timing.
The native worker receives the materialized `effective_min_hold_us` and
`min_release_gap_us` values and uses them as fixed durations. The native desktop
adapter does not add
another frame-relative floor; Rust only range-checks and validates these values
in QPC ticks. It does not learn or subtract SendInput cost.
`FrameTimingPolicy.min_hold_margin_us` is the
compatibility aggregate of the fixed Down grace and transport margin; the
explicit policy fields retain the frame-base, grace, transport, and release-gap
components. `min_hold_margin_source` records transport provenance.
The independent fixed `down_late_grace_us` sender policy is `500 µs` and is
converted once to QPC ticks. The policy coupling enforces:

```text
effective_min_hold_us = frame_base_hold_us + down_late_grace_us + transport_margin_us
```

Therefore an authorized Down accepted at the latest cutoff cannot reduce the
sender pre-call hold below the selected frame-base hold. Grace is never added
to an authored target or adapted during playback. Equality at the cutoff is allowed;
the first QPC tick beyond it is a missed Down. Up-only releases remain exempt.

At 60 FPS with the default margin:

| Hold | Requested | Effective hold | Release gap |
|---:|---:|---:|---:|
| 1.0 frame | 16,667 µs | 17,467 µs | 17,467 µs |
| 1.25 frames | 20,834 µs | 21,634 µs | 17,467 µs |
| 1.5 frames | 25,001 µs | 25,801 µs | 17,467 µs |

At 60 FPS with the default 1.0-frame selection, the exact same-key minimum
cycle is `17,467 + 17,467 = 34,934 µs` (about 28.63 repeated presses per
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
boundaries are valid;
same-timestamp same-key overlaps are rejected, while disjoint masks may still
coalesce. Runtime never delays, retries, or rewrites these authored targets.
Completion is evidence for sender-side telemetry and ownership accounting
only; it does not create a completion-relative hold floor or a new deadline.
Runtime deadline/overdue policy handles a late boundary without rewriting
authored timestamps or emitting a catch-up send.

A transport zero/partial result is terminal and is handled by fail-closed
cleanup; it is not retried in production. Strict timing evaluates completion
residuals, while normal playback preserves the schedule when transport
integrity remains valid.

Diagnostic mode may report sender-side start, completion, lateness, duration,
and release-floor evidence. Production retains only bounded worker-local
scalars and a fixed anomaly ring: hold-pair count/minima, pre-call and
completion shrink maxima, below-frame count, release-gap minima/violations,
headroom-consumption count/maximum, same-call retriggers, anchor overwrites,
unmatched Ups, and ring overwrites. Structural anomalies and timing
diagnostics have separate counters; the generic ring total is observability
only and is not the sole qualification predicate.
The production forensics block exposes an availability/version marker and
never allocates, locks, samples QPC, or consults the diagnostic observer.
These values are not game-onset or audio-onset measurements. The old estimator
and adaptive dispatch lead are not part of this model; historical lead fields
are compatibility-only zeros.
