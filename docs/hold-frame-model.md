# Hold-Frame Timing Model

Sky Auto Player accepts exactly `1.0`, `1.25`, and `1.5` frame holds. FPS is
selected by the user and is never inferred from runtime observations. The
model separates authored musical spacing from physical completion evidence.

## Authored policy

For every session the native application materializes:

```text
frame_us           = ceil(1_000_000 / fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
timing_margin_us   = persisted user value
min_hold_us        = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

Each musical generation has exactly one authored Down and one matching
authored Up. The compiler and the independent validator reject an open Down at
the end of the schedule, overlapping same-key Down, same-key matched Up+Down
at one timestamp, and hold or release intervals below the materialized floors.
Stale unmatched Up metadata remains non-musical. Safety and cleanup releases
do not belong to this ledger.

Timing Margin is applied once to authored hold and release spacing. It is
authored headroom and is not a runtime sender cutoff or a second physical
floor. Authored timestamps remain immutable for the whole session.

At 60 FPS with a 500 µs margin:

| Hold | Base hold | Authored minimum hold | Release gap |
| ---: | ---: | ---: | ---: |
| 1.0 frame | 16,667 µs | 17,167 µs | 17,167 µs |
| 1.25 frames | 20,834 µs | 21,334 µs | 17,167 µs |
| 1.5 frames | 25,001 µs | 25,501 µs | 17,167 µs |

## Causal authorization

A prepared Down-bearing boundary is eligible only after the exact boundary,
packet identity, and target generation were observed while its authored target
was strictly in the future. Lateness after that observation does not revoke
authorization. Lifecycle and identity changes still clear it.

An overdue Down without the proof is `UnobservedBacklog` and performs zero Down
`SendInput` attempts. Unseen overdue boundaries are dropped; the next future
boundary is authorized normally. This preserves no catch-up burst behavior.

## Physical floors

`PhysicalTimingGuard` stores fixed-size per-key floors from trustworthy complete
sender completions:

```text
musical_up_not_before = successful Down completion + frame_base_hold
down_not_before       = successful Up completion + frame_us
packet_not_before     = max(authored target, relevant floors)
```

The guard contains no Timing Margin, latest-start boundary, or physical
feasibility rejection. Normal and strict/diagnostic successful musical sends
use the same completion update. A failed or ambiguous transport result does
not synthesize a floor and follows fail-closed cleanup.

The resulting cases are:

1. A Down completing 300 µs after its target has an Up floor at target plus
   `frame_base_hold`; an authored Up at target plus the authored 17,167 µs
   remains unchanged.
2. A Down completing 800 µs late delays that Up only by the excess required
   by `frame_base_hold`.
3. An authorized Down remains one send attempt even after large scheduler
   lateness; its matching Up waits for the actual Down completion floor.
4. A successful Up completion delays the next same-key Down until completion
   plus one frame.
5. A mixed packet waits for the maximum Up and Down floor and sends one atomic
   Up-before-Down packet.
6. Unrelated authored targets never move because an earlier transition was
   late.
7. Focus, pause, suspend, target, and lifecycle reset paths clear or
   invalidate stale completion floors.

Emergency, focus-loss, and cleanup Ups retain immediate safety behavior and do
not wait on musical floors. Chords are never split and transport anomalies are
never retried.

## Sender evidence and diagnostics

The sender records authored target, physical floors, pre-call QPC, completion
QPC, transport masks, and final-gate evidence. These timestamps describe the
sender boundary; they do not measure game polling, rendering, audio onset, or
receipt by Sky.

The compatibility counters
`final_sender_window_expirations`, `prepared_normal_sender_expirations`,
`missed_physical_window_boundaries`, and
`release_floor_infeasible_boundaries` may remain in public snapshots. They are
deprecated, stay zero in production, and have no dispatch meaning. The retired
latest-start compatibility field is zero or unavailable. Active Down lateness
has one control classification: causal `UnobservedBacklog`.

The physical path uses one prepared `SendInput` packet. It validates masks,
places Up entries before Down entries, performs final control/focus/target
checks, and makes exactly one transport attempt. Partial or uncertain Down
transport is terminal and invokes existing fail-closed cleanup.
