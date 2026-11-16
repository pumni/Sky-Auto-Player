# Issue #666 — M4 diagnostics, cleanup, and qualification evidence

## Status and scope

M4 review checkpoint, not an acceptance verdict. The native qualification budget
has not been authorized and no native input, interactive focus, real suspend /
resume, or Sky gameplay session was run.

The M4 implementation commit used for both offline A/B arms is
`cede054d0c1f6617ed8cb1e95e52a70988c4e0fc` (tree
`fff324c835e37c0c4d1ebed354858d9b00e52a6c`; binary identity is in the external
run manifest). The final documentation/evidence commit may advance HEAD without
changing the measured Rust source; this report names the exact measurement
revision so the paired arms remain reproducible.

Production diagnostics, playback policy, public DTOs, fingerprints, and
telemetry schema were already satisfying the M1–M3 contracts at the accepted
M3 base. M4 adds compatibility regressions and a test-only, ignored Windows
waiter comparison; it does not add runtime telemetry fields or change how
packets are admitted, attempted, classified, cleaned up, or published.

## Diagnostics and compatibility matrix

| Contract | Before M4 | M4 check / after | Result |
|---|---|---|---|
| Telemetry schema | Schema 25 | Assert version and exact serialized key sets for `NativeTelemetryOutput`, `NativeTelemetrySummary`, `TimingSemantics`, and `RtTraceRecord` against `native-telemetry-schema-25-keys.json` | Same fields; no schema change |
| Sender evidence | Raw packet/status/count/error/attempt and authored source fields already present | Serialize a complete-count Up attempt with missing completion QPC and terminal-anomaly outcome | Attempt remains anomalous; no timing envelope sample |
| Missing timestamp | Schema's existing unavailable encoding and availability/provenance fields | Assert unavailable completion remains unavailable; no QPC is synthesized | Preserved |
| Invalid timestamp order | Existing sender contract rejects invalid intervals | Reversed completion QPC regression expects record construction error | Rejected; not clamped |
| Terminal/error projection | Worker terminal error, trigger, secondary errors, release outcome/mask and outcome already flow through snapshots/terminal publication | Existing error aggregation, anomaly, terminal release and native projection regressions are run in the required suites | No field or meaning change |
| Observer/lifecycle projection | Bounded queue, drop counters, snapshot cadence, observer finalization and UI status projection | Existing observer, telemetry, event-delivery and lifecycle tests are run in full gates | No cadence or projection change |
| Compatibility seams | Existing deprecated/public compatibility fields remain | No consumer contract removed; compatibility counters/fields retained | No public DTO/fingerprint edits |

The schema-25 wire format uses numeric zero for certain unavailable timestamp
slots, paired with `*_available: false` and `*_provenance: "unavailable"`.
The regression pins that existing encoding. Zero is not a measured QPC and
must not be interpreted as one. Reversed QPC evidence is rejected. Backend
`transport_anomaly_count` retains its existing backend-transport-only meaning;
an attempted timing anomaly is preserved by the worker's bounded internal
anomaly history and terminal error latch rather than silently redefining that
schema field.

The retained 50 ms live-snapshot cadence, empty/terminal cleanup behavior,
owner-generation release proof, one-attempt Down rule, verified cleanup + G
barrier, and final terminal aggregation are unchanged. Existing regressions
cover zero-obligation cleanup (no owner check or syscall), verified positive
release, inconclusive/failed release, old-owner generation drift, and anomaly
recovery followed by terminal error.

## Deterministic regressions

New M4 tests:

| Test | Invariant |
|---|---|
| `native_telemetry_schema_25_serialized_key_sets_are_pinned` | Exact public serialized key sets and schema version 25 remain pinned. |
| `schema_25_missing_completion_keeps_attempt_and_error_without_timing_envelope` | Complete raw count does not erase attempted-anomaly outcome; source action, masks, raw status/count/attempts, pre-call QPC and unavailable completion provenance survive, and no envelope sample is counted. |
| `schema_25_reversed_sender_qpc_is_rejected_instead_of_clamped_or_reconstructed` | Reversed sender QPC fails closed. |
| `nearest_rank_percentiles_use_ceiling_rank_and_preserve_empty_as_null` | p50/p95/p99.9 use the documented nearest-rank formula; an empty metric remains JSON null. |

Existing M2/M3 regressions remain the behavioral oracle for diagnostic
continuity:

- `full_count_up_only_reversed_qpc_recovers_then_finishes_as_error`
- `up_only_missing_completion_qpc_without_timing_error_still_latches_error`
- `forty_up_only_anomalies_keep_bounded_first_last_evidence_and_never_replay_downs`
- Snapshot/error aggregation and terminal DTO tests for verified, inconclusive,
  and failed release; owner-generation isolation and cleanup + G.

The 40-anomaly test asserts exact total 40, omitted 24, retained 16, evidence
from the first and last source actions, 80 total packet attempts with no Down
replay, verified cleanup before each following onset, and final Error. This
remains a mock QPC/sender test, not a game-reception test.

## Same-revision guard A/B experiment

**Status: completed offline**. The single candidate revision is
`cede054d0c1f6617ed8cb1e95e52a70988c4e0fc`; both arms use
the same test binary. This is a paired fixed-guard versus calibrated-guard
experiment on the waiter only. It is not an exact-base-vs-candidate A/B and it
does not change the production guard.

- Arm A: experimental fixed 1,000 µs guard.
- Arm B: existing startup-calibrated selector and production fallback.
- Same six-sample calibration probe per run; fixed arm records and discards its
  result so both arms pay the same startup cost.
- Quiet and Contention conditions; one warm-up per arm/cell.
- Six measured pairs per condition, with three AB and three BA pair orders.
- Each run: 900 deadlines at 60 FPS, 16,667 µs apart; 15 seconds per arm/run.
- 5,400 expected samples per arm/cell. Percentiles use nearest rank
  `sorted[ceil(p*n)-1]`; missing QPC values are excluded from percentiles and
  separately counted. The harness fails if a measured deadline is missing or
  invalid, or if any wait fails/interruption occurs.
- Contention: two unpinned normal-priority worker threads, each using 5 ms
  active / 5 ms sleep slices. CPU time and loop iterations are recorded.
- Host/workload/toolchain, exact source/tree, Cargo.lock hash, test-runner
  binary SHA-256, raw CSV hashes, command and exit code are in the external
  `run-manifest.json`.

This harness does **not** include authored note count, instrument profile,
packet staging/execution, final focus/owner/modifier/lease gate, sender syscall,
H/G dispatch-floor enforcement, cleanup, Prepare/Arm/Ready/Running/Finalize,
or gameplay. FPS/deadline cadence is 60 Hz, but margin/epoch policy are null
because the waiter-only experiment has no playback session. It reports wake
QPC-to-deadline lateness and waiter-return-to-observation QPC gap; the latter
is explicitly not a sender pre-call measurement. SendInput pre-call/completion,
envelope, note delivery, block/replan/anomaly, and cleanup measures remain
unavailable. Mock/scheduler results do not establish real SendInput latency or
that a game received a note.

### Results

| Condition | Arm | Samples | Wake lateness p50 / p95 / p99 / p99.9 / max (µs) | Integer-µs positive lateness | Spin / wall | Worker CPU / process CPU (s) |
|---|---|---:|---|---:|---:|---:|
| Quiet | Fixed 1 ms | 5,400 | 0 / 0 / 0 / 75 / 9,314 | 43 | 3.7904% | 2.766 / 2.766 |
| Quiet | Calibrated | 5,400 | 0 / 97 / 279 / 474 / 2,311 | 605 | 2.5630% | 2.375 / 2.375 |
| Contention | Fixed 1 ms | 5,400 | 0 / 0 / 0 / 259 / 3,933 | 40 | 3.8752% | 4.016 / 90.719 |
| Contention | Calibrated | 5,400 | 0 / 0 / 10 / 2,587 / 16,659 | 70 | 3.4873% | 4.266 / 90.766 |

Each cell had six measured runs (three AB, three BA), one warm-up per arm,
zero missing/invalid QPC samples, zero early wakes, zero failed waits, and zero
interrupted waits. Integer-µs positive-lateness counts use the converted
microsecond values; sub-microsecond positive intervals can convert to zero.
Raw `target_qpc` and `wake_qpc` remain in the CSV for exact tick-level review.

| Condition / arm | Wait-return-to-observation gap p50 / p95 / p99 / p99.9 / max (µs) | Selected guard per measured run (µs) | Startup probe duration p50 / p95 / max (µs) |
|---|---|---|---|
| Quiet / fixed | 6 / 8 / 9 / 20 / 60 | 1000, 1000, 1000, 1000, 1000, 1000 | 15015 / 15116 / 15116 |
| Quiet / calibrated | 6 / 8 / 9 / 39 / 121 | 441, 1000, 758, 456, 1000, 1000 | 14225 / 14777 / 14777 |
| Contention / fixed | 5 / 8 / 9 / 17 / 35 | 1000, 1000, 1000, 1000, 1000, 1000 | 14120 / 15075 / 15075 |
| Contention / calibrated | 5 / 8 / 9 / 43 / 99 | 1000, 1000, 906, 1000, 691, 1000 | 14518 / 15177 / 15177 |

Contention used two workers at 5 ms active / 5 ms sleep, normal/inherited
priority, unmodified affinity. The workers accumulated 202.719 CPU seconds and
2,858,035,480 loop iterations over the contention condition (including its
warm-ups). The machine was Windows 11 Home build 10.0.26300, AMD Ryzen 5 5500U,
12 logical processors, QPC 10,000,000 Hz; Rust/Cargo 1.99.1, host
`x86_64-pc-windows-msvc`.

The fixed arm had fewer integer-µs late observations in this run, while the
calibrated arm used less spin time and had a smaller Quiet maximum; under
Contention the calibrated arm also had a 16.659 ms outlier. Six pairs on one
host and a waiter-only path do not establish a general winner or justify a
production policy change. The median of the per-run startup-probe p99 values
was 645/548 µs for Quiet fixed / calibrated and 603/687 µs for Contention fixed
/ calibrated. No H/G or sender timing distributions exist in this experiment.

External artifact root: `%TEMP%\issue-666-m4-ab-cede054d`.

- Candidate tree clean before run: yes.
- Command exit: 0; `1 passed; 0 failed; 0 ignored; 0 measured; 288 filtered`;
  420.70 s test duration.
- Test binary SHA-256:
  `f469f54be6c29e3e9ab894e6d6afb850a35f9806172b290ecf334eb9e815f4ff`.
- `wait-samples.csv` SHA-256:
  `0ad5715d37decf8c48573e1a17d35f35ad8f38cb777eaa2beb22c599c8729d09`.
- `wait-runs.csv` SHA-256:
  `e15c7686c7cb5e18768fcab81c7615ae5a8a415a2a133187269cb5d75c735afa`.
- Cargo.lock SHA-256:
  `3a73099420f14704af7eaff54f2acb1e80ea89f52b876696bbc83b9015467110`.
- QPC frequency 10 MHz; all four cells have 5,400 / 5,400 valid samples.

No production guard recommendation is made from this waiter-only workload.

**Exact-base benchmark:** not run as part of this M4 task. M1's
baseline-equivalent structural benchmark is not an exact-base A/B and is not
gameplay or Arm latency evidence.

## Native qualification plan and proposed budget

[`issue-666-m4-native-qualification.json`](issue-666-m4-native-qualification.json)
is planning metadata only: `NOT RUN — awaiting Leader authorization`.

It separates deterministic mock cases from an operator-controlled ReceiveOnly
window using public Win32 `SendInput`, and separates both from optional Sky
gameplay. The ReceiveOnly proposal requests a single 60-minute budget for six
cases in Quiet and Contention, three repetitions per case/condition. It covers
ordering/raw sender evidence, immediate foreground loss, target/owner drift,
long-wait Stop/Skip, origin-owned verified cleanup, and control boundaries.
The proposed artifact root is outside the checkout; the first failed native
case is fail-stop with no retry. Raw zero/short, missing/reversed QPC, exact
H/G ±1 tick, and suspend callback fault injection remain deterministic mock
cases; public SendInput is not manipulated to fabricate those outcomes.

Sky gameplay has a separate proposed 20-minute budget and remains not
authorized. Before it can run, the repository owner and Leader must name the
song/chart/section, key mapping, instrument, game build/settings, expected-note
oracle, safe abort and artifact policy. Sender insertion is reported separately
from game reception. No real Windows suspend/resume was performed.

## Verification ledger

Windows x64 MSVC, Rust/Cargo 1.99.1. This ledger records intermediate results
as well as final required gates; a zero-test invocation is not counted as a
pass. Full required gates and test counts will be appended before handoff.

| Command / run | Exit | Result |
|---|---:|---|
| Initial focused compile/test attempt (schema regression) | 1 | Failed before test execution: benchmark CSV format had unused arguments and telemetry references used the wrong parent module. Fixed before later runs. |
| First corrected schema-key test run | 0 | 1 passed; 287 filtered; emitted an unused benchmark run-order field warning, fixed by asserting and recording AB/BA/pair evidence. |
| `cargo fmt --manifest-path rust/Cargo.toml --all` | 0 | Rust formatting applied. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib native_telemetry_schema_25_serialized_key_sets_are_pinned` | 0 | 1 passed; 287 filtered. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib schema_25_` | 0 | 3 passed; 285 filtered. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib up_only` | 1 | 10 passed, 1 failed, 277 filtered. The existing realtime-QPC anomaly-retention test returned a scheduler `Wait` before an authored onset during the grouped run. |
| Exact-name retry with `-- --exact` | 0 | 0 tests; not evidence and not counted as PASS. |
| Corrected isolated `forty_up_only_anomalies_keep_bounded_first_last_evidence_and_never_replay_downs` retry | 0 | 1 passed; 287 filtered. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib nearest_rank_percentiles_use_ceiling_rank_and_preserve_empty_as_null` | 0 | 1 passed; 288 filtered. |
| `git diff --check` before the implementation commit | 0 | No whitespace errors. |
| PowerShell AST parser for `scripts/run_issue666_m4_guard_ab_offline.ps1` | 0 | No parse errors. |
| First complete offline guard A/B wrapper on `cede054d0c1f6617ed8cb1e95e52a70988c4e0fc` | 0 | 1 explicitly selected ignored experiment passed; 5,400 valid samples per arm/condition, zero missing, failed or interrupted waits. Its raw results are superseded by a final-source rerun below. |
| Required Clippy gate, first invocation | 1 | Found `run_once` had 9 arguments (`clippy::too_many_arguments`). Refactored benchmark-only run-order inputs into `RunOrder`; no measured algorithm or runtime path changed. |
| Required Clippy gate after `RunOrder` refactor | 0 | `sky_dispatch_core`, `sky_dispatch_win32`, and `sky_player` all-target/all-feature Clippy clean with `-D warnings`. |
| Final-source offline guard A/B wrapper | pending | Will run after committing the lint-only signature refactor so the final source SHA is recorded. |
| Required formatting, Clippy, Rust suites, no-allocation, desktop-native, and `xtask check all` gates | pending | To run serially and append with actual exit codes and test counts. |

## Limits and risks

- No real SendInput, ReceiveOnly interactive session, foreground manipulation,
  Windows suspend/resume, or Sky gameplay has been performed.
- The A/B has only waiter timing evidence. It cannot explain the observed
  full dispatch latency distribution or prove notes were received.
- The existing QPC-driven anomaly test is timing-sensitive when run grouped;
  it failed once and passed when isolated. The full suite must be reported
  without masking any later recurrence.
- No change is proposed to production calibration/defaults, timing margins,
  FPS, tempo, DTO/schema, cleanup, ownership, lease/watchdog, or gameplay
  semantics.
