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

This is the fresh final-head confirmation. Production code is unchanged from `75f2eb3f84b77966b504a472144ca299d02eb456`; this is an evidence-only revision.

Benchmark command for every run:

```text
rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=10000&& set RT_HANDOFF_BENCH_SCOPE=baseline&& cargo run --profile dist --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json>"
```

Toolchain and host were unchanged: Rust 1.98.1 MSVC, same laptop, same power/setup, `--profile dist`, corrected production-calibrated real-wait benchmark. Exact chronological order and pair directions:

```text
H1 -> B1 -> B2 -> H2 -> H3 -> B3 -> B4 -> H4 -> H5 -> B5
H -> B, B -> H, H -> B, B -> H, H -> B
```

All ten runs had 10,000 observations, `statistics_eligible=true`, `acceptance_clean=true`, real-wait non-dispatch 0, overdue 0, and real-wait transport anomaly 0. Worker CPU time was not available in this benchmark report.

### `pre_call_qpc - target_qpc` (microseconds)

| run | base/head | p50 | p95 | p99 | p99.9 | max |
|---|---|---:|---:|---:|---:|---:|
| H1 | head | 2 | 20 | 220 | 2736 | 14276 |
| B1 | base | 3 | 133 | 375 | 1674 | 8201 |
| B2 | base | 2 | 4 | 28 | 1222 | 11487 |
| H2 | head | 2 | 20 | 191 | 2824 | 17249 |
| H3 | head | 3 | 5 | 128 | 1664 | 19704 |
| B3 | base | 4 | 9 | 208 | 6870 | 30260 |
| B4 | base | 4 | 119 | 313 | 2870 | 49280 |
| H4 | head | 38 | 396 | 604 | 1258 | 3123 |
| H5 | head | 4 | 5 | 128 | 385 | 4648 |
| B5 | base | 4 | 6 | 42 | 1285 | 14157 |

### `completion_qpc - pre_call_qpc` (microseconds)

| run | base/head | p50 | p95 | p99 | p99.9 | max |
|---|---|---:|---:|---:|---:|---:|
| H1 | head | 0 | 1 | 2 | 10 | 253 |
| B1 | base | 0 | 1 | 2 | 18 | 133 |
| B2 | base | 0 | 1 | 2 | 3 | 5 |
| H2 | head | 0 | 1 | 1 | 2 | 13 |
| H3 | head | 0 | 1 | 1 | 2 | 43 |
| B3 | base | 1 | 1 | 2 | 6 | 277 |
| B4 | base | 1 | 1 | 1 | 4 | 109 |
| H4 | head | 1 | 1 | 1 | 81 | 134 |
| H5 | head | 1 | 1 | 2 | 3 | 91 |
| B5 | base | 1 | 1 | 1 | 2 | 274 |

### Raw decomposition for every run

Values are microseconds. `wake` is target-to-wake lateness; `wake-final` is wake-to-final-policy; `final-pre` is final-policy-to-pre-call; `ready` is the available completion-to-next-ready/wait-entry proxy; `slack` is `target_minus_wait_entry`.

| run | wake p99 / p99.9 / max | wake-final p99 / p99.9 / max | final-pre p99 / p99.9 / max | ready p99 / p99.9 / max | slack p50 / p95 / p99 / p99.9 / min | non-dispatch / overdue / transport |
|---|---|---|---|---|---|---:|
| H1 | 216 / 2730 / 14273 | 6 / 34 / 822 | 0 / 0 / 0 | 8 / 26 / 284 | 9999 / 9999 / 9999 / 9999 / 9975 | 0 / 0 / 0 |
| B1 | 366 / 1630 / 7651 | 9 / 90 / 7982 | 0 / 0 / 0 | 10 / 49 / 424 | 9999 / 9999 / 9999 / 9999 / 9711 | 0 / 0 / 0 |
| B2 | 15 / 1213 / 11483 | 6 / 28 / 282 | 0 / 0 / 0 | 8 / 22 / 453 | 9999 / 9999 / 9999 / 9999 / 9984 | 0 / 0 / 0 |
| H2 | 188 / 2822 / 17245 | 4 / 12 / 98 | 0 / 0 / 0 | 5 / 10 / 72 | 9999 / 9999 / 9999 / 9999 / 9970 | 0 / 0 / 0 |
| H3 | 124 / 1661 / 19700 | 5 / 12 / 118 | 0 / 0 / 0 | 6 / 21 / 454 | 9999 / 9999 / 9999 / 9999 / 9986 | 0 / 0 / 0 |
| B3 | 203 / 6865 / 30254 | 5 / 26 / 168 | 0 / 0 / 0 | 6 / 24 / 238 | 9999 / 9999 / 9999 / 9999 / 9836 | 0 / 0 / 0 |
| B4 | 308 / 2866 / 49276 | 5 / 59 / 326 | 0 / 0 / 0 | 6 / 77 / 507 | 9999 / 9999 / 9999 / 9999 / 9882 | 0 / 0 / 0 |
| H4 | 593 / 1254 / 3120 | 5 / 136 / 212 | 0 / 0 / 0 | 6 / 124 / 165 | 9999 / 9999 / 9999 / 9999 / 9935 | 0 / 0 / 0 |
| H5 | 124 / 381 / 4628 | 5 / 19 / 176 | 0 / 0 / 0 | 6 / 30 / 329 | 9999 / 9999 / 9999 / 9999 / 9949 | 0 / 0 / 0 |
| B5 | 36 / 1282 / 14152 | 6 / 14 / 65 | 0 / 0 / 0 | 5 / 14 / 164 | 9999 / 9999 / 9999 / 9999 / 9667 | 0 / 0 / 0 |

The paired comparison is directionally mixed: head p99 is lower in 2/5 pairs and higher in 3/5; head p99.9 is lower in 3/5 and higher in 2/5; head max is lower in 3/5 and higher in 2/5. This is not a systematic head disadvantage in either direction. The observed series is host-noise sensitive, with no delivery regression, no systematic new central/tail regression attributable to Gate A, and no scheduler tuning justified. `final-policy-to-pre-call` remains zero in every run.

Raw reports, in chronological order:

`.benchmarks/gate-a-final-H1.json`, `.benchmarks/gate-a-final-B1.json`, `.benchmarks/gate-a-final-B2.json`, `.benchmarks/gate-a-final-H2.json`, `.benchmarks/gate-a-final-H3.json`, `.benchmarks/gate-a-final-B3.json`, `.benchmarks/gate-a-final-B4.json`, `.benchmarks/gate-a-final-H4.json`, `.benchmarks/gate-a-final-H5.json`, `.benchmarks/gate-a-final-B5.json`.

The earlier all-`H -> B` series (`gate-a-head-1` through `gate-a-base-5`) is historical/non-qualifying evidence and is not mixed into this final-head confirmation or its comparison.

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
