Closes #300

# Issue #300 evidence

Base: `9a32f0b8ce20d26e0fa31e48353ab6abd0822b3a`

Implementation revision head used for the evidence runs:
`3078d60f9ad79f9bd5825fe1342df4413722e1ce`

## Prepared-stream suspension reconciliation

- The normal stream is still built once from the existing coordinator/compiler
  preparation contract before worker startup-ready.
- `suspend_live_input()` returns the generation IDs it explicitly cancels.
  The prepared stream records those IDs in a startup-sized bounded ledger.
- A later frozen Up may bypass only when its generation is both in that ledger
  and still marked `Cancelled`; genuine ownership mismatches remain terminal
  coordinator errors.
- Matching that frozen Up now performs a dedicated, non-generic
  `Cancelled -> Released` reconciliation. It verifies the generation ledger
  location, packet, slot, and exact `Cancelled` state, moves the counters to
  `released`, and leaves duplicate, unlisted, wrong-slot, or wrong-state
  commits rejected.
- The shared reconciliation is exercised through the lower-level
  `suspend_live_input()` path and through a native prepared session:
  `Down(K) -> Up(K) -> Down(J)`. After suspension/resume, the frozen Up and
  sentinel Down both send, active/possibly-active state is safe, offsets are
  unchanged, and timeline rebases remain zero.
- A four-action native regression (`Down(K) -> Up(K) -> Down(J) -> Up(J)`) now
  resumes without an explicit quit and reaches natural `finished` completion:
  `released == generation_count`, `cancelled == 0`, `scheduled == 0`, no
  terminal error, clean backend release state, and zero timeline rebases.

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

## Host A/B timing evidence

Toolchain: `rustc 1.98.1 (48a229cea 2026-09-01)`,
`cargo 1.98.1 (797e8a9bc 2026-08-05)`, QPC frequency 10,000,000 Hz.
The base worktree was detached at the exact base SHA and the head worktree
was the reviewed implementation revision. Runs used the same laptop, power
and setup, `RT_HANDOFF_BENCH_SCOPE=baseline`,
`RT_HANDOFF_BENCH_ITERATIONS=10000`, and the same command:

`rtk cargo run -p sky_player --features test-support --example rt_handoff_bench -- <report.json>`

### Previously recorded five pairs

The previously accepted five-pair run remains recorded for comparison:

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

### Counterbalanced five pairs for the current revision

The required counterbalanced order was:

`head-1 -> base-1 -> base-2 -> head-2 -> head-3 -> base-3 -> base-4 -> head-4 -> head-5 -> base-5`.

All ten runs used 10,000 observations, were acceptance-clean and statistics
eligible, and reported host `non_dispatches=0`, `overdue=0`, and real-wait
`transport_anomaly_count=0`. The deterministic matrix reports one expected
transport anomaly per run from the intentionally injected ZeroProgress case;
that is separate from the host real-wait anomaly field.

`pre_call_qpc - target_qpc` (us):

| run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| cb-head-1 | 19 | 117 | 276 | 468 | 738 |
| cb-base-1 | 39 | 192 | 330 | 502 | 890 |
| cb-base-2 | 41 | 390 | 556 | 872 | 1110 |
| cb-head-2 | 18 | 71 | 233 | 539 | 5707 |
| cb-head-3 | 15 | 23 | 123 | 2404 | 14961 |
| cb-base-3 | 29 | 57 | 786 | 2087 | 9635 |
| cb-base-4 | 29 | 61 | 876 | 2470 | 9519 |
| cb-head-4 | 14 | 36 | 1197 | 5437 | 26961 |
| cb-head-5 | 14 | 40 | 1201 | 4352 | 39412 |
| cb-base-5 | 26 | 252 | 727 | 3284 | 19076 |

`completion_qpc - pre_call_qpc` (us):

| run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| cb-head-1 | 2 | 2 | 3 | 4 | 12 |
| cb-base-1 | 2 | 3 | 3 | 7 | 203 |
| cb-base-2 | 2 | 3 | 3 | 5 | 14 |
| cb-head-2 | 2 | 2 | 3 | 4 | 20 |
| cb-head-3 | 1 | 2 | 4 | 10 | 374 |
| cb-base-3 | 2 | 3 | 6 | 22 | 507 |
| cb-base-4 | 2 | 3 | 6 | 20 | 186 |
| cb-head-4 | 1 | 3 | 6 | 34 | 352 |
| cb-head-5 | 1 | 4 | 6 | 73 | 1887 |
| cb-base-5 | 1 | 3 | 5 | 20 | 773 |

The counterbalanced timing gate does **not** pass. Head p99 is better in the
first three pair comparisons and worse in the last two; p99.9 is worse in
three comparisons; and head max is higher in four of five comparisons. This
must not be classified as host noise from this evidence alone.

### Raw-report decomposition

Every baseline run requested `production_calibrated`; the source is
`build_wait_mode(..., adaptive_spin_enabled=true)` and therefore
`calibrated_spin_threshold_us(startup_wake_error)`. The current baseline JSON
does not serialize the numeric effective threshold, threshold source, or
startup wake p95/p99/max. That reporting limitation is retained explicitly;
the raw reports do serialize the waiter-tail decomposition below.

The columns are `wake_lateness p99/p99.9/max`, `wake_to_final_policy
p99/p99.9/max`, and `final_policy_to_pre_call p99/p99.9/max`, all in us.

| run | wake | wake -> final | final -> pre-call |
| --- | --- | --- | --- |
| base-1 | 331/517/1205 | 39/91/214 | 5/11/88 |
| head-1 | 141/375/664 | 28/66/137 | 0/0/0 |
| base-2 | 250/478/659 | 42/74/139 | 6/9/60 |
| head-2 | 287/518/773 | 26/49/184 | 0/0/0 |
| base-3 | 0/88/326 | 39/72/143 | 5/14/98 |
| head-3 | 195/386/577 | 27/48/90 | 0/0/0 |
| base-4 | 0/73/756 | 39/67/134 | 6/15/59 |
| head-4 | 690/942/1315 | 101/154/169 | 0/0/0 |
| base-5 | 0/94/315 | 39/64/212 | 5/14/50 |
| head-5 | 210/480/1246 | 25/41/103 | 0/0/0 |
| cb-head-1 | 254/446/725 | 26/57/266 | 0/0/0 |
| cb-base-1 | 290/461/849 | 41/100/260 | 5/10/59 |
| cb-base-2 | 518/832/1068 | 49/116/405 | 5/11/192 |
| cb-head-2 | 211/516/5690 | 26/82/1097 | 0/0/0 |
| cb-head-3 | 60/2098/14946 | 39/152/2404 | 0/0/0 |
| cb-base-3 | 516/1666/9601 | 110/936/2826 | 12/104/1780 |
| cb-base-4 | 718/2150/9457 | 102/807/3136 | 11/44/376 |
| cb-head-4 | 1036/5049/26914 | 75/913/5711 | 0/0/0 |
| cb-head-5 | 1094/4222/39395 | 65/808/2729 | 0/0/0 |
| cb-base-5 | 632/3019/19020 | 79/418/4672 | 9/33/591 |

The head precision suffix remains structurally clean (`final_policy_to_pre_call`
is zero in all head runs), while the disadvantage is in the wake and
pre-wait/wait interaction. `wait.rs` itself is unchanged; the prepared normal
dispatch loop is the relevant Phase-6 path around the existing waiter. No
spin, MMCSS, affinity, timer, power, or scheduler policy was tuned. The
timing result is therefore an unresolved Phase-6 investigation blocker, not a
successful performance classification.

## Verification

- `cargo test -p sky_player --features test-support --lib`: 299 passed;
- `cargo test -p sky_dispatch_core --lib`: 54 passed;
- `cargo test -p sky_dispatch_win32 --lib prepared --no-default-features`:
  10 passed;
- `cargo test -p sky_player --features test-support --test rt_dispatch_no_alloc`:
  23 passed;
- prepared precision source/transitive audit tests: passed;
- prepared suspension continuation, natural-finish reconciliation,
  wrong-slot/unlisted negative cases, offset immutability, overdue delivery,
  legal same-key, completion no-feedback, transport-fault, and all prepared
  control-race tests: passed;
- diff-scope proof: the revision changes coordinator post-send accounting and
  test-support/test coverage only; `wait.rs`, target/wait derivation,
  `dispatch_prepared_normal_frame`, and the precision sender helper are
  unchanged, so the prior optimized assembly audit remains applicable;
- `cargo xtask check static`: PASS (29 pre-existing/allowlisted architecture
  warnings);
- `cargo xtask check rust`: PASS (fmt, clippy `-D warnings`, all-features
  workspace tests);
- final GitHub required CI result for this revision: not run yet; timing gate
  is unresolved and requires coordinator direction before claiming acceptance.

No focus, completion-floor, late-send, HybridWaiter, watchdog, MMCSS, spin,
affinity, timer, or persisted schema policy was changed. Strict/diagnostic
dynamic dispatch and legacy recovery remain available.
