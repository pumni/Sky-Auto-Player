# Gate B / issue #312 evidence

Status: Gate B only. Gate C and Gate D were not started.

## Scope and exact revisions

- Required comparison base: `170be56e829507fb20f0eff16d65c90de6ebd315`
- Gate-B production implementation head used for every benchmark run: `78d1b776e548d59276a0f79a3859802a6641d43e`
- Production commits: `037e06e8749ba05af5b51cbfcba84d7fb42632df`, `78d1b776e548d59276a0f79a3859802a6641d43e`
- Toolchain: Rust 1.98.1 MSVC (`rustc 1.98.1 (48a229cea 2026-09-01)`)
- Host: same Windows laptop, same power/setup for all base/head runs

Gate B removes dead normal prepared timing-window plumbing. It does not change
authored targets, pause epochs, same-key validation, normal late delivery,
completion feedback policy, waiter policy, spin policy, MMCSS, priority,
affinity, power policy, watchdog cadence, focus cadence, or transport retry
policy.

## Normal path after Gate B

The prepared normal branch now derives only the immutable absolute target:

```text
prepared frame offset
  -> playback epoch + offset = authored physical target
  -> target/focus preflight
  -> HybridWaiter
  -> final atomic gates
  -> prepared sender
  -> post-send commit and telemetry
```

The normal prepared branch no longer constructs a `PhysicalTimingWindow` before
the wait, adds `timing_margin_ticks` to the target, queries a latest Down start,
or queries physical hold/release floors. `dispatch_prepared_normal_frame()` and
`record_prepared_normal_send_outcome()` no longer accept a full strict timing
window. The shared post-send accounting function accepts an optional window:
strict/dynamic callers pass the real window, while normal successful Down
accounting materializes `PhysicalTimingWindow::authored_only(target)` only
after SendInput has returned for observer/schema compatibility.

That authored-only diagnostic view is exactly:

```text
authored_target_qpc       = target
musical_up_not_before_qpc = target
down_not_before_qpc       = target
packet_not_before_qpc     = target
latest_down_start_qpc     = None
hold_floor_mask           = 0
release_floor_mask        = 0
```

Normal completion evidence remains serialized. `record_physical_floor_delays`
is retained for strict/dynamic evidence and is not meaningfully entered by the
normal authored-only view because both floor masks are zero.

The obsolete `normal_prepared_timing_window()` helper and its test-support
re-export were removed. `Timing Margin` remains in admission geometry and in
strict timing behavior.

## Strict separation

Strict/dynamic code still uses `PhysicalTimingGuard::query()` and the complete
`PhysicalTimingWindow`. The strict query still derives the authored target,
completion-relative floors, and `latest_down_start_qpc = target + margin` for a
Down. Strict sender cutoff and strict `DownExpiredBeforeSend` /
`PhysicalWindowExpired` recovery remain unchanged.

Relevant passing tests include:

- `c1_strict_cutoff_remains_physical_latest_start`
- `strict_timing_retains_physical_latest_start_rejection`
- `physical_timing_guard` latest-start/floor tests
- `authored_only_window_ignores_completion_floors`
- `prepared_normal_observer_uses_authored_only_timing_evidence`
- same-key legal retrigger and invalid pre-session geometry tests

## Source/static precision proof

The prepared source test `prepared_normal_branch_does_not_build_timing_window_before_wait`
checks the source segment from prepared-frame cursor/target derivation to the
wait handoff. That segment contains no `PhysicalTimingWindow`,
`latest_down_start_qpc`, `timing_margin_ticks`, or physical guard query. The
normal precision source audit separately checks that the pre-SendInput region
and immediate sender handoff contain no planner, coordinator, recovery,
pending-release traversal, allocation, lock, foreground syscall, lease math,
or formatting/string construction.

The normal precision helper still has only the intended final control/target
atomic gates and the prepared sender handoff. Down keeps its early and late
control gates; Up-only retains one final control gate and no focus dependency.

## Optimized dist assembly audit

Commands:

```text
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_player --profile dist --lib -- --emit=asm
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --profile dist --lib -- --emit=asm
rtk powershell -NoProfile -File scripts/audit_dispatch_assembly.ps1 -AssemblyPath rust/target/dist/deps/sky_player-6da0da9f341438e6.s
rtk rg -n "normal_prepared_timing_window|timing_margin_ticks|latest_down_start_qpc|send_prepared_normal_precision_frame|send_prepared_physical_packet_at_final_boundary|SetLastError|SendInput" rust/target/dist/deps/sky_player-6da0da9f341438e6.s rust/target/dist/deps/sky_dispatch_win32-ae48fb7937aa59a1.s
```

Generated artifacts:

- `rust/target/dist/deps/sky_player-6da0da9f341438e6.s`
- `rust/target/dist/deps/sky_dispatch_win32-ae48fb7937aa59a1.s`

The audit passed. The optimized shipping caller had zero retained
`send_prepared_normal_precision_frame` calls and one direct prepared sender
call. The Win32 sender assembly preserves the prepared payload handoff,
`SetLastError(0)`, authoritative `QueryPerformanceCounter`, one `SendInput`,
and completion QPC. The optimized shipping assembly contains no normal helper
or `timing_margin_ticks` symbol; the source/transitive audit is the proof for
the semantic absence of the removed target-plus-margin window construction.
The existing assembly report still includes dynamic/strict caller material
because those paths remain intentionally live.

## Deterministic acceptance

The prepared baseline retains the required aggregate:

| metric | result |
|---|---:|
| prepared physical boundaries | 16 |
| successful sends | 14 |
| intentional non-send/drop | 2 |
| normal `DownExpiredBeforeSend` | 0 |
| normal `UnobservedBacklog` | 0 |
| completion-feedback `PhysicalWindowExpired` | 0 |
| timeline rebases | 0 |
| clean real-wait transport anomalies | 0 |

The two deterministic non-sends remain the control-stop race and injected
`ZeroProgress`. The aggregate deterministic harness transport counter includes
the intentionally injected fault; the real-wait transport anomaly counter is
zero in every eligible host run. Legal same-key retrigger remains authored and
prepared as `Down(K) -> Up(K) -> Down(K) -> Up(K)`. Invalid same-key geometry
continues to fail before realtime worker execution.

The completion control/delayed pair continues to prove different completion
latency for packet N with identical packet N+1 authored target, physical target,
wait target, packet-not-before, send eligibility, classification, and zero
normal floor masks.

## Counterbalanced 5-pair real-wait A/B

Every run used the corrected benchmark ownership: prepared stream construction
finishes before target alignment, target alignment precedes wait entry, and
startup preparation is reported separately from musical lateness.

Command used for every run:

```text
rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=10000&& set RT_HANDOFF_BENCH_SCOPE=baseline&& cargo run --profile dist --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json>"
```

Exact chronological order:

```text
H1 -> B1 -> B2 -> H2 -> H3 -> B3 -> B4 -> H4 -> H5 -> B5
H -> B, B -> H, H -> B, B -> H, H -> B
```

All ten runs had `iterations=10000`, `statistics_eligible=true`,
`acceptance_clean=true`, `prepared=16`, `successful=14`, `non-send=2`,
real-wait non-dispatch `0`, overdue `0`, and transport anomaly `0`.
Values below are microseconds.

### `pre_call_qpc - target_qpc` and `completion_qpc - pre_call_qpc`

| chronological run | revision | pre p50 | pre p95 | pre p99 | pre p99.9 | pre max | completion p50 | completion p95 | completion p99 | completion p99.9 | completion max |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| H1 | head | 3 | 49 | 1366 | 9096 | 33882 | 1 | 1 | 2 | 6 | 2437 |
| B1 | base | 3 | 5 | 17 | 449 | 11088 | 1 | 1 | 1 | 6 | 48 |
| B2 | base | 3 | 5 | 6 | 208 | 4734 | 0 | 1 | 1 | 2 | 11 |
| H2 | head | 2 | 5 | 11 | 166 | 478 | 0 | 1 | 1 | 1 | 10 |
| H3 | head | 20 | 392 | 552 | 1024 | 25915 | 0 | 1 | 1 | 11 | 150 |
| B3 | base | 4 | 157 | 306 | 485 | 661 | 1 | 1 | 1 | 5 | 19 |
| B4 | base | 4 | 173 | 334 | 545 | 1290 | 1 | 1 | 1 | 2 | 13 |
| H4 | head | 3 | 34 | 194 | 356 | 407 | 1 | 1 | 1 | 3 | 202 |
| H5 | head | 3 | 5 | 5 | 128 | 320 | 0 | 1 | 1 | 2 | 56 |
| B5 | base | 3 | 5 | 6 | 141 | 292 | 1 | 1 | 2 | 2 | 25 |

### Raw timing decomposition

Columns are p99 / p99.9 / max in microseconds. `wake` is target-to-wake,
`wake-final` is wake-to-final-policy, `final-pre` is final-policy-to-pre-call,
and `ready` is the completion-to-next-ready/wait-entry proxy.

| run | revision | wake | wake-final | final-pre | ready | non-dispatch / overdue / transport |
|---|---|---|---|---|---|---:|
| H1 | head | 1361 / 9090 / 33878 | 9 / 108 / 1006 | 0 / 0 / 0 | 10 / 119 / 888 | 0 / 0 / 0 |
| B1 | base | 10 / 446 / 11083 | 5 / 12 / 184 | 0 / 0 / 0 | 5 / 13 / 492 | 0 / 0 / 0 |
| B2 | base | 0 / 206 / 4730 | 5 / 12 / 31 | 0 / 0 / 0 | 5 / 13 / 164 | 0 / 0 / 0 |
| H2 | head | 4 / 163 / 473 | 5 / 14 / 20 | 0 / 0 / 0 | 4 / 10 / 55 | 0 / 0 / 0 |
| H3 | head | 548 / 1021 / 25913 | 5 / 89 / 143 | 0 / 0 / 0 | 6 / 104 / 164 | 0 / 0 / 0 |
| B3 | base | 302 / 482 / 658 | 5 / 14 / 104 | 0 / 0 / 0 | 6 / 18 / 97 | 0 / 0 / 0 |
| B4 | base | 330 / 542 / 1288 | 5 / 15 / 217 | 0 / 0 / 0 | 5 / 15 / 16593 | 0 / 0 / 0 |
| H4 | head | 190 / 352 / 404 | 5 / 14 / 125 | 0 / 0 / 0 | 5 / 13 / 74 | 0 / 0 / 0 |
| H5 | head | 0 / 114 / 317 | 5 / 12 / 201 | 0 / 0 / 0 | 5 / 9 / 19 | 0 / 0 / 0 |
| B5 | base | 0 / 119 / 288 | 5 / 13 / 194 | 0 / 0 / 0 | 5 / 13 / 103 | 0 / 0 / 0 |

The target-to-wake distribution contains host-sensitive outliers on both
revisions (notably H1/H3 and B1/B4). The post-wake final-policy-to-pre-call
component is zero in every run, and wait-entry slack stayed at 9,999 us p50,
p95, p99, and p99.9 in every run. The paired data therefore does not establish
a systematic Gate-B timing regression attributable to the removed arithmetic;
it establishes delivery-clean behavior with host-wake noise. No scheduler
tuning was performed.

Startup-only prepared stream build evidence was kept separate from musical
lateness. Across these runs its p50 was 11–16 us, p95 16–22 us, p99 17–40 us,
and max 1067 us; it was never charged to a per-boundary target.

Raw reports, in chronological order, are:

`.benchmarks/gate-b-final-H1.json`, `.benchmarks/gate-b-final-B1.json`,
`.benchmarks/gate-b-final-B2.json`, `.benchmarks/gate-b-final-H2.json`,
`.benchmarks/gate-b-final-H3.json`, `.benchmarks/gate-b-final-B3.json`,
`.benchmarks/gate-b-final-B4.json`, `.benchmarks/gate-b-final-H4.json`,
`.benchmarks/gate-b-final-H5.json`, `.benchmarks/gate-b-final-B5.json`.

## Verification commands and results

Passed:

```text
rtk cargo fmt --manifest-path rust/Cargo.toml --all
rtk cargo test --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support
rtk cargo test --manifest-path rust/Cargo.toml -p sky_player --test rt_dispatch_no_alloc --features test-support
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_player --profile dist --lib -- --emit=asm
rtk cargo rustc --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --profile dist --lib -- --emit=asm
rtk powershell -NoProfile -File scripts/audit_dispatch_assembly.ps1 -AssemblyPath rust/target/dist/deps/sky_player-6da0da9f341438e6.s
rtk cargo xtask check static
rtk cargo xtask check rust
```

The focused/full `sky_player` lib suite passed with 307 tests. The no-allocation
suite passed with 23 tests. `cargo xtask check static` passed with only the
repository's existing allowlisted architecture warnings. `cargo xtask check
rust` passed fmt, clippy, and the locked workspace test phase.

`cargo xtask check all` remains to be run on the final documentation head.
Required GitHub CI is also pending the single PR push. No Gate C/D work or
scheduler tuning was performed.
