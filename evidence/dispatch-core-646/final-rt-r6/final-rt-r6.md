# FINAL-RT-R6 implementation and evidence

## Revision and scope

- Repository: `pumni/Sky-Auto-Player`
- Branch: `perf/dispatch-core-20261109`
- Starting parent: `3c155c127c8f609aa3b2ac4a91351f6444999287`
- Tested source tree: `121aecb4a08b80d59efbaa26dbf2fb0cab378fd3`
- Baseline measurement-only source tree: `0498afa1c6e1eefdc93984cb5a983c0399055a27`; the reconstructing patch is gzip-compressed in [sources/stale-benchmark-instrumentation-before.patch.gz](./sources/stale-benchmark-instrumentation-before.patch.gz).

This batch fixes stale metadata versus physical deadline arbitration in the existing B worker. An earlier stale deadline uses a metadata-only Frozen wait, drains only metadata due at that point, then re-evaluates. The physical-floor send path follows the same arbitration. Checked lookahead propagates coordinator errors and handles empty/equal/no-progress cases. The existing single-worker scheduler, 32-item step cap, due-only commit, Stop/control and SendInput gates, Frozen behavior, and memory budget were retained.

Benchmark provenance records authored target QPC, packet-not-before QPC, and pre-call QPC independently. Signed authored-to-pre-call and eligibility-to-pre-call residuals are reported separately; unavailable wakes remain unavailable. Sender start is sampled at the pre-call boundary. These additions and the worker race matrix are test-support/mock evidence, not proof from real native SendInput.

## Stale-deadline timing matrix

Windows 11 x64, QPC 10 MHz, actual B worker with mock sender, production-calibrated Frozen waiter, nearest-rank estimator. Before is measurement-only instrumentation on the baseline source; after is the tested arbitration source. Each listed repetition measured a physical Down. The 10k case retains observer-drop counts and is not marked statistics-eligible.

| Stale metadata | Runs | Before authored→pre-call p95 / p99 / max (µs) | After authored→pre-call p95 / p99 / max (µs) | After eligibility→pre-call p95 / p99 / max (µs) | Physical wake before→after | After metadata-only wakes |
|---:|---:|---:|---:|---:|---:|---:|
| 0 | 100 | 75 / 252 / 430 | 87 / 155 / 1750 | 87 / 155 / 1750 | 100/100 → 100/100 | 0 |
| 1 | 100 | 93 / 123 / 125 | 48 / 469 / 480 | 48 / 469 / 480 | 100/100 → 100/100 | 100 |
| 32 | 100 | 362 / 585 / 589 | 50 / 130 / 139 | 50 / 130 / 139 | 0/100 → 100/100 | 100 |
| 33 | 100 | 284 / 335 / 610 | 44 / 90 / 112 | 44 / 90 / 112 | 0/100 → 100/100 | 100 |
| 10,000 | 20 | 24,546 / 25,677 / 25,677 | 42 / 739 / 739 | 42 / 739 / 739 | 1/20 → 20/20 | 20 |

The raw baseline wake stream retains its missing samples (including 19 missing of 20 in the 10k case); no wake was synthesized. The after 10k run recorded 158,808 observer drops. These observer limits and outlier tails are retained in the raw JSON and matrix summary; the matrix is not qualification evidence. Authored and eligibility distributions match for sampled Down cases because authored target equaled packet eligibility there. The mixed floor-held regression independently verifies different authored/eligibility timestamps and signed residuals; an Up-only regression verifies sender-start provenance for Up.

Raw samples and provenance are in the [matrix summary](./stale-dispatch-timing-matrix-r6-summary.json), [before JSON](./benchmarks/stale-before-arbitration.raw.json), [after JSON](./benchmarks/stale-after-arbitration.raw.json), and captured stdout/stderr under [logs](./logs/). The exact baseline instrumentation patch is provided for reconstruction.

## Handoff benchmark

`rt_handoff_bench` ran 6 policies × 9 workloads = 54 cells at 100 iterations per cell. All 54 cells were clean against the existing acceptance checks; 0/54 were statistics-eligible. Transport was deterministic mock, waiter was `HybridWaiter::production`, effective policy was production-calibrated, waiter metrics were valid, and this report is not native qualification. The raw output and compact per-cell measurements are [here](./benchmarks/rt-handoff-iterations-100.raw.json) and [here](./handoff-benchmark-r6-summary.json). The requested 10k × 54 run was not performed without leader authorization.

## Verification

All recorded commands ran against source tree `121aecb4a08b80d59efbaa26dbf2fb0cab378fd3` and exited 0: fmt; strict workspace Clippy; core and Win32 suites; full player test-support; standalone noalloc; Tauri `tauri-test`; default Tauri check; root `cargo xtask check all`; and focused lookahead, stale-before-Down, floor-held timestamp, Up-only timestamp, simultaneous final Up/Down gate, Stop/Skip calibration, post-arm Quit, and 32/33/10k late-Quit regressions. The full player test-support run reported 505 passed and 1 ignored; standalone noalloc reported 29 passed; Tauri reported 330 passed and 4 ignored. The root check reported static, Rust, desktop, and all checks PASS. A post-evidence `cargo xtask check static` also exited 0. Exact command, duration, exit code, stdout/stderr path and SHA-256 are listed in [local-checks-r6.json](./local-checks-r6.json); raw logs are retained in [logs](./logs/).

## Evidence limits and remaining work

- Sender is mock/test-support. Real native interactive ReceiveOnly Compare/Final has not been run or proven by this batch.
- Handoff and stale timing evidence use deterministic/mock transport; `statistics_eligible` is false, so these measurements do not establish release qualification or threshold acceptance.
- CI was not awaited. Required GitHub CI and hosted signing remain external review gates.
- No merge, PR readiness transition, issue closure, or Source Accepted declaration is part of this implementation batch.