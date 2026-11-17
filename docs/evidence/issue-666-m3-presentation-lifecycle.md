# Issue #666 M3 — Presentation and Playback Lifecycle Evidence

## Reviewed baseline and candidate

| Snapshot | Commit | Tree | Parent |
| --- | --- | --- | --- |
| Reviewed baseline | `b8e5702f7cabd695dc2b617a9e382dca1d89c6d2` | `6d151ababd8c1bcbf1c5aacf2c167133cb6a8a9c` | `7f26d687e88e99a12bc58c57b1910d9f1989f7a7` |
| M3 test candidate, before this evidence file | `f197887668de6dd4195cf0d531857b58bd1e06ea` | `7252cf1a069ddaabbd521c07fd2ada18ef29303c` | `b8e5702f7cabd695dc2b617a9e382dca1d89c6d2` |

M3 changes only test code. Production event transport, playback lifecycle, public DTOs, and telemetry were not changed. Schema 25 remains the contract; `native_telemetry_schema_25_serialized_key_sets_are_pinned` passed in the complete `sky_player` suite. No M1 or M2 playback behavior was changed.

The candidate adds presentation consumer disconnect, panic, poisoned snapshot-slot, and terminal-finalizer callback regressions. It also pins that a snapshot `Channel::send` error sets the shared delivery-failure flag, and exercises Skip while snapshot projection is blocked.

## Event failure call graph

Snapshot producers and ordered lifecycle messages converge on the same native event hub:

```text
monitor snapshot / presentation snapshot / diagnostics / State / Finalize
  -> NativePlaybackPresentation snapshot slot or bounded critical queue
  -> NativeEventHub::publish
  -> one NativeEventQueue and one Channel<UiEvent>
  -> publish_native_events -> Channel::send
  -> send error or panic: fail_native_event_publisher
  -> NativeEventQueue::fail_delivery: set shared failure flag, close and clear queue
  -> request_event_delivery_termination: panic_release() + quit()
  -> session monitor observes event_delivery_failure
  -> cleanup_failed_event_delivery: Quit + stop_requested + panic_release/quit
  -> worker terminal path and bounded join attempts
  -> release_terminal_resources: verified release or fail-closed quarantine
  -> Finalize completion callback reports Ok or Err and retires active ownership
```

Source anchors: [`native_runtime.rs`](../../apps/desktop/src-tauri/src/native_runtime.rs#L1425) has the single `NativeEventQueue` and its `delivery_failure` atomic; `publish` is at line 1488; `publish_native_events`, `fail_native_event_publisher`, and `request_event_delivery_termination` are at lines 1786, 1860, and 1877. [`presentation.rs`](../../apps/desktop/src-tauri/src/native_runtime/playback/presentation.rs#L92) contains snapshot admission; critical queue admission and fail-closed handling are at lines 151 and 187; the consumer starts at line 221. [`session.rs`](../../apps/desktop/src-tauri/src/native_runtime/playback/session.rs#L1715) contains the worker join boundary, the monitor checks the shared failure flag around lines 1789–1940, and cleanup is at line 2605. [`retirement.rs`](../../apps/desktop/src-tauri/src/native_runtime/playback/retirement.rs#L399) releases or quarantines activity; terminal ownership and callback handling are at lines 437 and 463.

## Failure classification matrix

| Class | Detection and playback effect | Stop / Skip / Quit / shutdown | Supervisor progress and worker join | Critical finalization and originating-owner cleanup |
| --- | --- | --- | --- | --- |
| 1. Best-effort snapshot drop or coalescing | Replacing a pending frame or dropping a snapshot when the bounded event buffer contains only lifecycle events increments snapshot metrics and returns success. Playback remains live. | Stop, Skip, internal Quit, and shutdown remain available through the separate native control path. | Supervisor continues progress and lease renewal. Normal terminal join proceeds. | State and Finalize messages retain their ordered queue. Any tracked Up still uses its original release proof. |
| 2. Isolated diagnostics or snapshot publication error | **Not representable in this architecture.** A failure after snapshot/diagnostics projection enters shared error paths; it is not tagged as a safe telemetry-only condition. No degraded-playback state is created. | No separate control result can be inferred. See classes 3 and 4 for actual errors. | No independent optional-failure mode exists. | No safe basis to claim lifecycle delivery or origin cleanup continues under a telemetry-only error. |
| 3. Critical lifecycle queue overflow or disconnected consumer | `try_publish_critical` marks the presentation queue failed/closed and the shared active delivery flag, then requests cleanup. | Stop is idempotent once Stopping begins; Skip is no longer a live-session command after the terminal transition. Internal `quit()` is already requested. Shutdown remains available. There is no separate UI `playback.quit` command. | Progress renewal stops as the worker is being terminated; it is not suppressed while playback is treated as healthy. Join retries and activity ownership remain in force. | The callback path can report failure and retire the terminal record; delivery of a terminal UI event is not promised after lifecycle delivery has failed. Cleanup Up is permitted only under the tracked origin proof; otherwise the obligation remains and release is quarantined. |
| 4. Shared publisher/consumer disconnect, panic, or poison | A `Channel::send` error/panic, presentation consumer panic, or poisoned presentation snapshot slot sets the same failure flag and fails closed. A snapshot send failure is therefore not telemetry-only. | Same terminal behavior as class 3. A separate blocked-but-not-failed presentation remains controllable; the Skip regression covers that case. | While a send is merely blocked, monitor progress remains live. Once failure is reported, the monitor quits playback and continues join/retirement work. | Finalize callback receives an explicit error when presentation fails. Terminal owner cleanup remains proof-bound; unverified release quarantines the activity reservation. |
| 5. Independent supervisor progress, owner, lease, or control failure | Progress publication failure, current owner loss, lease expiry, or accepted control proceeds through the independent safety path. A stale owner poll is applied only if its authority and generation remain current. | Stop/Quit/shutdown remain terminal safety controls. Skip only applies while the session is still live. | Failed lease/progress causes fail-closed termination. It is not converted into optional presentation degradation. Join and cleanup continue. | Owner loss can prevent release; the implementation withholds an Up for another HWND/PID/generation and retains the obligation or quarantines physical activity. |
| 6. Terminal finalizer or retirement failure | Finalize completion is asynchronous and carries `Result<(), String>`. `clear_terminal_ownership` requires the same active `Arc`; release evidence is verified or quarantined before retirement. | Stop after terminalization is idempotent; Skip is not applicable. Shutdown remains bounded at the app event-publisher boundary. | Worker quiescence is required before terminal release. Join timeout keeps the session exclusive until delayed cleanup; it does not mint release proof. | A finalizer panic or publication error reaches the completion callback as `Err`. A failed or unverified physical release cannot make the next activity reservation available. |

Queue-level snapshot drops are safe because they happen before the shared sender call and do not set `delivery_failure`. By contrast, any `Channel::send` error—including one whose in-flight event is a snapshot—calls `fail_native_event_publisher`, closes and clears the shared queue, sets the shared flag, and requests `panic_release()` plus `quit()`. `event_hub_counts_snapshot_eviction_on_close_and_failed_channel_send` now asserts this global failure explicitly.

## Before and after state transitions

There is no production behavior delta at M3. The reviewed transition remains:

```text
snapshot replaced / full lifecycle-only buffer
  -> snapshot metrics increment
  -> delivery_failure stays false
  -> playback and supervisor continue
```

An actual send or consumer failure remains:

```text
shared transport/consumer error
  -> event_delivery_failure = true; queue closed
  -> Quit + stop request + panic_release/quit
  -> worker join and verified cleanup or quarantine
  -> terminal callback reports success/failure and retires ownership
```

There is no “presentation degraded, playback continues” transition. Adding one would require evidence that a telemetry-only failure leaves an independent lifecycle and control delivery path alive. The current implementation has one `Channel<UiEvent>`, one queue, one `delivery_failure` atomic, and one send-error handler for every event kind.

## Regression evidence

| Coverage | Test and assertion | Result |
| --- | --- | --- |
| Snapshot backpressure, drop, coalescing, and lifecycle ordering | `older_critical_events_preserve_the_freshest_pending_snapshot_in_order`; `newer_snapshot_coalesces_a_frame_held_behind_an_older_critical_event`; `event_hub_coalesces_snapshots_and_fails_closed_for_lifecycle_overflow`; `blocked_event_publisher_coalesces_snapshots_and_preserves_lifecycle_order` | PASS; nonfatal queue drops are counted; ordered events remain ordered. |
| Shared snapshot send error | `event_hub_counts_snapshot_eviction_on_close_and_failed_channel_send` | PASS; failed snapshot is counted as dropped and the shared failure flag is true. |
| Critical queue overflow and channel/consumer disconnect | `presentation_critical_queue_overflow_fails_closed_and_releases`; `failed_critical_channel_delivery_requests_tracked_terminal_cleanup`; `disconnected_presentation_consumer_fails_closed` | PASS; fail-closed cleanup occurs; no terminal success is inferred from the disconnect. |
| Presentation consumer panic and poison | `presentation_consumer_panic_fails_closed_and_releases`; `poisoned_presentation_snapshot_slot_fails_closed_and_releases` | PASS; terminal is failed, worker quiesces, release is verified, and a successor activity reservation becomes available only after cleanup. |
| Terminal Finalize callback | `terminal_finalize_callback_reports_presentation_failure` | PASS; callback receives the exact presentation error, failure is observable in native state, and no successful Finished event is buffered. |
| UI publisher blocked past lease; shutdown | `blocked_ui_publication_past_lease_keeps_supervision_live_and_shutdown_releases` | PASS; accepted progress continues beyond the lease interval, no lease expiry is recorded, shutdown returns in under 2 s, publisher close is bounded, and the activity reservation is released only after worker join and verified release. |
| Snapshot projection blocked past lease; Skip | `blocked_snapshot_projection_past_lease_keeps_supervisor_live` | PASS; progress advances at least 160 times beyond 3 s, snapshots coalesce, Skip is accepted while projection is blocked, and terminal outcome is `skipped` after the consumer resumes. |
| Stop, Skip, shutdown before Arm | `stop_skip_and_shutdown_cancel_the_published_pre_arm_session` | PASS in the terminal-release integration group. |
| Owner proof and tracked release | `terminal_tracked_cleanup_uses_owner_proof_without_foreground_requirement`; `terminal_cleanup_never_releases_a_tracked_key_through_a_rebound_owner`; `musical_up_from_a_is_withheld_after_rebind_to_b`; `musical_up_from_a_is_withheld_after_same_hwnd_generation_aba`; `mixed_up_a_down_b_is_withheld_without_consuming_down_b` | PASS; cleanup uses A's proof when valid and withholds Up on B or same-HWND generation ABA. |
| Bounded join, retirement, activity lease | `join_timeout_keeps_session_exclusive_until_delayed_terminal_cleanup`; `native_worker_cleanup_barrier_keeps_session_exclusive_until_join_and_release`; `finished_and_failed_events_are_retirement_barriers_for_the_next_reservation` | PASS in terminal-release and event-delivery groups. |
| Two successive sessions and late callback fencing | `rapid_playback_sessions_queue_terminal_before_successor_starting`; `sender_trace_beginning_physical_session_b_clears_session_a_and_rejects_late_a_completion` | PASS; A's terminal lifecycle precedes B's Starting, and a late A trace completion cannot restore A after B begins. |

The isolated-telemetry-error test requested conditionally is not present: no separate error class or independent telemetry transport exists to inject while asserting healthy lifecycle delivery. The backpressure/drop cases above are representable and stay nonfatal. The new disconnect and snapshot-send tests prove why a failed send cannot safely be relabeled as an isolated presentation error.

## Independent lifecycle and ownership proof

`NativePlaybackService::monitor` publishes progress independently of snapshot projection and performs owner identity polling on its own loop. The blocked UI test proves progress through a blocked `Channel::send`; the blocked snapshot-projection test proves progress through a stalled presentation consumer. When the shared failure flag is set, progress is intentionally no longer renewed because the monitor has switched the session to fail-closed termination.

Physical cleanup continues to use the worker's tracked release ledger. `terminal_cleanup_target` loads each pending release mask, obtains `release_owner_proof_for_mask`, validates current target plus original owner proof, and holds the final permit through the safety syscall. A missing or changed proof withholds Up. `release_terminal_resources` records verified release or quarantines the activity reservation; `clear_terminal_ownership` only clears the matching active session. The Tauri mock failure tests prove verified cleanup and reservation retirement; sky_player owner tests prove A→B and same-HWND ABA withholding. These are source/test proofs, not gameplay delivery claims.

The app-scoped event publisher can be blocked inside platform `Channel::send`, which is not cancellable. `NativeEventHub::close` waits only for `EVENT_PUBLISHER_CLOSE_BUDGET`; the physical shutdown path does not wait indefinitely on that UI callback. Two-session coverage confirms lifecycle terminal publication is queued before a successor's Starting event.

## Design comparison and missing invariant

| Choice | Safety properties | Limitation |
| --- | --- | --- |
| Keep the current shared hub fail-closed | Any ambiguous send/consumer failure terminates playback, triggers cleanup, and does not claim UI delivery. Queue-level snapshot coalescing remains best effort. | A snapshot-only channel failure also ends playback. |
| Treat snapshot/diagnostics send failure as telemetry-only | Could preserve playback after a genuinely isolated telemetry fault. | Not justified: all event kinds share one queue, one Tauri channel, one publisher, and one failure flag. A dead or ambiguous channel cannot prove that lifecycle delivery still works. |
| Split optional telemetry from lifecycle/control delivery | Could permit truthful degraded telemetry while keeping ordered critical lifecycle delivery available. | Requires separate failure domains and ordering/finalization rules, plus a degradation signal that can still reach the UI. A second queue alone is insufficient if both sends share the same failed WebView/IPC transport. |

The exact missing invariant is: **a telemetry failure must be classified independently while a separately reliable lifecycle/control route remains available and its continued liveness is observable.** Until that invariant is implemented and reviewed, a shared send, consumer, or diagnostics publication error remains fail-closed.

## API, schema, and native qualification

Production API/schema delta: none. Telemetry schema 25 and generated event DTOs are unchanged. No realtime playback or sender allocation was introduced. The four dedicated `rt_dispatch_no_alloc` tests passed. The measured healthy final-admission fixture retained one scheduler evaluation, one QPC read, one foreground query, one owner-PID check, one process-identity check, one sender attempt, and zero allocations for Down-only, Up-only, mixed, and mixed-to-Up-only packets; the fixture also reported zero real `GetForegroundWindow` and `GetProcessTimes` calls.

All new event-failure tests use mocks. They do not qualify Windows input insertion, Sky gameplay reception, real foreground switching, ReceiveOnly, or native suspend/resume. Those are outside M3 and remain separate qualification work.

## Verification ledger

Environment: Windows x64 MSVC, `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, Cargo `1.99.1 (5f94df478 2026-08-27)`.

| Command | Exit | Result |
| --- | ---: | --- |
| `git diff --check` | 0 | PASS before test commit. |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 | PASS. |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | 0 | PASS. |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_player --lib --no-default-features -- -D warnings` | 0 | PASS, no warnings. |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | 0 | PASS, no warnings. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | 91 passed across unit/property/golden targets; 0 failed/ignored. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | 278 passed, 1 ignored opt-in benchmark; 0 failed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | 0 | 328 passed, 1 ignored offline benchmark; 0 failed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | 4 passed; 0 allocations asserted. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features "test-support qpc-read-counting" --lib measured_final_admission_operations_for_healthy_packet_shapes -- --nocapture --test-threads=1` | 0 | 1 passed; healthy operation counts recorded above. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test` | 0 | 352 passed, 4 ignored benchmarks; 0 failed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test native_runtime::tests::event_delivery::` | 0 | 12 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test native_runtime::tests::playback_integration::presentation_events::` | 0 | 10 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test native_runtime::tests::playback_integration::supervisor_lease::` | 0 | 6 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test native_runtime::tests::playback_integration::terminal_release::` | 0 | 11 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support prepared_plan_` | 0 | 3 selected and passed for review-thread QPC equivalence. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support engine::worker::dispatch_loop::tests::transport_outcome::` | 0 | 4 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support stale_owner_poll_failure_cannot_invalidate_rebinding_or_block_final_admission` | 0 | 1 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support engine::worker::dispatch_loop::tests::owner_proof::` | 0 | 11 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support engine::worker::dispatch_loop::tests::preflight_busy::` | 0 | 6 selected and passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support terminal_cleanup_` | 0 | 2 selected and passed, including rebound-owner withholding. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support terminal_tracked_cleanup_uses_owner_proof_without_foreground_requirement` | 0 | 1 selected and passed. |
| `cargo xtask check desktop-native` | 0 | PASS. |
| `cargo xtask check static` | 0 | PASS; 67 advisory warnings across categories below; 0 baseline-covered security findings. |
| `cargo xtask check all` | 0 | PASS sequentially: static, Rust workspace fmt/Clippy/tests, and desktop checks. |

Focused-filter audit: an initial guessed `dispatch_loop_transport_outcome_tests::` filter selected 0 tests. It was corrected to the compiled module path `engine::worker::dispatch_loop::tests::transport_outcome::`, which selected 4 and passed. No zero-test run is reported as a pass.

Static architecture advisories: 67 total — `context_fields` 1, `dispatch_function_lines` 1, `engine_test_module_lines` 1, `facade_lines` 1, `native_runtime_module_lines` 2, `production_glob_import` 2, `regular_module_lines` 36, `unsafe_boundary` 19, and `xtask_module_lines` 4. The new `presentation_events.rs` is 600 lines, below the 700-line advisory threshold. No thresholds or allowlists changed.

## PR review-thread reconciliation

All seven inline threads were replied to with the correcting commit, current implementation, and selected regression result, then resolved after the test results above. Resolution was based on the fix and regression, including the still-current stale-owner poll thread; it was not based on outdated status.

| Thread | Original defect | Correcting source and regression | Status |
| --- | --- | --- | --- |
| `PRRT_kwDOSp2_Gc63rypH` | Recommended plan advisory used base action count | `6975385912cc5fbec08be74badefdd034544f64d`; recommended action-count regression passed 1/1. | Replied and resolved |
| `PRRT_kwDOSp2_Gc63rypI` | QPC ceil rounding and prepared/raw boundary equivalence unpinned | `14967eea23f309954630fdd36f633a61c9e26a29`; odd-frequency prepared/raw fixture; `prepared_plan_` passed 3/3. | Replied and resolved |
| `PRRT_kwDOSp2_Gc63teBR` | Attempted Up-only anomaly could finish as success | `e00299a4a05ab7fdb8f9002f8570460b4903d237`, bounded by `eac4dd4cbe434f9a36b7fa0b1453143bf0fdb217`; transport outcome passed 4/4. | Replied and resolved |
| `PRRT_kwDOSp2_Gc63teBV` | Recovered anomaly Strings could grow without bound | `eac4dd4cbe434f9a36b7fa0b1453143bf0fdb217`; bounded first/last anomaly regression passed in transport outcome 4/4. | Replied and resolved |
| `PRRT_kwDOSp2_Gc63upXY` | Stale owner poll could invalidate a rebound generation | `e12f87fd69522941c9581eadc710744824b79097`; writer-serialized `invalidate_polled_owner_if_current`; barrier regression passed 1/1. | Replied and resolved while current, not outdated |
| `PRRT_kwDOSp2_Gc63upXa` | Terminal cleanup could release A's key through B | `e12f87fd69522941c9581eadc710744824b79097` plus `51789df4271d7e6a8baf0e1b8cbd0ba8ba12f2d6`; A→B, same-HWND ABA, and terminal cleanup regressions passed. | Replied and resolved |
| `PRRT_kwDOSp2_Gc633e7d` | Busy preflight discarded a valid stamp and could terminate an unattempted onset | `7bbdb3f0af498f88009a04afc45692d817c783bf`; preflight Busy group passed 6/6. | Replied and resolved |

Hosted CI for the pushed, exact final PR head is queried after push; the SHA and required-check statuses are recorded in the final checkpoint. The source/test candidate SHA above is the code-bearing commit; the following evidence commit changes documentation only. M3 does not qualify real Windows input insertion or Sky gameplay reception.
