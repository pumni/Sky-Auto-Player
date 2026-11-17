# Issue #666 M3 — Session Ownership and Lifecycle Evidence

**Status:** ready for Leader review; M3 has not been accepted.

**Revision under review:** `refactor/prepared-playback-runtime`, based on
accepted M2 `eac4dd4cbe434f9a36b7fa0b1453143bf0fdb217`.

**Environment:** Windows x64 MSVC (`x86_64-pc-windows-msvc`), Rust
`1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`,
workspace lockfile.

## Implementation

`SessionTarget` now owns the target HWND/generation, owner PID/generation, and
an `Arc<WindowIdentityAuthority>` retaining the query-only process handle and
startup process-creation proof. Publishing another target generation first
invalidates owner authority. A worker that already holds an authority clone
may finish its bounded query, but the generation proof cannot authorize a
replacement HWND or process. The handle remains alive while the worker or
watchdog still holds shared session state, including after the public session
owner is dropped.

For focus-required musical Down, Up-only, and mixed packets, final admission
rechecks foreground HWND/PID, retained process creation time, target
generation, focus/control state, and the applicable lease immediately before
the sender boundary. The target authority is cloned with a nonblocking lock;
contention is a pre-call rejection and does not consume an onset. The final
process query and authority clone do not allocate. The real-time allocation
regression includes this path.

Resumable and terminal tracked safety release use their separate retained
ownership proof and do not require foreground focus. Terminal release now
withholds its syscall when the target generation or retained owner proof is
unavailable. The outstanding obligation remains recorded and finalization
publishes an error; no release evidence or completion time is fabricated.

The supervisor watchdog, progress lease, callback epoch/rundown, power
registration lifetime, focus pause/restore, one-attempt Down, H/G barriers,
sender boundary, and suspend-as-terminal behavior are retained. No watchdog,
lease, callback, ownership, or presentation guard was removed or relaxed.
Telemetry schema 25 and public DTO/fingerprint are unchanged.

## Lifecycle and ownership matrix

All mock rows below were executed by the Windows `test-support` package suites
or the desktop-native test gate. “Actual” describes the observed test result;
it does not claim game delivery.

| Area | Test / exercised path | Expected | Actual |
| --- | --- | --- | --- |
| Generation publication | `target::tests::owner_identity_is_bound_to_one_generation_and_invalidated_on_transition`; `owner_bind_racing_target_writer_never_authorizes_the_new_generation`; `target_aba_and_repeated_transitions_invalidate_frozen_proof` | Old HWND/PID/process authority cannot authorize a new generation or reused numeric HWND | PASS; old binding becomes stale and publication races fail closed |
| Authority lifetime | `retained_owner_authority_tracks_generation_and_worker_clone_lifetime` | New target drops stored old binding; an in-flight worker clone keeps it alive until the query finishes | PASS; the old authority drops after the clone is released |
| Cancel before Arm / target admission | `stop_skip_and_shutdown_cancel_the_published_pre_arm_session`; `physical_startup_target_failure_retires_without_reentrant_active_lock`; `target_integrity_admission_precedes_focus_and_session_arm` | Pre-Arm cancellation and target-integrity failure retire before physical activation; no stale publication can arm | PASS in `cargo xtask check desktop-native` |
| Final musical gate | `worker_final_gate_rechecks_retained_process_identity_before_down` | Continuous process identity allows the schedule; terminated identity rejects before Down syscall | PASS; continuous case finishes, terminated case records error and sends no Down |
| Musical Up-only and terminal release | `musical_up_only_rechecks_process_identity_and_cleanup_remains_fail_closed` | Process loss blocks musical Up and terminal safety Up; preserve release obligation and fail closed | PASS; only the original Down syscall occurs, Up is withheld, obligation remains, outcome is Error |
| Safety path without focus | `terminal_tracked_cleanup_uses_owner_proof_without_foreground_requirement` | Valid owner proof permits tracked cleanup after foreground loss | PASS; Skip causes one tracked Up, clears the obligation, and stays Skipped |
| Owner unwind and lease | `dropped_session_owner_leaves_watchdog_to_release_then_drops_target_binding` | Owner drop cannot strand a Down; watchdog wakes cleanup and shared authority retires after both threads exit | PASS; one Down plus one cleanup Up; retained authority then drops |
| Armed/target/power races | `suspend_at_epoch_prepared_barrier_prevents_armed_publication_and_down`; `skip_panic_suspend_target_owner_and_lease_stops_win_ready_to_armed`; shared suspend-vs-Armed and owner-generation race tests | Exactly one lifecycle gate wins; no stale target or suspended session sends a Down | PASS in `sky_player` suite |
| Long wait, stop, shutdown and join | `shutdown_timeout_keeps_the_physical_reservation_and_omits_release_proof`; `native_worker_cleanup_barrier_keeps_session_exclusive_until_join_and_release`; `join_timeout_keeps_session_exclusive_until_delayed_terminal_cleanup` | Wake/cleanup/join remain bounded; replacement session waits for release evidence | PASS in `cargo xtask check desktop-native` |
| Worker panic / terminal outcome | `startup_panic_is_contained_before_armed_and_produces_error_not_finished`; `recovered_join_error_cannot_be_masked_by_a_requested_stop`; `terminal_handoff_uses_worker_result_and_release_evidence_after_join` | Panic and join failure cannot be projected as success; finalization and release evidence stay coupled | PASS in `sky_player` and desktop-native suites |
| Watchdog / presentation isolation | `native_monitor_runtime_stall_expires_lease_and_resumption_cannot_heal_it`; `blocked_snapshot_projection_past_lease_keeps_supervisor_live`; `blocked_terminal_presentation_does_not_hold_supervisor_after_release`; `blocked_ui_publication_past_lease_keeps_supervision_live_and_shutdown_releases` | UI or projection stall cannot keep playback authority alive or stop supervisor progress | PASS in `cargo xtask check desktop-native`; watchdog and lease remain active |
| Callback epoch / rundown | `stale_resume_callback_cannot_clear_new_session_suspend`; `reset_waits_for_callback_that_already_acquired_epoch_authority`; `stale_suspend_cannot_publish_boundary_counters_or_signal_after_reset`; `suspend_latches_before_callback_qpc_sampling_and_blocks_armed_commit` | Old callbacks cannot mutate a later epoch; close waits for an already-authorized callback | PASS in `sky_player` suite |
| Callback registration lifetime | `stable_callback_context_routes_current_epoch_and_ignores_retired_epoch`; `failed_notification_unregistration_is_reported_and_keeps_callback_state_alive`; `close_and_unregistration_failures_are_retried_during_raii_drop`; `callback_context_cardinality_is_bounded_across_session_registration_stress` | Late callback is fenced; in-flight callback context is retained through unregister/rundown and bounded across sessions | PASS in `cargo xtask check desktop-native` |
| Rapid resume and consecutive sessions | `suspend_then_resume_before_worker_observation_does_not_restore_startup_or_down_gate`; `suspend_boundary_is_captured_before_worker_wakes_and_survives_resume_notification`; `stale_resume_callback_cannot_clear_new_session_suspend` | Quick resume does not undo terminal suspend; an old session callback cannot alter the next session | PASS in `sky_player` suite |
| Focus pause/restore | `production_worker_focus_restore_resumes_after_grace_with_mock_sender`; M2 resume barrier and tracked cleanup regressions | Active Up obligation is released only through the safety path; Down resumes only after cleanup completion + G | PASS in `sky_player` suite and no-allocation tests |
| Healthy real-time path | `production_staged_worker_mixed_up_down_packet_no_alloc`; `production_staged_worker_1_5_and_15_key_chords_no_alloc`; `lifecycle_resume_barrier_exact_g_stages_and_sends_once_without_allocating`; `lifecycle_resume_barrier_g_minus_one_waits_without_allocating_or_sending` | No heap allocation; packet and H/G behavior unchanged | PASS; 4/4 dedicated RT tests |
| Windows process object | `focus::tests::retained_process_object_reports_child_termination_without_pid_reopen` | Retained query-only process handle detects child exit without reopening by PID | PASS using a test-owned child process and Windows process APIs |

The first targeted run of the watchdog owner-drop test failed its capture-shape
assertion because the mock represents terminal cleanup as `SingleEmitter Up`,
not as a packet. The assertion was corrected to the actual sender contract; the
rerun passed and checks the exact Down packet, one cleanup Up, watchdog wake,
and authority drop. The first Up-only process-loss runs also exposed that
terminal finalization bypassed owner proof; terminal cleanup now checks that
proof and the regression passes with no second syscall.

## Verification commands and results

All commands were run on the environment listed above. Exit codes below are
the actual process exit codes. Commands are ordered as run; repeated invocations
are listed separately.

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all` | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all` | 0 | PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all` (after the final generation-recheck edit) | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all` (after the final generation-recheck edit) | 0 | PASS |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib retained_owner_authority_tracks_generation_and_worker_clone_lifetime` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support worker_final_gate_rechecks_retained_process_identity_before_down` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc production_staged_worker_mixed_up_down_packet_no_alloc` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support dropped_session_owner_leaves_watchdog_to_release_then_drops_target_binding` (initial assertion) | 1 | FAIL; expected packet cleanup capture but actual mock uses `SingleEmitter Up` |
| Same watchdog owner-drop command (corrected assertion) | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support musical_up_only_rechecks_process_identity_and_cleanup_remains_fail_closed` (before terminal proof fix) | 1 | FAIL; found terminal cleanup issued an unverified second syscall |
| Same Up-only process-loss command (second pre-fix observation) | 1 | FAIL; confirmed `ProcessTerminated` still allowed terminal cleanup, obligation cleared |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support musical_up_only_rechecks_process_identity_and_cleanup_remains_fail_closed` (after terminal proof fix) | 0 | PASS, 1 selected; only original Down syscall, obligation retained, Error outcome |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support terminal_tracked_cleanup_uses_owner_proof_without_foreground_requirement` (first compile) | 1 | FAIL; test referenced a snapshot field that does not exist |
| Same terminal cleanup test (first assertion removed) | 1 | FAIL; another obsolete snapshot assertion did not compile |
| Same terminal cleanup test (after correcting assertion) | 0 | PASS, 1 selected |
| `git diff --check` (pre-final report/source pass) | 0 | PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` (pre-final report/source pass) | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` (pre-final report/source pass) | 0 | PASS |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` (first run) | 1 | FAIL; Windows test literal needed the new test-only identity override field |
| Same Clippy command (after initializing the test field) | 0 | PASS, warnings denied |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` (after the final generation-recheck edit) | 0 | PASS, warnings denied |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support worker_final_gate_rechecks_retained_process_identity_before_down` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support musical_up_only_rechecks_process_identity_and_cleanup_remains_fail_closed` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support dropped_session_owner_leaves_watchdog_to_release_then_drops_target_binding` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc production_staged_worker_mixed_up_down_packet_no_alloc` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | PASS, 84 tests; 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | PASS, 277 passed, 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | 0 | PASS, 275 unit + 4 RT integration tests; 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | PASS, 4 tests; 0 failed |
| `cargo xtask check desktop-native` | 0 | PASS; desktop shell checks and Tauri test suite completed |
| `cargo xtask check desktop-native` (after the final generation-recheck edit) | 0 | PASS |
| `cargo xtask check all` (before report file was added) | 1 | FAIL; local Markdown link check correctly found the not-yet-created M3 report |
| `cargo xtask check all` (after report creation, before the final target-generation recheck edit) | 0 | PASS; workspace, desktop, Bun, and Playwright stages completed |
| `cargo xtask check all` (after final target-generation recheck edit, first run) | 1 | FAIL; workspace test subprocess returned 101 in `sky_player --lib`; the captured xtask diagnostic was truncated before the failing test name |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --all-features --lib` | 0 | PASS, 275 passed; 0 failed |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-features --locked *> rust/target/m3-workspace-test.log; $testExit = $LASTEXITCODE; Get-Content rust/target/m3-workspace-test.log -Tail 160; exit $testExit` | 0 | PASS; full workspace test command completed, 0 failed |
| `cargo xtask check all` (final retry on final code) | 0 | PASS; docs/static/security checks, workspace Rust Clippy/tests, desktop tests, Bun check, and Playwright E2E completed; architecture check reported 64 advisory warnings |
| `git diff --check` (final source/report) | 0 | PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` (final source/report) | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` (final source/report) | 0 | PASS |

No `cargo xtask check all` invocations overlapped. The initial failed run
stopped at the missing report link. After the last source edit, one workspace
test subprocess returned 101 without preserving its failing test name in the
captured diagnostic; reruns of the all-features `sky_player` library, the full
workspace test command, and then the complete xtask gate all passed. The final
xtask run completed workspace Rust tests, desktop-native tests, Bun checks,
and Playwright E2E. Architecture warnings are advisory and the architecture
check returned PASS.

## Limits and qualification boundary

- Sender and lifecycle fault tests use deterministic mocks. They prove
  controller decisions, syscall-attempt counts at the mock boundary, cleanup
  obligations, ordering, and state publication; a mock does not prove that a
  game received a note.
- One Win32 test uses a real Windows process handle and child termination to
  verify `GetProcessTimes` continuity. Foreground HWND/PID, power notification,
  sender insertion, and gameplay reception were not qualified against Sky or
  another interactive application.
- No real `SendInput`, gameplay, real suspend/resume, or native interactive
  focus test was run. No native test budget was used.
- The accepted M1/M2 invariants remain in force, including one-attempt Down,
  actual sender-completion QPC for H/G, schema 25, and bounded cleanup. M3 adds
  no performance/gameplay qualification and changes no M4 policy.

## M3 review corrections

These corrections address Leader review of M3 commit
`9d4907a81e51f2df53bfc831084f1d4ec38461f0` (tree
`0f2090eaf51f3c267d310bbbd6ae777fa7270b11`). They do not change target-rebind
policy or permit cleanup to cross an owner generation.

### Owner poll and close race matrix

| Race | Expected | Actual |
| --- | --- | --- |
| Poll snapshots A at generation N; A reports `ProcessTerminated`; publish and bind B at N+1 before applying the poll result | A's result must not change B's `owner_live`, failure, retained authority, or pre-Arm admission | PASS: stale poll returns false; B's retained authority remains live, B poll succeeds, final admission sends exactly one Down and one Up, and session finishes |
| A remains current and reports `QueryUnavailable` | Invalidate A; cancel pre-Arm if applicable; no Down | PASS: current owner is invalidated; zero sender calls; final outcome is Error |
| Close snapshots A; publish and bind B before late close applies | Close must not clear B | PASS: B's authority remains retained and its poll succeeds |

Poll result application and late close compare the snapshotted target generation
and retained authority `Arc` identity while holding the writer serialization
shared with publication and bind. PID or HWND equality alone is insufficient.
An old poll still wakes the worker so it can observe control-plane changes; it
does not invalidate or cancel the new owner. Existing target-change-before-Arm
startup cancellation remains unchanged.

### Tracked release-owner matrix

Each physical slot keeps a fixed-capacity origin proof before a Down can create
an in-flight release obligation. The proof freezes HWND/generation, PID, and
retained authority object identity where process identity is required. A
verified cleanup holds the final generation-conditional writer permit through
the safety syscall. It does not require foreground focus. Drift or uncertainty
withholds the syscall and preserves the release mask.

| Scenario | Expected | Actual |
| --- | --- | --- |
| Down(A), publish/bind B, terminal Quit | No Up on B; A obligation retained; Error | PASS: 1 Down call, 0 safety Up calls, mask `0x0001`, 0 release attempts, Error |
| Down at HWND A / generation N, republish the same numeric HWND at N+1 and bind new authority | Numeric HWND equality must not transfer the release proof | PASS: same result as A-to-B; 1 Down, 0 safety Up, mask `0x0001`, Error |
| Down(A), lose foreground, terminal Stop while A is unchanged and its retained authority is continuous | Safety Up may run without foreground; release is verified | PASS: 1 Down plus 1 safety Up; mask clears and requested Skip remains Skipped |
| Down(A), manual pause, rebind B before pause cleanup | Resumable cleanup cannot use B's proof for A | PASS: 1 Down, 0 safety Up; mask `0x0001`, Error |
| Down(A), terminal cleanup continuity query is in flight while B is published | Final generation check must reject before syscall | PASS: 1 Down, 0 safety Up; mask `0x0001`, Error |
| A release proof is captured without owner identity, then an authority binds to that same target generation | A later binding cannot upgrade the older release obligation | PASS: final release permit rejects the stale unowned proof; no syscall can be admitted through the new proof |
| Current target uses an explicitly PID-only owner binding | Preserve a valid PID proof when process-handle identity is not required | PASS: proof retains PID 77, has no authority object, and acquires the current-generation release permit |
| No tracked release obligation | No proof required and no spurious safety syscall/error | PASS: 0 sender calls, mask `0`, 0 release attempts, Finished |

The generation-change test uses deterministic barriers after the OS continuity
query and before final permit acquisition. The final permit then serializes the
last origin check with publication/bind/close and remains held through the
cleanup syscall, so a later writer is ordered after a release already proved
against the current owner. The same verifier is used by terminal finalization
and actual resumable focus/manual-pause/post-anomaly cleanup paths. Musical
Up-only admission is unchanged.

The focus-restore mock fixture now binds its synthetic process identity and
foreground PID explicitly. This exercises the same focus-required owner proof
that the production gate requires; it does not add a real OS/game test.

### Correction checkpoint verification

Commands were run sequentially on Windows x64 MSVC with Rust/Cargo 1.99.1 and
the checked-in lockfile. The rows below cover the correction checkpoint; the
earlier table records the original M3 implementation run. Intermediate
failures and reruns from this review correction are listed separately below.

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support engine::tests::startup_admission -- --test-threads=1` (after static barrier conversion) | 0 | PASS, 42 passed; 0 failed; 240 filtered out |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` (final direct invocation) | 0 | PASS, warnings denied |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | PASS, 80 unit + 4 integration/property tests; 84 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | PASS, 277 passed; 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` (final direct invocation) | 0 | PASS, 284 unit + 4 RT integration tests; 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` (final direct invocation) | 0 | PASS, 4 passed; 0 failed |
| `cargo xtask check desktop-native` (final direct invocation) | 0 | PASS; all three Tauri feature checks and `tauri-test` library tests completed |
| `cargo xtask check all` (first correction run) | 1 | FAIL in architecture audit: three test-only `Box<dyn Fn()>` barriers were classified as production dynamic emitters; no later stage ran |
| `cargo xtask check all` (after converting hooks to fixed barrier data; before this result table refresh) | 0 | PASS; Rust workspace Clippy/tests, Bun checks, Playwright E2E, and desktop checks completed; architecture audit emitted 66 advisory warnings |
| `cargo xtask check all` (final invocation after the correction report was refreshed) | 0 | PASS; Rust workspace Clippy/tests, Bun checks, Playwright E2E, and desktop checks completed; architecture audit emitted 66 advisory warnings |
| `cargo xtask check all` (after same-generation no-owner-upgrade regression) | 0 | PASS; Rust workspace Clippy/tests, Bun checks, Playwright E2E, and desktop checks completed; architecture audit emitted 66 advisory warnings |
| `cargo xtask check all` (final invocation after PID-only proof compatibility regression and report refresh) | 0 | PASS; Rust workspace Clippy/tests, Bun checks, Playwright E2E, and desktop checks completed; architecture audit emitted 66 advisory warnings |
| `git diff --check` (after source and correction report edits) | 0 | PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` (after source and correction report edits) | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` (after source and correction report edits) | 0 | PASS |

Intermediate focused attempts, not counted as final-gate passes:

| Command / attempt | Exit | Result and correction |
| --- | ---: | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all` followed by the startup-admission test run | 0 / 1 | Formatting passed; compile failed because `terminal_cleanup_target` needed a named borrow lifetime. Added the target lifetime and reran the 42-test suite successfully. |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` (first run after adding the release permit) | 1 | FAIL: `collapsible_if` in the owner-proof check. Collapsed the condition; subsequent direct Clippy invocations passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support production_worker_focus_restore_resumes_after_grace_with_mock_sender -- --nocapture` (before mock fixture identity was bound) | 1 | FAIL: the focus-required mock fixture published a foreground HWND without a matching synthetic retained owner identity, so no Down was admitted. Bound a synthetic process authority and explicit owner PID; the focused test and full package suite passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support forty_up_only_anomalies_keep_bounded_first_last_evidence_and_never_replay_downs` (first focused attempt) | 1 | FAIL: timing-sensitive harness evaluation observed `Wait` just before the next packet target. The isolated rerun passed; the final package suite and workspace suite also passed this regression. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support failed_post_call_cleanup_keeps_attempted_down_blocked_with_pending_release_obligation -- --exact engine::worker::dispatch_loop_m2_tests::failed_post_call_cleanup_keeps_attempted_down_blocked_with_pending_release_obligation` | 0 | 0 tests selected because the fully qualified name did not match the module path; not counted as evidence. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support failed_post_call_cleanup_keeps_attempted_down_blocked_with_pending_release_obligation` | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support production_worker_focus_restore_resumes_after_grace_with_mock_sender` (after fixture correction) | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support forty_up_only_anomalies_keep_bounded_first_last_evidence_and_never_replay_downs` (isolated rerun) | 0 | PASS, 1 selected |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support release_proof` (after adding the PID-only proof compatibility check) | 0 | PASS, 3 selected |

One exploratory full `sky_player` run was interrupted while diagnosing the
mock target's `HWND=0` path; it did not produce an exit code or a test result
and is not counted as a gate. A subsequent full package invocation before the
fixture corrections returned exit 1 (278 passed, 4 failed): the failures were
the explicit test-owner invalidation expectation, the cleanup diagnostic
substring assertion, the mock focus fixture above, and the timing-sensitive
anomaly case. The test invalidation behavior and diagnostic text were restored,
the fixture was corrected, and timing was rerun in isolation. The final direct
package run is PASS as recorded above.

The final `git diff --check` and both format checks are rerun after this report
edit; their actual exit codes are recorded in the gate table above.
