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

For a selected ratio and FPS, the native planner first materializes the requested hold:

```text
frame_us = ceil(1_000_000 / fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
timing_margin_us = persisted_user_value
effective_min_hold_us = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
down_late_cutoff_us = 2000
```

The user-owned Timing Margin defaults to `500 µs`, ranges from `0` through
`3,000 µs` in `100 µs` steps, and applies equally to Hold and Release Gap.
The independent Late Down tolerance defaults to `2,000 µs`, ranges from `0`
through `5,000 µs` in `100 µs` steps, and is frozen per prepared session.
Calibration never supplies part of the authored timing equation. It may
produce an advisory recommendation from the selected Late Down tolerance plus
measured transport-reserve evidence; with the default cutoff and the
unqualified `300 µs` reserve, the fallback recommendation is `2,300 µs`.
Calibration is never shown as qualified when fallback is used.
Production calibration uses one pair metric per Down/Up SendInput
packet, based on `T_D/P_D/C_D` and `T_U/P_U/C_U`; Raw Input receipt timing is
not part of qualification. It uses exactly the six `1/5/15 × hot/cold`
buckets, at least 100 clean pairs per bucket, and at most 200 attempts per
bucket. Its transport-reserve candidate is the maximum positive
`sendinput_shrink_us.max` across required buckets plus a `100 µs` guard. A
candidate at or below `2,000 µs` qualifies and reserves at least `300 µs`; a
candidate above `2,000 µs` is out of the trusted envelope and uses the
unqualified transport reserve. A qualified recommendation is rounded up to
the next `100 µs` after adding the currently selected Late Down tolerance.
With the default `2,000 µs` cutoff and fallback `300 µs` reserve, that advisory
value is `2,300 µs`. It is informational only and never writes a setting.
Users adjust Timing Margin with its bounded stepper; a change affects only the
next prepared session.

The release gap is one base game frame plus the exact Timing Margin. It is an
authored schedule value, not a runtime delay or a guarantee that the game
sampled Up before the next Down.

Production hold forensics keeps these two contracts separate. Static schedule
validation still requires the authored target gap to be at least
`min_release_gap_us` (`frame_us + timing_margin_us`). The conservative
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
adapter does not add another frame-relative floor; Rust only range-checks and
validates these values in QPC ticks. It does not learn or subtract SendInput
cost. The independent user-owned `down_late_grace_us` sender cutoff defaults to
`2,000 µs` and is converted once to QPC ticks for each prepared session. It
never participates in the authored policy,
which enforces:

```text
effective_min_hold_us = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

The two defaults are intentionally decoupled: the `500 µs` authored margin
is smaller than the `2,000 µs` sender cutoff. An authorized late Down can
therefore reduce sender-observed hold below the selected frame base even though
the authored schedule remains valid. This is an explicit user-owned timing
trade-off and remains observable in sender forensics. Zero is also a valid
margin choice. The cutoff is never added to an authored target or adapted
during playback. Equality at the cutoff is
allowed; the first QPC tick beyond it is a missed Down. Up-only releases
remain exempt.

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
