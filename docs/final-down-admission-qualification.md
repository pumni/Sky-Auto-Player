# Final Down admission policy qualification

This document records the implementation and qualification evidence for issue
#483. The code candidate was built from `904bbdfcdb8f4552122c0257cafc57e7f9cf92b6`
on top of baseline `be4b35f4aaae9e1e68b7ebe29c91f5bf310c8bcb`. The linked CSV
ledger preserves every native run ID, verdict, failure reason, and report hash
from this qualification session:
[final-down-admission-native-runs.csv](final-down-admission-native-runs.csv).

## Architecture change

Before this change, prepared and strict/dynamic dispatch maintained separate
final Down admission paths. They both checked safety authority, but the
duplicated control, target, focus/owner, modifier, and lease decisions could
drift. Their scheduling and rejection adapters also differed by design.

The worker now expresses common final Down authority as `FinalDownVerdict`.
Prepared and strict/dynamic paths share the control, target/focus/owner,
modifier, and post-modifier lease checks, including the final lease-age proof.
`apply_final_down_rejection` owns shared invalidation and metrics. Each path
keeps its own scheduling result adapter, so a common safety decision does not
force the two scheduling paths into one implementation. The exact call graph,
decision table, and retained path differences are in
[rt-dispatch-architecture.md](rt-dispatch-architecture.md).

The refactor preserves one physical sender and one `SendInput` attempt per
musical packet. Authored timestamps, packet grouping, user timing preferences,
spin policy, MMCSS behavior, H/G completion floors, final Down authority, and
tracked release obligations are unchanged. No game-process access or input
mechanism was added.

## Deterministic verification

All focused tests below ran against code commit `904bbdfcdb8f4552122c0257cafc57e7f9cf92b6`.

| Command | Result |
| --- | --- |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support worker::dispatch::` | 43 passed, 1 ignored, 414 filtered out |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support prepared_and_strict_paths_match_shared_final_down_fault_matrix` | 1 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support` | 457 passed, 1 ignored |

The differential test exercises both real dispatch adapters against the same
fault matrix. It covers accepted Down, target ABA, focus loss and foreground
owner changes, retained process identity failures, modifier state before and
after its checkpoint, expired or unproven lease, QPC lease-read failure, and
Stop, Skip, Panic, Pause, or Suspend arriving after lease observation.

## Static audit correction

The first `cargo xtask check all` attempt at documentation commit
`783ec77810fd76dc594ae177fcbbf7503c1b21fe` stopped in the static architecture
audit. It found `finalize_authored_down_admission` at 207 lines, over the
180-line dispatch-function limit; later verification groups did not run in
that attempt. The verdict-to-outcome mappings now live in two `#[cold]`
rejection-only helpers, reducing that function to 177 lines. Calls to those
helpers occur only after a non-Allowed verdict, so the accepted Down path keeps
the same checks, order, QPC samples, and dispatch boundary.

After this correction, the focused dispatch test command passed with 43 passed,
1 ignored, and 414 filtered out. The prepared/strict differential fault-matrix
test passed with 1 passed and 457 filtered out. The native reports above were
produced by code commit `904bbdfcdb8f4552122c0257cafc57e7f9cf92b6`; the
dispatch correction changes only rejection-result mapping after authority has
rejected the Down. The later pre-Armed Panic case applies only when no input
could have been emitted, no release result exists, and the tracked obligation
mask is zero. Neither change affects an admitted Down. In accordance with the
updated foreground qualification direction, no further physical sessions
were started for these changes. No timing improvement is claimed.

## Desktop lifecycle test scheduling

The full `cargo xtask check all` gate at code commit
`a7195b4af2c0d7d93a5feaf4bcaadda98465d83e` passed the static audit, Rust
workspace, Bun checks, and end-to-end tests, then failed in the Tauri test
group. Its first failure named
`failed_critical_channel_delivery_requests_tracked_terminal_cleanup` and
`physical_startup_target_failure_retires_without_reentrant_active_lock`.
Repeated parallel runs exposed different deadline-sensitive lifecycle tests.
The most recent default-parallel package
run failed `blocked_ui_publication_past_lease_keeps_supervision_live_and_shutdown_releases`
with 8 accepted renewals instead of 100; its exact isolated run passed. A run
with four test threads failed the second-session startup checkpoint in
`native_worker_cleanup_barrier_keeps_session_exclusive_until_join_and_release`.
These checks use timing-sensitive mock workers and interfere with one another
under parallel test scheduling on this 12-logical-processor host.

The test setup now arms the blocked-UI worker only after its monitor is ready,
uses an Armed checkpoint for lease-expiry projection, waits for asynchronous
startup retirement, and gives mock worker checkpoints a larger bounded wait.
The 100-renewal assertion remains unchanged. A startup race also showed that
Panic can cancel a physical worker before Armed; when no release result exists
and the tracked obligation mask is zero, the no-input startup case now records
no release obligation. An Armed worker still requires normal verified release
evidence. The new pre-Armed Panic regression covers both outcomes.

The complete Tauri library test suite passed with
`RUST_TEST_THREADS=1`: 271 passed. The final full repository gate uses the same
test-thread setting to keep deadline-based fault tests deterministic; this is
test-runner scheduling only and does not alter production worker scheduling or
physical input behavior.

## Windows native evidence

The native receive-only sink was run on Windows 11 Pro build 26300 with an AMD
Ryzen 5 5600 (6 cores / 12 logical processors), Rust 1.99.1, and the Balanced
power plan. The candidate harness SHA-256 is
`d42e7cafcd488eaba13098fb5416a8b2d5e98307ebf6b6670dd8263faba4d5b2`; the
baseline harness SHA-256 is
`842905fa2b7deb157bb99ba5d6e2e2850d253bebddad75aab87c7006f8519f97`. Both
used runner SHA-256
`04ac54165368f3ef8204ed1ff5e88a44b8b96e4054aea052b3b17dacb69395df`.
The runner binds the run to the receive-only window handle and process
identity, and requires that exact window to be foreground for physical input.
It recorded `AllowSetForegroundWindow` permission as false on these runs; that
call is not credited as the cause of a pass.

### Pause / resume release-floor oracle

The zero-margin candidate run
`native-case-20261019T111225-4683997e` passed with 42 observed sink events and
20 requested and completed release-floor cycles. Independent inspection of
the 20 cycle records confirmed, for every cycle:

- a successful Up completion was observed and `floor_passed` was true;
- the resumed authored Down target preceded the cleanup-relative release
  barrier;
- the computed Down not-before time was at or beyond that barrier;
- the actual Down sender-start QPC was at or beyond the barrier; and
- Down sender attempts equaled the cycle index plus one.

The oracle reported 20 release-floor delay boundaries, zero hold-floor and
release-floor violations, zero failed releases, zero stuck keys, and an empty
release-obligation mask. This exercises the condition the acceptance oracle is
meant to distinguish: the authored target is earlier than the required
physical release floor, while the emitted Down respects that floor.

The first 500 microsecond-margin attempt,
`native-case-20261019T110944-c2c64e0c`, failed its precondition: the resumed
authored Down target was about 2.2 ms after the cleanup floor, so it did not
exercise a floor delay. Two zero-margin retries,
`native-case-20261019T111034-4e9d245d` and
`native-case-20261019T111044-10b3a32d`, were inconclusive because the runner
could not establish the exact receive-only foreground HWND. These runs remain
in the ledger; the passing zero-margin run is the qualified oracle evidence.

### Partial paired dispatch sample

An A/B sample had started before the Lead Architect narrowed the foreground
qualification request. When work resumed, its orchestration session was no
longer live. The plan was 20 pairs for each of four scenarios (160 sessions
total). The collected artifacts contain 52 completed sessions and one
baseline sink-setup directory without a harness invocation or acceptance
report. No playback or sink process remained at inspection, and no further
foreground sessions were launched.

| Scenario | Side | Planned | Pass | Inconclusive | Fail | Setup only | Not started |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| dense-alternating | baseline | 20 | 11 | 5 | 4 | 0 | 0 |
| dense-alternating | candidate | 20 | 12 | 8 | 0 | 0 | 0 |
| mixed-up-down | baseline | 20 | 3 | 3 | 0 | 1 | 13 |
| mixed-up-down | candidate | 20 | 1 | 5 | 0 | 0 | 14 |
| chord-sweep | both | 40 | 0 | 0 | 0 | 0 | 40 |
| near-minimum-retrigger | both | 40 | 0 | 0 | 0 | 0 | 40 |

The completed runs comprise 27 passes, 21 inconclusive results, and 4
baseline failures. The mixed-up-down scenario had 27 unstarted sessions in
addition to its one setup-only directory. All inconclusive results were
foreground-HWND acquisition failures. Three baseline failures did not receive
expected physical evidence before the drain deadline; one reported a transport
or residue anomaly. These results do not support a paired timing conclusion,
and no timing improvement is claimed. The per-run ledger retains every
completed run ID, exact reason, report SHA-256, source revision, and harness
identity. The setup-only directory is listed as `NOT_RUN` because it has no
acceptance report.

## Final repository gate

The final `cargo xtask check all` gate is run against the committed evidence
and documentation state. Its exact HEAD and result are recorded with the
Draft PR delivery; the deterministic test results above identify their code
input explicitly.
