# User-owned Timing Margin physical acceptance — 2026-09-12

Raw evidence for run `timing-margin-20260912T154546-6d40ebef` is published in
the [run artifact directory](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/).
The files listed in `checksums.sha256` were copied byte-for-byte from the
original `.benchmarks` run directory; the local originals were retained. The
raw `summary.json` and `run-config.json` therefore still contain their original
machine-local absolute paths. Use the links in this document for the durable
GitHub location.

This is evidence submitted for independent review, not a physical-acceptance
sign-off. PR [#233](https://github.com/pumni/Sky-Auto-Player/pull/233) remains
draft. The reviewed source head is
`3d069b3603c162793d642ae1fde4756639f66e35`; its exact-head CI run is
[34683625571](https://github.com/pumni/Sky-Auto-Player/actions/runs/34683625571).
The [publication manifest](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/publication-manifest.json)
records the source revision, runner identities, and acceptance-harness SHA-256.

## Run and method

The feature-gated native acceptance harness exercised the production `SendInput`
path against the project-owned `ReceiveOnly` sink and an inert focus probe. It
did not target Sky or another game and did not modify game files. The runner
used schema-v3 ready records to bind both observers to this run, their process
IDs, HWNDs, and event-log identities. The raw ready records, runner, invocations,
harness output, and JSONL reports are included in the artifact directory.

The run configuration was `hold_frames = 1.0`, default margin `800 µs`, and
fixed Down late grace `500 µs`. The 13-scenario regression ran at the default
margin, followed by the five-value margin sweep; focus-loss ran last. The sweep
uses the same production runner and acceptance path as the regression.
This run bundle does not record host OS build, CPU, or toolchain versions, so
this report makes no claim about those environment details.

## Results

All 18 invocations and reports have harness status `PASS`: one report for each
of the 13 regression scenarios and five `timing-margin-sweep` reports. The
regression scenarios were `canonical-single`, `canonical-chord`,
`canonical-max-chord`, `hold`, `rapid-retrigger`, `mixed-up-down`,
`target-hwnd-change`, `pause-resume`, `stop-cleanup`, `skip-cleanup`,
`cleanup-full-release`, `w4-noncanonical`, and `focus-loss`.

The report's authored packet timestamps were checked as Hold / Release Gap /
Hold deltas. Each delta equals the corresponding report policy value:

| Timing Margin | Down late grace | Authored Hold / Release Gap / Hold |
| ---: | ---: | ---: |
| 0 µs | 500 µs | 16,667 / 16,667 / 16,667 µs |
| 500 µs | 500 µs | 17,167 / 17,167 / 17,167 µs |
| 800 µs | 500 µs | 17,467 / 17,467 / 17,467 µs |
| 1,000 µs | 500 µs | 17,667 / 17,667 / 17,667 µs |
| 3,000 µs | 500 µs | 19,667 / 19,667 / 19,667 µs |

For all five sweep rows, the computed deltas from
`details.authored_packet_targets[].scheduled_us` equal `min_hold_us` and
`min_release_gap_us`. In particular, margin `0` authors `16,667 µs`; the raw
packet targets contain no hidden `+500` or `+300 µs`. All 18 reports carry
`down_late_grace_us = 500`.

The event evidence is bound consistently: the sink ready record and all 140
sink event records share the run ID, `ReceiveOnly` role, and event-log ID. The
stream header is sequence 0; event sequences are continuous from 1 through
140, with 55 `key_press` and 85 `key_release` records. The sum of per-report
sink event counts is also 140. The focus-probe event file contains only its
stream header, and both the summary and per-report counters show zero probe
keyboard events.

Across the reports, terminal active, possibly-active, and stuck-key counts are
zero; failed releases, partial `SendInput` events, and zero-progress
`SendInput` failures are also zero.

## Forensics requiring reviewer attention

The harness status does not mean every production forensics counter is zero.
The sweep reports record:

| Timing Margin | Completion Hold below frame | Observed Release Gap below base frame | Forensics anomalies |
| ---: | ---: | ---: | ---: |
| 0 µs | 1 | 1 | 1 |
| 500 µs | 0 | 1 | 1 |
| 800 µs | 0 | 0 | 0 |
| 1,000 µs | 0 | 0 | 0 |
| 3,000 µs | 0 | 0 | 0 |

The `0 µs` and `500 µs` reports therefore need review despite their harness
`PASS` status. In the raw forensics, the measured completion-to-next-Down
pre-call intervals are below the base-frame floor for those samples. The
production forensics implementation defines this comparison against the base
frame visibility floor and notes that transport may consume authored sender
headroom. These counters do not change the authored packet delta results above,
but they are material physical-path evidence and are not being represented as
clean. The raw report includes the tick values and all supporting fields.

This controlled receive-only sink run does not measure game-observed latency
or host-contention behavior. No physical rerun or artifact rewriting was done
for publication.

## Raw artifacts

- [Raw harness reports](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/rt-native-acceptance.jsonl)
- [Sink events](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/sink-events.jsonl), [sink ready identity](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/sink-ready.json)
- [Focus-probe events](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/focus-probe-events.jsonl), [probe ready identity](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/focus-probe-ready.json)
- [Run configuration](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/run-config.json), [runner](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/runner.ps1), [invocations](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/invocations.jsonl)
- [Harness output](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/harness-output.log), [run summary](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/summary.json)
- [SHA-256 checksums](2026-09-user-owned-timing-margin-physical-acceptance-data/timing-margin-20260912T154546-6d40ebef/checksums.sha256)
