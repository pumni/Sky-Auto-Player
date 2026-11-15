# Issue #666 — M2 Packet Pipeline and One-Attempt Down

This evidence records the M2 implementation on
`refactor/prepared-playback-runtime`. It is an implementation and deterministic
regression report; it is not native input or gameplay qualification.

## Accepted decisions and boundaries

- Down onset is consumed once the transport reports a syscall attempt that
  contains it. Zero, short, complete, and ambiguous post-call outcomes cannot
  replay that authored onset. A pre-call rejection has zero attempts and leaves
  the onset eligible for a later safe replan.
- Every anomalous attempted packet retains raw transport evidence and a
  conservative tracked release obligation. The separate safety path retains
  its ownership proof and does not require foreground focus. A verified release
  establishes the resume barrier from its actual completion QPC plus G.
- Recovery can dispatch later, never-attempted Down onsets in authored order
  after cleanup and fresh admission. Recovery does not change the anomalous
  packet into success; a session with any attempted post-call anomaly retains a
  terminal-error latch and finishes with an error after dispatching later
  onsets. The latch is promoted during finalization so it does not prevent
  verified recovery.
- Anomaly evidence retains a saturating total counter, an omitted-record
  counter, and at most 16 detail records (first eight plus rolling last eight).
  Each detail string is capped at 512 bytes and remains secondary evidence when
  a stronger terminal error is already present. Error finalization depends on
  the anomaly counter, not on retained detail records.
- Musical Up-only packets use fresh focus, HWND/PID/owner-generation,
  control, power, and lease admission at the final boundary. Target/focus and
  modifier races reject before syscall and do not advance the Down cursor.
  Stop/Skip/Quit/Panic, suspend, expired lease, and owner failures remain
  fail-closed under the existing control/ownership contract.
- Telemetry schema 25, public DTOs, sender boundary, packet order, pre-roll,
  H/G policy, focus pause/resume, and existing cleanup policy are unchanged.
  Healthy real-time dispatch keeps its fixed-buffer/no-heap-allocation path.

## Implemented pipeline

The worker stages/materializes one bounded candidate before waiting. At the
deadline it performs final admission, invokes the tracked prepared sender, then
commits only a fully evidenced successful packet to the scheduler/journal. A
pre-call gate rejection replans with the candidate still pending. An attempted
transport anomaly is recorded without a success commit, leaves release
obligations in the tracked journal, and enters ownership-verified reconciliation.
After verified cleanup, the executor consumes only the attempted Down identity;
the actual cleanup completion QPC and G gate the next Down. Unattempted later
onsets remain ordered and eligible. Cleanup or ownership uncertainty leaves the
session blocked/fail-closed with the obligation retained.

## Deterministic fault matrix

| Fault / boundary | Expected result | Regression evidence / observed result |
|---|---|---|
| Complete raw packet | One prepared packet, Up before Down, successful sender completion commits scheduler state | `sky_dispatch_win32` tracked packet tests and `sky_player` telemetry suite pass; existing packet-order/max-input tests remain active |
| Zero insertion after syscall with Down | One Down attempt only; preserve raw count/error; keep attempted Down as cleanup obligation; never replay it | `deferred_mixed_zero_insert_keeps_attempted_down_release_obligation`, `deferred_down_zero_insert_retains_in_flight_until_verified_cleanup`, and `persistent_zero_progress_down_is_not_replayed_and_next_unattempted_chord_is_sent` pass; captured musical call count is one for that onset |
| Short/split mixed insertion | Preserve partial raw evidence and exact Up/Down masks; do not retry an attempted Down; reconcile through safety cleanup | `attempted_mixed_down_is_reconciled_once_then_next_unattempted_onset_waits_for_cleanup_g` injects a mixed Up(A)/Down(B) packet with one inserted INPUT and missing completion QPC, observes one mixed attempt, cleanup, then only later onset C |
| Missing pre-call QPC | No syscall attempt; preserve prior Up obligation only; Down remains pending | `deferred_mixed_pre_call_qpc_failure_preserves_only_prior_up_obligation` passes with attempts=0 and cleanup mask containing no new Down |
| Missing completion QPC after syscall | Attempt consumed, no fabricated completion or successful timing sample; reconcile and retain anomaly | Mixed Down and telemetry QPC regressions pass; evidence records unavailable completion QPC and final result remains an error |
| Invalid/reversed sender QPC | Do not qualify timing or infer completion; reconcile conservative obligations | `invalid_sender_qpc_order_is_reconciled_without_qualified_timing_samples` passes and checks anomaly fields / error result |
| Full-count musical Up-only with reversed QPC and `timing_error=None` | Exactly one Up syscall; safety cleanup verifies, later Down(B) runs once after cleanup + G, anomaly has no qualified timing sample, final outcome is error | `full_count_up_only_reversed_qpc_recovers_then_finishes_as_error` passes and asserts source identities, masks, ordered QPC evidence, one Up attempt, later B send, cleanup barrier, and terminal error |
| Missing completion QPC with `timing_error=None` | Same recovery and terminal error; absence is detected from the QPC fields, not a backend error flag | `up_only_missing_completion_qpc_without_timing_error_still_latches_error` passes; retained evidence says `completion_qpc=unavailable` |
| 40 attempted Up-only anomalies | Count all 40; retain at most 16 records; omit exactly 24; preserve first/last samples; each Down identity has one syscall; cleanup + G precedes each later onset; final outcome is error | `forty_up_only_anomalies_keep_bounded_first_last_evidence_and_never_replay_downs` passes and checks masks/source IDs, exact counters, retained first/last, cleanup and barrier per anomaly, and terminal error summary |
| Terminal Skip after a recoverable anomaly | Stop dispatch immediately; preserve `skip` trigger and anomaly evidence; anomaly latch keeps the outcome fail-closed as error | `attempted_anomaly_error_preserves_concurrent_skip_cause_at_finalize` passes and checks both terminal trigger and anomaly summary/detail |
| Verified cleanup succeeds | Clear tracked obligation only after verified release; set barrier from actual cleanup completion plus G | Mixed Down and Up-only recovery tests capture cleanup completion; the core executor test checks barrier−1 waits and exact barrier permits the next onset |
| Cleanup fails | Do not advance/replay attempted Down; retain release obligation and fail closed | `failed_post_call_cleanup_keeps_attempted_down_blocked_with_pending_release_obligation` passes |
| Final focus lost for Down or musical Up-only | No syscall; preserve onset/cursor; replan/pause until focus and identity are fresh | `focus_loss_before_first_down_keeps_onset_unattempted_until_safe_resume` and `musical_up_only_final_focus_loss_replans_without_crossing_sender_boundary` pass; terminal safety Up still runs under ownership proof |
| HWND/PID/generation changes after stage/wait | Reject stale candidate before syscall; candidate can be restaged for current identity | `target_generation_race_and_modifier_race_leave_down_unattempted` passes; sender call count remains zero until safe restage |
| Modifier changes at final boundary | Reject Down before syscall; same onset remains eligible after modifier recovery | Same target/modifier regression passes and observes one call only after recovery |
| Quit, Skip, Panic release, suspend, lease expiration at final gate | No new packet attempt or cursor advance; terminal control remains fail-closed | `terminal_control_races_at_final_gate_do_not_attempt_or_advance_down_onset` covers each signal; passes |
| Mixed packet replans as Up-only when modifier blocks Down | Keep the Down onset pending; re-stage only the independently due Up and pass its fresh musical gate | `mixed_packet_with_final_modifier_rejection_replans_as_up_only` captures initial Down then Up-only, confirms no mixed/down syscall, confirms next Down remains current, and verifies journal masks are clear |
| Focus loss at a mixed final gate | Do not send musical Up to an unverified target; leave the Up for replan and keep terminal cleanup separate | `musical_up_only_final_focus_loss_replans_without_crossing_sender_boundary`; focus loss leaves no musical syscall, while terminal safety Up succeeds under owner proof |
| H/G at boundary and ±1 tick | Actual completion QPC is the timing origin; no clock substitution | `attempted_down_is_consumed_only_after_verified_cleanup_and_next_onset_waits_for_g` passes; existing executor tests include H/G boundary checks and odd QPC frequency conversion |
| Scheduler/journal continuity and fresh session state | Attempted generation is cancelled/consumed once; next unattempted generation stays ordered; release and in-flight masks agree | Mixed and Up-only recovery tests assert source action IDs, masks, generation accounting, release masks, barrier, and syscall captures; full player and Win32 tests pass |
| Healthy allocation path / stop and new session lifecycle | No regression to zero-allocation guard, terminal cleanup, replay/new-session lifecycle | `rt_dispatch_no_alloc`, full player suite, and existing session lifecycle tests are part of required gates below |

## Initial M2 verification (before review corrections)

The verification host reported Windows x64 MSVC. `rustc --version` returned
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo --version` returned
`cargo 1.99.1 (5f94df478 2026-08-27)`, and `rustc -vV` reported
`x86_64-pc-windows-msvc` / release `1.99.1`.

| Command | Exit | Result |
|---|---:|---|
| `git diff --check` | 0 | PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | 0 | PASS |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` (first run) | 1 | FAIL: `collapsible_if` in focus restoration; collapsed the nested conditional |
| Same Clippy command (rerun) | 0 | PASS |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | PASS: 80 unit + 1 executor-properties + 1 golden + 2 properties = 84 passed; no failures or ignored tests |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | PASS: 277 passed, 1 ignored, 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | 0 | PASS: 263 library tests + 4 integration tests passed, no failures or ignored tests; doc tests 0 |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | PASS: 4 passed, 0 failed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib dispatch_loop` | 0 | PASS: 30 passed, 0 failed |
| `cargo xtask check desktop-native` | 0 | PASS |
| `cargo xtask check all` (first run) | 0 | PASS, including Rust workspace and Bun/Playwright desktop E2E |
| `cargo xtask check all` (rerun after recording verification results) | 0 | PASS, including documentation integrity, Rust workspace, Bun/Playwright desktop E2E, and desktop-native checks |
| `cargo xtask check static` (first wrapper invocation) | not captured | The exec wrapper yielded a session before surfacing the exit code; this invocation is not counted as PASS |
| `cargo xtask check static` (two captured reruns) | 0 each | Both PASS: docs integrity, architecture, security, toolchain and static contracts |

Focused iteration history (all invocations were on Windows x64 MSVC):

| Focused command / invocation | Exit | Result |
|---|---:|---|
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support attempted_down_is_consumed_only_after_verified_cleanup_and_next_onset_waits_for_g` (first run) | 1 | Test borrow-check error; copied the expected onset identity before mutating executor state |
| Same focused core test (rerun) | 0 | 1 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib dispatch_loop` (first run) | 1 | Compile failure from misplaced helper/re-export while adding safety cleanup admission |
| Same dispatch-loop command (second run) | 1 | One obsolete test assertion expected only one packet capture; corrected to assert the preserved anomaly record and actual captures |
| Same dispatch-loop command (final focused run) | 0 | 30 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib attempted_mixed_down_is_reconciled_once_then_next_unattempted_onset_waits_for_cleanup_g` (three initial attempts) | 1 each | Corrected test fixture issues in sequence: nested-module imports/closure ownership, authored timestamp order, then source-action order |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib m2_tests::` (first run) | 1 | Fixed malformed assertion formatting |
| Same M2 test filter (rerun) | 0 | 7 passed at that iteration; the later final dispatch-loop run covers all 8 current M2 tests |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` (pre-correction run) | 1 | 259 passed, 3 old telemetry assertions failed against the intended recovery behavior; assertions were updated to require cleanup, no replay, later onset continuation, and raw anomaly evidence |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib mixed_packet_with_final_modifier_rejection_replans_as_up_only` (first run) | 1 | Non-exhaustive test race enum after adding Panic case; added the explicit unreachable arm |
| Same mixed-replan test (second run) | 1 | Exposed a real path gap: the worker only split mixed packets after stale-target rejection, not a final modifier rejection; broadened safe mixed replan to stage Up-only under fresh focus/control admission |
| Same mixed-replan test (final run) | 0 | 1 passed; packet capture proves no Down retry |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib persistent_zero_progress_down_is_not_replayed_and_next_unattempted_chord_is_sent` (first run) | 1 | Fixed assertion to identify telemetry by `source_action_index` and `sent_count` |
| Same persistent-zero-progress test (rerun) | 0 | 1 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib post_send_qpc_failure_is_reconciled_without_reconstructed_completion` | 0 | 1 passed; missing post-call QPC remains unavailable and the sender attempt is not replayed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib telemetry_metrics::` | 0 | 44 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib invalid_sender_qpc_order_is_reconciled_without_qualified_timing_samples` (first run) | 1 | Adjusted old expected status to assert raw pre-call/completion evidence and fail-closed final error |
| Same invalid-QPC test (rerun) | 0 | 1 passed |

The original M2-specific dispatch harness contains eight deterministic tests:
mixed attempted Down recovery and continuation; cleanup failure with pending
obligation; musical Up-only focus rejection plus separate safety Up; unattempted
Down preservation across focus pause/resume; attempted Up-only recovery;
target-generation and modifier races; mixed-to-Up-only replan after modifier
rejection; and terminal control/power/lease races. Stop uses the existing
`playback.stop` route, which sets the session terminal stop request and calls
`player.quit()`; existing startup/wait integration tests cover the same quit
signal before first Down. Review-correction tests additionally cover full-count
Up-only QPC anomalies with recovery and terminal failure, missing QPC without a
backend timing error, Skip plus anomaly retention, and 40-anomaly bounded
retention with per-onset syscall identity and cleanup barriers.

Both `cargo xtask check all` invocations ran serially; neither overlapped another
E2E run. Each reported 64 advisory architecture/unsafe-boundary warnings, while
the architecture phase passed and the security phase reported zero findings.

## M2 review correction verification

The following gates ran on the source containing both Leader-requested
corrections, on Windows x64 MSVC with the locked Rust/Cargo 1.99.1 toolchain.
`cargo xtask check all` ran serially twice: once after `desktop-native`, then
again after adding this correction ledger so documentation integrity checked
the updated evidence. Neither Playwright phase overlapped another E2E run.

| Command | Exit | Result |
|---|---:|---|
| `git diff --check` | 0 | PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | 0 | PASS |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | 0 | PASS |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | PASS: 80 unit + 1 executor-properties + 1 golden + 2 properties = 84 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | PASS: 277 passed, 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | 0 | PASS: 270 library tests + 4 no-allocation integration tests passed; doc tests 0 |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | PASS: 4 passed |
| `cargo xtask check desktop-native` | 0 | PASS |
| `cargo xtask check all` (first correction gate) | 0 | PASS: static, workspace Rust, Bun checks, Playwright E2E, and desktop checks; 64 advisory architecture/unsafe warnings and 0 security findings |
| `cargo xtask check all` (rerun after evidence update; final verification invocation) | 0 | PASS: documentation integrity, static/security, workspace Rust, Bun checks, Playwright E2E, and desktop checks; 64 advisory architecture/unsafe warnings and 0 security findings |

Toolchain confirmation: `rustc --version` exited 0 and reported
`1.99.1 (58cf8f9c3 2026-10-13)`; `cargo --version` exited 0 and reported
`1.99.1 (5f94df478 2026-08-27)`; `rustc -vV` exited 0 and reported
`x86_64-pc-windows-msvc`.

Focused correction test history (every invocation ran on Windows x64 MSVC):

| Command / invocation | Exit | Result |
|---|---:|---|
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support full_count_up_only_reversed_qpc -- --nocapture` | 0 | PASS: 1 reversed-QPC Up-only continuation/final-error test |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support attempted_anomaly_error_preserves_concurrent_skip_cause_at_finalize -- --nocapture` (first run) | 1 | FAIL: assertion expected `skip`; implementation reports the existing canonical cause `skipped`; corrected expected value |
| Same Skip preservation command (rerun) | 0 | PASS: 1 test; error latch and `skipped` trigger both retained |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support anomaly_history::tests -- --nocapture` | 0 | PASS: 3 history tests, including 40 records, 16 retained, 24 omitted, fixed record cap, and error latch after detail drain |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support up_only -- --nocapture` | 0 | PASS: 10 matching tests, including reversed and missing completion QPC variants and the 40-anomaly workload |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support m2_tests:: -- --nocapture` (first run) | 1 | FAIL: Rust borrow checker required the chained summary iterator binding to be mutable for `.any()`; fixed binding |
| Same M2 test filter (rerun) | 0 | PASS: 12 M2 tests |

Final staging whitespace check:

| Command / invocation | Exit | Result |
|---|---:|---|
| `git diff --cached --check` (first staged review) | 1 | FAIL: new regression file had one blank line at EOF; removed it and restaged |
| `git diff --cached --check` (rerun) | 0 | PASS |

The final full `sky_player` suite above reran the regression cases after the
finalizer ordering change. The per-test ledger demonstrates that reversed QPC
with full raw count and missing completion QPC with `timing_error=None` both
recover through verified cleanup, admit later Down(B) exactly once, and end in
error without qualifying the anomalous packet's timing. The 40-anomaly case
observed `total=40`, `omitted=24`, `retained=16`; source IDs for all 40 Down
syscalls were unique, and each verified cleanup completion + G barrier preceded
the next authored Down.

## Limits

- Tests use deterministic mock senders, QPC harnesses, and tracked-state probes.
  They verify sender calls and transport evidence; they do not prove a game
  received a note, foreground delivery, hardware behavior, or real
  `SendInput`/gameplay qualification.
- No native acceptance session was run; no native test budget was used.
- The baseline-equivalent benchmark run inside a candidate binary is structural
  M1 evidence only. It is not exact-base A/B evidence and says nothing about
  gameplay or real Arm performance.
- H/G tests verify deterministic QPC arithmetic and actual supplied completion
  timestamps. Production distribution and hardware scheduling remain outside
  this test evidence.

## Review status

M2 is submitted for Leader review on the pushed branch. This evidence does not
mark M2 accepted or authorize M3.
