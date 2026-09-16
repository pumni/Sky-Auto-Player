# Issue #300 evidence

Base: `9a32f0b8ce20d26e0fa31e48353ab6abd0822b3a`

Implementation revision head used for the evidence runs:
`9d19a7be0f37b55ed323f815fad0960fb1686f8a`

## Prepared-stream suspension reconciliation

- The normal stream is still built once from the existing coordinator/compiler
  preparation contract before worker startup-ready.
- `suspend_live_input()` returns the generation IDs it explicitly cancels.
  The prepared stream records those IDs in a startup-sized bounded ledger.
- A later frozen Up may bypass only when its generation is both in that ledger
  and still marked `Cancelled`; genuine ownership mismatches remain terminal
  coordinator errors.
- The shared reconciliation is exercised through the lower-level
  `suspend_live_input()` path and through a native prepared session:
  `Down(K) -> Up(K) -> Down(J)`. After suspension/resume, the frozen Up and
  sentinel Down both send, active/possibly-active state is safe, offsets are
  unchanged, and timeline rebases remain zero.

## Precision envelope

`send_prepared_normal_precision_frame()` is the clearly named normal suffix.
It accepts the immutable prepared frame, target/focus/control atomics, target
proof, system-power state, and tracked backend; it does not accept a
`RuntimeDispatchCoordinator`, planner, pending-release state, or recovery
state. It performs the final atomic gates and exactly one prepared sender
transaction. Frozen coordinator commit, active-key accounting, telemetry, and
cursor advancement start only after the helper returns.

The transitive source test checks the helper and its immediate Win32 sender
callee for forbidden coordinator/planner/recovery/allocation/lock references,
and checks `SetLastError(0) -> clock.now() -> SendInput(...)` in the prepared
Win32 input envelope.

The prepared stream retains immutable physical frames only for real
`SendInput` boundaries; metadata-only entries do not inflate the physical
count. Physical masks are disjoint and `deferred_up_mask == 0`.

## Deterministic acceptance

The prepared normal acceptance matrix passed semantic assertions, not only
counts, for:

`single_note`, `maximum_valid_atomic_chord`,
`dense_different_key_boundaries`, `legal_same_key_sequence`,
`late_wake_inside_normal_tolerance`, `late_wake_outside_normal_tolerance`,
`late_first_boundary_pressures_following_boundary`,
`stop_interrupt_race_at_target`, `intentionally_injected_zero_progress_drop`,
`completion_floor_control`, and `completion_floor_delayed`.

The deterministic totals remain:

- prepared physical boundaries: 16;
- successful sends: 14;
- non-send/drop boundaries: 2;
- timeline rebases: 0;
- normal completion-feedback `PhysicalWindowExpired`: 0;
- normal scheduler-lateness `UnobservedBacklog`: 0;
- normal `DownExpiredBeforeSend`: 0.

The prepared control-race matrix covers watchdog/panic, quit, skip, pause,
focus loss, system suspend, and target-generation change. Each case has zero
sender attempts and no cursor advancement. ZeroProgress/partial/integrity
faults remain fail-closed. The legal same-key sequence remains immutable and
authored ordered.

The completion control/delayed pair retains different completion latency for
packet N (8 us versus 40,000 us) while reporting the same authored/prepared
schedule, packet N+1 target, packet-not-before, wait target, send eligibility,
and classification. Both normal hold/release floor masks are zero; delayed
completion does not move later scheduling, and no completion-feedback
`PhysicalWindowExpired` is produced.

## Five interleaved host A/B pairs

Toolchain: `rustc 1.98.1 (48a229cea 2026-09-01)`,
`cargo 1.98.1 (797e8a9bc 2026-08-05)`, QPC frequency 10,000,000 Hz.
The base worktree was detached at the exact base SHA and the head worktree
was the reviewed implementation revision. Runs used the same laptop, power
and setup, `RT_HANDOFF_BENCH_SCOPE=baseline`,
`RT_HANDOFF_BENCH_ITERATIONS=10000`, and the same command:

`rtk cargo run -p sky_player --features test-support --example rt_handoff_bench -- <report.json>`

Exact order was `base-1, head-1, base-2, head-2, base-3, head-3, base-4,
head-4, base-5, head-5`. Every run was acceptance-clean and statistics
eligible with 10,000 observations, host non-dispatches 0, overdue 0, and
transport anomalies 0. Every run also reported prepared 16, successful 14,
non-send 2, and timeline rebases 0.

`pre_call_qpc - target_qpc` (us):

| run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| base-1 | 35 | 215 | 365 | 558 | 1246 |
| head-1 | 20 | 48 | 163 | 397 | 687 |
| base-2 | 39 | 168 | 291 | 519 | 699 |
| head-2 | 20 | 172 | 312 | 543 | 794 |
| base-3 | 35 | 42 | 49 | 141 | 367 |
| head-3 | 20 | 91 | 215 | 407 | 598 |
| base-4 | 35 | 42 | 46 | 115 | 803 |
| head-4 | 166 | 583 | 712 | 967 | 1337 |
| base-5 | 34 | 42 | 47 | 140 | 356 |
| head-5 | 20 | 97 | 231 | 502 | 1281 |

`completion_qpc - pre_call_qpc` (us):

| run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| base-1 | 2 | 3 | 3 | 5 | 47 |
| head-1 | 2 | 3 | 3 | 5 | 56 |
| base-2 | 2 | 3 | 3 | 4 | 160 |
| head-2 | 2 | 3 | 3 | 5 | 24 |
| base-3 | 2 | 3 | 3 | 12 | 149 |
| head-3 | 2 | 2 | 3 | 6 | 20 |
| base-4 | 2 | 3 | 3 | 7 | 55 |
| head-4 | 2 | 3 | 3 | 4 | 61 |
| base-5 | 2 | 3 | 3 | 7 | 181 |
| head-5 | 2 | 2 | 3 | 4 | 15 |

The head maximum is higher in four pairs, so it was investigated rather than
discarded as noise. In the corresponding raw reports, head
`final_policy_to_pre_call_us` is `p99/p99.9/max = 0/0/0` in all five runs;
the head-4 and head-5 tails are present in `wake_lateness_us` and
`wake_to_final_policy_us`, not in the precision suffix. Head p99 is lower than
base in pair 1, while pair 3's base had an unusually quiet wake tail, and
completion-relative tails are not systematically worse. This supports host
wake noise rather than a Phase-6 sender-path tail
regression. No spin, MMCSS, affinity, timer, power, or scheduler policy was
changed.

## Optimized Windows assembly audit

Commands:

- `rtk cargo rustc -p sky_player --profile dist --lib -- --emit=asm`
- `rtk cargo rustc -p sky_dispatch_win32 --profile dist --lib -- --emit=asm`
- `rtk powershell -File scripts/audit_dispatch_assembly.ps1 -AssemblyPath rust/target/dist/deps/sky_player-6da0da9f341438e6.s`

Toolchain-generated optimized assembly artifacts:

- `rust/target/dist/deps/sky_player-6da0da9f341438e6.s`;
- `rust/target/dist/deps/sky_dispatch_win32-ae48fb7937aa59a1.s`.

The optimized shipping caller at assembly line 26972 calls the retained
`send_prepared_normal_precision_frame` symbol. The symbol body at lines
68305-68410 contains the final atomic checks followed by one call to
`TrackedKeyState::send_prepared_physical_packet_at_final_boundary`; no planner,
coordinator, pending-release, recovery, allocation, or lock call appears
before that sender call. The Win32 final sender symbol is at lines 25975-25998.
The prepared packet syscall path shows `SetLastError(0)` at line 55717,
`QueryPerformanceCounter` at 55724, one `SendInput` at 55765, and completion
QPC at 55776. The existing assembly script completed with exit code 0; its
legacy planner/recovery reports are report-only and are outside the normal
prepared suffix.

No `llvm-objdump` or `dumpbin` was installed on this host, so this audit uses
the exact optimized MSVC-target assembly emitted by `rustc`; source/transitive
and runtime no-allocation gates provide the complementary evidence.

## Verification

- `cargo test -p sky_player --features test-support --lib`: 298 passed;
- `cargo test -p sky_dispatch_core --lib`: 54 passed;
- `cargo test -p sky_dispatch_win32 --lib prepared --no-default-features`:
  10 passed;
- `cargo test -p sky_player --features test-support --test rt_dispatch_no_alloc`:
  23 passed;
- prepared precision source/transitive audit tests: passed;
- prepared suspension continuation, overdue delivery, legal same-key,
  completion no-feedback, transport-fault, and all prepared control-race tests:
  passed;
- `cargo xtask check static`: PASS (29 pre-existing/allowlisted architecture
  warnings);
- `cargo xtask check rust`: PASS (fmt, clippy `-D warnings`, all-features
  workspace tests);
- final GitHub required CI result: pending after the final evidence push.

No focus, completion-floor, late-send, HybridWaiter, watchdog, MMCSS, spin,
affinity, timer, or persisted schema policy was changed. Strict/diagnostic
dynamic dispatch and legacy recovery remain available.
