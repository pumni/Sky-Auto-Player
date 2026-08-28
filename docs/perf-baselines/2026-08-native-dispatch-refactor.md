# Native Dispatch Refactor Performance Evidence (2026-08-28)

This report records post-refactor evidence for the native supervisor and
physical dispatch timing changes. It is evidence for the host, build, and
scopes listed here, not a universal performance guarantee or evidence that
Sky itself received or rendered every note.

## Environment and provenance

- Latest repository revision at documentation time: `9d2496dbb655de33ff96aabe4e0e49254bfc0d8d`
- Phase-A production-path benchmark revision: `1f33ea0efe79397b77605efd9796ff5c9e7a95c6`
- Native acceptance wheel and harness revision: `1cfb1f0efe79397b77605efd9796ff5c9e7a95c6`
- Host: Windows 11 build `10.0.26200`, AMD64 Family 23 Model 104 Stepping 1
- Rust: `rustc 1.98.0 (88d9e12ae 2026-08-18)`
- QPC frequency: `10,000,000 Hz`
- Benchmark transport: deterministic test-support mock transport; real
  `SendInput` and game receipt are not measured

The evidence below separates three scopes:

1. `real_wait` is a short timing diagnostic using the actual wait path, not a
   statistically eligible qualification run.
2. `phase_a_production_boundary` is the 10,000-iteration coordinator/admission/
   commit qualification path with deterministic direct boundary crossing; it
   excludes waiter scheduling and the real Windows input call.
3. The native acceptance harness exercises the test-support dispatch session
   and its sender-side correctness invariants. It does not observe Sky's input
   polling, rendering, or audio.

## Startup calibration

Production calibration uses six wake samples only when the absolute probe
deadline leaves both the full 20 ms calibration budget and the 2 ms startup
readiness reserve. Each sample is checked against that deadline. The selected
threshold is `max(p99, robust) + 50 µs`, clamped to `250..1000 µs`, and frozen
for the session.

The 10,000-iteration phase-A run recorded:

| p50 | p95 | p99 | robust | selected threshold |
| ---: | ---: | ---: | ---: | ---: |
| 614 µs | 776 µs | 776 µs | 1,249 µs | 1,000 µs (cap) |

The separate 20-iteration real-wait diagnostic recorded p50 `671 µs`, p95
`736 µs`, p99 `736 µs`, robust `866 µs`, and selected `916 µs`; that diagnostic
is not statistically eligible.

## Real-wait handoff diagnostic

Values below are the maximum `dispatch_start_error_us` p99/p99.9 across the
nine scenarios in each mode. Process CPU is the benchmark-reported process CPU
time for the mode. This run used 20 iterations, so its p99/p99.9 and CPU values
are diagnostic only (`statistics_eligible=false`).

| Mode | Effective spin | Process CPU | Max p99 | Max p99.9 | Failed scenarios |
| --- | ---: | ---: | ---: | ---: | ---: |
| Production calibrated | 916 µs | 62,500 µs | 106 µs | 106 µs | 0 |
| Fixed 1,000 µs | 1,000 µs | 187,500 µs | 40 µs | 40 µs | 0 |
| Fixed 700 µs | 700 µs | 140,625 µs | 233 µs | 233 µs | 0 |
| Fixed 400 µs | 400 µs | 62,500 µs | 302 µs | 302 µs | 0 |
| Fixed 250 µs | 250 µs | 62,500 µs | 445 µs | 445 µs | 2 |
| Fixed 1,500 µs | 1,500 µs | 250,000 µs | 40 µs | 40 µs | 0 |

The calibrated value happened to be below the 1,000 µs cap in this diagnostic,
but this short run cannot establish a production CPU or tail-latency claim.

## Ten-thousand-iteration phase-A qualification

The qualification run used:

```text
$env:RT_HANDOFF_BENCH_ITERATIONS='10000'
$env:RT_HANDOFF_BENCH_SCOPE='phase_a_production_matrix'
$env:RT_HANDOFF_BENCH_MODE='phase_a_production_boundary'
cargo run --manifest-path rust/Cargo.toml -p sky_player_rs --example rt_handoff_bench --features test-support -- <output>
```

The report is preserved at
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-rt-handoff-1f33ea0-10k.json`.
All nine scenarios completed 10,000 iterations, for 90,000 matrix
observations. `acceptance_clean=true` and `statistics_eligible=true`; across
the matrix the maximum p99/p99.9 dispatch-start error was `117/143 µs`, with
zero early dispatches, deadline misses, or non-dispatches. The production
calibration metadata reports six samples and selected the 1,000 µs cap.

This phase is useful evidence for the frozen-plan and admission/commit path,
but it uses deterministic direct QPC boundary crossing. Its process-CPU result
must not be interpreted as production waiter CPU evidence.

## Native acceptance harness (historical snapshot)

The earlier two-repeat hot diagnostic was invalid because the test-support
session defaulted to `down_late_grace_us=0` while the production materialized
policy uses `500 µs`; it reported 72 pre-call hold-shrink violations. The
acceptance harness now passes and fingerprints the production `500 µs` grace
explicitly. The invariant remains strict: any counter is still a failure.

The qualification command was:

```text
uv run --env-file .env python scripts/bench_native_acceptance.py --dispatch-repeats 40 --actions 128 --polyphony 1 --scenario paired --gap-profile cold --start-delay-us 100000 --skip-command-samples --budget-seconds 600 --label native-qualification-cold-1cfb1f0 --output <output>
```

The report is preserved at
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-native-acceptance-1cfb1f0-cold-10k.json`.
It completed 40 suites with 10,240 physical boundaries and
`statistics_eligible=true`, `acceptance_clean=true`. The cold-gap profile
materializes the normal 30 ms hold policy while retaining the default mock
transport latency; it is a sender-side acceptance workload, not a claim about
the real game's frame timing. All correctness counters were zero, including
pre-call shrink-over-grace, production completion below frame, missed or early
dispatch, dropped keys, transport failures, and cleanup failures.

For transparency, the default hot 60 FPS diagnostic after the policy fix still
reported `production_completion_hold_below_frame_count=4` and was therefore
invalid under the then-current strict completion gate; its pre-call
shrink-over-grace count was zero. The follow-up evidence reclassifies this
post-`SendInput` completion width as transport diagnostics only, while keeping
target/pre-call, release-gap, transport, and cleanup checks hard. No invariant
or threshold was weakened to obtain the clean cold-gap qualification result.
The detailed follow-up report is
[`2026-08-native-dispatch-followup.md`](2026-08-native-dispatch-followup.md).
The older baseline and invalid-run artifacts remain at
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-acceptance-artifacts-20260828`.
