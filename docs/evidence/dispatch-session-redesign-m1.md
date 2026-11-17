# Dispatch Session Redesign M1 Evidence

## Caller and route map

| Symbol / path | Before → after | Verification |
| --- | --- | --- |
| `NativePlaybackService::create_native_player` in Desktop `native_runtime/playback/session.rs` | Production already prepared the owner authority beside the HWND. It now binds that target once, retains the verified authority, then arms the same player; no alternate production player path was added. | `cargo xtask check desktop-native`; startup and desktop lifecycle coverage in the Rust suites. |
| `NativeDispatchSession::set_target_hwnd` and target publication in `engine/{session,target}.rs` | Dynamic target publication/republication became first-write binding. A zero or changed HWND invalidates the session; the same HWND is idempotent. Rebinding after Arm is no longer available. | `startup_admission::target_replacement_after_arm_invalidates_session_and_never_rebinds`; dispatch owner-proof regression. |
| `republish_target_generation_for_revalidation` / `DispatchTarget::republish_same_hwnd` | Removed. A new target lifetime must get a new session instead of transferring release obligations. | `rg` confirms no production caller; startup and owner-proof regressions. |
| `bind_window_identity_authority` and `poll_window_identity_authority_result` | The one-time bind remains before Arm. Polling still checks the retained process identity and latches continuity failure for the bound session; Desktop still routes `CurrentInvalid` to panic-release. | `cargo xtask check desktop-native`; owner continuity and terminal cleanup tests in `sky_player`. |
| `close_window_identity_authority` / Desktop `release_power_resources` | Removed the explicit close call during power-resource release. The authority stays owned by the session target proof and is dropped with the retired session after its worker is quiescent. Power request release and suspend/resume unregistration remain. | Desktop-native check and session retirement/lifecycle tests. |
| `FinalDownAdmission`, worker `PermitBusy` branch, and replan retry | Removed writer/owner permits and the busy backoff. Each musical packet still performs fresh foreground, PID, and process-continuity admission; invalidated fixed targets exit fail-closed. | Focused operation-count probe below; dispatch safety, raw-zero, failure, and owner-proof tests. |
| `dispatch_loop_deadline_interrupted_tests.rs`, `dispatch_loop_late_retry_tests.rs`, `dispatch_loop_preflight_busy_tests.rs` | Removed tests for the deleted busy/rebind synchronization. Regressions now assert immutable-target invalidation, held-origin cleanup, controls, and safety behavior directly. | Full `sky_dispatch_core`, `sky_dispatch_win32`, and `sky_player` test run. |

## Operation counts

The isolated probe is `measured_final_admission_operations_for_healthy_packet_shapes`
with `--all-features`, `qpc-read-counting`, and one test thread. Baseline ran on
the original PR branch SHA `629c06ccb40d96046c8565d4462873869a44490b` from a
clean detached worktree. Candidate ran on the final M1 implementation reported
in PR #668's `M1 READY FOR LEADER REVIEW` handoff. The probe is a mock-sender
structural count, not a latency or gameplay measurement.

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
Candidate target stamp reads count full snapshot/generation reads; the final
fixed-target invalidation checks use one atomic boolean read and do not repeat
the HWND/generation comparison. Candidate mutex and writer-permit counts are
zero by structure: their runtime paths and test counters were removed, and the
source search has no remaining callsites.

The candidate probe reports `real_GetForegroundWindow=0` and
`GetProcessTimes=0`: the harness uses its foreground/process test doubles. The
query counts above verify that each healthy packet still performs one fresh
foreground, PID, and process-continuity admission in that harness. No Windows
native timing or Sky reception claim follows from these counts.

## Local checks

The final handoff on PR #668 records exact commands, exit codes, test totals,
and the full review SHA. `cargo test --locked -p sky_player --features
test-support --test rt_dispatch_no_alloc` exercises the allocation oracle for
1, 5, and 15-key dispatch, mixed packets, and the resume barrier.
