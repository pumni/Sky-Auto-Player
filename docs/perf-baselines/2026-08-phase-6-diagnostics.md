# Phase 6 diagnostics observer benchmark

Date: 2026-08-30

Code source for the benchmark: `95708c913b9761aaf122183c04e22c8d1827b54d`.
Environment: Windows 11, CPython `3.14.7`, uv `0.12.7`, Rust `1.98.0`.
CPU model capture was unavailable on this host.

## Method

The reproducible harness is `scripts/bench_phase6_diagnostics.py`. It exercises
the production `_SnapshotRenderer` → `DesktopDiagnosticsService` observer path
with the same synthetic native `ProgressCounters` and `BackendHealth` in both
modes. Each mode performs 2,000 snapshot calls per repeat over 9 repeats. The
synthetic UI clock advances by 101 ms per call, so the service's 10 Hz gate is
exercised without accidental floating-point coalescing.

This is test-backend observer qualification only: it starts no game, emits no
physical input, and does not move diagnostics into the native scheduler. The
unchanged Rust timing/admission regression suite remains the native timing
qualification. No arbitrary performance threshold was added from this result.

## Measured result

| Mode | Observer p50 (us/snapshot) | p95 (us/snapshot) | Max (us/snapshot) | Mean (us/snapshot) | Events emitted |
| --- | ---: | ---: | ---: | ---: | ---: |
| Diagnostics off | 8.97 | 12.41 | 12.41 | 9.36 | 0 |
| Diagnostics on | 26.05 | 29.08 | 29.08 | 26.48 | 18,000 |
| On minus off | +17.08 | +16.68 | +16.68 | +17.12 | — |

The measured observer overhead is approximately 17.1 microseconds per 101 ms
synthetic UI interval, or about 0.017% of a 100 ms diagnostics cadence. The
workload uses a test backend, so it does not claim physical/native timing
qualification. Late-event counters are not a meaningful comparison here
because no native scheduler is running; the same supplied counters are observed
in both modes.

## Native playback A/B qualification

The reproducible harness is `scripts/bench_phase6_native_diagnostics.py`. It
runs the same authored plan through the production Python
`PlaybackEngine`/`RustDispatchRuntime` path in both legs. The session factory
is replaced only at the native binding seam with Rust's
`sky_player_rs.TestDispatchSession`, which uses the real Rust
`NativeDispatchSession` scheduler and `BackendConfig::Mock`; it never calls
Windows `SendInput`. Diagnostics is disabled for the OFF leg and enabled plus
consumed from the existing renderer callback for the ON leg. Each leg ran the
same 16-note/32-action plan for 7 repeats. The native wheel was built from
closure head `4eb437a30692b99752b89781c3f323037578cd3a` with Rust `1.98.0`.
Plan fingerprint:
`65261cd52bfe6e0853f3444a3a4dad4607bfe0e3236d79c109ab6dc6ec2fb59c`.

Command:

```text
uv run python scripts/bench_phase6_native_diagnostics.py --notes 16 --repeats 7 \
  --output docs/perf-baselines/2026-08-phase-6-diagnostics-native-ab.json
```

The native timing statistics below are per-session summary values aggregated
across the 7 repeats. `dispatch_start_error_us` is the closest existing native
onset metric; native max lateness, completion lateness, and the native
late-event counters are also recorded. All runs finished. Each leg recorded one
2 ms late-event bucket entry across the seven sessions, with zero 5 ms/10 ms
entries and zero mock-backend drop/chord-split anomalies.

| Mode | Diagnostics samples consumed | Native max-lateness p50 (us) | Native max-lateness p95 (us) | Dispatch-start p50 (us) | Dispatch-start p95 (us) | Completion-lateness p95 (us) | Max native late counters (2/5/10 ms) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Diagnostics off | 0 | 1,660 | 2,152 | 2 | 5 | 0 | 1 / 0 / 0 |
| Diagnostics on | 49 | 1,637 | 2,161 | 2 | 5 | 0 | 1 / 0 / 0 |
| On minus off | +49 | -23 | +9 | 0 | 0 | 0 | 0 / 0 / 0 |

The ON leg did not increase the measured p50/p95 scheduler onset metrics, and
the native max-lateness p50 moved down by 23 microseconds while p95 moved up by
9 microseconds. Late-event counters were identical. This is qualification
evidence for the Rust scheduler plus deterministic mock backend, not a claim
about game-observed latency or physical SendInput behavior. No timing
threshold, deadline, lead, or compensation was changed based on this
measurement.

Raw result: `docs/perf-baselines/2026-08-phase-6-diagnostics-native-ab.json`.

## Verification relationship

Diagnostics is produced from the existing low-rate renderer snapshot callback,
is latest-wins/coalescible at the Core event boundary, and never feeds back into
note deadlines, waits, dispatch, or frontend acknowledgements. Existing Rust
timing and admission tests are run unchanged by the repository verification
matrix.
