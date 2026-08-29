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

## Verification relationship

Diagnostics is produced from the existing low-rate renderer snapshot callback,
is latest-wins/coalescible at the Core event boundary, and never feeds back into
note deadlines, waits, dispatch, or frontend acknowledgements. Existing Rust
timing and admission tests are run unchanged by the repository verification
matrix.
