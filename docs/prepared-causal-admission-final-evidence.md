# Phase 6 — Prepared Causal Admission Final Evidence

Status: qualification evidence for issue #357. This package does not change
production dispatch policy or claim game observation, Raw Input delivery,
rendering, or audio onset.

## Revisions and host

- Exact fetched base and `origin/main`: `98f0dd1f9348f8235b17ee6d1748ace1f8ca060b`.
- Branch: `qualify/357-prepared-causal-final`.
- Worktree: `D:\Dev\Sky-Auto-Player-357`.
- Qualification implementation head: `124f9202261e599581047d0206e6d480097c176a`.
  This is the exact measured Phase-6 head before this evidence-only correction;
  the correction commit is the final PR head reported in the checkpoint.
- Program: #350. Child merge list: #351/#358 at
  `2e8c83ff02de6e2ff6550c0f21a3a6394f408d2f`; #352/#359 at
  `1e4a47708a2fd35077472fc4741e7386582274d3`; #353/#360 at
  `e40189f67f5a6aae045a3b021b9748ceeb4c81cf`; #354/#361 at
  `9a6d08cff7a567cc88ea0ca3a432fa060267715c`; #355/#362 at
  `7872d323160a89b7698ab5e0fff03f794e33af3a`; #356/#366 at
  `98f0dd1f9348f8235b17ee6d1748ace1f8ca060b`; and this Phase-6 child
  PR #369.
- Toolchain: Rust `1.98.1 (48a229cea 2026-09-01)`, Cargo `1.98.1`,
  `x86_64-pc-windows-msvc`, LLVM `22.1.8`.
- Host: Windows 11 Home, Dell Inspiron 5515, 12 logical processors, QPC
  frequency `10,000,000 Hz`, PowerShell `5.1.26100.9444`.
- Final Phase-6 verdict state at this correction: evidence complete and
  coordinator acceptance pending; exact-head Windows/unit and required-gate
  rerun is required for the correction head before STOP.

Tracked changes are limited to this evidence document and the existing
`rt_handoff_bench` argument bound. The latter changes only the accepted
test-support environment input from `60,000` to `1,000,000` microseconds so
the requested 100/250/500/1000 ms evidence buckets are reachable. It does not
alter the 20 ms classification threshold, spin policy, scheduler, authored
targets, admission, cutoff, transport, retry, or coordinator behavior.

## Qualification commands and raw reports

The measurement harness was run with `RT_HANDOFF_BENCH_SCOPE=real_wait_core`,
real `HybridWaiter::production`, deterministic mock transport, and
`RT_HANDOFF_BENCH_DUE_US` in microseconds. Reports are local generated
artifacts under `.benchmarks/` and are not tracked:

- `phase6-final-real-wait-core-5ms.json` (`20` observations per key count);
- `phase6-real-wait-core-20ms.json` (`20` per key count);
- `phase6-real-wait-core-25ms.json` (`20` per key count);
- `phase6-real-wait-core-100ms.json` (`8` per key count);
- `phase6-real-wait-core-250ms.json` (`4` per key count);
- `phase6-real-wait-core-500ms.json` (`2` per key count);
- `phase6-real-wait-core-1000ms.json` (`1` per key count).

The exact PowerShell command shape used for the gap matrix was:

```powershell
$env:CARGO_TARGET_DIR = 'C:\Temp\sky-auto-player-357-target'
$env:RT_HANDOFF_BENCH_SCOPE = 'real_wait_core'
$env:RT_HANDOFF_BENCH_MODE = 'real_wait'
foreach ($case in @(
  @{ Label = '5ms'; Us = 5000; Iterations = 20 },
  @{ Label = '20ms'; Us = 20000; Iterations = 20 },
  @{ Label = '25ms'; Us = 25000; Iterations = 20 },
  @{ Label = '100ms'; Us = 100000; Iterations = 8 },
  @{ Label = '250ms'; Us = 250000; Iterations = 4 },
  @{ Label = '500ms'; Us = 500000; Iterations = 2 },
  @{ Label = '1000ms'; Us = 1000000; Iterations = 1 }
)) {
  $env:RT_HANDOFF_BENCH_DUE_US = [string]$case.Us
  $env:RT_HANDOFF_BENCH_ITERATIONS = [string]$case.Iterations
  cargo run --locked --manifest-path rust/Cargo.toml -p sky_player `
    --features test-support --example rt_handoff_bench -- `
    ".benchmarks/phase6-real-wait-core-$($case.Label).json" `
    --scope real_wait_core --mode real_wait
}
```

The 5 ms report was named `phase6-final-real-wait-core-5ms.json`; the other
names above are the corresponding recorded report names. The exact foreground
invocation used the same `cargo run` command with
`RT_HANDOFF_BENCH_SCOPE=real_foreground_focus`,
`RT_HANDOFF_BENCH_REQUIRE_FOCUS=1`, `RT_HANDOFF_BENCH_REAL_FOREGROUND=1`,
`RT_HANDOFF_BENCH_ITERATIONS=100`, `RT_HANDOFF_BENCH_DUE_US=100000`, output
`.benchmarks/phase6-real-foreground-final.json`, and
`--scope real_foreground_focus --mode real_wait`. The exact baseline
invocations used `RT_HANDOFF_BENCH_SCOPE=baseline`,
`RT_HANDOFF_BENCH_MODE=real_wait`, `RT_HANDOFF_BENCH_DUE_US=5000`/`25000`,
`RT_HANDOFF_BENCH_ITERATIONS=20`, and outputs
`.benchmarks/phase6-prepared-baseline-5ms.json` and
`.benchmarks/phase6-prepared-baseline-25ms.json`. The explicit invocations
were:

```powershell
$env:RT_HANDOFF_BENCH_SCOPE = 'real_foreground_focus'
$env:RT_HANDOFF_BENCH_MODE = 'real_wait'
$env:RT_HANDOFF_BENCH_REQUIRE_FOCUS = '1'
$env:RT_HANDOFF_BENCH_REAL_FOREGROUND = '1'
$env:RT_HANDOFF_BENCH_ITERATIONS = '100'
$env:RT_HANDOFF_BENCH_DUE_US = '100000'
cargo run --locked --manifest-path rust/Cargo.toml -p sky_player `
  --features test-support --example rt_handoff_bench -- `
  '.benchmarks/phase6-real-foreground-final.json' `
  --scope real_foreground_focus --mode real_wait

$env:RT_HANDOFF_BENCH_SCOPE = 'baseline'
$env:RT_HANDOFF_BENCH_MODE = 'real_wait'
$env:RT_HANDOFF_BENCH_ITERATIONS = '20'
$env:RT_HANDOFF_BENCH_DUE_US = '5000'
cargo run --locked --manifest-path rust/Cargo.toml -p sky_player `
  --features test-support --example rt_handoff_bench -- `
  '.benchmarks/phase6-prepared-baseline-5ms.json' --scope baseline --mode real_wait

$env:RT_HANDOFF_BENCH_DUE_US = '25000'
cargo run --locked --manifest-path rust/Cargo.toml -p sky_player `
  --features test-support --example rt_handoff_bench -- `
  '.benchmarks/phase6-prepared-baseline-25ms.json' --scope baseline --mode real_wait
```

The counter executable was:

```powershell
cargo run --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --example rt_prepared_acceptance
```

The lower observation counts at long gaps are an explicit wall-clock bound;
they are not statistics-eligible. The existing 20 ms label is unchanged and
is evidence classification only.

### Cold-gap and key-count matrix

The production-adaptive waiter reported the same result for Down key counts
1, 5, and 15. `hot/cold` is the wait count classified against the existing
20,000 microsecond threshold; `non/overdue` is non-dispatch/overdue count;
misses are `UnobservedBacklog / PhysicalWindowExpired /
FinalSenderWindowExpired`.

| gap | observations per key count | hot/cold | 1-key | 5-key | 15-key |
|---:|---:|---:|---|---|---|
| 5 ms | 20 | 20 / 0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 |
| 20 ms | 20 | 20 / 0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 |
| 25 ms | 20 | 0 / 20 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 |
| 100 ms | 8 | 0 / 8 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 |
| 250 ms | 4 | 0 / 4 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 |
| 500 ms | 2 | 0 / 2 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 |
| 1000 ms | 1 | 0 / 1 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 | clean; 0/0; 0/0/0 |

Every row above has `acceptance_clean=true`, no transport anomaly, and no
causal/physical/sender miss. The 5/20 ms rows are hot; 25 ms and above are
cold. This is the observed threshold behavior, not a recommendation to tune
the threshold.

### Timing decomposition

Values are microseconds for the 1-key production-adaptive Down scenario. Every
cell below is exactly `p50/p95/p99/p99.9/max`; at low sample counts the tail
labels are descriptive order statistics, not statistics-eligible estimates.

| gap | target→pre-call | wake→final policy | final policy→pre-call | pre-call→completion |
|---:|---|---|---|---|
| 5 ms | 24/187/187/187/209 | 21/31/31/31/56 | 2/4/4/4/7 | 2/2/2/2/3 |
| 20 ms | 76/2374/2374/2374/6714 | 29/39/39/39/52 | 5/8/8/8/8 | 2/3/3/3/4 |
| 25 ms | 24/40/40/40/54 | 21/34/34/34/34 | 3/5/5/5/21 | 2/2/2/2/2 |
| 100 ms | 22/33/33/33/52 | 20/29/29/29/46 | 3/4/4/4/6 | 1/2/2/2/2 |
| 250 ms | 1269/2095/2095/2095/2611 | 31/37/37/37/58 | 4/4/4/4/7 | 2/2/2/2/3 |
| 500 ms | 388/388/388/388/1507 | 28/28/28/28/44 | 4/4/4/4/7 | 1/1/1/1/2 |
| 1000 ms | 42/42/42/42/42 | 37/37/37/37/37 | 5/5/5/5/5 | 1/1/1/1/1 |

The production-adaptive wait evidence is also copied here so the decision does
not depend on untracked JSON. Wake lateness is target→wake; spin is per-sample
spin time p50/p95/p99/p99.9/max, followed by total spin / total wall time,
spin duty, calibrated threshold, and process CPU time/duty. `startup wake` is
the calibration probe p50/p95/p99/max. Long-gap tail cells are descriptive only.

| gap | target→wake | spin time | total spin / wall | spin duty | calibrated spin | startup wake | process CPU |
|---:|---|---|---:|---:|---:|---|---|
| 5 ms | 0/161/161/161/175 | 110/349/349/349/350 | 2944 / 105980 us | 2.778% | 403 us | 73/353/353/353 us | 78125 us / 8.164% |
| 20 ms | 40/2340/2340/2340/6681 | 0/602/602/602/625 | 3852 / 423211 us | 0.910% | 817 us | 465/557/557/557 us | 46875 us / 1.222% |
| 25 ms | 0/0/0/0/0 | 695/870/870/870/901 | 13266 / 504800 us | 2.628% | 1000 us | 89/500/500/500 us | 125000 us / 2.739% |
| 100 ms | 0/0/0/0/0 | 223/346/346/346/553 | 1876 / 802081 us | 0.234% | 612 us | 508/519/519/519 us | 31250 us / 0.416% |
| 250 ms | 1237/2053/2053/2053/2544 | 0/0/0/0/0 | 0 / 1008547 us | 0.000% | 830 us | 561/741/741/741 us | 15625 us / 0.173% |
| 500 ms | 336/336/336/336/1474 | 0/0/0/0/0 | 0 / 1002746 us | 0.000% | 651 us | 514/540/540/540 us | 15625 us / 0.173% |
| 1000 ms | 0/0/0/0/0 | 769/769/769/769/769 | 769 / 1000375 us | 0.077% | 845 us | 549/620/620/620 us | 0 us / 0.000% |

The exact report counters for every row were waiter failures `0`,
`failure_reasons={}`, observation gaps `0`, and `wait_count=observation_count`.
The harness has no independent interrupt/replan counter; it recorded no
interrupt/replan outcome in these runs. That schema limitation is stated here
instead of converting an absent counter into an unqualified claim.

## Real foreground proof

Command scope: `real_foreground_focus`, `RT_HANDOFF_BENCH_REQUIRE_FOCUS=1`,
`RT_HANDOFF_BENCH_REAL_FOREGROUND=1`, `100` observations per scenario,
`HybridWaiter::production`, report:
`.benchmarks/phase6-real-foreground-final.json`.

The host supplied nonzero HWND `328412`; setup sampled it outside timed
admission, and the timed Down path used the real Win32 foreground query rather
than `TEST_FOREGROUND_HWND`. `acceptance_clean=true`; CPU time was `531250 µs`
and reported process CPU duty was `6.439%`.

| scenario | fresh query count | non-dispatches | transport anomalies |
|---|---:|---:|---:|
| single Down-heavy | 1 per observation | 0 | 0 |
| dense alternating | 1 per observation | 0 | 0 |
| 15-key chord | 1 per observation | 0 | 0 |
| Mixed | 1 per observation | 0 | 0 |
| UpOnly control | 0 per observation | 0 | 0 |

This is an interactive-host PASS for the query-count evidence. It is not a
claim that the foreground process was Sky or that the game observed input.

## Deterministic causal, transport, and ownership evidence

The full deterministic prepared report from `rt_handoff_bench --scope
baseline` retained the accepted totals:

| counter | result |
|---|---:|
| prepared physical boundaries | 16 |
| successful full sends | 14 |
| intentional non-send/drop boundaries | 2 |
| timeline rebases | 0 |
| deterministic transport anomalies | 1 intentional injected `ZeroProgress` |
| clean real-wait transport anomalies | 0 |

All eleven semantic baseline cases passed: `single_note`,
`maximum_valid_atomic_chord`, `dense_different_key_boundaries`,
`legal_same_key_sequence`, `late_wake_near_target`,
`late_wake_well_after_target`, `late_first_boundary_pressures_following_boundary`,
`stop_interrupt_race_at_target`, `intentionally_injected_zero_progress_drop`,
`completion_floor_control`, and `completion_floor_delayed`.

The ten numbered #357 regression proofs map one-for-one to executable tests as
follows. All listed tests were included in the passing focused library runs
(`sky_dispatch_win32` 239 tests and `sky_player` 337 tests); no report string
or hard-coded counter substitutes for an executable proof.

| # | required proof | executable test(s) | observed result |
|---:|---|---|---|
| 1 | HybridWaiter sub-µs remainder never arms an infinite wait | `kernel_wait_floors_are_safe_at_10mhz`; `kernel_wait_floors_are_safe_at_3579545hz` | PASS; positive sub-µs remainders remain spin, timer arm is bounded |
| 2 | Minimum-hold future authorization, equality accepted, +1 QPC tick expired with zero full-packet attempt | `prepared_future_authorization_before_stall_later_unseen_backlog_future`; `prepared_cutoff_composition_is_exact_and_sender_authoritative`; `target_crossing_at_down_latest_start_is_allowed`; `target_crossing_past_down_latest_start_makes_zero_send_attempts` | PASS |
| 3 | Long-hold Down retains only schedule-derived slack | `prepared_down_policy_materializes_exact_hold_slack_and_unpaired_state` | PASS; 20,000 us hold with 500 us minimum produces 19,500 us slack |
| 4 | 2/3/15 stale boundaries do not replay as a Down catch-up burst | `prepared_no_catch_up_burst_matrix_is_exact_for_2_3_and_15_boundaries`; `several_unobserved_down_boundaries_drain_without_catch_up` | PASS |
| 5 | Mixed stale/expired Down emits no full packet and exactly one valid Up-prefix transaction | `prepared_mixed_backlog_sends_only_the_canonical_up_prefix`; `prepared_mixed_sender_cutoff_sends_only_the_canonical_up_prefix`; `prepared_mixed_up_prefix_transport_failure_is_fail_closed` | PASS |
| 6 | Dropped generation's frozen future Up remains immutable, emits once when owned, and does not resurrect status | `prepared_stale_up_after_dropped_down_is_physical_noop_for_accounting`; `frozen_up_after_dropped_expired_same_key_continues_next_generation`; `prepared_normal_resumable_suspension_reconciles_frozen_up_and_continues` | PASS |
| 7 | Suspension-cancelled prior Up plus newly missed Mixed Down reconciles exactly once | `prepared_suspension_times_miss_preserves_cancellation_and_drops_down` | PASS |
| 8 | UpOnly is independent of Down foreground authorization | `prepared_down_final_foreground_proof_has_exact_query_scope`; `prepared_authorization_excludes_up_only_and_survives_spurious_replan` | PASS; UpOnly query count is zero |
| 9 | Require-focus Down performs exactly one fresh final foreground proof | `prepared_down_final_foreground_proof_has_exact_query_scope`; `final_admission_requires_fresh_foreground_match_and_rechecks_atomic_focus` | PASS; one query for eligible Down |
| 10 | Partial/ambiguous SendInput is terminal and never retried as musical Down | `partial_down_packet_never_issues_a_second_send`; `partial_mixed_packet_never_issues_a_second_packet_call`; `single_attempt_packet_does_not_retry_zero_progress`; `normal_prepared_zero_progress_is_fail_closed_without_cursor_advance` | PASS |

The executable deterministic proofs are:

| proof | test |
|---|---|
| exact future authorization before a physical stall, later unseen backlog, then a future authorized boundary | `prepared_future_authorization_before_stall_later_unseen_backlog_future` |
| waiter-entry stall preserves exact authorization | `future_classification_then_waiter_entry_stall_keeps_exact_boundary_authorized` |
| unpaired Down has no invented cutoff but still needs authorization | `prepared_unpaired_down_has_no_cutoff_but_requires_causal_authorization` |
| no catch-up for 2/3/15 overdue boundaries | `prepared_no_catch_up_burst_matrix_is_exact_for_2_3_and_15_boundaries` |
| Mixed backlog emits only the immutable Up prefix | `prepared_mixed_backlog_sends_only_the_canonical_up_prefix` |
| Mixed sender expiry emits only the immutable Up prefix | `prepared_mixed_sender_cutoff_sends_only_the_canonical_up_prefix` |
| `DroppedExpired`/stale Up ownership does not resurrect a Down | `prepared_stale_up_after_dropped_down_is_physical_noop_for_accounting` |
| frozen Up continuation through resumable suspension | `prepared_normal_resumable_suspension_reconciles_frozen_up_and_continues` |
| all full-packet outcomes fail closed with one prepared transaction | `prepared_full_packet_transport_outcome_matrix_is_fail_closed` |
| all Up-prefix transport outcomes fail closed | `prepared_up_prefix_transport_fault_matrix_is_fail_closed` |
| backlog/prefix/sender-expiry counter report | `prepared_acceptance_counters_cover_backlog_prefix_and_sender_expiry` |

The exact Phase-3 acceptance executable `rt_prepared_acceptance` was also
run. Its output is explicitly `report_kind=counter_report` (not a matrix
runner): 4 boundaries, 1 successful full send, 3 intentional non-send/missed,
2 normal backlog, 1 sender expiry, 2 Up-prefix recovery sends, 0 transport
anomalies, and 0 rebases.

Lifecycle/ownership coverage includes `prepared_suspension_times_miss_preserves_cancellation_and_drops_down`,
`native_prepared_normal_resume_sends_frozen_up_and_following_sentinel`,
`native_prepared_normal_resume_naturally_finishes_after_reconciled_up`,
`target_change_is_rejected_at_the_final_send_boundary`,
`supervisor_lease_treats_future_heartbeat_as_fresh`, and the focus lifecycle
tests below. Accepted terminal cleanup has no stuck-key result in the
canonical native sink run.

## Foreground lifecycle and query-count tests

The focused library run passed the following Phase-4 proofs:

- `prepared_down_final_foreground_proof_has_exact_query_scope` — Down with
  `require_focus=true` queries once; `require_focus=false` and UpOnly query
  zero times.
- `stale_published_focus_false_rejects_matching_foreground_before_fresh_query`
  — stale published false rejects with zero query.
- `fresh_focus_rejection_preserves_current_frame_and_miss_lifecycle_state` —
  cursor, frame, backlog, sender-expiry, `DroppedExpired`, and recovery state
  remain untouched.
- `same_frozen_prepared_frame_is_readmitted_after_normal_focus_restore` —
  normal focus restore invalidates/reacquires authorization and sends the same
  frozen current frame.
- `final_admission_requires_fresh_foreground_match_and_rechecks_atomic_focus`
  and `final_down_target_admission_checks_target_before_focus` — final
  ordering and target/focus rechecks.

## Native receive-only sink workflow

The exact build and runner commands were:

```powershell
$env:CARGO_TARGET_DIR = 'C:\Temp\sky-auto-player-357-target'
cargo build --manifest-path rust/Cargo.toml --locked --release `
  -p sky_player --features real-input-acceptance --bin rt-native-acceptance
& .\scripts\run_native_acceptance_case.ps1 -Scenario <scenario> -TimingMarginUs 500
& .\scripts\run_native_acceptance_case.ps1 -Scenario dense-alternating `
  -TimingMarginUs 500 -CpuContention
```

The runner starts only `scripts/native_acceptance_sink.ps1` in
`ReceiveOnly` mode, verifies its schema-v3 HWND/process identity, and passes
that exact HWND to the feature-gated `rt-native-acceptance` binary. No
arbitrary application received input. The complete supported scenario matrix
was executed on the interactive host at timing margin 500 us:

| supported scenario | physical verdict | evidence path / result |
|---|---|---|
| `canonical-single` | PASS | `native-case-20260920T190700-5f081a9a`, sink `1..2` (2 events) |
| `canonical-chord` | PASS | `native-case-20260920T190704-f3fcabb6`, sink `1..4` (4 events) |
| `canonical-max-chord` | PASS | `native-case-20260920T190706-259b4f5d`, sink `1..30` (30 events) |
| `hold` (exact minimum hold) | PASS | `native-case-20260920T190708-597fbe76`, sink `1..2` |
| `long-single-sequence` | FAIL closed | `native-case-20260920T190710-798a63b6`, KeyUp-before-matching-KeyDown |
| `dense-alternating` | FAIL closed | `native-case-20260920T190713-6a522cdd`, KeyUp-before-matching-KeyDown |
| `chord-sweep` | FAIL closed | `native-case-20260920T190716-3a236414`, KeyUp-before-matching-KeyDown |
| `near-minimum-retrigger` | FAIL closed | `native-case-20260920T190719-ab893691`, KeyUp-before-matching-KeyDown |
| `rapid-retrigger` (legal same-key) | PASS | `native-case-20260920T190721-51941c9a`, sink `1..6` |
| `release-gap-stress` | FAIL closed | `native-case-20260920T190723-d745d3d0`, KeyUp-before-matching-KeyDown |
| `mixed-up-down` | PASS | `native-case-20260920T190742-0ca22884`, sink `1..4` |
| `cleanup-full-release` | PASS | `native-case-20260920T190744-ea7fb840`, sink `1..30` |
| `focus-loss` / restore | PASS | `native-case-20260920T190845-84557c6b`, sink `1..19` |
| `target-hwnd-change` / generation | PASS | `native-case-20260920T190853-4238aaa0`, sink `1..15` |
| `pause-resume` | PASS | `native-case-20260920T190807-7105c68f`, sink `1..19` |
| `suspend-resume` | PASS | `native-case-20260920T190810-80e05dcd`, sink `1..19` |
| `stop-cleanup` | PASS | `native-case-20260920T190813-cbc7b789`, sink `1..2` |
| `skip-cleanup` | PASS | `native-case-20260920T190855-17c6871f`, sink `1..2` |
| `supervisor-lease-expiry` | PASS | `native-case-20260920T190827-22d73316`, sink `1..15` |
| `w4-noncanonical` | PASS | `native-case-20260920T190833-7f9a7add`, sink `1..2` |
| `timing-margin-sweep` | FAIL closed | `native-case-20260920T190834-65f980e8`, KeyUp-before-matching-KeyDown |
| `dense-alternating` + two hidden PowerShell busy workers | FAIL closed | `native-case-20260920T190906-42b6c79e`, same terminal reason |

The native harness supports lifecycle/control cases above but does not expose
the deterministic stale-boundary, unpaired-terminal-Down, transport-fault,
or exact QPC-cutoff injection seams. Those requirements are therefore mapped
to the executable deterministic tests in the numbered proof table, not
substituted with the physical scenario total. `hold`, `dense-alternating`, and
the release-gap stress author `hold_us = effective minimum hold` and the
minimum release gap. In particular, dense alternating has zero authored hold
slack: its Down reaches the static latest-start boundary with no musical slack
to absorb wake/scheduling delay. Its `DownExpiredBeforeSend` and resulting
KeyUp-before-matching-KeyDown are therefore exact-minimum-hold geometry under
the accepted static-validity contract, not evidence of a generic cold-gap
failure and not a reason to change scheduler or spin policy.

The host was interactive, so these are PASS/FAIL results rather than
`INCONCLUSIVE`. No result is a claim that a game received input; the sink is a
project-owned receive-only observer.

The requested musical/lifecycle coverage crosswalk is explicit here. “No” in
the physical column means the native harness has no safe scenario seam for
that injected deterministic condition; the named deterministic test is the
qualification evidence instead.

| Phase-6 requirement | exact native scenario or deterministic test | physical run | verdict / evidence |
|---|---|:---:|---|
| exact-minimum hold | native `hold`; `prepared_down_policy_materializes_exact_hold_slack_and_unpaired_state` | Yes | PASS; `...190708-597fbe76` / deterministic PASS |
| long single hold/sequence | native `long-single-sequence` | Yes | FAIL closed; `...190710-798a63b6` |
| dense alternating | native `dense-alternating`, plus `-CpuContention` | Yes | FAIL closed in idle/contention; `...190713-6a522cdd`, `...190906-42b6c79e` |
| legal same-key retrigger | native `rapid-retrigger` | Yes | PASS; `...190721-51941c9a` |
| near-minimum same-key retrigger | native `near-minimum-retrigger` | Yes | FAIL closed; `...190719-ab893691` |
| Mixed Up/Down | native `mixed-up-down`; `prepared_mixed_backlog_sends_only_the_canonical_up_prefix` | Yes | PASS physical; `...190742-0ca22884`; deterministic PASS |
| 2/3/15 stale Down-bearing boundaries | `prepared_no_catch_up_burst_matrix_is_exact_for_2_3_and_15_boundaries` | No | PASS deterministic; native harness cannot inject stale prepared boundaries |
| differing authored release times | `prepared_down_policy_uses_minimum_paired_chord_slack_and_ignores_unpaired_members`; native `chord-sweep` | Yes | deterministic PASS; physical FAIL closed at `...190716-3a236414` |
| unpaired terminal Down + cleanup | `prepared_unpaired_down_has_no_cutoff_but_requires_causal_authorization`; native `cleanup-full-release` covers cleanup only | No | PASS deterministic / cleanup PASS `...190744-ea7fb840`; no native unpaired-terminal injection seam |
| pause/resume | native `pause-resume` | Yes | PASS; `...190807-7105c68f` |
| focus loss/restore | native `focus-loss`; `focus_restore_after_grace_releases_and_resumes` | Yes | PASS; `...190845-84557c6b` / deterministic PASS |
| system suspend/resume | native `suspend-resume`; `system_suspend_invalidates_future_down_and_resume_requires_fresh_authorization` | Yes | PASS; `...190810-80e05dcd` / deterministic PASS |
| target HWND/generation change | native `target-hwnd-change`; `target_change_is_rejected_at_the_final_send_boundary` | Yes | PASS; `...190853-4238aaa0` / deterministic PASS |
| stop cleanup | native `stop-cleanup` | Yes | PASS; `...190813-cbc7b789` |
| skip cleanup | native `skip-cleanup` | Yes | PASS; `...190855-17c6871f` |
| supervisor lease expiry | native `supervisor-lease-expiry`; `supervisor_lease_treats_future_heartbeat_as_fresh` | Yes | PASS; `...190827-22d73316` / deterministic PASS |
| full cleanup/release | native `cleanup-full-release` | Yes | PASS; `...190744-ea7fb840` |

### Trace of the prepared 25 ms sender-expiry residual

The residual is from the distinct prepared baseline report
`.benchmarks/phase6-prepared-baseline-25ms.json`, under
`modes.baseline.raw_real_wait_report`; it is not from the clean
`real_wait_core` 25 ms matrix row. That baseline report has 20 observations:
19 complete sends, one `down_final_sender_window_expired`, one observation gap,
and `transport_anomaly_count=0`. Its sender-expiry evidence is:

| field | value |
|---|---:|
| `missed_down.final_sender_window_expired` | 1 |
| `missed_down.physical_window_expired` / `unobserved_backlog` | 0 / 0 |
| `dispatch_start_error_us` p50/p95/max | 7930 / 16829 / 18279 |
| `missed_pre_call_lateness_us` | 28773 |
| excess beyond latest valid Down start | 3773 us |
| final-policy→pre-call p50/max | 0 / 0 us |
| pre-call→completion p50/max | 2 / 11 us |

The source path is explicit: `rt_handoff_bench.rs::add_observation` maps
`DownMissKind::DownExpiredBeforeSend` to
`missed_down_final_sender_window_expired` and
`down_final_sender_window_expired`; the prepared dispatch recovery path emits
that miss only after the static latest-start check. The sender therefore made
zero full-packet attempts, as required. The measured excess is upstream
wake/dispatch lateness relative to the static sender-validity boundary, not a
transport retry or a hidden sender-expiry policy. The clean 25 ms matrix row
must remain separately identified as clean.

## Allocation and optimized assembly

- `cargo test --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc`: 23 passed.
- Dist assembly was emitted for `sky_player` and `sky_dispatch_win32` with
  `cargo rustc --profile dist -- --emit=asm`.
- Exact dist/audit command shape:

  ```powershell
  $env:CARGO_TARGET_DIR = 'C:\Temp\sky-auto-player-357-target'
  cargo rustc --manifest-path rust/Cargo.toml --profile dist -p sky_player `
    --features test-support --lib -- --emit=asm
  cargo rustc --manifest-path rust/Cargo.toml --profile dist -p sky_dispatch_win32 `
    --lib -- --emit=asm
  & .\scripts\audit_prepared_normal_assembly.ps1 `
    -AssemblyPath C:\Temp\sky-auto-player-357-target\dist\deps\sky_player-6da0da9f341438e6.s
  & .\scripts\audit_dispatch_assembly.ps1 `
    -AssemblyPath C:\Temp\sky-auto-player-357-target\dist\deps\sky_player-6da0da9f341438e6.s
  ```

  The receive-only decoder command was
  `pwsh -NoProfile -File scripts/native_acceptance_sink.ps1 -SelfTest`.
- `scripts/audit_prepared_normal_assembly.ps1` passed against the exact
  generated `sky_player` assembly: one prepared sender call, zero retained
  precision-helper calls, zero scoped forbidden calls, and zero scoped copy or
  division symbols.
- The broader report-only `audit_dispatch_assembly.ps1` was also run. It
  exits nonzero because the legacy `dispatch_due_from_plan` report target
  contains one `memmove`; the scoped normal-prepared audit above is clean and
  the Phase-6 change did not touch that legacy path.

## Verification

Passed after the final tracked edit:

```text
cargo test --manifest-path rust/Cargo.toml -p sky_dispatch_core --lib       # 56
cargo test --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --lib     # 239
cargo test --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib  # 337
cargo test --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc  # 23
cargo xtask check static
cargo xtask check rust
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
```

`check static` passed with 30 existing allowlisted architecture warnings and
zero security findings. `check rust` passed fmt, clippy `-D warnings`, and
the locked all-features workspace test phase after using the isolated
`CARGO_TARGET_DIR=C:\Temp\sky-auto-player-357-target` because the D: build
volume was full. The initial same gate attempt failed only at Windows linker
resource exhaustion (`LNK1318`/OS error 112), not a source or test failure.

## Disposition and residual risks

The cold-gap decision is explicit:

- Cold/long-gap wake was **not materially and consistently worse** than hot
  wake on this host. The 20 ms hot row had a target→pre-call max of 6714 us,
  while the clean 25 ms cold row had max 54 us; isolated cold wake tails were
  observed at 250 ms (2544 us) and 500 ms (1474 us). The long-gap rows have
  only 8/4/2/1 observations and are descriptive, not tail-statistics claims.
- Startup calibration used six samples, a 250 us spin floor, a 20,000 us
  calibration budget, a 2,000 us readiness reserve, and effective adaptive
  thresholds of 403/817/1000/612/830/651/845 us for the 5/20/25/100/250/500/
  1000 ms rows. The observed production-adaptive cold target→wake tails were
  covered in every clean row; the 25 ms startup robust estimator (1091 us)
  exceeded its 1000 us threshold, but its observed target→wake distribution
  was zero and its 20 observations were clean. This is evidence, not a tuning
  recommendation.
- The isolated prepared-25 ms miss is dominated by wait/wake-to-static-validity
  lateness: final-policy→pre-call was zero and pre-call→completion stayed
  bounded. Dense native misses are dominated by exact-minimum-hold zero-slack
  geometry and the accepted static cutoff. Neither result is evidence for a
  production transport retry, causal-admission, foreground, or coordinator
  change.
- Separate frozen hot/cold spin experiment: **INSUFFICIENT EVIDENCE**. The
  non-monotonic hot/cold observations and low long-gap sample counts do not
  justify opening or implementing a tuning experiment from this package.

The deterministic causal/transport/ownership contract, unchanged 16/14/2
totals, zero rebases, intentional single anomaly, no-allocation gate, scoped
optimized normal-prepared audit, and real foreground query-count evidence are
recorded. The host qualification is not a blanket PASS because the prepared
25 ms baseline has the one traced sender-expiry residual and native
dense-alternating fails closed under both idle and CPU contention. These are
reported results, not reasons to change scheduler/spin/cutoff/admission policy
in Phase 6.

No claim is made about game Raw Input, render timing, audio onset, or gameplay
observation. Any future tuning discussion must be a coordinator-approved
follow-up and must not be inferred from this measurement package.
