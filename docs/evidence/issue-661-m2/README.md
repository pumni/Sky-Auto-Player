# Issue #661 — M2 implementation evidence

**Status:** implementation candidate for Leader review

**Issue / PR:** [#661](https://github.com/pumni/Sky-Auto-Player/issues/661) / [draft PR #662](https://github.com/pumni/Sky-Auto-Player/pull/662)

**Branch:** `refactor/dispatch-runtime-cleanup`

**Accepted M1 base:** `1538c29c443fdfa3d815a92a83dabd2ab06ea93c` (tree `3abb133b0cc8ba337d2bf41535023abd6849abd9`)
**M2 implementation commit / tree:** `d00191aad6e82b3d655f9b1b96f5514902ad7ace` / `d2e2940f4e6df24e1fc84bb2db251e9190323b81`. The final evidence-only documentation commit and full M2 HEAD are reported in the handoff.

This report covers M2 only. M3 remains on hold. No gameplay transport, timing policy, deadline, key mapping, final admission gate, or wire schema was changed.

## M2 changes

### A. Observer-aware staging

`DispatchExecutor::up_slot_timing_summary` walks active bits without constructing `UpSlotTiming` records. Staging keeps the slot count, first source action, authored Up target, release not-before QPC, and hold-floor mask in a scalar summary. It constructs the fixed-capacity `SmallVec<UpSlotTiming>` only when the observer queue is present. Observer-on records still retain every slot's source and timing data. Observer-off expected/not-captured accounting uses the scalar slot count.

The stale mixed-target path uses the scalar summary to rebuild the original Up-only timing window. The regression deliberately schedules the mixed Down before the Up so that using the mixed packet's earlier deadline would fail the test. Out-of-range mask bits are ignored in the same way as `up_slot_timings`; active scheduler slots remain bounded to 15.

### B. Production diagnostics work

`WorkerCore.health` and its boot-time fixed-window initialization had no runtime reader; the observer-owned `WorkerHealthState` and live health windows remain. Internal `WorkerMetricsLocal.timeline_rebase_*` values had no live writer. Those internal fields and their merge copies were removed while exported snapshot keys remain at their invariant values (`0` / `None`).

Live dispatch still checks the existing 50 ms publication cadence. Backend counters are now copied only after that cadence is due and a nonblocking dispatch snapshot writer is available. Focus/manual-pause lifecycle publication, terminal publication, and finalization keep their direct backend-counter refreshes. Snapshot lock ordering, memory ordering, and ownership are unchanged.

### C. Scheduling glue

`SchedulerAction::Send` now carries the copied `DownChordMetadata` returned by scheduler evaluation. Staging no longer saves a chord index and resolves/checks the current chord a second time. A small staging-admission value carries the existing target, focus, observer, and control inputs through stale-target Up-only replans. Prepared packets are still built before waits; final Down admission still rechecks target generation and the existing focus, control/power, modifier, and supervisor lease conditions.

### D. Consumer-driven audit

The Rust workspace, native calibration, examples, scripts, Windows feature-gated surfaces, engine snapshots, Tauri diagnostics DTO, generated TypeScript DTO, and diagnostics consumers were searched. No Desktop or TypeScript file changed. Exported diagnostic fields remain compatible, and active planner, health, sender, and H/G behavior remains in place.

## Invariant and regression results

| Invariant | Regression evidence | Result |
|---|---|---|
| Observer off stages no per-slot sidecar; observer on preserves each slot's source and timing | `staging_captures_up_slots_only_when_observer_is_enabled`; `coalesced_up_packet_preserves_each_authored_source_in_consumer` | Pass |
| Expected Up slots remain partitioned as captured, dropped, and not captured by mode, including sidecar or ring loss | `absent_observer_counts_up_slots_as_not_captured_by_mode`; `missing_pool_storage_is_counted_once_when_observer_queue_is_also_full`; `coalesced_up_sidecar_loss_is_counted_when_parent_observation_is_dropped`; `telemetry_ring_drop_accounts_each_coalesced_up_slot`; observer abort accounting tests | Pass |
| Mixed stale-target replan sends Up-only at the original Up deadline and retains its hold-floor mask | `stale_target_replans_mixed_candidate_to_independent_up_only_packet` with an earlier Down target and later Up target | Pass |
| Deadline reevaluation carries the Down metadata from that evaluation | `scheduler_deadline_reevaluation_carries_down_metadata_from_evaluation` | Pass |
| Fresh final Down admission and control rejection remain enforced | `final_control_rejects_staged_down_and_up_before_send`; `live_scheduler_final_down_gate_rejects_target_lease_and_control_without_commit`; target generation race tests | Pass |
| Focus loss does not send musical input; restoration and cleanup stay guarded | `focus_loss_at_first_down_admission_never_sends_physical_down`; `focus_loss_during_post_wait_hook_never_sends_physical_down`; `focus_loss_probe_cannot_report_cleanup_success`; lifecycle cleanup matrix tests | Pass |
| Sender attempt accounting, raw QPC evidence, and transport journal remain intact | `failed_sender_paths_retain_bounded_attempt_and_clock_evidence`; `pre_call_qpc_failure_records_zero_physical_attempts`; `mixed_post_call_qpc_failure_preserves_transport_journal_without_scheduler_commit`; Win32 tracked sender tests | Pass |
| Healthy staged playback remains allocation-free | `rt_dispatch_no_alloc`: 4 passed, including mixed Up+Down and 1/5/15-key packets | Pass |
| Core H/G, scheduler timeline, and planner compression behavior remain covered | Full `sky_dispatch_core` suite, including completion-based hold/release floors and `absolute_scheduler_hol_compression_preserves_all_down_intents` | Pass |

The `dispatch_loop` targeted run passed 22 tests; the two focus-loss tests passed; the Win32 focus-loss cleanup and tracked note-on accounting targeted tests each passed.

## Hot-path work: before and after

| Operation | Before M2 | After M2 | Evidence / limit |
|---|---|---|---|
| Up-slot observer staging | Every packet with Up built and populated a `SmallVec<UpSlotTiming>`, including when no observer queue existed. `SmallVec` is inline for up to 15 entries, so this was record construction/copying, not a heap allocation. | Every Up packet retains a scalar summary; the per-slot vector is built only while an observer queue is enabled. Observer-on attribution is unchanged. | Staging regression, per-slot source regression, full no-allocation integration suite. The scalar still visits the active Up bits to preserve exact summary timing. |
| Backend-counter preparation | With live diagnostics enabled, each dispatched packet copied backend masks/counters into `WorkerMetricsLocal` before the existing cadence gate, even when no snapshot could be published yet. This included two `count_ones` operations and scalar counter reads/copies. | The same copy runs only when the existing 50 ms cadence is due and the nonblocking snapshot writer is acquired. Lifecycle and finalization refreshes remain immediate. | `dispatch_projection_update_runs_only_for_a_publishable_snapshot` checks cadence skip and writer contention. The snapshot was already cadence-limited; no snapshot freshness contract changed. |
| Snapshot projection | Already cadence-gated, plus lifecycle/terminal publication. | Unchanged cadence and transition publication. | Metrics cadence and lifecycle contention tests pass. |
| Down chord metadata | Evaluation returned metadata; staging retained an index and looked up/validated the current chord again. | Evaluation's copied metadata flows directly into staging. | Deadline reevaluation test passes; final admission checks remain separate and fresh. |
| Unused health state | `WorkerCore` held a second `WorkerHealthState` initialized at boot without a runtime reader. | The unused field and initialization are gone; observer health windows remain live. | Workspace reader search; health-window and observer tests pass. |

No baseline/candidate benchmark was run. This is reported as **cleanup**, with no speedup claim. No syscall duration is presented as game latency.

## Internal field/helper audit

| Field / helper | Live consumer or evidence | Decision | Output effect |
|---|---|---|---|
| `WorkerCore.health` | Whole-Rust search found only its boot-time initialization and declaration; no reader. | **REMOVE** | Internal worker state only; no output change. `WorkerHealthState` itself remains for the observer. |
| `WorkerMetricsLocal.timeline_rebase_count`, `*_total_us`, `*_max_us`, `*_total_ticks`, `*_max_ticks`, `*_last_reason` | No runtime writer; only observer merge and snapshot projection read them. | **REMOVE** | `EngineSnapshot` fields and acceptance JSON keys remain, supplied as `0` / `None`. |
| `timeline_rebase_reason` | Only mapped the invariant internal reason code to `None`. | **REMOVE** | Same exported `timeline_rebase_last_reason: null`. |
| `timeline_rebase_*` in engine snapshot / acceptance output | Acceptance checks and JSON report consume these fields. | **KEEP** | Keys and zero/`None` values preserved. |
| `missed_down_*` and `final_sender_window_expirations` | Tauri DTO, generated `DiagnosticsSnapshotDto`, diagnostics view, mocks/tests, and qualification scripts. Current Rust snapshot projection supplies zero for fields without a writer. | **KEEP** | No wire or TypeScript schema change. |
| `applied_lead_ticks` | Serialized `RtTraceRecord` field, initialized to zero under the current authored-timeline semantics. | **KEEP** | Trace key and value semantics preserved. |
| `WorkerHealthState`, `HealthWindow`, health/observation counters and windows | Observer thread, observation/trace processing, health logic, and tests consume them. | **KEEP** | No health or diagnostics behavior change. |
| `resolve_scheduler_down_chord` and `down_chord_index` | Private index-to-current-chord lookup had no consumer beyond staging. | **REMOVE** | No output; metadata now flows from the evaluation result. |
| `up_slot_timings` | Observer attribution records consume every slot when observation is enabled. | **KEEP** | Same observer records and trace attribution. |
| `try_publish_metrics` | Unit-test convenience wrapper only after production switched to the cadence-update helper. | **KEEP, test-only** | No production symbol or behavior change. |
| Raw/prepared/tracked sender helpers | Calibration uses raw and targeted prepared senders; worker and sender tests use tracked prepared paths. | **KEEP** | No sender API or syscall change. |
| Planner compression branches | Core scheduler behavior tests prove delayed gaps and all Down intents are preserved. | **KEEP** | Timing and output contracts unchanged. |
| Production H/G calculations | Executor owns dispatch timing; production forensics independently checks actual completion-derived floors; timing model is an offline advisory. | **KEEP separately** | No calculation merge or H/G semantics change. |

## Validation

All commands below ran on Windows against the final M2 code. The diff check ran after the evidence files were added. Exit codes are actual process exit codes.

| Command | Exit | Result |
|---|---:|---|
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 | Pass |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | 0 | Pass |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core` | 0 | 75 unit + 2 property passed; 0 failed/ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | 276 passed; 0 failed; 1 ignored (retained child-process probe) |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib` | 0 | 253 passed; 0 failed/ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | 4 passed; 0 failed/ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib dispatch_loop` | 0 | 22 passed; 231 filtered |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib focus_loss_` | 0 | 2 passed; 251 filtered |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support focus_loss_probe_cannot_report_cleanup_success` | 0 | 1 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support tracked_note_on_uses_one_send_attempt_without_retry` | 0 | 1 passed |
| `git diff --check` | 0 | Pass. |

**First-run failures:** An intermediate focused scheduler test compile failed while the test module and mutable metrics signatures were being edited; it passed after those edits. The first draft clippy run failed on a test-only wrapper import and two staging functions over the argument-count lint. The first cached diff check also flagged the generated CRLF report before its Windows line-ending rule was recorded. These were corrected; the final clippy and cached diff checks passed. These were implementation/evidence-draft failures, not flaky runs. No flaky result was observed.

**Not run:** native Final qualification; `cargo xtask check all` (both are later qualification gates).

## Lifecycle evidence

Command: `pwsh -NoProfile -File scripts/run_dispatch_lifecycle_evidence.ps1` — exit 0.

The clean-checkout report is [dispatch-lifecycle-report.json](dispatch-lifecycle-report.json), SHA-256 `6bd5844ee4ffd6c132eda5292e992e12b766d532b007a4a5ae89cd0a79ad0e63` for the generated Windows CRLF bytes. Git stores the text-normalized blob and the repository rule checks it out with CRLF on Windows. The report identifies source commit `d00191aad6e82b3d655f9b1b96f5514902ad7ace`, tree `d2e2940f4e6df24e1fc84bb2db251e9190323b81`, schema v2, `live_scheduler`, and `offline_test_support` with deterministic mock transport. Result: PASS; no native `SendInput` calls. No-pause lifecycle: four musical attempts, zero cleanup attempts, final obligation mask zero. Pause/cleanup/resume: two musical attempts, two cleanup attempts, `G-1` waits, exact `G` sends, final obligation mask zero.

## Remote status at handoff

The CI run triggered by the initial M2 push (commit `8d2bcdcae8f0d9830b2faf06ca6964ce115f4154`) was in progress at the first status query (run `43691665068`). The latest full M2 HEAD status is recorded in the PR handoff comment.

Blockers / Leader decisions: none identified.

M3: held; no M3 work started.
