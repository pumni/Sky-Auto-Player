# Gate A / issue #312 evidence

Status: Gate A only. Gate B/C/D were not started.

## Scope and base

- Required base: `05434a0e3bd9a5eb1eaeca0b607357325da1d834`
- Gate-A implementation commit before this evidence document: `ba713f8`
- Scope: structural hot-path cleanup only; no scheduler, wait policy, timing geometry, focus cadence, watchdog cadence, MMCSS, affinity, power, or transport-policy changes.

The normal prepared dispatcher semantics remain: immutable prepared frame, absolute target, HybridWaiter and calibrated spin, cheap final control/target/focus checks, authoritative pre-call QPC, one prepared SendInput transaction, completion evidence, and bounded post-send commit.

## Candidates

### A1 — precision call boundary

Removed the production `inline(never)` attribute from `send_prepared_normal_precision_frame`. ThinLTO is allowed to inline or retain the helper naturally. The assembly audit no longer requires the helper symbol to remain; it audits the optimized shipping caller and the inlined sender region.

### A2 — panic read-mostly path

Healthy control iterations use an Acquire load. The clearing RMW is reached only when the flag is observed set (or the explicit command-exit path requires consumption). Existing terminal identity and interrupt behavior are preserved:

- user panic: `panic_release_requested`;
- watchdog expiry: `supervisor_lease_expired`.

The deterministic source/race tests cover a panic arriving after an earlier false sample and prove that the final control gate still suppresses dispatch.

### A3 — system-power read-mostly path

`take_pending()` now performs an Acquire load and returns immediately when no pending bit is present. `fetch_and` is used only when pending bits are observed. Suspend-before-arm, long-wait, final-gate, resume, and pending-bit preservation tests remain covered.

### A4 — Up-only single admission

Down-bearing frames retain the early control gate, target/focus admission, and late control gate. Up-only frames have one final control gate immediately before the sender and do not acquire a focus dependency. Deterministic zero-send coverage exists for quit, skip, pause, panic, watchdog, system suspend, and a final-gate race.

### A5 — prepared stream exhaustion

An exhausted prepared stream with an unfinished coordinator is now a terminal invariant error. It cannot fall through to `plan_next_dispatch_projected()`. Strict/diagnostic dynamic planning remains available where it is still part of that path.

### A6 — outer focus read

The outer focus read was intentionally retained. It is a cheap atomic read and preserves the distinct preroll/focus-pause behavior and test-support focus transition path. The precision helper remains authoritative for final Down admission. No speculative focus cleanup was made.

## Deterministic regression

The prepared baseline reports retain the expected aggregate:

| metric | result |
|---|---:|
| prepared physical boundaries | 16 |
| successful sends | 14 |
| intentional non-send/drop | 2 |
| normal `DownExpiredBeforeSend` | 0 |
| normal `UnobservedBacklog` | 0 |
| completion-feedback `PhysicalWindowExpired` | 0 |
| timeline rebases | 0 |
| real-wait transport anomalies | 0 |

The two non-sends are the intentional control stop and injected `ZeroProgress`. The deterministic report's transport-anomaly counter includes the expected injected fault only; the real-wait host run has zero transport anomalies.

Coverage retained: legal same-key retrigger, invalid pre-session geometry, control races for panic/watchdog/quit/skip/pause/focus/suspend/target stamp, and ZeroProgress/partial/integrity fail-closed transport behavior.

## Source and optimized assembly proof

Source checks cover the normal precision helper and its transitive immediate sender path. The pre-SendInput region contains no coordinator, planner, recovery, pending-release traversal, allocation, lock, foreground syscall, lease arithmetic, or formatting/string construction. The helper accepts no coordinator/planner/recovery state.

Commands:

```text
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_player --profile dist --lib -- --emit=asm
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --profile dist --lib -- --emit=asm
rtk powershell -NoProfile -File scripts/audit_dispatch_assembly.ps1 -AssemblyPath rust/target/dist/deps/sky_player-6da0da9f341438e6.s
```

Toolchain: Rust 1.98.1, Windows MSVC, `dist` profile.

Observed optimized files:

- `rust/target/dist/deps/sky_player-6da0da9f341438e6.s`
- `rust/target/dist/deps/sky_dispatch_win32-ae48fb7937aa59a1.s`

The optimized shipping caller has zero retained `send_prepared_normal_precision_frame` call references and one direct prepared sender call. The compiler-generated caller contains only conditional atomic RMWs for the hard-stop paths; there is no unconditional healthy-path panic or power consume RMW. The Win32 sender region preserves `SetLastError(0)`, the authoritative QPC call, exactly one `SendInput`, and completion QPC. The audit reports the actual generated assembly rather than requiring a retained helper symbol.

## Counterbalanced real-wait A/B

Benchmark command for every run:

```text
rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=10000&& set RT_HANDOFF_BENCH_SCOPE=baseline&& cargo run --profile dist --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json>"
```

Order: `head-1 -> base-1 -> head-2 -> base-2 -> head-3 -> base-3 -> head-4 -> base-4 -> head-5 -> base-5`.

All runs had 10,000 observations, `statistics_eligible=true`, `acceptance_clean=true`, real-wait non-dispatch 0, overdue 0, and real-wait transport anomaly 0.

### `pre_call_qpc - target_qpc` (microseconds)

| run | p50 | p95 | p99 | p99.9 | max |
|---|---:|---:|---:|---:|---:|
| head-1 | 2 | 5 | 897 | 4849 | 18335 |
| base-1 | 2 | 10 | 1460 | 4028 | 14437 |
| head-2 | 2 | 148 | 995 | 3290 | 5079 |
| base-2 | 3 | 52 | 616 | 3210 | 6057 |
| head-3 | 2 | 5 | 838 | 3159 | 9087 |
| base-3 | 35 | 369 | 735 | 3290 | 27502 |
| head-4 | 3 | 6 | 1079 | 2671 | 7377 |
| base-4 | 2 | 7 | 1295 | 4322 | 6430 |
| head-5 | 2 | 6 | 184 | 457 | 11164 |
| base-5 | 2 | 4 | 5 | 752 | 8223 |

### `completion_qpc - pre_call_qpc` (microseconds)

| run | p50 | p95 | p99 | p99.9 | max |
|---|---:|---:|---:|---:|---:|
| head-1 | 0 | 1 | 2 | 20 | 378 |
| base-1 | 0 | 1 | 2 | 13 | 192 |
| head-2 | 0 | 1 | 2 | 8 | 29 |
| base-2 | 0 | 1 | 2 | 9 | 239 |
| head-3 | 0 | 1 | 3 | 26 | 205 |
| base-3 | 0 | 1 | 2 | 18 | 320 |
| head-4 | 0 | 1 | 2 | 16 | 1152 |
| base-4 | 0 | 1 | 3 | 24 | 1979 |
| head-5 | 0 | 1 | 1 | 2 | 12 |
| base-5 | 0 | 1 | 1 | 2 | 151 |

Each report also retains target-to-wake, wake-to-final-policy, final-policy-to-pre-call, completion-to-ready (the available completion-to-next-ready proxy), non-dispatch, overdue, and transport-anomaly fields. The samples are noisy and do not support a scheduler-tuning or universal improvement claim. There is no attributable delivery regression or systematic new tail regression in this Gate-A comparison.

Raw reports:

`.benchmarks/gate-a-head-1.json`, `.benchmarks/gate-a-base-1.json`, `.benchmarks/gate-a-head-2.json`, `.benchmarks/gate-a-base-2.json`, `.benchmarks/gate-a-head-3.json`, `.benchmarks/gate-a-base-3.json`, `.benchmarks/gate-a-head-4.json`, `.benchmarks/gate-a-base-4.json`, `.benchmarks/gate-a-head-5.json`, `.benchmarks/gate-a-base-5.json`.

## Focused workload A/B

Corrected command (the initial invocation omitted the required mode and is not a product result):

```text
rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=1000&& set RT_HANDOFF_BENCH_SCOPE=phase_a_production_matrix&& cargo run --profile dist --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json> --mode phase_a_production_boundary"
```

Both final head and base focused reports ran 1,000 iterations per scenario under controlled CPU contention (two hidden busy workers), with all delivery/control/transport anomaly counts zero. This direct-crossing scope is not statistics-eligible because it intentionally excludes the waiter.

| workload | head p99 / p99.9 / max (us) | base p99 / p99.9 / max (us) | delivery |
|---|---:|---:|---|
| Up-only-heavy, 15 keys | 101 / 105 / 110 | 101 / 118 / 119 | 1000/1000 |
| 15-key chord | 104 / 144 / 147 | 103 / 109 / 124 | 1000/1000 |
| dense mixed alternating, 14 events | 102 / 121 / 150 | 102 / 111 / 160 | 1000/1000 |

Focused reports: `.benchmarks/gate-a-head-focused.json` and `.benchmarks/gate-a-base-focused.json`.

## Verification

Passed before PR creation:

```text
rtk cargo fmt --manifest-path rust/Cargo.toml --all
rtk cargo test --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_player --profile dist --lib -- --emit=asm
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --profile dist --lib -- --emit=asm
rtk powershell -NoProfile -File scripts/audit_dispatch_assembly.ps1 -AssemblyPath rust/target/dist/deps/sky_player-6da0da9f341438e6.s
rtk git diff --check
rtk cargo test --manifest-path rust/Cargo.toml -p sky_player --test rt_dispatch_no_alloc --features test-support
rtk cargo xtask check static
rtk cargo xtask check rust
rtk cargo xtask check all
```

The library test suite passed with 305 tests. The no-allocation suite passed with 23 tests. `cargo xtask check static`, `cargo xtask check rust`, and `cargo xtask check all` all passed; `check all` also passed the desktop checks and end-to-end test phase. The initial focused benchmark invocation without `--mode phase_a_production_boundary` failed at the harness argument contract and was corrected; it was not counted as a product result.

No production timing policy was tuned. No Gate B/C/D work was started.
