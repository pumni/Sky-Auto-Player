# Issue #666 post-integration replan timing correction

## Scope and source identities

This checkpoint changes the policy for a stable-target final owner-permit Busy
result. It does not revise the accepted musical Up-origin, stale desktop owner
poll, proven raw-zero Down-only, or nonblocking final permit safety rules.

| Role | Commit | Tree | Branch/worktree |
|---|---|---|---|
| Accepted baseline | `2e5519de8889471b42ef791ea49ba035ff14060c` | `de40720f01e1aae1abeed22cb05317c2a80b03b0` | detached baseline worktree |
| Correction source commit | `a1ec8f648f92e4fe55c10a6f43913bed2d941327` | `3227277af3a8f413902c320cbbf0e2a9dc287d52` | `refactor/prepared-playback-runtime`; parent `2e5519de8889471b42ef791ea49ba035ff14060c` |
| Evidence commit | this file is committed directly after the correction source commit | recorded in the checkpoint handoff | `refactor/prepared-playback-runtime` |

The correction was tested on Windows x64 MSVC with `rustc 1.99.1`
(`x86_64-pc-windows-msvc`) and Cargo 1.99.1. There is no Sky gameplay,
interactive focus, real `SendInput`, or real suspend/resume evidence in this
checkpoint.

## Production flow audit

The due-packet flow is:

1. The outer worker handles commands, samples QPC, and calls
   `evaluate_scheduler_action_for_staging` on `DispatchExecutor`.
2. A future scheduler `Wait` is projected at its selected H/G/authored
   deadline. A due `Send` is staged by
   `stage_scheduler_action_preserving_due_up`.
3. When a candidate has a future boundary, `pre_wait_candidate_admission`
   rechecks target generation, focus, and controls before
   `wait_for_next_boundary`.
4. After wake, the worker calls `dispatch_staged_candidate`. Final admission
   checks controls, modifiers for Down, foreground HWND/PID, process creation
   identity, target generation, lease, power, and the release-owner proof.
5. The final writer/owner permit is held across the physical sender call.
   Only after sender evidence is reconciled does the scheduler and tracked
   journal commit.

H/G eligibility is represented by `SchedulerAction::Wait`, not a final-gate
`Replan`. A future deadline uses one evaluation at current QPC and one
projection at the scheduler deadline, then goes through the ordinary
interruptible boundary wait and fresh final admission. An onset that has become
due but has not crossed the sender boundary remains pending.

## Replan cause and caller map

| Cause | Production source/caller | Result and wait policy |
|---|---|---|
| Target generation/owner drift | `pre_wait_candidate_admission`; final `final_down_atomic_revalidation`; `try_lock_current_release_owner_proof` returns `NotCurrent` | Generic `Replan`. A stale staged target keeps the existing interruptible unstable-target wait/publication behavior and is restaged; no proof is borrowed from the new generation. Owner identity failures that are terminal remain terminal. |
| Writer mutex Busy | `SessionTarget::try_lock_current_release_owner_proof` | Internal `DispatchAttemptOutcome::PermitBusy`. If target and focus still match, first miss gets one immediate outer scheduler re-evaluation. A second consecutive miss selects `now + 1,000 µs` with zero spin and an interruptible wait. |
| Owner-authority mutex Busy | `try_owner_authority_for_generation` during fresh proof or the final permit's `owner_authority.try_lock()` | Same internal Busy policy as writer contention, while preserving fresh owner verification on every attempt. No blocking mutex acquisition was added. |
| Focus lost/recovering | Final fresh foreground gate and `handle_final_focus_loss`; outer paused/focus-grace state machine | Generic `Replan` or the existing pause state. No packet is sent; resume grace, reconciliation, focus checks, and Down barrier remain unchanged. |
| Pause | `final_control_precheck` / `pre_wait_candidate_admission` | Existing generic `Replan` or pause transition. The new Busy wait helper checks controls first and does not start a contention wait when a control is pending. |
| Stop, Skip, Panic, lease, suspend/power | `process_command_control`, `final_control_precheck`, final lease and power gates | Existing terminal/cleanup policy. No Busy retry authorizes a send, and controls are rechecked before every next attempt. Lease unproven/expired remains fail-closed. |
| Modifier-blocked Down or mixed packet | `final_down_modifier_verdict`, then `stage_up_only_for_mixed_replan` | Generic replan. An eligible independent Up can still use its frozen owner proof in the Up-only fallback; the Down onset stays pending. A Busy mixed attempt cannot transfer a stale Up proof. |
| H/G or authored deadline not reached | `evaluate_scheduler_action_for_staging` | `SchedulerAction::Wait`, not `Replan`. Existing projection and deadline waiter are retained. |
| Mixed-to-Up-only candidate unavailable | `stage_up_only_for_mixed_replan` returns `None` | Generic replan, or a Busy-class result only when the triggering final gate was permit contention. No missing candidate consumes a scheduler cursor or obligation. |
| Staging cannot produce a candidate | `stage_scheduler_action_preserving_due_up` returns `None` in the outer worker | Existing `wait_after_unstable_target` path and fresh outer evaluation. This is not final permit contention. |

`wait_after_unstable_target` remains used by blocked-resume replan, the
unstageable-candidate path, and stale target handling before the wait. Its
`wait_after_unstable_target_with_deadline` helper still chooses a future packet
deadline when present, otherwise the existing `now + 1,000 µs` interval.
Non-Busy final replan causes retain the baseline policy. Only stable-target
writer/owner-authority Busy is handled by the new bounded retry policy.

## Before and after behavior

At the accepted baseline, the final worker branch represented every admission
rejection as generic `Replan`. If controls allowed waiting and the staged target
still matched, the branch called `wait_after_unstable_target(replan_now,
replan_now, ...)`; the helper therefore selected `replan_now + 1,000 µs`, even
for a due packet that had not called the sender and still had the same target.
The baseline helper regression
`unstable_target_retry_wait_is_interruptible_before_deadline` passed on the
detached baseline worktree (1 passed, exit 0). This is source-path and
deterministic-helper evidence, not a wall-clock benchmark.

The correction carries a private typed `DispatchAttemptOutcome` from final
admission to the actual outer worker replan policy. The exported
`StagedDispatchOutcome` and test-support method keep their original public
shape; permit Busy is an internal worker result. For a due candidate with
current target and focus:

* First permit Busy: return directly to one full scheduler/staging/final
  re-evaluation; do not wait and do not advance the onset or journal.
* Continued permit Busy: select exactly `now + 1,000 µs`, wait through the
  existing zero-spin interruptible waiter, then re-evaluate.
* Continued contention thereafter: alternate one immediate retry and one
  bounded wait. This prevents an unbounded CPU spin while avoiding a timer
  delay on the first stable-target miss.
* Pending controls preempt the wait. The next worker/control pass retains the
  existing Stop/Skip/Panic/Pause/lease/power handling.
* Target drift or lost focus never receives the stable-target fast retry.
  Each attempt still performs fresh admission and proof checks.

The test hook is placed after the fresh foreground/PID/process-continuity
proof and immediately before the final writer/authority permit. A supervisor
thread then owns the authority mutex, so the regression reaches the final
permit contention branch rather than only an earlier fresh-proof Busy return.

The musical permit lock order remains writer `try_lock`, then
`owner_authority.try_lock`, then generation/HWND/PID/owner-status/`Arc::ptr_eq`
comparison. On success, the writer guard remains held through the sender
syscall. On either Busy result, the attempt returns before sender, cursor, or
journal mutation. The verified safety-cleanup path is unchanged: callers
`suspend_live_input_for_dispatch` and `finalize_worker` use
`verify_safety_cleanup_target`, which validates the originating proof and can
then use `lock_current_release_owner_proof` with serialized blocking
synchronization. That policy remains distinct from the bounded musical retry.
The existing mixed anomaly cleanup, terminal tracked Up, and resumable cleanup
tests remain in the focused suite.

## Deterministic regression outcomes

Focused command:

```powershell
cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support dispatch_loop::tests::
```

Result on the final source before commit: **54 passed, 0 failed, 0 ignored**.

`due_down_owner_permit_contention_uses_outer_loop_retry_and_bounded_wait`
drives production `DispatchExecutor` evaluation, candidate staging, final
admission, and the same `wait_after_dispatch_replan` helper called by the
worker loop. Barrier/channel coordination holds the authority mutex across two
real final-permit misses; the second miss remains held through the selected
backoff. The source/test trace from the final run was:

```text
outer-replan production trace: scheduler_eval=[1@672608225563,1@672608227490,1@672608243440], dispatch_outcome=[PermitBusy@672608226861,PermitBusy@672608228073,Dispatched@672608244706], busy1_to_eval2=629ticks/62us, backoff=[selected_deadline@672608238078,duration=1000us,wake_re_evaluate@672608243440,busy2_to_re_evaluate=1536us], busy2_to_sender_pre_call=1561us, sender=[start@672608243684,attempts=1,raw_inserted=1,completion@672608243684], journal=[before=(active:0,possibly:0,release:0,inflight:0),after_commit=(active:1,possibly:0,release:1,inflight:0)], terminal=None
```

The timestamps above are one deterministic test execution's QPC observations,
not a latency distribution. The test asserts one scheduler evaluation in each
iteration, first-miss immediate continuation, exact 1,000 µs selected
backoff, no sender/cursor/mask mutation before admission, one sender attempt
after authority release, raw inserted count 1, and the post-commit obligation
mask. The reported 62 µs immediate re-evaluation, 1,536 µs busy-to-next
evaluation, and 1,561 µs busy-to-sender values are from this one run only; they
are not benchmarks or reception evidence.

The regression `scheduler_h_floor_and_g_floor_trace_wait_projection_and_single_sender_commit`
passed. Its final captured trace included current Down evaluation count 1,
future H and G projection counts 2 each, H wake delta 57,122 µs, G wake delta
17,124 µs, H/G minus-one-tick `Wait`, exact-floor and plus-one-tick `Send`,
three mock sender attempts with raw insertion `[1,1,1]`, and final masks
`active=1, release=1, in_flight=0`. These fixture durations include the
synthetic sender completion offsets and are not gameplay latency.

The regression
`stop_skip_panic_and_expired_lease_preempt_busy_permit_backoff` models repeated
permit misses through the same outer policy: one immediate re-evaluation,
then one non-interrupted bounded 1 ms wait. Stop, Skip, Panic, or latched lease
expiry is then injected; the next Busy policy pass returns immediately,
clears the retry token, and sends nothing. The existing
`interrupt_wakes_the_bounded_owner_permit_retry_wait` test confirms an event
interrupt is observed instead of waiting through the deadline.

Owner and mixed-packet regressions in the same focused suite retain the
accepted integration coverage:

| Case | Outcome |
|---|---|
| Same-owner Up/Down and healthy mixed packet | Dispatch once; normal masks and sender commit. |
| Up(A) permit Busy, then retry under A | No call or obligation clear while Busy; one fresh retry sends. |
| A→B and same-HWND generation ABA | Up(A) stays withheld; A's release obligation remains. |
| Mixed Up(A)+Down(B) and modifier-blocked mixed fallback | Invalid Up never crosses B authority; Down(B) remains pending; eligible Up(A) still uses its origin proof. |
| Generation changes during authority contention | Final revalidation fails closed; no old proof is borrowed for the new owner. |

## Before/after proof-operation counts

The baseline and candidate were measured with the same fixture and command:

```powershell
cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support,qpc-read-counting measured_final_admission_operations_for_healthy_packet_shapes -- --nocapture --test-threads=1
```

Both runs passed 1 test and printed identical counts. Each cell below is
`baseline → candidate`. The measurement uses test-only counters in
`target.rs`, `clock.rs`, `focus.rs`, the tracked sender, executor, and ledger;
the exact consumer is the `ProductionDispatchTestHarness` test
`measured_final_admission_operations_for_healthy_packet_shapes`.

| Fixture | Scheduler evaluations | QPC reads in measured suffix | Foreground queries / real `GetForegroundWindow` | Owner PID checks | Process identity checks / real `GetProcessTimes` | Authority Arc clones (target + release proof) | Authority mutex attempts / writer permits | Generation revalidations | Ledger mask visits | Mock sender attempts |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Down-only | 1→1 | 1→1 | 1/0→1/0 | 1→1 | 1/0→1/0 | 2 (1+1)→2 (1+1) | 2/1→2/1 | 11→11 | 2→2 | 1→1 |
| Up-only | 1→1 | 1→1 | 1/0→1/0 | 1→1 | 1/0→1/0 | 2 (1+1)→2 (1+1) | 2/1→2/1 | 10→10 | 2→2 | 1→1 |
| Mixed | 1→1 | 1→1 | 1/0→1/0 | 1→1 | 1/0→1/0 | 3 (1+2)→3 (1+2) | 2/1→2/1 | 11→11 | 4→4 | 1→1 |
| Mixed-to-Up-only | 1→1 | 1→1 | 1/0→1/0 | 1→1 | 1/0→1/0 | 3 (1+2)→3 (1+2) | 2/1→2/1 | 13→13 | 3→3 | 1→1 |
| Blocked generation replan | 1→1 | 1→1 | 1/0→1/0 | 1→1 | 1/0→1/0 | 1 (1+0)→1 (1+0) | 2/1→2/1 | 11→11 | 0→0 | 0→0 |
| Modifier-blocked onset | 1→1 | 0→0 | 0/0→0/0 | 0→0 | 0/0→0/0 | 0 (0+0)→0 (0+0) | 0/0→0/0 | 4→4 | 0→0 | 0→0 |

`real GetForegroundWindow=0` and `real GetProcessTimes=0` here are expected:
the harness substitutes foreground and process identity status. The hook still
counts one foreground sample, owner PID check, and process identity
continuity check for admitted musical packets. Production source invokes the
native foreground query in `final_down_fresh_target_admission` and one
`GetProcessTimes` in `WindowIdentityAuthority::process_identity_continuity`
for a fresh continuity check. The test's QPC count covers its measured
final-admission suffix; the real worker's outer-loop QPC sample and native
sender pre/post QPC samples are outside this mock-sender fixture and were not
reported as measured counters. Those source call sites are unchanged by this
correction.

Scheduler evaluation count in the due-packet contention regression is exactly
one at each outer iteration. Its first stable Busy selects no wait; its second
selects one 1,000 µs wait. H/G projection and wake counts are separately
reported above. No source census is presented as a wall-clock benchmark.

Healthy zero allocation is established by the focused
`rt_dispatch_no_alloc` integration gate in the verification ledger, not by the
operation-count fixture's printed placeholder. No allocation, lock counter,
or new runtime instrumentation was added to production builds; the scheduler
counter is `#[cfg(test)]` only.

## Code paths changed and retained

| Old path/helper | Status | Actual caller | Invariant retained | Evidence |
|---|---|---|---|---|
| Generic final `Replan` outer branch that re-read target state and always chose an unstable-target wait for a current target | Consolidated into typed internal attempt result plus `wait_after_dispatch_replan` | `dispatch` after `dispatch_staged_candidate` | Generic target/focus/control replan policy stays the same; stable permit Busy gets one immediate retry then an interruptible bounded wait | Due permit-contention outer-policy regression; source comparison at baseline and candidate. |
| `StagedDispatchOutcome` | Retained with original public variants and wire/API shape | Test-support harness and existing callers | No public enum variant or method signature change | Compile and full `sky_player` tests. |
| New `DispatchAttemptOutcome::PermitBusy` | Added internally | Final dispatch outcome consumed by `dispatch` | Typed cause; no string parsing; distinguishes stable owner-permit Busy from drift/control replan | Barrier test plus writer/authority contention tests. |
| `wait_after_unstable_target` | Retained | Blocked-resume, unstageable staging, stale pre-wait target paths | Existing future deadline selection, zero-spin waiter, interrupt/control wake | Existing baseline test and final regression suite. |
| `wait_after_unstable_target_with_deadline` | Retained; selected deadline is now returned to the caller for test trace accounting | Generic final replan and stable permit-Busy backoff | Same interval conversion, overflow/error behavior, and interruptible boundary waiter | Exact selected-deadline assertion and trace. |
| Mixed-to-Up-only fallback | Retained | Final Down rejection and pre-wait candidate admission | Frozen Up origin proof; rejected Down stays pending; no duplicate sender call | Existing mixed owner and modifier fallback tests. |
| New timing regression source initially in one oversized include file | Split into two test includes (448 and 352 lines) in the same unit-test module | `dispatch_loop::tests` | Test behavior and production path are unchanged; test-only layout avoids adding a module-size advisory | Focused dispatch suite remains 54/54. |
| M4 shared foreground test fixture | Test-only isolation guard added | `raw_zero_candidate_owner_drift_after_identity_query_fails_before_sender` | Prevents parallel test interference; no production code change | Parallel focused suite passes 54/54. |

The zero-spin retry uses existing `HybridWaiter` mechanics. This correction
does not alter adaptive spin, FPS, margin, tempo, H/G calibration, telemetry
schema, DTOs, ownership checks, safety cleanup, watchdog, lease, focus recovery,
or sender behavior. No unrelated architecture cleanup was included.

## Verification ledger

Final source is Windows x64 MSVC, Rust/Cargo 1.99.1. All commands below are
run sequentially where ordered; `cargo xtask check all` is not parallelized.
The exact final SHA and CI status are reported in the checkpoint handoff.

| Command | Result |
|---|---|
| `git diff --check` | exit 0; final evidence-only edit was rechecked with `git diff --check` and `cargo xtask check static` before commit |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | exit 0 |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | exit 0 |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | exit 0; no warnings accepted under `-D warnings` |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | exit 0; 84 passed (80 unit, 1 executor property, 1 golden, 2 property); 0 failed/ignored; doc tests 0 |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | exit 0; 278 passed, 1 ignored, 0 failed; ignored test is the retained-process child helper; doc tests 0 |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | exit 0; 310 unit tests passed, 1 ignored offline waiter-only A/B benchmark; embedded `rt_dispatch_no_alloc` target 4 passed; 0 failed; doc tests 0 |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | exit 0; 4 passed, 0 failed/ignored; healthy dispatch allocation gate |
| `cargo xtask check desktop-native` | exit 0; PASS. Three `sky_desktop_shell` cargo-check configurations and the Tauri binding-generator test command completed; successful child test counts are suppressed by the wrapper. |
| `cargo xtask check all` | exit 0; PASS on final runtime/test source. Sequential Rust workspace, static/security/supply-chain checks, Bun frozen install/check/headless E2E, and desktop check passed. The wrapper suppresses successful child stdout, so aggregate test totals are not exposed. Architecture audit emitted 67 advisory warnings; no retry or intermediate failure in this final run. |

Additional focused and comparison commands:

| Command | Result |
|---|---|
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support dispatch_loop::tests::` | exit 0; 54 passed, 0 failed/ignored (final source) |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support,qpc-read-counting measured_final_admission_operations_for_healthy_packet_shapes -- --nocapture --test-threads=1` | Baseline and candidate each exit 0, 1 passed; healthy operation rows were identical as recorded above. The initial baseline invocation without `qpc-read-counting` selected 0 tests and was rerun correctly. |
| Focused Busy, H/G boundary, and operation-count trace tests | Each final-source targeted test exit 0, 1 passed; trace values and exact assertions are recorded above. |
| `cargo xtask check static` | exit 0 after final evidence text edit; evidence links and static repository checks pass. |

Intermediate iteration ledger:

* An early focused dispatch run exposed test fixture interference on the shared
  mock foreground, an assertion that still expected generic `Replan`, and a
  writer test hook left installed for the retry. The M4 foreground regression
  now takes the existing test lock and resets its global state; expectations
  use the internal result where appropriate; the writer hook is cleared before
  retry. The focused dispatch suite then passed 54/54.
* During the private result refactor, one compile attempt failed on a missing
  `pub(crate)` re-export and two stale test enum references. The internal
  re-export and test assertions were corrected; the focused dispatch suite
  then passed 54/54.
* The first baseline operation-count command omitted
  `qpc-read-counting` and therefore selected zero tests. It was rerun with the
  feature shown above; baseline and candidate each passed 1 test with identical
  operation rows.
* Cargo printed package-cache lock waits during the parallel baseline and
  candidate count commands; both completed successfully with exit code 0.

The final full `cargo xtask check all` completed sequentially with exit code 0
on the final runtime/test source before the evidence-only ledger update. The
ledger update was followed by `cargo xtask check static` and `git diff
--check`, both exit 0. The aggregate wrapper reports Rust and desktop PASS but
captures successful child output, so it exposes no aggregate test count; the
package-level required test counts above are reported from their direct
commands. The ignored Win32 helper and offline waiter benchmark are named
above. The exact correction source SHA/tree and parent are recorded above. The
final evidence commit SHA/tree and hosted CI state for that exact final HEAD
are recorded in the checkpoint handoff. Hosted CI is not awaited as a
condition to push.
Deterministic mock tests do not establish Sky note reception or a latency
improvement.
