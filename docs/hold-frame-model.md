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
The native worker receives only the materialized `effective_min_hold_us` and
uses it as a fixed duration. It does not learn or subtract SendInput cost.

At 60 FPS with the default margin:

| Hold | Requested | Effective |
|---:|---:|---:|
| 1.0 frame | 17,167 µs | 17,167 µs |
| 1.25 frames | 21,334 µs | 21,334 µs |
| 1.5 frames | 25,500 µs | 25,500 µs |

## Completion-anchored release floor

For a successful Down packet, `C` is the QPC completion boundary returned by
the single `SendInput` transport call. A same-key release cannot be committed
before:

```text
release_floor = C + effective_min_hold_us
effective_release = max(authored_release, release_floor, retry_not_before)
```

The floor is checked in QPC ticks with checked arithmetic. It is a sender-side
visibility safeguard and does not claim to measure when Sky or the game
render loop observed the input. The completion anchor prevents the sender's
own call duration from shortening the configured hold.

## Feasibility and diagnostics

Authored validation rejects a same-key interval below the selected hold floor.
Runtime recovery may defer a release after a slow Down or retry backoff; that
defer is represented in diagnostics and does not move unrelated future
authored actions. Strict timing evaluates completion residuals, while normal
playback preserves the schedule when transport integrity remains valid.

The native observer reports sender-side start, completion, lateness, duration,
and release-floor evidence. These values are not game-onset or audio-onset
measurements. The old estimator and adaptive dispatch lead are not part of
this model; historical lead fields are compatibility-only zeros.
