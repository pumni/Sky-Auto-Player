# Rust key-dispatch optimization audit — 2026-08-28

## Scope and provenance

- Starting HEAD: `72634af45ac5b62dbab63fcbeeaf61f4e9697d9d`.
- Starting worktree: clean; HEAD matched the requested baseline, so no related newer
  diff required review.
- Toolchain: `rustc 1.98.0 (88d9e12ae 2026-08-18)` / Cargo `1.98.0`.
- QPC: `10,000,000 Hz` on the benchmark host.
- Final source changes are limited to `sky_dispatch_core/Cargo.toml` and `rust/Cargo.lock`.
  No production dispatch source, timing constant, packet path, retry policy, or Win32
  boundary changed.

## Audit summary

The canonical physical path remains prepared packet → hybrid wait → bounded calibrated
spin → final gate → authoritative sender-side QPC → one packetized `SendInput` call →
completion QPC → commit/cleanup. The existing tests confirm that lease-only wakes use
zero spin and exact physical-target wakes retain the frozen calibrated threshold.

The hot path already satisfies the important structural constraints observed here:
the no-allocation suite passed 20/20, the canonical sender audit found no new copy or
division helper in the required clean target bodies, and no blocking lock, retry, or
additional input mechanism was introduced.

## Dependency audit and changes

Production usage was checked against all five requested manifests:

| Crate | Production direct dependencies observed | Result |
| --- | --- | --- |
| Workspace `rust/Cargo.toml` | No direct dependencies; workspace/profile/toolchain declaration only | No dependency change needed. |
| `sky_dispatch_core` | `serde`, `smallvec`, `thiserror` | `serde_json` was only used by golden tests and benchmarks; moved to `dev-dependencies`. |
| `sky_dispatch_win32` | workspace core, `serde`, `serde_json`, `smallvec`, `thiserror`, `windows-sys` | No unused production direct dependency found. |
| `sky_player_rs` | workspace crates, `pyo3`, `serde`, `serde_json`, `smallvec`, `crossbeam-queue`, `parking_lot`, `thiserror` | No unused production direct dependency found. |
| `sky_updater` | `pep440_rs`, `serde`, `serde_json`, `sha2`, `thiserror`, `zip`, `windows-sys` | No unused production direct dependency found. `zip` remains `2.4.2`. |

Implemented:

1. `rust/crates/sky_dispatch_core/Cargo.toml`: moved `serde_json = "1"` from
   `[dependencies]` to `[dev-dependencies]`. This is build/dependency hygiene, not a
   runtime speed claim.
2. `rust/Cargo.lock`: updated `thiserror` and `thiserror-impl` from `2.0.19` to
   `2.0.20`; no other lockfile package changed.

The official PyO3 release list still identifies `0.29.0` as the current 0.29 release,
so the exact `pyo3 = "=0.29.0"` constraint was not changed. The official thiserror
release list identifies `2.0.20` as the newer patch release:
[PyO3 releases](https://github.com/PyO3/pyo3/releases),
[thiserror releases](https://github.com/dtolnay/thiserror/releases).

## Verification

| Check | Result |
| --- | --- |
| Baseline `uv run python scripts/check.py rust` | PASS: format, check, Clippy, workspace tests. |
| Candidate `uv run python scripts/check.py rust` | PASS: format, check, Clippy, workspace tests. |
| Candidate `uv run python scripts/check.py static` | PASS: ruff, pyright, context, security, architecture. |
| Candidate `uv run python scripts/check.py tests` | PASS: 946 passed, 14 skipped, 1 xfailed. Elevated run was required because sandbox ACL blocked pytest temp/AppData cleanup. |
| `cargo check --workspace --all-targets --locked` | PASS; without `--all-features`, two pre-existing test-only warnings are emitted. |
| `rt_dispatch_no_alloc` | PASS: 20/20; canonical production hard paths remain zero-allocation. |
| Release all-target build | PASS with `--release --locked`. |
| `scripts/audit_dispatch_assembly.ps1` | PASS. `dispatch_due_from_plan` and `recover_missed_down_boundary` remain free of reported division/copy helpers. |

## Benchmark evidence

The raw reports were generated with the same host, Rust compiler, QPC frequency, and
10,000 iterations. They are kept as local ignored artifacts under `.benchmarks/`:

- `baseline-phase-a-10k.txt`
- `candidate-phase-a-10k.txt`
- `baseline-real-wait-core-10k-due1ms.txt`
- `candidate-real-wait-core-10k-due1ms.txt`
- `experiment-cold-path-phase-a-10k.txt`

### Phase-A production-boundary matrix

This uses the full coordinator/admission/commit path with deterministic mock transport;
waiter scheduling and the real syscall are excluded. The aggregate was not clean in
either run because this tight direct-boundary workload exposed a small number of
deadline misses/observation gaps. Values below are worst across the nine scenarios.

| Metric | Baseline | Candidate | Absolute delta |
| --- | ---: | ---: | ---: |
| Startup-selected threshold (µs) | 1,000 | 753 | -247 |
| Process CPU duty | 97.081% | 96.667% | -0.414 pp |
| Process CPU time | 8,343,750 µs | 8,281,250 µs | -62,500 µs |
| Dispatch-start p95 | 121 µs | 122 µs | +1 µs |
| Dispatch-start p99 | 142 µs | 141 µs | -1 µs |
| Dispatch-start p99.9 | 213 µs | 277 µs | +64 µs |
| Dispatch-start max | 5,545 µs | 672 µs | -4,873 µs |
| Non-dispatch / cutoff misses | 3 / 3 | 2 / 2 | -1 / -1 |

The threshold is selected by a startup probe and changed between these runs, so this
single pair is not sufficient evidence of a timing improvement. It supports keeping
the current calibrated policy rather than replacing it with a fixed threshold.

### Real-wait core A/B

This run used 10,000 waits for each of six scenarios per mode, with a 1 ms due interval.
It is a HybridWaiter/QPC scheduling workload, not a 60-FPS game claim. All aggregate
reports were `statistics_eligible=false` because the host produced deadline misses.
The table reports worst scenario tails per mode; `miss` is non-dispatch/cutoff misses.

| Mode | CPU duty baseline → candidate | Start p95 / p99 / p99.9 (µs) | Spin p99 / p99.9 (µs) | Miss baseline → candidate |
| --- | ---: | ---: | ---: | ---: |
| Calibrated (`1000 → 730 µs`) | 60.516% → 38.012% | 34/265/3865 → 140/662/5344 | 996/1237 → 702/1602 | 51 → 262 |
| Fixed 400 µs | 22.551% → 22.494% | 325/1188/6689 → 284/620/1923 | 367/583 → 441/982 | 1849 → 459 |
| Fixed 700 µs | 35.676% → 36.105% | 140/290/822 → 120/406/1389 | 632/729 → 661/1011 | 45 → 132 |
| Fixed 1000 µs | 61.728% → 59.796% | 35/158/779 → 84/594/6037 | 991/1053 → 1047/1825 | 6 → 29 |

The calibrated threshold changed from 1,000 to 730 µs because the startup wake probe
changed (`robust_us` 1,201 → 680). Fixed controls also moved in both directions across
tail and miss metrics. There is no consistent candidate runtime win, and no production
spin threshold change is justified.

## Rust 1.98 experiment

Three `std::hint::cold_path()` annotations were tried at genuinely terminal branches
in `authored.rs`: missing crossing evidence, duration overflow, and missing authoritative
start evidence. The targeted timing test passed on rerun and the no-allocation gate
remained green, but the release assembly clean-target counts did not change and the
Phase-A result was within host/probe noise. The annotations were reverted.

Rejected/not performed: fixed production spin thresholds, adaptive lead or send-cost
compensation, fat LTO, new concurrency primitives, allocator changes, `zip` major
upgrade, `sha2` maintenance migration, and architectural rewrites. None had a measured
bottleneck in this audit.

## Real `SendInput` qualification

Not run. `SKY_NATIVE_TARGET_HWND` was unset and `.benchmarks/sink.json` was absent, so
there was no project-owned target window. The run was not redirected to an arbitrary
foreground application. Consequently this audit does not qualify the physical timestamp
chain (`physical_target_qpc`, `wake_qpc`, `final_policy_qpc`, sender pre-call QPC,
completion QPC) under an actual `SendInput` syscall, nor MMCSS/priority provenance or
game receipt.

## Risk and recommendation

- Correctness risk: low; only dependency classification and patch-level thiserror lock
  resolution changed, with all Rust correctness, security, architecture, and allocation
  gates green.
- Timing risk: no intentional runtime timing change. Existing host scheduling variance
  and tight-deadline misses remain visible.
- Windows residual risk: real `SendInput` duration, scheduler preemption near the syscall,
  and project-owned sink/game observation remain unqualified.

Recommendation: **MERGE NOW** for the dependency hygiene/patch update as a separate
chore. **KEEP CURRENT** for dispatch timing and calibrated wait policy. **INVESTIGATE
SEPARATELY** with the required interactive project-owned sink before making any timing
retune or claiming production physical-path improvement.
