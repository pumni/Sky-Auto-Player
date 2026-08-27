# Native Dispatch Refactor Performance Evidence (2026-08-28)

This report records the post-refactor evidence for the native supervisor and
physical dispatch timing changes. It is evidence for the host and revisions
listed here, not a universal performance guarantee.

## Environment and provenance

- Revision: `a37dea93fa932c8952b119395fa1a979f7e5148c`
- Host: Windows 11 build `10.0.26200`, AMD64 Family 23 Model 104 Stepping 1
- Rust: `rustc 1.98.0 (88d9e12ae 2026-08-18)`
- QPC frequency: `10,000,000 Hz`
- Benchmark: `rt_handoff_bench`, `real_wait`, full scope, deterministic mock transport
- Command: `cargo run --manifest-path rust/Cargo.toml -p sky_player_rs --example rt_handoff_bench --features test-support -- <output>`
- Run settings: `RT_HANDOFF_BENCH_ITERATIONS=20`, `RT_HANDOFF_BENCH_DUE_US=10000`

The run is diagnostic rather than statistically eligible because it uses 20
iterations. The complete JSON report is preserved outside the repository at
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-rt-handoff-a37dea9.json`.

## Startup calibration

The production policy uses `max(p99, robust) + 50 µs`, clamped to
`250..1000 µs`, and freezes the selected value for the session. The measured
startup wake-error statistics were:

| p50 | p95 | p99 | robust | selected threshold |
| ---: | ---: | ---: | ---: | ---: |
| 336 µs | 619 µs | 706 µs | 1,488 µs | 1,000 µs (cap) |

This host therefore selected the conservative cap. The benchmark confirms the
calibration is bounded; it does not justify lowering the production cap for
this host.

## Real-wait handoff matrix

Values below are the maximum `dispatch_start_error_us` p99/p99.9 across the
nine scenarios in each mode. Process CPU is the benchmark-reported process CPU
time for the mode. The benchmark does not expose separate worker CPU or spin
duty fields, so those are recorded as unavailable rather than inferred.

| Mode | Effective spin | Process CPU | Max p99 | Max p99.9 | Failed scenarios |
| --- | ---: | ---: | ---: | ---: | ---: |
| Production calibrated | 1,000 µs | 171,875 µs | 34 µs | 34 µs | 0 |
| Fixed 1,000 µs | 1,000 µs | 156,250 µs | 41 µs | 41 µs | 0 |
| Fixed 700 µs | 700 µs | 140,625 µs | 91 µs | 91 µs | 0 |
| Fixed 400 µs | 400 µs | 62,500 µs | 185 µs | 185 µs | 0 |
| Fixed 250 µs | 250 µs | 62,500 µs | 343 µs | 343 µs | 2 |
| Fixed 1,500 µs | 1,500 µs | 171,875 µs | 42 µs | 42 µs | 0 |

All production-calibrated scenarios recorded zero early dispatches and zero
deadline-missed-before-send events. The fixed 250 µs mode failed two scenarios
because the short final spin did not provide complete start-error samples; it
was not selected as the production floor.

## Native acceptance harness

The repository acceptance command was also run with the clean refactor wheel
at commit `1037063f474c5a3d3ee3c89e1e84a8438bcae4fe` using:

```text
uv run --env-file .env python scripts/bench_native_acceptance.py --repeats 2 --actions 128 --output native-phase5.json
```

The result was correctly marked invalid by the existing correctness gate, not
forced green: `pre_call_hold_shrink_over_grace_count=72` and
`production_completion_hold_below_frame_count=11`. Integrity, early-dispatch,
missed-boundary, transport, and cleanup counters remained zero. The untouched
baseline showed the same host/environment issue with counts `75` and `8`, so
no acceptance threshold was loosened. The baseline and failed-run artifacts
are preserved at
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-acceptance-artifacts-20260828`.

