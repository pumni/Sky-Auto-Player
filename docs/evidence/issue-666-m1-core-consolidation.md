# Issue #666 — M1 core consolidation evidence

## Reviewed baseline and candidate

| Snapshot | Commit | Tree | Parent |
| --- | --- | --- | --- |
| Reviewed starting HEAD | `76d14258848911fa7d285fa4d33d86526514f4eb` | `a6b557f0964b2e4a6526b27834875f17749b25bf` | `ef7ae25d8ae2b8790b70b7e05abd8ce018253a1e` |
| M1 implementation candidate, before this evidence-only commit | `7f26d687e88e99a12bc58c57b1910d9f1989f7a7` | `cb1dafe925b37938a4588c151125546fea47ebac` | `22945714efc1d78eaf19c2457ee54f6eee696d74` |

The repository base named in the task is `96e35b6c7b9946289e2bd6b02a658a83811e80ec`. The reviewed starting HEAD was checked out in a detached worktree before implementation. Its arrays, memory metrics, timing-advisory output, and error/cancellation behavior were recorded there; the candidate assertions below use literal expected values from that run. The prepared and legacy constructors both matching the same new helper is not used as the oracle.

Implementation commits after the reviewed starting HEAD:

1. `b7b92c0b` — move playback CPU baseline sampling before epoch QPC.
2. `5675d617` — share Down-arena materialization.
3. `c7f01de9` — split deadline tests by behavior.
4. `22945714` — pin the prepared-plan capacity matrix.
5. `7f26d687` — pass checkpoint callbacks without redundant closures (Clippy correction).

## Playback CPU baseline and epoch QPC ordering

At the reviewed HEAD, `boot.rs` sampled the selected `epoch_base` QPC value before reading the playback-specific thread and process CPU counters. The earlier observer/startup pair near the beginning of worker startup had separate provenance and was already retained for the observer.

The candidate now completes the pre-arm control, focus, target, owner, and power checks, samples the playback-specific thread/process pair, and then samples `epoch_base`. It computes the playback epoch with the existing pre-roll and calibration path, then stores the captured pair and existing epoch wall-time attribution in the playback timing fields. The observer retains its copied startup pair. QPC failure still follows the existing admission-failure path. No schedule-sized work was added after epoch preparation, and no CPU query was added to dispatch.

The deterministic startup-ordering test is `playback_cpu_baselines_precede_epoch_qpc_without_rebasing_observer_metrics` in `engine/tests/startup_admission.rs`. Its synchronization trace asserts:

`startup CPU pair < observer pair < playback CPU pair < selected epoch QPC sample completion`

It also asserts the observer pair equals the startup pair and that the sender count remains zero at `ArmedPublished`; it quits before dispatch. Result: 1 selected test passed.

## Independent executor-arena oracle

### Baseline arrays

The independent fixture uses QPC frequency `3,579,545 Hz`, a non-divisible frequency, and authored actions with source indices 10–16. Prepared and legacy arena arrays on the reviewed source were captured separately and matched the following literal candidate oracle.

| Chord index | Authored time (us) | QPC offset (ticks) | Source action | Down mask | Intent range | Reason |
| ---: | ---: | ---: | ---: | ---: | --- | --- |
| 0 | 1,001 | 3,584 | 10 | `0x0003` | `[0, 2)` | first chord |
| 1 | 1,801 | 6,447 | 13 | `0x0001` | `[2, 3)` | second A |
| 2 | 2,601 | 9,311 | 15 | `0x0004` | `[3, 4)` | third C |

| Source action | Generation | Key slot | Scan code | Reason | Paired-Up source | Paired-Up offset (us / ticks) |
| ---: | ---: | ---: | --- | --- | ---: | ---: |
| 10 | 0 | 0 | `0x15` | first chord | 11 | 1,301 / 4,657 |
| 10 | 1 | 1 | `0x16` | first chord | 12 | 1,701 / 6,089 |
| 13 | 2 | 0 | `0x15` | second A | 14 | 2,501 / 8,953 |
| 15 | 3 | 2 | `0x17` | third C | 16 | 3,201 / 11,459 |

The candidate's `independent_reviewed_head_arena_arrays_and_qpc_rounding_are_unchanged` test compares both constructors to those literals. The same fixture also pins prepared capacity at 3/3 chord headers and 4/4 intents, with 568 bytes retained in the plan and 3,978 bytes in the overlapping peak on 64-bit targets. `from_prepared_schedule()` still moves these arrays into fresh runtime state; it does not reconstruct or clone them.

The ceiling conversion remains `ceil(us * frequency / 1_000_000)`. At `3,579,545 Hz`, the 101-us and 77-us floor inputs convert to 362 and 276 ticks. The separate timing-advisory oracle at `1,000,003 Hz` records the odd-frequency boundary rounding and preserves the reviewed report:

| Packet | Authored | Ready | Sender completion | Worker free | H displacement | G displacement | Total floor | Lateness | Slack / class |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 / Zero |
| 1 | 17,168 | 17,168 | 17,168 | 17,168 | 0 | 0 | 0 | 0 | 0 / Zero |
| 2 | 34,335 | 34,336 | 34,336 | 34,336 | 0 | 1 | 1 | 1 | -1 / Negative |
| 3 | 51,502 | 51,504 | 51,504 | 51,504 | 2 | 0 | 2 | 2 | -2 / Negative |

Report totals remain 4 packets, 0 positive, 2 zero, and 2 negative; signed-slack percentiles are p50 `-1`, p95 `0`, and p99 `0`. The test asserts these values and the exact per-packet projection.

### Shared builder and policy boundaries

Before this change, `PreparedDispatchSchedule::prepare_with_cancellation()` and `DispatchExecutor::new_with_cancellation()` each counted Down intents, reserved two vectors, and materialized the compact chord and intent records. They had distinct validation, timestamp, cancellation, and memory-admission policies.

Now both call the private `build_down_arena()` in `sky_dispatch_core/src/executor.rs`. That helper owns the common checked intent count, storage-size calculation, vector reservation, compact record materialization, checked intent-range arithmetic, and final count consistency check. It uses only the existing chord and intent vectors; there is no temporary whole-schedule allocation.

| Policy | Prepared-plan adapter | Raw-executor adapter |
| --- | --- | --- |
| Feasibility validation | Existing minimum H/G feasibility validation before materialization | Not added |
| Time conversion | QPC ceiling conversion with prepared overflow/error messages | Caller-supplied `us_to_ticks` conversion with legacy error details |
| Cancellation placement | Existing validation and unmatched-Up checkpoints; first arena chord checkpoint retained | Existing checkpoint before count completion, periodic materialization checkpoints, and post-arena checkpoint retained |
| Arena capacity admission | Prepared retained/overlapping capacity calculation and 512-MiB retained-dispatch budget remain outside the helper | Actual vector capacities checked against the executor storage budget |
| Unmatched-Up metadata | Existing two-pass metadata construction remains in the prepared adapter | Not applicable |
| Schedule-end conversion | Existing prepared conversion and error classification | Existing caller-supplied conversion and error classification |
| Prepared-plan consumption | `from_prepared_schedule()` moves the plan's arrays | Not applicable |

Focused regression fixtures preserve error and cancellation behavior:

| Case | Expected reviewed behavior, retained on candidate |
| --- | --- |
| Invalid compiled batch range | Prepared validation returns `prepared native schedule admission failed: schedule contains an invalid compiled batch range for packet 0`. |
| Legacy chord timestamp conversion failure | Caller detail remains in `scheduled timestamp conversion failed for chord at 1001us: oracle conversion failure`. |
| Prepared schedule-end conversion overflow | Returns `schedule end offset timestamp conversion overflow`. |
| Legacy schedule-end conversion failure | Preserves the adapter wrapper and caller's `near-overflow conversion` detail. |
| Raw storage size overflow / budget excess | Checked arithmetic rejects overflow; storage admission still rejects capacity beyond its existing budget. `try_reserve_exact` failure remains reported by the existing allocation-specific errors. |
| Prepared retained budget | Exactly `536,870,912` bytes is admitted; one byte more returns the existing `prepared playback capacity (...) exceeds the 536870912 byte retained-dispatch budget` error. |
| Prepared materialization cancellation | Cancelling at callback 7 returns the callback's exact message and preserves the first materialization checkpoint boundary. |
| Raw 9,000-chord cancellation | Callback 4 still cancels after 8,192 timestamp conversions (4,096 chords), with no later conversion. |

The focused oracle covers zero/one/many chords, fresh raw executor state, invalid ranges, caller conversion errors, near-overflow timestamps, callback cancellation, and budget boundaries. Core property/scale tests continue covering large schedules. No error class, callback boundary, or memory budget was intentionally changed.

### Capacity comparison

The following prepared-plan metrics were captured at the reviewed SHA and are hard-coded in `prepared_retained_capacity_matrix_matches_reviewed_head`; the candidate matches every count and capacity.

| One-key repeat chords | Chord capacity | Intent capacity | Plan bytes | Overlapping peak bytes |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 264 | 480 |
| 1 | 1 | 1 | 352 | 1,574 |
| 100 | 100 | 100 | 9,064 | 82,358 |
| 5,000 | 5,000 | 5,000 | 440,264 | 4,080,758 |

For a 100,000-chord one-key schedule, the independent baseline and candidate memory test both report: compiler input capacity 16,000,000 bytes; compiler temporary capacity 446 bytes; runtime schedule heap capacity 65,600,112 bytes; executor heap capacity 8,800,000 bytes plus a 920-byte executor struct; combined retained schedule/executor capacity 74,401,032 bytes; and compiler/startup peak bound 81,600,774 bytes. These are calculated capacity metrics, not process RSS; complete worker/process overhead is not measured.

## Deadline behavior test split

The old 1,074-line `dispatch_loop_deadline_trace_tests.rs` is now split into ordinary path modules. The original seven test functions and their assertions were compared with the split bodies after normalizing line endings; the bodies and output markers are unchanged. The five files total 1,085 lines (11 lines of module/import overhead); the largest is 395 lines, down from 1,074. No architecture threshold or warning allowlist changed.

At the reviewed starting HEAD, the M1 source/test diff covers 13 files with 1,916 insertions and 1,213 deletions. `executor.rs` is +16 net lines after consolidating the common materializer; `boot.rs` is +28 net lines for ordering instrumentation and the corrected startup path; `dispatch_loop.rs` is +8 net lines for explicit test modules. These counts precede this evidence file and its evidence-only commit.

| Behavior | Test file | Lines |
| --- | --- | ---: |
| H/G floor and scheduler projection | `dispatch_loop_deadline_hg_projection_tests.rs` | 280 |
| Late-Due staged-candidate order | `dispatch_loop_deadline_late_due_tests.rs` | 395 |
| Post-Due Stop/Skip/focus/owner validation | `dispatch_loop_deadline_post_due_control_tests.rs` | 137 |
| Interrupted waiter and bounded Busy retry | `dispatch_loop_deadline_interrupted_tests.rs` | 232 |
| Real-waiter smoke | `dispatch_loop_deadline_real_waiter_tests.rs` | 41 |

The old filter `engine::worker::dispatch_loop::tests::deadline_trace::` selected 7 passing tests on the baseline. The documented replacement filter `engine::worker::dispatch_loop::tests::deadline_` selects the same 7 passing tests on the candidate. Test names, trace markers, and ignored offline benchmark discoverability remain intact; `offline_waiter_guard_ab_benchmark` remains an explicitly ignored, discoverable test.

### Late `WaitBoundary::Due` trace

The independent controlled-QPC test preserves the production Due arm's previously staged B2 Down candidate. Its timeline is:

1. A+B Down `(Up 0x0000, Down 0x0003)` commits at the epoch: cursor 1, active/release `0x0003`, possible/in-flight `0`.
2. B1 Up `(0x0002, 0x0000)` sends at or after `max(authored 25,000 us, H)`: cursor stays 1, active/release `0x0001`, possible/in-flight `0`.
3. B2 Down is authored at 42,500 us, but G makes its physical deadline 47,167 us. It is staged as `(0x0000, 0x0002)`.
4. The controlled waiter returns `Due` with dispatch QPC at A's 100,000-us Up eligibility. The Due branch retains and sends B2 first: cursor 2, active/release `0x0003`, possible/in-flight `0`.
5. The next scheduler iteration freshly selects A Up with C Down and sends `(Up 0x0001, Down 0x0004)`: cursor 3, active/release `0x0006`, possible/in-flight `0`.

The recorded send order is therefore `(0,3)`, `(2,0)`, `(0,2)`, `(1,4)`. Each sender record has one attempt. B2 crosses the sender boundary once and every sender start is at or beyond its relevant H/G/authored physical target. A's independent release obligation and owner proof remain unchanged through B2; A's proof is removed only when A Up is sent. B2 retains the original owner proof. Up-before-Down order in the final coalesced packet is preserved.

The trace marker records `staged_down=0x0002`, first late packet `(up:0x0000,down:0x0002)`, next iteration `(up:0x0001,down:0x0004)`, final cursor 3, masks `(active:0x0006,possible:0,inflight:0,release:0x0006)`, and `attempts(B2)=1`.

### Due, Interrupted, and PermitBusy distinctions

| Result | Candidate handling |
| --- | --- |
| `Due` after overshoot | Keep the staged packet; final admission still rechecks controls, focus, and owner. Fresh scheduler selection happens on the next outer iteration. The trace above pins this existing behavior. |
| `Interrupted` | Discard the staged candidate and return to outer-loop handling for the control/focus wake. The existing interrupted-wait tests cover focus and bounded owner-permit retry wakeups. |
| `PermitBusy` then `Replan` | Return to fresh outer scheduler evaluation; it does not use the Due staged-send path. Existing preflight-Busy and replan-timing tests remain separately selected. |

The post-Due regression verifies Stop, Skip, focus, and owner changes block sender admission. This work records no conflict with an established contract and makes no coalescing or packet-ordering change.

## Focused test and operation evidence

All listed focused filters selected the intended nonzero tests and passed on the candidate:

| Command/filter | Selected and passed |
| --- | ---: |
| `cargo test ... executor::arena_oracle_tests` | 6 |
| `cargo test ... playback_cpu_baselines_precede_epoch_qpc_without_rebasing_observer_metrics` | 1 |
| `cargo test ... engine::worker::dispatch_loop::tests::deadline_` | 7 |
| `cargo test ... dispatch_schedule_uses_ceil_converted_g_at_non_divisible_qpc_frequency` | 1 |
| `cargo test ... preflight_busy::` | 6 |
| `cargo test ... replan_timing::` | 2 |
| `cargo test ... owner_proof::` | 11 |
| `cargo test ... safety` | 11 |
| `cargo test ... cancellation` | 3 |
| `cargo test ... measured_final_admission_operations_for_healthy_packet_shapes` | 1 |
| 100,000-chord `executor_memory_scaling_100_1000_10000_and_budget_proof` | 1 |

The healthy-path operation-count fixture was run on both the reviewed baseline worktree and the candidate. Counts were identical for Down-only, Up-only, mixed, mixed-to-Up-only, blocked-generation replan, and blocked-modifier replan:

| Shape | QPC reads | Foreground reads / direct API reads | Owner PID / process identity | Generation revalidations | Sender calls | Allocations |
| --- | ---: | --- | --- | ---: | ---: | ---: |
| Down-only | 1 | 1 / 0 | 1 / 1 | 11 | 1 | 0 |
| Up-only | 1 | 1 / 0 | 1 / 1 | 10 | 1 | 0 |
| Mixed | 1 | 1 / 0 | 1 / 1 | 11 | 1 | 0 |
| Mixed-to-Up-only | 1 | 1 / 0 | 1 / 1 | 13 | 1 | 0 |
| Blocked generation replan | 1 | 1 / 0 | 1 / 1 | 11 | 0 | 0 |
| Blocked modifier replan | 0 | 0 / 0 | 0 / 0 | 4 | 0 | 0 |

The foreground column is monitor-hint reads / direct `GetForegroundWindow` reads. No healthy-path foreground operation was added.

## Verification ledger

Environment: Windows x64 MSVC, Rust `1.99.1`, Cargo `1.99.1`. Exit codes below are from the candidate implementation, including the Clippy correction commit. No failed command was counted as a pass. An initial `sky_player --lib` Clippy attempt returned exit 1 for two `redundant_closure` warnings in `executor.rs`; both call sites now pass `&mut checkpoint`, and the final required Clippy runs passed.

| Command | Exit | Result |
| --- | ---: | --- |
| `git diff --check` | 0 | clean |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 | pass |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | 0 | pass |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_player --lib --no-default-features -- -D warnings` | 0 | pass |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | 0 | pass |
| `cargo check --locked --manifest-path rust/Cargo.toml -p sky_player --lib --no-default-features` | 0 | pass |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | 87 unit + 4 integration/property/golden tests passed; 0 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | 278 passed; 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | 0 | 328 unit + 4 integration tests passed; 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | 4 passed |
| `cargo xtask check desktop-native` | 0 | pass |
| `cargo xtask check static` | 0 | pass; 67 architecture advisories |
| `cargo xtask check all` | 0 | pass; Rust, static/security, Bun, E2E, and desktop stages passed |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-features --locked -- --list` | 0 | discovery only: 1,568 test cases across 31 test binaries; the preceding `check all` executed the workspace suite successfully |

`cargo xtask check all` was run sequentially after `desktop-native` and `static`. The all-features workspace test discovery includes ignored tests; this does not execute the explicitly ignored offline waiter A/B experiment. The dedicated no-allocation suite passed all four tests.

Architecture advisory count was 68 at the exact reviewed baseline and is 67 at the candidate. The only category delta is `regular_module_lines`, 37 to 36, because the 1,074-line deadline test module was split. Other baseline category counts are unchanged: context fields 1, dispatch function lines 1, engine test module lines 1, facade lines 1, native runtime module lines 2, production glob import 2, unsafe boundary 19, and xtask module lines 4. No threshold or allowlist changed.

## Hosted status and qualification limits

Hosted PR checks must be queried for the final pushed evidence commit and recorded in the final handoff; local `xtask` results do not substitute for hosted CI on that exact SHA. The PR remains Draft and open.

This evidence is a source-level and deterministic mock/test comparison. It does not qualify Windows `SendInput` insertion, native foreground timing, or Sky gameplay reception. No ReceiveOnly run, interactive focus test, gameplay run, or native suspend/resume qualification was performed.
