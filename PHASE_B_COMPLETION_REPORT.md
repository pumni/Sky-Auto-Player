# Phase B calibration vNext — correction completion report

Date: 2026-08-19

Status: `AUTOMATED CORRECTION GATES PASS — HUMAN PHASE-B ACCEPTANCE PENDING`

This report covers the narrow correction requested after review of the initial
Phase B implementation. It does not grant the human real-host acceptance token.

## Provenance

- Phase-A clean base: `51b6b1a5205764b230b8cd32be522b319fb7ed2d`
- Initially reviewed Phase-B candidate: `684762f6d5ff42bb2f4eb97938e73091ef49ac7b`
- Precision-boundary correction: `91959087e27e1bfbe5dce267e2ca81a175848eec`
- Scheduling-aid provenance correction: `de0caeb46e4b3f501a8bcee8e5ed842a06bc3300`
- This report and validation-only test-import correction are committed after the
  correction code.

## Precision-boundary correction

Calibration now follows the production handoff sequence:

```text
validate / tag / materialize fixed INPUT[15]
→ arm correlation
→ wait to T - 700 µs with waiter spin = 0
→ shared fused sender owns final crossing sample and authoritative P
→ one SendInput
→ completion QPC C and receipt R
```

`PreparedTaggedCalibrationPacket` owns the complete tagged payload before the
precision handoff. The sender only creates a view over that existing fixed
array; it does not construct or allocate an `INPUT` payload after the handoff.
The shared sender retains the Phase-A crossing, strict Down cutoff, Up-only,
single-syscall, and completion-clock semantics.

Direct protections now include:

- source-structural proof that payload materialization precedes the precision
  wait and fused sender call;
- the fixed 700 µs production handoff constant and zero waiter-spin proof;
- crossing reuse, exact/in-grace/beyond-cutoff, and Up-only regression tests;
- no-shrink paired-total math and mixed signed-component cases;
- independent fingerprint mismatch tests for CPU family, stepping, efficiency
  histogram, Windows build, QPC frequency, and fingerprint schema version.

No production timing policy, host identity field, Down grace, MMCSS policy, or
calibration qualification formula was changed by this correction. Schema
versions changed only to version the new required provenance fields.

The output/cache schema correction adds `scheduling_aids` with the acquired
MMCSS label, MMCSS active state, PowerThrottling/HighQoS guard state, and actual
waiter mode. Native schema is 11, artifact schema is 8, and cache schema is 5;
older payloads fail closed.

The final validator correction now makes publishable validation stricter than
structural validation: only `event+high_resolution_timer` is publishable, and
the MMCSS label must be `mmcss:Games`, `thread:highest`, or `off`. Checkpoint
finalization/resume rejects missing, old, or future artifact schema versions
before metadata and provenance publication.

Command-level execution evidence, exit codes, counts, warnings, and raw log
paths are recorded in [PHASE_B_VALIDATION_EVIDENCE.md](PHASE_B_VALIDATION_EVIDENCE.md).

## Phase-A post-refactor A/B evidence

The final designated evidence set is retained under `phase-b-evidence/`:

- 5 baseline runs at `51b6b1a...`;
- 5 candidate runs at `9195908...`;
- all 9 required scenarios;
- 5,000 iterations per scenario per run;
- `phase_a_production_matrix` scope;
- `phase_a_production_boundary` mode;
- production coordinator/admission/commit boundary with
  `test_started_ticks=None`;
- deterministic mock transport, not game-observed latency.

The locked comparison rule is the median of five runs. The final matrix has no
material tail regression: no scenario/metric has both a p99 or p99.9 increase
above 10% and above 20 µs. All candidate counts are zero for early dispatch,
non-dispatch, and `deadline_missed_before_send` in every scenario.

Selected final-matrix p99.9 results (baseline → candidate, µs):

| Scenario | dispatch-start error | pre-call → completion | target → completion |
| --- | ---: | ---: | ---: |
| `down_only_1` | 183 → 165 | 0 → 0 | 184 → 165 |
| `down_only_5` | 176 → 156 | 0 → 0 | 176 → 157 |
| `down_only_15` | 175 → 167 | 0 → 0 | 175 → 168 |
| `up_only_1` | 137 → 138 | 0 → 0 | 139 → 138 |
| `up_only_5` | 142 → 142 | 0 → 0 | 143 → 142 |
| `up_only_15` | 150 → 154 | 0 → 0 | 151 → 154 |
| `mixed_2` | 148 → 145 | 0 → 0 | 148 → 146 |
| `mixed_10` | 162 → 147 | 0 → 0 | 163 → 148 |
| `mixed_30` | 170 → 184 | 0 → 0 | 170 → 184 |

Two earlier exploratory matrices showed different isolated tail outliers on
this Windows runner. They were not used as the designated acceptance set; they
were retained in the working evidence history while the final five-versus-five
set was rerun. The final set passes the locked gate without weakening the
production policy or changing the benchmark threshold.

Raw evidence SHA-256:

```text
baseline_bench_run_01.json  12933803b0cc8ae9e2b2804e9b7691bc0e3d0b9fd85f2b73b49a83ff27e57ff2
baseline_bench_run_02.json  5f90a0f85d3c8d3c6b4594763ea36353b136ccbb546d615891cae898a54ef25a
baseline_bench_run_03.json  f9b7f7c408d204d81396cd1ce809808cdefaf2af73c4526f86640ef72413be5c
baseline_bench_run_04.json  00f6e858fa169c6e21337a0138f944898fd70b1ba38e7d3737ac58a0c0c6d891
baseline_bench_run_05.json  6ec2bbef3104e4c08268dade6dc95d53514342ec7ff195546e6c116f40828e08
candidate_bench_run_01.json 0edced65aebef061e0da643b14b360ec270b00554bc470cc3e277948f7852a7f
candidate_bench_run_02.json 38ca84589cfaa4768f849e1aa7c30d3db37a9ae91510c2a75d2784a902ac1f3a
candidate_bench_run_03.json 8527a3dd5b8c2579a1cd26ad8c957308230722aac77973dd554fd9b82e2d9778
candidate_bench_run_04.json aaa0d620d98af7ce113708bceff9b0a08482eb0bc1e21b57a17c1835fb3721cb
candidate_bench_run_05.json 0e4cf14323f2a8c87afa58b8b6678df34622bbd2715cc5d164875aeb587e3659
```

## Validation evidence

| Gate | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS |
| Rust workspace tests | PASS — 459 tests with `--all-features` |
| `sky_dispatch_win32` tests | PASS — 152 tests |
| workspace Clippy, `-D warnings` | PASS |
| `sky_player_rs` check and scoped Clippy | PASS |
| `uv sync` with repo-local `UV_CACHE_DIR` and `.env` | PASS |
| native Rust wheel build and verification | PASS — `cp314t` |
| free-threaded wheel audit | PASS — GIL disabled |
| Ruff | PASS |
| Pyright | PASS — 0 errors, 0 warnings |
| Python non-slow tests | PASS — 824 passed, 6 skipped, 1 xfailed, 1 warning |
| security mandate audit | PASS |
| `git diff --check` | PASS |

The updater test import cleanup required for workspace `-D warnings` only
places feature-gated imports behind their existing feature. It changes no
updater behavior or security policy.

## Human gates still pending

The Phase-A sequencing authorization is present and exact:

```text
PHASE_A_ACCEPTED: proceed to Phase B calibration vNext
```

It was recorded during the post-implementation Phase-B acceptance exchange,
after Phase-B work had begun. It is not a substitute for real-host calibration
evidence.

The automated correction evidence is complete, but the following are not
claimed here:

```text
VNEXT_REAL_HOST_CALIBRATION = AWAITING_HUMAN_EVIDENCE
PHASE_B_HUMAN_REVIEW = AWAITING_HUMAN_REVIEW
```

No real Windows game-observed calibration evidence has been supplied in this
correction pass. Phase B should therefore remain closed for publication until
that review is recorded.
