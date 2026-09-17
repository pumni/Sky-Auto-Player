Closes #301

# Phase 7 final qualification evidence

Base: `9f54a3e7d990b5f05656d083cefc452607782ffe`

Qualification implementation head: `2f14b03784eca8b6f26fa4c474c0d9e88e1b6d24`.
The final PR head also contains this evidence document; its exact SHA is in the
PR description.

## Environment

- Windows 11 Home `10.0.26200` build `26200`, Dell Inspiron 5515, AMD Ryzen 5
  5500U, 12 logical processors, Balanced power plan.
- Rust `1.98.1`: `rustc 1.98.1 (48a229cea 2026-09-01)`, Cargo
  `1.98.1 (797e8a9bc 2026-08-05)`, `stable-x86_64-pc-windows-msvc`.
- QPC frequency: 10,000,000 Hz.
- Before editing, `git fetch origin` confirmed
  `origin/main == 9f54a3e7d990b5f05656d083cefc452607782ffe`.

## Final contract and cleanup

- Normal playback remains immutable prepared-frame dispatch:
  authored absolute target -> `HybridWaiter`/calibrated spin -> cheap final
  atomics -> final-policy QPC -> authoritative pre-call QPC -> one prepared
  `SendInput` transaction -> completion QPC -> bounded post-send commit.
- Normal completion-relative hold/release samples, minimum intervals, and
  floor-violation counts are raw physical forensics. They remain serialized and
  visible even when nonzero; they do not veto a normal shipping verdict.
- Strict/diagnostic `PhysicalTimingGuard` remains authoritative for its
  physical latest-start/floor rejection. Existing strict tests still reject a
  Down that misses that intentional bound.
- Normal late-note tolerance, worker tolerance ticks, UI/DTO/bridge surfaces,
  and late-rescue counters remain retired. Legacy
  `default_normal_down_start_tolerance_us` is accepted safely, discarded
  during canonical migration, and never mapped to Timing Margin.
- Same-key overlap remains pre-session rejection/corruption evidence; legal
  positive-gap retriggers and disjoint physical packets remain unchanged.
- No MMCSS, affinity, priority, spin, timer, power, adaptive-lead, focus
  cadence, watchdog cadence, or scheduler policy was changed.

## Gate 1: normal versus strict physical forensics

`production_visibility_qualification()` now treats normal completion-relative
floor values as evidence only. Focused tests prove that either nonzero normal
counter still returns `PASS`, the strict forensics contract returns `FAIL`, and
missing/wrong sink events plus transport anomalies remain fail-closed. The
shipping strict path is additionally covered by
`c1_strict_cutoff_remains_physical_latest_start` and
`strict_timing_retains_physical_latest_start_rejection`.

Raw fields retained in every native report include sample counts, minimum
observed completion-relative intervals, hold/release violation counts, target
and wake QPCs, final-policy QPC, pre-call QPC, completion QPC, wake-to-send,
`SendInput` duration, transport status, cleanup/release state, and focus,
power, target, and lease evidence.

## Deterministic prepared acceptance

Raw report: `.benchmarks/phase7-final-head-confirmation-final.json`.

| Result | Value |
| --- | ---: |
| Prepared physical boundaries | 16 |
| Successful sends | 14 |
| Intentional non-send/drop boundaries | 2 |
| Timeline rebases | 0 |
| Normal `DownExpiredBeforeSend` | 0 |
| Normal `UnobservedBacklog` | 0 |
| Completion-feedback `PhysicalWindowExpired` | 0 |
| Real-wait non-dispatches | 0 |
| Real-wait overdue classifications | 0 |
| Real-wait transport anomalies | 0 |

The two non-sends are the control-stop race and injected `ZeroProgress`.
Semantic expectations pass for every deterministic scenario. Completion
control/delayed evidence keeps different packet-N completion latency while
packet-N+1 authored/physical target, packet-not-before, wait target,
eligibility, and classification remain identical. Legal same-key order remains
`Down(K) -> Up(K) -> Down(K) -> Up(K)`; impossible geometry fails before worker
execution. ZeroProgress, partial, and integrity faults remain fail-closed.

## Fresh Windows production-session matrix

All rows below are fresh release-build runs at `TimingMarginUs=500` from
qualification implementation head `2f14b03784eca8b6f26fa4c474c0d9e88e1b6d24`.
Every report has `source_tree_clean=true`, expected and observed sink counts
matching, zero chord splits, zero missed Downs, zero unintended rebases, zero
transport anomalies, active/possibly-active/stuck state clean, and `PASS`.
Hold/release values are diagnostic counts, not normal verdict gates. The
suspend row's one structural/forensics observation is the expected system-power
transition evidence; it is not a transport anomaly.

| Scenario | Prepared/authored | Sink expected/observed | Hold / release forensics | Structural | Evidence |
| --- | ---: | ---: | ---: | ---: | --- |
| long single/repeated | 48 | 48 / 48 | 4 / 7 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144005-20cce6c4` |
| dense alternating + CPU contention | 32 | 32 / 32 | 2 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144010-f4b4b05f` |
| chord sweep 2..15 keys | 238 | 238 / 238 | 114 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144013-4f2fe1a6` |
| legal near-minimum retrigger | 8 | 8 / 8 | 1 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144018-24883126` |
| mixed different-key Up/Down | 4 | 4 / 4 | 0 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144021-bb76026d` |
| focus loss/restoration | 4 | 19 / 19 | 0 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144024-c4e3a366` |
| manual pause/resume | 4 | 19 / 19 | 0 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144032-13801dfb` |
| system suspend/resume | 4 | 19 / 19 | 0 / 0 | 1 expected power diagnostic | `.benchmarks/physical-native-cases/native-case-20260917T144036-2ab41e99` |
| quit cleanup | 2 | 2 / 2 | 0 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144040-0096628e` |
| skip cleanup | 2 | 2 / 2 | 0 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144042-c1428da4` |
| supervisor lease expiry | 0 gameplay | 15 cleanup Up / 15 | 0 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144045-db7a7607` |
| timing-margin sweep | 4 | 4 / 4 | 0 / 0 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144051-c4430a85` |
| release-gap stress | 1026 | 1026 / 1026 | 31 / 40 | 0 | `.benchmarks/physical-native-cases/native-case-20260917T144054-b6fbc9f0` |

The contention setup used two hidden `pwsh.exe` busy workers; the playback
session and waiter remained the production path. Focus used the existing exact
foreground validation for probe/restoration and the published focus hint at
the precision boundary. Suspend/resume used `NativeDispatchSession`'s
authoritative system-power endpoint. Lease expiry interrupted the long wait
before its authored target, emitted no gameplay KeyDown, retained terminal
reason `supervisor_lease_expired`, and emitted only the bounded cleanup Up
sweep.

The exact physical command was:

`rtk powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run_native_acceptance_case.ps1 -Scenario <scenario> -TimingMarginUs 500`

The contention row additionally used `-CpuContention`. The runner starts the
project-owned receive-only sink and passes all scenarios through
`NativeDispatchSession` plus `BackendConfig::Production`; it never calls a raw
sender. Injected ZeroProgress/partial/integrity acceptance remains covered by
the test-support transport seam and focused Rust tests:
`normal_prepared_zero_progress_is_fail_closed_without_cursor_advance`,
`mixed_packet_partial_fault_stops_before_committing_retrigger`, and the Win32
packet integrity/partial tests.

## Counterbalanced timing qualification

Corrected benchmark ownership was used: prepared-stream construction, then
target alignment, then wait entry. Startup construction is reported separately
and is not charged to each musical deadline. The counterbalanced order was:
`base-1 -> head-1 -> base-2 -> head-2`; each run used 10,000 observations,
was statistics-eligible, acceptance-clean, and had non-dispatch, overdue, and
real-wait transport-anomaly counts of zero. `head-1` and `head-2` are the
production-equivalent implementation run before the final report-only
serialization commit; the exact final implementation is independently covered
by the final-head confirmation row.

### `pre_call_qpc - target_qpc` (µs)

| Run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| base-1 | 2 | 5 | 8 | 2,193 | 16,593 |
| head-1 | 2 | 5 | 7 | 254 | 4,610 |
| base-2 | 3 | 5 | 92 | 2,157 | 9,688 |
| head-2 | 2 | 48 | 225 | 485 | 4,223 |
| final head confirmation | 2 | 5 | 145 | 450 | 10,265 |

### `completion_qpc - pre_call_qpc` (µs)

| Run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| base-1 | 0 | 1 | 1 | 1 | 9 |
| head-1 | 0 | 1 | 1 | 2 | 23 |
| base-2 | 0 | 1 | 1 | 3 | 33 |
| head-2 | 0 | 1 | 1 | 2 | 19 |
| final head confirmation | 0 | 1 | 1 | 2 | 59 |

There is no systematic head disadvantage in p99/p99.9 or maxima across the
counterbalanced pair; this is reported as no attributable cleanup regression,
not as a scheduler improvement claim. Exact final-head confirmation raw
decomposition:

- target -> wake p99/p99.9/max: `142 / 446 / 4,385`;
- wake -> final policy p99/p99.9/max: `5 / 14 / 5,880`;
- final policy -> pre-call p99/p99.9/max: `0 / 0 / 0`;
- target-minus-wait-entry p50/p95/p99/p99.9/min: `9,999 / 9,999 / 9,999 / 9,999 / 9,985`;
- prepared-stream build startup-only p50/p95/p99/max: `14 / 17 / 22 / 77`;
- `wake_to_send_max_us` and `sendinput_duration_max_us` remain serialized in
  each native report; the final-head raw report records both fields.

Raw timing reports:

- `.benchmarks/phase7-base-ab-1.json`;
- `.benchmarks/phase7-head-ab-1.json`;
- `.benchmarks/phase7-base-ab-2.json`;
- `.benchmarks/phase7-final-head-confirmation.json`;
- `.benchmarks/phase7-final-head-confirmation-final.json` (the sole report labeled
  `final head confirmation` for this revision).

Exact benchmark command:

`rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=10000&& set RT_HANDOFF_BENCH_SCOPE=baseline&& cargo run --release --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json>"`

## Verification

Focused Gate-1 tests passed:

- `normal_floor_forensics_are_preserved_without_becoming_shipping_authority`;
- `strict_physical_forensics_still_reject_floor_violations`;
- `unrelated_shipping_invariants_remain_fail_closed`;
- existing strict `PhysicalTimingGuard` latest-start rejection tests;
- native acceptance harness tests: 21 passed.

The final verification command/result list, including full `sky_player`,
no-allocation, desktop/Tauri, generated-binding, native acceptance,
`cargo xtask check static`, `cargo xtask check rust`, `cargo xtask check all`,
and required GitHub CI, is recorded in the PR body after the final clean run.

## Limitations

These are Windows production-session/sink qualification reports, not a game
observer or Raw Input latency claim. The sink observes the project-owned
receive-only window. Completion-relative floor violations are intentionally
preserved as diagnostics and are not hidden or converted into normal timing
policy. Historical Phase-0/Phase-3 tail values remain comparison-only.
