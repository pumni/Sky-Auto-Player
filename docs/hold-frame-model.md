# Hold-Frame Timing Model

Sky Auto Player uses two independent, user-selected timing values: the game FPS configured in
Sky and one of the supported hold selections `1.0`, `1.25`, or `1.5` frames. The default is
`1.0` frame at `60` FPS. FPS is never detected, inferred, clamped from runtime observations,
or changed automatically.

For a selected ratio and FPS:

```text
frame_us = ceil(1_000_000 / fps)
base_hold_us = round(hold_frames * frame_us)
effective_hold_us = base_hold_us + min_hold_margin_us
hold_us = min_hold_us = effective_hold_us
```

The default device-delivery margin is `500 µs`; calibration may provide a measured margin and
deterministic diagnostics may use zero. The materialized hold must always be at least one frame.

At 60 FPS with the default margin:

| Hold | Base | Effective |
|---:|---:|---:|
| 1.0 frame | 16,667 µs | 17,167 µs |
| 1.25 frames | 20,834 µs | 21,334 µs |
| 1.5 frames | 25,000 µs | 25,500 µs |

The Python/native boundary continues to pass `game_fps` and the effective `min_hold_us`.
Completion anchoring, adaptive lead, wait strategy, focus handling, native scheduling, and the
SendInput-only security boundary are unchanged.
