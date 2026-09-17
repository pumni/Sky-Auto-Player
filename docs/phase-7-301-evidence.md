Closes #301

# Phase 7 final qualification evidence

Base: `9f54a3e7d990b5f05656d083cefc452607782ffe`

The final HEAD SHA is recorded in the PR description after the evidence commit.
This document is part of that final HEAD.

## Environment

- Toolchain: `rustc 1.98.1 (48a229cea 2026-09-01)`;
  `cargo 1.98.1 (797e8a9bc 2026-08-05)`;
  `stable-x86_64-pc-windows-msvc`.
- Host: Windows 11 Home `10.0.26200` build `26200`, Dell Inspiron 5515,
  AMD Ryzen 5 5500U, 12 logical processors, Balanced power plan.
- QPC frequency in the reports: 10,000,000 Hz.
- The exact base was fetched and verified before editing:
  `origin/main == 9f54a3e7d990b5f05656d083cefc452607782ffe`.

## Production cleanup

- Removed the live normal `normal_down_start_tolerance_us` setting from the
  domain, persisted DTO/config, Tauri bridge, generated TypeScript, UI, native
  timing options, worker timing state, admission, and test-support state.
- Legacy persisted `default_normal_down_start_tolerance_us` is accepted during
  migration, removed from canonical saved data, and never mapped to Timing
  Margin or another grace period. A migration test loads different legacy
  values and proves that the resulting timing policy is identical.
- Removed unreachable late-rescue recording and its normal telemetry/report
  fields. Strict physical latest-start rejection and its raw observation
  evidence remain.
- Narrowed same-key overlap accounting to corruption/forensics evidence;
  pre-session compiler/admission validation and disjoint physical packet
  semantics remain authoritative.
- Added ADR-0014 as the normative post-Phase-6 dispatcher contract. ADR-0011,
  ADR-0012, and ADR-0013 are explicitly historical/superseded records.
- No MMCSS, affinity, priority, spin, timer, power, adaptive-lead, or scheduler
  policy was changed. The normal precision helper remains independent of
  coordinator/planner/recovery/pending-release state.

## Deterministic acceptance

The final prepared acceptance report is
`.benchmarks/phase7-final-head-baseline.json`. It is clean and statistics
eligible with 10,000 real HybridWaiter observations.

| Result | Value |
| --- | ---: |
| Prepared physical boundaries | 16 |
| Successful SendInput transactions | 14 |
| Intentional non-send/drop boundaries | 2 |
| Timeline rebases | 0 |
| Normal `DownExpiredBeforeSend` | 0 |
| Normal `UnobservedBacklog` | 0 |
| Completion-feedback `PhysicalWindowExpired` | 0 |
| Real-wait non-dispatches | 0 |
| Real-wait overdue classifications | 0 |
| Real-wait transport anomalies | 0 |

The two non-sends are the intentional control-stop race and injected
`ZeroProgress`. The report asserts each scenario's semantic disposition, not
only aggregate counts. It also retains the completion control/delayed pair:
packet N completion differs (8 us versus 40,000 us), while packet N+1 authored
target, physical target, packet-not-before, wait target, eligibility, and
classification are identical; normal hold/release floor masks are zero.

The legal same-key sequence remains immutable and ordered as
`Down(K) -> Up(K) -> Down(K) -> Up(K)`. Impossible same-key geometry is still
rejected before worker realtime execution. Clean late prepared frames make one
send attempt; stop/skip/pause/focus/system-power/target races suppress sends;
ZeroProgress, partial, and integrity faults remain fail-closed.

## Windows physical qualification

The production release harness was built with
`--features real-input-acceptance` and uses `NativeDispatchSession`,
`BackendConfig::Production`, and the project-owned receive-only sink. Fresh
runs at `TimingMarginUs=500` were made on both the Phase-7 worktree and the
detached exact base:

| Revision | Scenario | Result | Evidence |
| --- | --- | --- | --- |
| Phase-7 worktree | `timing-margin-sweep` | FAIL: fixed physical hold/release-floor diagnostic | `.benchmarks/physical-native-cases/native-case-20260917T115052-50b886f8` |
| Exact base | `timing-margin-sweep` | Same FAIL and reason | `.benchmarks/phase7-base-physical/timing-margin-sweep` |
| Phase-7 worktree | `release-gap-stress` | FAIL: fixed physical hold/release-floor diagnostic | `.benchmarks/physical-native-cases/native-case-20260917T120143-d802b751` |
| Exact base | `release-gap-stress` | Same FAIL and reason | `.benchmarks/phase7-base-physical/release-gap-stress` |

Both revisions delivered the expected sink events, finished with clean active
and possibly-active state, zero transport anomaly, zero missed Down, zero
rebase, and no stuck keys; the run was rejected only by the existing physical
hold/release forensics. This is therefore a pre-existing/environmental
qualification limitation, not a Phase-7 cleanup regression. The existing
earlier physical matrix records remain available under
`.benchmarks/physical-input-reliability/` and
`.benchmarks/physical-native-cases/`, but are not relabeled as fresh final-HEAD
qualification. No production change was made to hide or weaken this result.

## Counterbalanced timing qualification

All runs used the corrected ownership order: prepared stream construction,
then target alignment, then wait entry. Construction is reported as startup
evidence and is not charged to the musical deadline. Every run below used
10,000 observations, was acceptance-clean/statistics-eligible, and had
`non_dispatches=0`, `overdue=0`, and real-wait `transport_anomaly_count=0`.
Values are microseconds. The interleaved order was head-1, base-1, head-2,
base-2, head-3, base-3.

### `pre_call_qpc - target_qpc`

| Run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| head-1 | 4 | 27 | 286 | 4,983 | 65,523 |
| base-1 | 4 | 127 | 3,012 | 11,284 | 140,833 |
| head-2 | 4 | 371 | 3,327 | 10,385 | 30,580 |
| base-2 | 4 | 343 | 3,945 | 10,984 | 119,894 |
| head-3 | 4 | 385 | 4,312 | 10,859 | 14,127 |
| base-3 | 4 | 660 | 4,738 | 12,175 | 50,497 |
| final head confirmation | 3 | 5 | 10 | 369 | 5,039 |

### `completion_qpc - pre_call_qpc`

| Run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| head-1 | 0 | 1 | 1 | 10 | 208 |
| base-1 | 1 | 1 | 2 | 25 | 278 |
| head-2 | 1 | 1 | 2 | 16 | 245 |
| base-2 | 1 | 1 | 2 | 50 | 4,179 |
| head-3 | 0 | 1 | 2 | 36 | 951 |
| base-3 | 1 | 2 | 3 | 67 | 4,743 |
| final head confirmation | 0 | 1 | 1 | 5 | 88 |

Head p99, p99.9, and maximum are lower than the paired base in all three
interleaved comparisons. This is reported as “no attributable regression”,
not as a scheduler improvement claim. The final-head report also records:

- target-to-wake p99/p99.9/max: `2 / 415 / 13,803`;
- wake-to-final-policy p99/p99.9/max: `5 / 12 / 902`;
- final-policy-to-pre-call p99/p99.9/max: `0 / 0 / 0`;
- wake-to-send p99/p99.9/max: `11 / 1,132 / 13,809`;
- prepared-stream build startup-only p50/p95/p99/max: `11 / 17 / 22 / 97`;
- target-minus-wait-entry p50/min: `9,999 / 9,948`;
- SendInput duration evidence is retained in the raw JSON report.

Raw timing reports:

- `.benchmarks/phase7-head-baseline.json`;
- `.benchmarks/phase7-base-1.json`;
- `.benchmarks/phase7-head-2.json`;
- `.benchmarks/phase7-base-2.json`;
- `.benchmarks/phase7-head-3.json`;
- `.benchmarks/phase7-base-3.json`;
- `.benchmarks/phase7-final-head-baseline.json`.

The exact benchmark command was:

`rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=10000&& set RT_HANDOFF_BENCH_SCOPE=baseline&& cargo run --release --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json>"`

The physical command was:

`rtk powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run_native_acceptance_case.ps1 -Scenario <timing-margin-sweep|release-gap-stress> -TimingMarginUs 500`

## Verification commands and results

Focused and affected tests passed:

- `rtk cargo test --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support` — 297 passed;
- `rtk cargo test --manifest-path rust/Cargo.toml -p sky_player --test rt_dispatch_no_alloc --features test-support` — 23 passed;
- `rtk cargo test --manifest-path rust/Cargo.toml -p sky_app_core -p sky_native_adapters` — 72 passed;
- `rtk cargo test --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test --locked` — 192 passed;
- `rtk bun run test` — 162 passed;
- `rtk bun run typecheck`, `rtk bun run lint`, `rtk bun run format:check`, and `rtk bun run build:web` — passed;
- `rtk cargo test --manifest-path rust/Cargo.toml -p sky_player --test rt_dispatch_no_alloc --features test-support` — 23 passed (final focused rerun);
- `rtk cargo build --release --locked --manifest-path rust/Cargo.toml -p sky_player --features real-input-acceptance --bin rt-native-acceptance` — passed;
- `rtk cargo xtask bindings generate` — passed and generated bindings are committed.

Canonical gates run or pending final CI:

- `rtk cargo fmt --manifest-path rust/Cargo.toml --all` and
  `rtk cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — passed;
- `rtk cargo xtask check static` — passed;
- `rtk cargo xtask check rust` — passed;
- `rtk cargo xtask check all` — passed;
- required GitHub CI — final result recorded with the PR check.

The final PR diff was reviewed for scope: production edits are limited to
retiring the normal late-note policy surfaces, preserving strict diagnostics,
renaming the same-key forensics metric, and correcting current architecture
documentation/comments. No timing tuning or unrelated refactor is included.

## Limitations

The fresh physical production runs above are not clean because the same fixed
physical floor diagnostic reproduces on the exact base. This remains an
explicit qualification limitation and requires coordinator review; it is not
silently converted into success. GitHub required CI is the final external
gate and is not represented as passed until its checks complete.
