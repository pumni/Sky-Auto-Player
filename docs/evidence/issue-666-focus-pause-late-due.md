# Issue #666 focus-pause and late-Due checkpoint

## Revision identities

| Role | Commit | Tree | Parent |
|---|---|---|---|
| Original base | `96e35b6c7b9946289e2bd6b02a658a83811e80ec` | `c4629f87301243a8b3408c9990620ad78873b21c` | merge parents `2277ac34a21a57a8d5ec313bba55ad78fc60794f`, `6d61837b45812f351f5899356c913c3750d9495f` |
| Reviewed HEAD | `e2f880d335e6fe95b1c9fc6a4314f6c513cbcf58` | `c250cff95fbcb9dde09a324af9c9db31e8261847` | `a5ca51f1ec9dd30c635d4b7399fb826538bebbe2` |
| Focus/late-Due implementation candidate | `ef7ae25d8ae2b8790b70b7e05abd8ce018253a1e` | `4a5942857db13c3d83a94db04b8bf44863ea1ccd` | `e2f880d335e6fe95b1c9fc6a4314f6c513cbcf58` |
| Evidence commit | committed directly after the implementation candidate; its exact identity is in the final checkpoint | | |

Branch: `refactor/prepared-playback-runtime`. PR: [#667](https://github.com/pumni/Sky-Auto-Player/pull/667), open and Draft. The implementation candidate contains the production correction, deterministic worker tests, and the active playback contract update. The evidence commit adds this record.

## Focus rejection source audit

In original base `96e35b6`, `final_down_fresh_target_admission` performed the fresh foreground check and returned `FinalDownVerdict::FocusLost` (`admission.rs` around line 223). `verify_final_musical_packet_admission` handled `fresh_target_verdict == FocusLost` by calling `handle_final_focus_loss` (around lines 604–620). That handler sampled QPC, cleared `runtime.verified_target`, reset `focus_restore_started_ticks`, entered `PauseReason::Focus`, and published progress.

At reviewed HEAD `e2f880d`, the fresh OS query had moved into `fresh_final_release_owner_proof` (around lines 483–518). Its `NotForeground` result still became `FocusLost`, but the new error arm in `verify_final_musical_packet_admission` applied rejection and returned generic `Replan` without calling the focus handler. The earlier hint/fresh-target and final atomic-revalidation branches still called the handler. Both `verify_final_down_admission` and `verify_final_up_admission` use the common musical verifier, so the omission affected Down-bearing and musical Up-only packets.

Candidate `ef7ae25` adds a branch-local call to `handle_final_focus_loss` only when the fresh release-owner proof returns exactly `FocusLost`. It runs after the normal rejection invalidation and before returning `Replan`. QPC or playback-transition errors propagate as `FinalDownAdmissionResult::Terminal`. `OwnerRejected(ProofBusy)` remains `PermitBusy`; other owner/process/query/target/lease/modifier/control failures retain their existing paths. The shared musical gate still requires the release-owner proof before any musical Up. Verified safety cleanup and its origin-proof checks were not changed.

## Stale focus hint regression and state trace

`engine::worker::dispatch_loop::tests::safety::fresh_foreground_rejection_enters_focus_pause_and_recovers_after_grace_cleanup` drives the production final admission with `require_focus = true`, original HWND/generation stable, and monitor-published `focus_active = true`. Down(A) is committed and Down(B) is staged and due. The test moves actual foreground to HWND 2/PID 202 at the fresh OS gate, receives `NotForeground`, and restores actual foreground to HWND 1/PID 101 before the focus monitor publishes a changed hint. The hint remains true throughout the rejection.

Immediately before the rejection, the worker has one successful sender entry: Down(A), `(up=0x0000, down=0x0001)`. Cursor is 1; active/release-obligation masks are `0x0001`; possible and in-flight masks are zero; A's frozen release proof names the original HWND, generation, and PID 101. B is pending at cursor 1 with mask `0x0002` and has not crossed the sender boundary.

Immediately after the rejection, the result is `FinalDownAdmissionResult::Replan`; the fresh foreground query count is exactly 1; cursor stays 1; sender capture still contains only A; `precision_packet_attempts` stays 1 and generation activation stays 1. B has no sender attempt or transport capture entry. A remains active with release obligation `0x0001`, possible/in-flight remain zero, and its frozen owner proof is unchanged. `runtime.verified_target` and `focus_restore_started_ticks` are `None`; the Focus pause is present; the published progress snapshot is paused and has a pause-start QPC. The focus loss metric increments once even though the hint stayed true.

The same safety filter covers the previously working `focus_loss_before_first_down_keeps_onset_unattempted_until_safe_resume` control: `focus_active = false` plus actual foreground loss rejects before sender and leaves the onset pending. `musical_up_only_fresh_foreground_loss_pauses_without_crossing_sender_boundary` proves the common final gate pauses an Up-only packet on fresh foreground loss. `fresh_focus_rejection_keeps_qpc_failure_terminal` proves QPC failure in the established focus handler remains terminal. The new branch returns the handler's terminal transition error unchanged.

## Focus recovery and owner proof

The recovery fixture sets a 10 ms focus-restoration grace. It records a restoration QPC and computes the exact grace deadline; cleanup is not called before that deadline. The test advances by observing QPC with `yield_now`, not by treating a fixed sleep as the oracle. After grace, `suspend_live_input_for_dispatch` performs the verified cleanup under A's original proof and emits exactly one Up for scan `0x15`. The release obligation and A proof then clear. The resume barrier is exactly cleanup completion plus the existing G interval.

An instrumented preflight runs once after verified cleanup (event order `cleanup`, then `preflight`). The Focus pause exits only after the recovery path and progress is republished as running. B remains pending throughout; it waits until the cleanup G barrier and is then sent once. Its proof names the original HWND/generation/PID. No attempted Down is replayed.

`focus_recovery_cleanup_rejects_rebound_and_same_hwnd_aba_owner` separately covers a rebound HWND and same-HWND PID/generation ABA. In both cases verified cleanup rejects before the cleanup emitter, retains A's active/release obligation and original frozen proof, and never releases A under the new owner. Existing `preflight_busy` tests remain separately selectable and pass independently.

## Late `WaitBoundary::Due` characterization

`engine::worker::dispatch_loop::tests::deadline_trace::late_due_preserves_staged_down_then_reselects_eligible_up_and_down` uses a controlled QPC epoch and a fixed test-only `WaitResult` for the selected deadline. The production waiter is unchanged outside `cfg(test)`; the test seam injects a value tied to the expected target QPC and uses no dynamic callback.

The authored fixture is Down(A+B) at 0 ms, Up(B) at 25 ms, Down(B) at 42.5 ms, Down(C) at 90 ms, Up(A) at 100 ms, Up(B) at 130 ms, and Up(C) at 160 ms. With the configured H and G floors of 17.167 ms, B's second Down is staged at authored 42.5 ms for its G floor at 47.167 ms. The waiter returns `Due` at 100 ms, when A's authored Up and C's Down are also physically eligible. The Due branch preserves the staged B Down.

| Send order | Selected action | Packet `(up, down)` | Cursor after | Active / possible / in-flight / release after |
|---:|---|---|---:|---|
| 1 | Down(A+B), source 6680 | `(0x0000, 0x0003)` | 1 | `0x0003 / 0 / 0 / 0x0003` |
| 2 | Up(B), source 6681 | `(0x0002, 0x0000)` | 1 | `0x0001 / 0 / 0 / 0x0001` |
| 3 | Preserved staged Down(B), source 6682 | `(0x0000, 0x0002)` | 2 | `0x0003 / 0 / 0 / 0x0003` |
| 4 | Fresh next-iteration Up(A) + Down(C), next action 6683 | `(0x0001, 0x0004)` | 3 | `0x0006 / 0 / 0 / 0x0006` |

The four sender entries each have one attempt. B's staged Down is attempted once. Packet 2 starts no earlier than its 25 ms authored target and H floor; packet 3 starts no earlier than B's 47.167 ms G floor; packet 4 starts no earlier than A's 100 ms authored/H target. A keeps its original release proof through packet 3, and packet 4 consumes that independent Up obligation. B and C retain proofs for the same original target owner. All packets use the existing Up-before-Down masks.

This is the production late-wake ordering: `Due` dispatches the already staged B Down first. The next outer scheduler evaluation selects the now-eligible Up(A) and Down(C) together. It does not silently coalesce newly due work into the staged packet. The observed trace matches the currently documented staged-candidate contract; no coalescing or ordering change is proposed.

## `Due`, `Interrupted`, and `PermitBusy`

| Result | Worker behavior | Deterministic evidence |
|---|---|---|
| `Due`, including overshoot | Keep the staged candidate, then run final control/owner/focus admission before sender. After its commit, the next outer scheduler pass selects new work. | The late-Due trace above; `due_candidate_rechecks_stop_skip_focus_and_owner_before_sender` proves Stop, Skip, focus loss, and owner rebind reject before sender. Focus rejection enters Focus pause; owner rebind does not fabricate one. |
| `Interrupted` | Discard the staged candidate and return to the worker loop for control/focus handling and fresh scheduler selection. | `interrupted_focus_wake_discards_staged_candidate_for_outer_loop_handling`; no packet, cursor movement, or release obligation is committed before outer-loop focus handling. |
| `PermitBusy` | Return to fresh scheduler selection. The first Busy still gets one immediate retry; continued contention uses the existing bounded interruptible backoff. It is not the Due path. | Existing owner-proof and preflight-contention tests; the deadline filter also retains `interrupt_wakes_the_bounded_owner_permit_retry_wait`. |

## Healthy-path operation counts

`measured_final_admission_operations_for_healthy_packet_shapes` passed with the `qpc-read-counting` feature. Down-only, Up-only, mixed, and mixed-to-Up-only each report one scheduler evaluation, one QPC read, one foreground query, one owner-PID check, one process-identity check, one sender attempt, and zero allocations. The fixture's mock reports zero real `GetForegroundWindow` and zero `GetProcessTimes` calls. Blocked-generation replan also has one fresh foreground query and no sender; modifier-blocked replan has zero foreground queries and no sender. The focus correction calls no additional foreground query on the healthy path.

## Verification record

All commands below ran on Windows x64 MSVC. Exit status is 0 unless stated otherwise.

Focused filters, each selecting the intended nonzero test count:

| Focused command | Result |
|---|---|
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support engine::worker::dispatch_loop::tests::safety::` | 11 passed, 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support engine::worker::dispatch_loop::tests::deadline_trace::` | 7 passed, 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support owner_proof` | 14 passed, 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support preflight_busy` | 6 passed, 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features "test-support qpc-read-counting" measured_final_admission_operations_for_healthy_packet_shapes -- --nocapture` | 1 passed, 0 failed |

Required local checks:

| Command | Result |
|---|---|
| `git diff --check` | pass |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | pass |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | pass |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_player --lib --no-default-features -- -D warnings` | pass |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | pass: 85 tests (81 unit, 1 executor property, 1 golden, 2 properties) |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | pass: 278 passed, 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | pass: 327 passed, 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | pass: 4 passed |
| `cargo xtask check desktop-native` | pass |
| `cargo xtask check all` | pass; workspace Rust checks/tests, Bun checks/E2E, and desktop checks passed. Architecture audit reported 68 advisory warnings and no error. |

The ignored Sky Player test remains discoverable as the explicit M4
`offline_waiter_guard_ab_benchmark`; it does not call SendInput. The Win32
package also reports its existing ignored test. No warning suppression or CI
gate changes were made.

## Hosted CI

PR [#667](https://github.com/pumni/Sky-Auto-Player/pull/667) remains open and
Draft. The source candidate `ef7ae25d8ae2b8790b70b7e05abd8ce018253a1e` has
hosted CI run [43904969020](https://github.com/pumni/Sky-Auto-Player/actions/runs/43904969020),
which completed successfully on that exact SHA. Static/security, desktop web,
Windows compatibility/unit tests, current Tauri candidate, packaged Tauri NSIS
qualification, and the required CI gate all passed. The path classifier
skipped release-only and updater jobs. The evidence commit is docs-only; its
exact-HEAD status is checked after push and recorded in the final checkpoint.
The live [PR checks](https://github.com/pumni/Sky-Auto-Player/pull/667/checks)
page tracks the exact current branch tip.

## Limits

These tests use mock input and deterministic QPC/waiter fixtures. They do not
prove Windows input insertion, Sky gameplay reception, interactive focus
behavior, ReceiveOnly behavior, or native suspend/resume qualification. No
real `SendInput`, gameplay, focus interaction, or native power qualification
was run.
