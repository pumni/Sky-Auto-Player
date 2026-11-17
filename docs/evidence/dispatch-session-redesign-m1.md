# Dispatch Session Redesign M1 Evidence

## Caller and route map

| Symbol / path | Before → after | Verification |
| --- | --- | --- |
| `NativePlaybackService::create_native_player` in Desktop `native_runtime/playback/session.rs` | Production already prepared the owner authority beside the HWND. It now binds that target once, retains the verified authority, then arms the same player; no alternate production player path was added. | `cargo xtask check desktop-native`; startup and desktop lifecycle coverage in the Rust suites. |
| `NativeDispatchSession::set_target_hwnd` and target publication in `engine/{session,target}.rs` | Dynamic target publication/republication became first-write binding. A zero or changed HWND invalidates the session; the same HWND is idempotent. Rebinding after Arm is no longer available. | `startup_admission::target_replacement_during_long_physical_wait_autonomously_stops_held_origin`; dispatch owner-proof regression. |
| `republish_target_generation_for_revalidation` / `DispatchTarget::republish_same_hwnd` | Removed. A new target lifetime must get a new session instead of transferring release obligations. | `rg` confirms no production caller; startup and owner-proof regressions. |
| `bind_window_identity_authority` and `poll_window_identity_authority_result` | The one-time bind remains before Arm. Focus-required Arm now verifies the retained exact HWND/PID authority and continuous identity before worker launch. Polling latches continuity failure into the worker's terminal target state; Desktop still routes `CurrentInvalid` to panic-release as an additional request. | Production-backend Arm admission and owner continuity regressions; `cargo xtask check desktop-native`. |
| `close_window_identity_authority` / Desktop `release_power_resources` | Removed the explicit close call during power-resource release. The authority stays owned by the session target proof and is dropped with the retired session after its worker is quiescent. Power request release and suspend/resume unregistration remain. | Desktop-native check and session retirement/lifecycle tests. |
| `FinalDownAdmission`, worker `PermitBusy` branch, and replan retry | Removed writer/owner permits and the busy backoff. Each musical packet still performs fresh foreground, PID, and process-continuity admission; invalidated fixed targets exit fail-closed. | Focused operation-count probe below; dispatch safety, raw-zero, failure, and owner-proof tests. |
| `dispatch_loop_deadline_interrupted_tests.rs`, `dispatch_loop_late_retry_tests.rs`, `dispatch_loop_preflight_busy_tests.rs` | Removed tests for the deleted busy/rebind synchronization. Regressions now assert immutable-target invalidation, held-origin cleanup, controls, and safety behavior directly. | Full `sky_dispatch_core`, `sky_dispatch_win32`, and `sky_player` test run. |

## M1 correction: terminal invalidation and Arm admission

The correction keeps the existing session API and schema. A focus-required
`Arm` now validates the fixed HWND/generation, matching PID, retained process
authority, and continuous identity before spawning worker or watchdog threads.
Missing, mismatched, failed, or late authority closes admission permanently.
Owner polling latches failure into the same immutable target state the worker
checks before staging and between wait/control transitions. Target/owner
invalidation signals the native wait; the worker itself then records terminal
failure. Test regressions do not call `quit` or `panic_release` before checking
bounded autonomous completion.

| Caller / regression | Result and evidence kind |
| --- | --- |
| Desktop `NativePlaybackService::create_native_player` → `NativeDispatchSession::arm` | Existing production caller already binds target then retained authority before Arm. `start_prepare::target_integrity_admission_precedes_focus_and_session_arm` now checks that ordering. `cargo xtask check desktop-native` validates the caller build. |
| `rt_native_acceptance` → `acceptance::owner_identity::bind` → `arm` | Existing acceptance caller binds through the same `NativeDispatchSession` API before Arm. Its real retained-handle path is compile-checked; no ReceiveOnly/native acceptance or performance run is included in this correction. |
| `startup_admission::production_require_focus_arm_rejects_missing_failed_wrong_or_late_owner_proof` / `...accepts_exact_continuous_bound_owner` | Test-support synthetic process authority exercises production-backend Arm policy with an empty schedule. It verifies no worker handles on rejection and a worker on exact continuous proof. It is not a real process-handle or gameplay-input qualification. |
| `startup_admission::target_invalidated_before_scheduler_staging_finishes_without_input` | Mock sender, real native worker/waiter lifecycle; target loss before scheduler staging ends with zero attempts and no release obligation. |
| `startup_admission::target_replacement_during_long_physical_wait_autonomously_stops_held_origin` | Mock sender with the worker's interruptible physical wait; replacement HWND/PID wakes the wait and autonomously ends after one Down, retaining the origin obligation with zero Up attempts. |
| `startup_admission::owner_identity_loss_during_long_wait_wakes_worker_and_preserves_release_obligation` | Synthetic poll detects process termination while the worker waits for a distant Up; join is bounded without a test-side stop, the origin obligation remains, and no Up crosses to a replacement. |
| `startup_admission::same_hwnd_owner_aba_is_terminal_and_never_sends_origin_up_to_replacement` | Synthetic retained-owner drift plus a same-numeric-HWND foreground PID change; autonomous terminal completion retains the held-key obligation and sends no replacement-owner Up. |
| `startup_admission::stop_skip_panic_and_suspend_interrupt_staged_long_wait_before_input` | Mock sender; all four controls interrupt a real long wait before the first staged Down can be attempted. Existing ready/arming and due-candidate tests remain in place. |
| `terminal_release::native_worker_release_error_after_stop_quarantines_successor_admission` / `joined_worker_with_unverified_release_quarantines_future_physical_activity` | Desktop lifecycle/mock sender verifies uncertain release keeps successor admission quarantined. The direct worker tests above verify the worker terminal path independently of monitor panic/quit. |

Focus loss/recovery with a still-continuous target remains covered by the
existing focus-pause/resume tests. Existing one-attempt, H/G, Up-only fallback,
raw zero/partial/QPC, stale callback/epoch, and no-allocation tests remain in
the required Rust suites. No old Busy/rebind implementation or M2/M3 work was
restored.

## Operation counts

The isolated probe is `measured_final_admission_operations_for_healthy_packet_shapes`
with `--all-features`, `qpc-read-counting`, and one test thread. Baseline ran on
the original PR branch SHA `629c06ccb40d96046c8565d4462873869a44490b` from a
clean detached worktree. The recorded candidate was the previously reviewed
M1 implementation at PR SHA `493516c29a5403bc20c2eadff1f03e9386ef5115`.
The correction's full `--all-features` test run reran this probe but did not
re-record a paired operation-count report. The probe is a mock-sender structural
count, not a latency or gameplay measurement.

| Packet shape | Scheduler evals baseline → candidate | QPC reads baseline → candidate | Foreground / PID / continuity queries baseline → candidate | Process-authority `Arc` clones baseline → candidate | Authority mutex attempts baseline → candidate | Writer permit attempts baseline → candidate | Target stamp reads baseline → candidate | Sender attempts baseline → candidate | Allocations baseline → candidate |
| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Down-only | 1 → 1 | 1 → 1 | 1 / 1 / 1 → 1 / 1 / 1 | 2 → 0 | 2 → 0 | 1 → 0 | 11 → 4 | 1 → 1 | 0 → 0 |
| Up-only | 1 → 1 | 1 → 1 | 1 / 1 / 1 → 1 / 1 / 1 | 2 → 0 | 2 → 0 | 1 → 0 | 10 → 4 | 1 → 1 | 0 → 0 |
| Mixed | 1 → 1 | 1 → 1 | 1 / 1 / 1 → 1 / 1 / 1 | 3 → 0 | 2 → 0 | 1 → 0 | 11 → 4 | 1 → 1 | 0 → 0 |
| Mixed-to-Up-only | 1 → 1 | 1 → 1 | 1 / 1 / 1 → 1 / 1 / 1 | 3 → 0 | 2 → 0 | 1 → 0 | 13 → 6 | 1 → 1 | 0 → 0 |

The baseline's `Process-authority Arc clones` is the probe's combined authority
clone count. Candidate separately reports zero process-authority clones. Its
`release_identity_arc_clones` count is 2 / 2 / 3 / 3 for the four shapes; those
are clones of the small `ReleaseOwnerIdentity` holder, not clones of
`WindowIdentityAuthority` or its process handle. They allocate no memory.
Candidate target stamp reads count full snapshot/generation reads; validity
checks use the monotonic target-invalidated flag and owner-failure latch without
repeating the HWND/generation comparison. Candidate mutex and writer-permit counts are
zero by structure: their runtime paths and test counters were removed, and the
source search has no remaining callsites.

The candidate probe reports `real_GetForegroundWindow=0` and
`GetProcessTimes=0`: the harness uses its foreground/process test doubles. The
query counts above verify that each healthy packet still performs one fresh
foreground, PID, and process-continuity admission in that harness. No Windows
native timing or Sky reception claim follows from these counts.

## Local checks

The M1 correction handoff on PR #668 records commands, exit codes, test scope,
and the exact review SHA. From `rust/`, these passed:

```powershell
cargo fmt --all -- --check
cargo clippy --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings
cargo test --locked -p sky_player --features test-support --lib engine::tests::startup_admission::
cargo test --locked -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-features
```

The focused startup-admission run passed all 45 tests. The full run includes
`rt_dispatch_no_alloc`, whose four tests cover 1, 5, and 15-key dispatch, mixed
packets, and the resume barrier. From the repository root,
`cargo xtask check desktop-native` and `git diff --check` passed. The native
qualification/performance runner was not run.
