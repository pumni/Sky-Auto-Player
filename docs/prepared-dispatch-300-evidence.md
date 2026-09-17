Closes #300

# Issue #300 evidence

Base: `9a32f0b8ce20d26e0fa31e48353ab6abd0822b3a`

Benchmark implementation revision used for the evidence runs:
`4be4a65fe60288bed55645a88f0c845eef0dd0b9`

Current PR head (documentation-only commit after those runs):
`187a3dae726b7c139e23eb55c52a387d67c62d87`

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

`rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=10000&& set RT_HANDOFF_BENCH_SCOPE=baseline&& cargo run --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json>"`

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

### Historical counterbalanced five pairs before benchmark-order correction

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

This is historical evidence from before the benchmark-order correction below.
Its timing gate is superseded: head p99 is better in the first three pair
comparisons and worse in the last two; p99.9 is worse in three comparisons;
and head max is higher in four of five comparisons. It was not classified as
host noise.

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
is zero in all head runs). This historical result motivated the benchmark
ownership correction; no spin, MMCSS, affinity, timer, power, or scheduler
policy was tuned.

### Corrected benchmark ownership qualification

The old prepared real-wait setup armed `now + margin` before stream
construction. The test-support path now configures the production waiter,
materializes the prepared stream, resets setup counters, and only then aligns
the current immutable frame to `now + margin`. The alignment helper derives
the offset from the prepared frame and does not call the mutable coordinator
planner. Stream-build duration is reported as startup/setup-only evidence.

The ordering regression test is
`prepared_benchmark_stream_build_precedes_alignment_and_wait_entry`; it
asserts build completion <= target alignment <= wait entry and that the
aligned target remains at or after the alignment sample.

The exact corrected command for every run was:

`rtk cmd /c "set RT_HANDOFF_BENCH_ITERATIONS=10000&& set RT_HANDOFF_BENCH_SCOPE=baseline&& cargo run --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_handoff_bench -- <report.json>"`

Head runs used the PR worktree at `bae99b551068c953e23100f1f687fd1922a0d9dd`.
Base runs used a detached worktree at
`9a32f0b8ce20d26e0fa31e48353ab6abd0822b3a`. The base worktree received only
temporary test-support report instrumentation for wait-entry QPC; its
production and benchmark scheduling behavior remained the base behavior.

The required counterbalanced order was:

`head-1 -> base-1 -> base-2 -> head-2 -> head-3 -> base-3 -> base-4 -> head-4 -> head-5 -> base-5`.

All ten runs used Rust 1.98.1, 10,000 observations, were acceptance-clean and
statistics-eligible, with host `non_dispatches=0`, `overdue=0`, and real-wait
`transport_anomaly_count=0`. Every deterministic matrix retained
`prepared=16`, `successful=14`, `non-send=2`, `DownExpired=0`, completion
feedback `PhysicalWindowExpired=0`, `UnobservedBacklog=0`, and
`timeline_rebase=0`. The deterministic matrix's one injected ZeroProgress
anomaly remains intentional and separate from the host real-wait anomaly.

The following values are p50/p95/p99/p99.9/max in microseconds unless noted.

| run | pre_call-target | wake lateness p99/p99.9/max | wake -> final p99/p99.9/max | final -> pre p99/p99.9/max | target - wait-entry p50/p95/p99/p99.9/min | stream build startup-only p50/p95/p99/max |
| --- | --- | --- | --- | --- | --- | --- |
| head-1 | 16/205/387/740/4589 | 369/720/4567 | 37/89/298 | 0/0/0 | 9997/9998/9998/9998/9982 | 76/90/112/632 |
| base-1 | 25/41/96/550/4866 | 48/462/4821 | 50/134/256 | 7/14/66 | 9971/9986/9987/9989/9397 | n/a |
| base-2 | 25/38/68/221/1859 | 0/138/1816 | 49/111/245 | 7/14/107 | 9980/9986/9987/9988/9303 | n/a |
| head-2 | 14/22/41/416/4530 | 2/296/4504 | 32/88/741 | 0/0/0 | 9998/9998/9998/9999/9902 | 45/86/107/426 |
| head-3 | 15/23/66/341/4048 | 44/317/4026 | 33/65/280 | 0/0/0 | 9997/9998/9998/9999/9831 | 67/90/115/3311 |
| base-3 | 24/37/64/358/6202 | 2/325/6150 | 46/125/302 | 6/14/38 | 9980/9986/9987/9989/9462 | n/a |
| base-4 | 25/40/93/357/6490 | 41/316/6459 | 49/88/289 | 8/16/177 | 9972/9986/9986/9988/9551 | n/a |
| head-4 | 14/22/39/611/3465 | 3/597/3445 | 31/56/278 | 0/0/0 | 9998/9998/9998/9999/9972 | 48/86/111/829 |
| head-5 | 14/22/38/260/3396 | 1/241/3362 | 32/48/167 | 0/0/0 | 9998/9998/9998/9999/9968 | 49/87/110/491 |
| base-5 | 25/37/66/406/3441 | 1/351/3392 | 47/81/1954 | 7/13/154 | 9978/9986/9987/9988/9256 | n/a |

For completeness, corrected `completion_qpc - pre_call_qpc` percentiles were:

| run | p50 | p95 | p99 | p99.9 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| head-1 | 1 | 3 | 3 | 7 | 21 |
| base-1 | 1 | 3 | 4 | 6 | 55 |
| base-2 | 2 | 3 | 4 | 7 | 118 |
| head-2 | 1 | 2 | 4 | 9 | 16 |
| head-3 | 1 | 2 | 4 | 8 | 69 |
| base-3 | 1 | 2 | 4 | 6 | 780 |
| base-4 | 1 | 3 | 4 | 11 | 222 |
| head-4 | 1 | 2 | 3 | 8 | 200 |
| head-5 | 1 | 2 | 3 | 8 | 15 |
| base-5 | 1 | 3 | 4 | 9 | 230 |

The corrected wait-entry evidence shows the head receives the intended
approximately 10 ms future budget before entering the waiter. Prepared stream
construction is outside that interval. Head p99/p99.9 are not systematically
worse across the counterbalanced pairs, and maxima cross/interleave: head is
lower in four of five pairwise maxima and higher only in the second pair.
The corrected timing gate therefore passes without any scheduler-policy
tuning. `final_policy_to_pre_call` remains zero for every head run.

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
- diff-scope proof: this revision changes only benchmark/test-support ordering,
  QPC evidence, the benchmark ordering regression test, and this evidence
  document; no production dispatch/wait/precision implementation changed;
  the prior optimized assembly audit remains applicable;
- `cargo xtask check static`: PASS (29 pre-existing/allowlisted architecture
  warnings);
- `cargo xtask check rust`: PASS (fmt, clippy `-D warnings`, all-features
  workspace tests);
- corrected counterbalanced five-pair host qualification: PASS; all ten runs
  were 10,000-observation, delivery-clean, and statistics-eligible;
- final GitHub required CI result for this benchmark-only revision: pending.

No focus, completion-floor, late-send, HybridWaiter, watchdog, MMCSS, spin,
affinity, timer, or persisted schema policy was changed. Strict/diagnostic
dynamic dispatch and legacy recovery remain available.
