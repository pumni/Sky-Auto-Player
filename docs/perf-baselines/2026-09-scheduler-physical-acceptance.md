# Scheduler physical SendInput acceptance — 2026-09-12

Code source: `d02eb7ddf1c4ba2ab9374d26b9874a91e8b6ddf7`.

## Scope and method

This is physical Windows input-path evidence for the production dispatch
profile. The feature-gated acceptance runner used real `SendInput` delivery to
the project-owned, receive-only WinForms sink and an inert focus probe. It did
not target Sky or another game, and changed no game files. Each target was
bound by schema-v3 ready evidence to the exact `pwsh.exe` process, HWND, start
time, and event-log identity.

Environment: Windows 11 Home build 26200, AMD Ryzen 5 5500U (12 logical
processors), PowerShell 7.6.6, Rust 1.98.1. The same host, build, sink
implementation, and 13-scenario workload were used at each grace value. Each
grace used a fresh sink/probe pair; focus-loss ran last so foreground
activation could not affect subsequent scenarios. The runs used
`DispatchProfile::Production`, `BackendConfig::Production`, and did not enable
`StrictTimingDiagnostic`.

Scenarios per grace: `canonical-single`, `canonical-chord`,
`canonical-max-chord`, `hold`, `rapid-retrigger`, `mixed-up-down`,
`focus-loss`, `target-hwnd-change`, `pause-resume`, `stop-cleanup`,
`skip-cleanup`, `cleanup-full-release`, and `w4-noncanonical`.

Raw per-run reports are in [acceptance.jsonl](2026-09-scheduler-physical-acceptance-data/acceptance.jsonl).
Each grace produced 13/13 `PASS` reports. The 120 sink events recorded for
each grace match the sum of the per-run report counts; each inert focus probe
recorded zero keyboard events. Ready records bind each event stream to the
exact observer process and HWND.

- 500 µs: [sink events](2026-09-scheduler-physical-acceptance-data/500/sink-events.jsonl), [sink identity](2026-09-scheduler-physical-acceptance-data/500/sink.json), [focus-probe events](2026-09-scheduler-physical-acceptance-data/500/probe-events.jsonl).
- 750 µs: [sink events](2026-09-scheduler-physical-acceptance-data/750/sink-events.jsonl), [sink identity](2026-09-scheduler-physical-acceptance-data/750/sink.json), [focus-probe events](2026-09-scheduler-physical-acceptance-data/750/probe-events.jsonl).
- 1000 µs: [sink events](2026-09-scheduler-physical-acceptance-data/1000/sink-events.jsonl), [sink identity](2026-09-scheduler-physical-acceptance-data/1000/sink.json), [focus-probe events](2026-09-scheduler-physical-acceptance-data/1000/probe-events.jsonl).

## Results

| Down grace | Physical scenarios | Largest cumulative session-max pre-call lateness | Hard-late / missed Down boundaries / missed Down keys / cutoff / backlog misses | Hold / release-gap samples | Hold-below-frame / release-gap-below-policy / same-call-retrigger / forensics anomalies | Partial / zero-progress / stuck keys |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 500 µs | 13/13 PASS | 77 µs | 0 / 0 / 0 / 0 / 0 | 28 / 2 | 0 / 0 / 0 / 0 | 0 / 0 / 0 |
| 750 µs | 13/13 PASS | 86 µs | 0 / 0 / 0 / 0 / 0 | 28 / 2 | 0 / 0 / 0 / 0 | 0 / 0 / 0 |
| 1000 µs | 13/13 PASS | 35 µs | 0 / 0 / 0 / 0 / 0 | 28 / 2 | 0 / 0 / 0 / 0 | 0 / 0 / 0 |

All 28 pre-call observations in each batch landed in the `<250 µs` bucket;
the remaining six buckets were zero. `keys_dropped`, chord splits, anchor
overwrites, and unmatched Ups were also zero. Full per-session minimum
hold/release ticks and other Production forensics remain in the raw JSONL
reports. The maximum column is the largest cumulative session-max value among
the 13 session reports for that grace; it is not a recent-sample trend.

## Decision and limits

No run observed a hard-late Down, cutoff, backlog miss, hold/release-gap
regression, retrigger anomaly, partial or zero-progress `SendInput`, or stuck
key. Increasing grace to 750 or 1000 µs rescued no additional note in this
workload. Keep the shipped production default at **500 µs**; this evidence does
not justify changing it.

This is a controlled receive-only sink qualification, not game-observed
latency or a host-contention stress test. The separate elevated-target UIPI
mismatch case was not exercised; these runs qualify physical delivery to the
controlled current-user sink and the listed scheduler/control scenarios.
