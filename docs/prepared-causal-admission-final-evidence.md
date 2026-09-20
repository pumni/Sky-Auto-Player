# Phase 6 — Prepared Causal Admission Final Evidence

Status: qualification evidence for issue #357. This package does not change
production dispatch policy or claim game observation, Raw Input delivery,
rendering, or audio onset.

## Revisions and host

- Exact fetched base and `origin/main`: `98f0dd1f9348f8235b17ee6d1748ace1f8ca060b`.
- Branch: `qualify/357-prepared-causal-final`.
- Worktree: `D:\Dev\Sky-Auto-Player-357`.
- Qualification head: the commit containing this document and the
  measurement-only benchmark input-range extension.
- Toolchain: Rust `1.98.1 (48a229cea 2026-09-01)`, Cargo `1.98.1`,
  `x86_64-pc-windows-msvc`, LLVM `22.1.8`.
- Host: Windows 11 Home, Dell Inspiron 5515, 12 logical processors, QPC
  frequency `10,000,000 Hz`, PowerShell `5.1.26100.9444`.

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

Values are microseconds for the 1-key production-adaptive Down scenario. The
columns are p50/p95/p99/p99.9/max. At low sample counts p99.9 is necessarily
the same order statistic as p99.

| gap | target→pre-call | wake→final policy | final policy→pre-call | pre-call→completion |
|---:|---|---|---|---|
| 5 ms | 24/187/187/187/209 | 31/31/56 | 4/4/7 | 2/2/2/2/3 |
| 20 ms | 76/2374/2374/2374/6714 | 39/39/52 | 8/8/8 | 2/3/3/3/4 |
| 25 ms | 24/40/40/40/54 | 34/34/34 | 5/5/21 | 2/2/2/2/2 |
| 100 ms | 22/33/33/33/52 | 29/29/46 | 4/4/6 | 1/2/2/2/2 |
| 250 ms | 1269/2095/2095/2095/2611 | 37/37/58 | 4/4/7 | 2/2/2/2/3 |
| 500 ms | 388/388/388/388/1507 | 28/28/44 | 4/4/7 | 1/1/1/1/2 |
| 1000 ms | 42/42/42/42/42 | 37/37/37 | 5/5/5 | 1/1/1/1/1 |

The waiter evidence also records wake-to-target, spin time/duty, and startup
wake-error distributions in each raw JSON report. No waiter failure, interrupt,
or replan occurred in this matrix.

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

The existing project-owned `scripts/native_acceptance_sink.ps1` receive-only
window was used; no arbitrary application received input.

- `canonical-single`, timing margin 500 µs: PASS, sink event window `1..2`
  (2 events), evidence under
  `.benchmarks/physical-native-cases/native-case-20260920T175126-21162313`.
- `canonical-chord`, timing margin 500 µs: PASS, sink event window `1..4`
  (4 events), evidence under
  `.benchmarks/physical-native-cases/native-case-20260920T175229-853cc176`.
- `dense-alternating`, idle and with two hidden PowerShell CPU workers:
  FAIL closed in the runner with `KeyUp arrived before its matching KeyDown`
  after all Down boundaries reached `DownExpiredBeforeSend`; no production
  timing policy was changed to mask this host/scenario result. Evidence:
  `native-case-20260920T175153-ea6b8e67` (idle) and
  `native-case-20260920T175142-868333d6` (contention).

The host was interactive, so the sink results are PASS/FAIL evidence rather
than `INCONCLUSIVE`. The dense failure is retained as a qualification risk;
it is not reclassified as a game-observation result.

## Allocation and optimized assembly

- `cargo test --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc`: 23 passed.
- Dist assembly was emitted for `sky_player` and `sky_dispatch_win32` with
  `cargo rustc --profile dist -- --emit=asm`.
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

The deterministic causal/transport/ownership contract, unchanged 16/14/2
totals, zero rebases, intentional single anomaly, no-allocation gate, scoped
optimized normal-prepared audit, hot/cold threshold classification, and real
foreground query-count evidence are recorded. The host qualification is not a
blanket PASS because prepared 25 ms baseline evidence recorded one
`FinalSenderWindowExpired` and the native dense-alternating sink scenario
failed closed under both idle and CPU contention. These are reported results,
not reasons to change scheduler/spin/cutoff/admission policy in Phase 6.

No claim is made about game Raw Input, render timing, audio onset, or gameplay
observation. Any future tuning discussion must be a coordinator-approved
follow-up and must not be inferred from this measurement package.
