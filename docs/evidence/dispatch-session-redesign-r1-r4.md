# Dispatch Session Redesign R1–R4 Correction Evidence

## Source identity and scope

| Item | Value |
| --- | --- |
| Repository / review | `pumni/Sky-Auto-Player`, Draft PR #668 |
| Branch | `refactor/dispatch-session-redesign` |
| Review baseline | `f48e5a75828a598d028f363ee6fa3ad1eb59ac84` |
| Baseline tree | `3408359b135caefcea265031dbffa0e0bfc8a261` |
| PR base | `d0f65e7c412041a6d502f81f469872198db7adc5` |
| Authorized scope | Integrated R1–R4 core correction; no Arc journal or timing bookkeeping optimization |

This evidence is for source review and offline/mock verification. The Native
Hold remains active: no Final, Canary, Smoke, Compare, ReceiveOnly acceptance,
or real `SendInput` run is authorized. No artifact or manifest from the review
baseline is reused for this source.

## Correction map

| Finding | Changed code / contract | Result |
| --- | --- | --- |
| R1: cleanup retry can outlive target/owner invalidation | `sky_dispatch_win32/input/tracked/{cleanup,packet_send}.rs`, `input/outcome.rs`; `sky_player/engine/worker/{cleanup.rs,cleanup/finalize.rs}`, `admission.rs` | Terminal and resumable worker cleanup pass an admission callback into the bounded retry loop. Each nonempty attempt prepares its packet, refreshes the origin proof, revalidates immutable target stamp and required owner/PID/process continuity, then enters the sender. A rejected admission ends retries and carries prior physical evidence forward. |
| R1: unresolved obligations and evidence | `sky_dispatch_win32/input/tracked/cleanup.rs`, `input/tests/cleanup.rs`; `sky_player/engine/test_support/dispatch_harness_tests.rs` | Only admitted sender attempts are counted as physical attempts. Raw sender/QPC/probe evidence from prior calls remains available; missing or partial releases keep unresolved masks quarantined. A withheld retry is separately reported and is not counted as a sender call. Zero obligations return without owner query, probe, or send. |
| R2: modifier polling can miss an independent Up and its waiter evidence | `sky_dispatch_core/executor.rs`, `sky_player/engine/worker/{dispatch_loop.rs,dispatch_loop_scheduler.rs}` | Wait until the earlier of the bounded 1 ms poll or earliest active Up deadline. Typed results are retained: Up-deadline waits publish raw wake/timer evidence and attach wake QPC only to a packet containing an Up at that exact release deadline; maintenance polls remain out of the musical observer queue. Modifier rejections use the existing scalar `final_gate_control_rejections`; interruptions and typed backend/clock failures update the existing wait counters. An already-due Up immediately returns to normal selector/final admission without arming a zero-length wait. The deadline includes authored Up and H; same-key completion G remains enforced. No new telemetry field or schema was added. |
| R3: immutable-target rejection retained an impossible staging fallback | `sky_player/engine/worker/dispatch_loop_scheduler.rs`, `dispatch_loop.rs` | A `None` from mixed `stage_candidate` now propagates directly; it does not retry Up-only staging against the same immutable target. The distinct `stage_up_only_for_mixed_replan` remains reachable after a final Down admission rejection while an independent Up is valid. The separate blocked-resume wait remains available with the same valid target for live foreground recovery. |
| R4: unused/redundant production internals | `sky_player/engine/worker/{dispatch_loop.rs,dispatch_loop_scheduler.rs,startup.rs,admission.rs}`, `engine/target.rs`, `dispatch/hold_forensics.rs`, `boot/timing_admission.rs`, `worker.rs`, raw-zero operation probe | Removed or simplified confirmed dead pieces. Detailed anomaly records are test-only; production retains scalar anomaly/structural/timing counters and the schema-25 overwrite field reports zero when no production ring is captured. Removed the unread worker timing-margin cache and its redundant conversion; user margin and evidence metadata remain. No H/G rounding or public schema changed. |
| Owning contract | `docs/playback-engine.md` | Documents per-attempt cleanup admission, modifier Up deadlines, and immutable-target terminal behavior while preserving focus-resume recovery. |

The guard is a fresh fail-closed check immediately before each sender entry; it
does not serialize target invalidation and cannot eliminate the unavoidable
instruction-scale interval between proof and the Win32 call. It closes the
unchecked 15/50/100 ms retry windows without restoring a writer permit or lock.
The `ReleaseAllOutcome` and `TerminalReleaseEvidence` field layouts remain
unchanged; `GuardedReleaseOutcome` adds the withheld reason needed by the two
worker callers.

### Cleanup callers and exception map

| Caller | Route and decision |
| --- | --- |
| Terminal worker finalization | `finalize_worker` → `terminal_cleanup_target` → `release_terminal_tracked_scope_with_attempt_admission` → `verify_safety_cleanup_target` for each sender attempt. This does not require current foreground focus; it requires the same release origin and current immutable target/owner continuity. If the target stamp is already invalid/unavailable, the attempt reaches the backend guard with no stamp and is explicitly withheld; unresolved obligations and failed-release state remain quarantined. |
| Resumable suspend cleanup | `suspend_live_input_for_dispatch` → `suspend_scheduler_input` → `release_tracked_with_attempt_admission` → the same fresh `verify_safety_cleanup_target`. A missing stamp is rejected inside the per-attempt admission path, preserving the failed-release mask; any withheld or unverified release returns an error before `on_verified_cleanup_at`, so no resume barrier is published. |
| Startup abort/admission cleanup | The two `startup_admission.rs` callers retain `release_all`. They run before worker dispatch, where no Down can yet create a tracked obligation; the backend's zero-obligation path issues no query, probe, or input. This startup cleanup is not the terminal or resumable retry loop. |
| Canonical parent-loss emergency release | `sky_dispatch_win32::emergency_release_canonical` retains its separate full-instrument emergency contract and fresh backend. It is not a session-origin cleanup and was not changed. |
| Backend low-level cleanup helpers | Existing unguarded `release_scope`, `release_all`, and `release_terminal_tracked_scope` APIs remain for existing backend/unit-test callers. The production worker terminal and resumable callers use the guarded entrypoints above. No caller was removed. |

### R3 reachability and resume map

| Path | Reachability after immutable binding | Evidence |
| --- | --- | --- |
| `stage_candidate` returns `None` because no valid fixed target stamp exists or the target latch changed during staging | Terminal worker error; no retry that assumes a later target publication | `startup_admission::target_invalidated_before_scheduler_staging_finishes_without_input` |
| A mixed staging attempt returns `None` under the immutable target binding | No Up-only restage against that same unavailable/invalid target; caller handles the failed staging result | `stage_scheduler_action_preserving_due_up` now returns the original `None` directly. |
| A staged candidate's immutable target stamp becomes stale before its interruptible wait | Terminal worker error; no target-republication retry | `startup_admission::target_replacement_during_long_physical_wait_autonomously_stops_held_origin` |
| Mixed Up+Down final Down admission rejects only the Down portion while an independent Up remains valid | Reachable `stage_up_only_for_mixed_replan` retained; normal final Up gate still runs | `modifier_replan::production_worker_modifier_replan_backoff_preserves_due_up_and_fresh_chord_gate`; `dispatch_loop::safety::mixed_packet_with_final_modifier_rejection_replans_as_up_only` |
| Live OS focus is lost and later restored while the bound owner remains continuous | Reachable focus pause/grace/resume path retained | `startup_admission::production_worker_focus_restore_resumes_after_grace_with_mock_sender` |
| Focus-free mock session with no bound target | Explicit supported test path retained; default test options use `require_focus = false` and the mock backend | `startup_admission::startup_boundary_publishes_after_worker_startup` |

### R4 caller/deletion census

| Symbol / state | Before → after | Caller evidence / retained surface |
| --- | --- | --- |
| `wait_after_unstable_target_with_deadline` | Deleted; its only caller discarded the returned deadline via `.0`. | `wait_after_unstable_target` remains for the separate blocked-resume/focus path and its tests. |
| `wait_after_modifier_held` result handling | Returns a typed `AlreadyDue` or `Waited { purpose, deadline, planned_wait, WaitResult }`. | Up-deadline wait evidence is captured and guarded for one selector pass; maintenance poll observations are not queued; interrupts and failed waits are counted through the shared counters. |
| `ProductionForensicsAnomaly` and 32-record ring | Removed from non-test builds; raw record writes and overwrite tracking are compiled only under `cfg(test)`. | Production still exports scalar anomaly counts, structural/timing counters, and schema-25 keys; the ring-overwrite scalar is zero when no ring exists. Tests retain detailed anomaly fixtures. |
| `WorkerTimingState::timing_margin_ticks` | Removed along with its admission conversion and test-harness initialization. | `timing_margin_us` and the test evidence conversion in `DispatchTimingConfigEvidence` remain. Materialized H/G timing values and their rounding are unchanged. |
| `WorkerSchedulingGuards::is_priority_active` | Deleted. | No caller existed in the `sky_player` crate. Other live scheduling guard accessors remain. |
| `TargetProofOperationCounts.authority_arc_clones` | Deleted from the zero-initialized test counter and operation output. | It was never incremented. The actively counted `ReleaseOwnerProof` identity clone counter remains. |
| `apply_final_down_rejection(_target, _expected)` | Removed the unused arguments and corrected the stale authority-invalidation comment. | The helper remains and has nine final-admission call sites. |
| `release_scope`, `release_all`, `release_terminal_tracked_scope` | Retained. | Existing backend tests and startup abort/admission callers remain; the production terminal/resumable worker routes use the guarded methods. |
| Tauri DTOs, public playback schema, emergency release policy | Retained unchanged. | No schema or release-ownership policy change is part of this correction. |

No Arc journal redesign, timing observer offload, or speculative timing
bookkeeping optimization is included.

## Focused offline/mock regressions

Commands were run from the repository root unless a manifest is specified.

| Command / test filter | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all` | Passed. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_dispatch_win32 -p sky_player --all-features --no-run` | Passed; all selected targets compiled. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_dispatch_win32 --all-features guarded_terminal_cleanup -- --nocapture` | Passed, 3 tests: per-retry invalidation/raw-zero/QPC evidence, healthy retry/zero obligations, and partial/uncertain evidence retention. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features cleanup_revalidates_owner_after_first_incomplete_attempt -- --nocapture --test-threads=1` | Passed, 2 tests: terminal and resumable paths withhold the second sender call and retain/quarantine the obligation. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features modifier_backoff -- --nocapture --test-threads=1` | Passed, 3 tests including owner invalidation interrupt and controlled-QPC deadline selection. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features independent_up_deadline -- --nocapture --test-threads=1` | Passed, 1 test covering H and completion-based G floors. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features production_worker_modifier_replan_backoff -- --nocapture --test-threads=1` | Passed; 79 bounded admission iterations during the 120 ms held-modifier window, independent Up dispatched, pause/focus interrupts acknowledged, and chord Down passed fresh gates after focus/modifier recovery. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features terminal_controls_interrupt_modifier_replan_without_attempting_blocked_down -- --nocapture --test-threads=1` | Passed; quit, skip, panic, and suspend controls interrupt the modifier replan without a blocked Down. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features target_invalidated_before_scheduler_staging_finishes_without_input -- --nocapture --test-threads=1` | Passed. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features target_replacement_during_long_physical_wait_autonomously_stops_held_origin -- --nocapture --test-threads=1` | Passed. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features production_worker_focus_restore_resumes_after_grace_with_mock_sender -- --nocapture --test-threads=1` | Passed. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features startup_boundary_publishes_after_worker_startup -- --nocapture --test-threads=1` | Passed with default focus-free policy and mock backend; worker completed the ordinary Down/Up schedule without a bound target. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features production_scheduler_lifecycle_matrix_releases_or_quarantines_accepted_downs -- --nocapture --test-threads=1` | Passed; verified valid pause/focus cleanup and that target invalidation withholds the first Up, records zero sender attempts, preserves the obligation, sets failed-release quarantine, and publishes no resume barrier. |

The focused tests above are offline/mock regressions; local gate results and
artifact status are recorded below. The exact commit/tree and hosted Windows
CI are recorded in the final PR handoff comment.

## Full local gates and hosted CI

Final local gates passed after the target-invalidation and owner-loss oracles
were updated to require withholding and quarantine:

| Command | Result |
| --- | --- |
| `cargo xtask check rust` | Passed from the repository root: `cargo fmt --check`, workspace/all-target Clippy with `-D warnings`, and the full workspace all-features test suite. |
| `cargo xtask check desktop-native` | Passed: Tauri desktop-runtime/default/all-features checks, tauri-test library tests, and generated binding comparison. |
| `pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1` | Passed: PowerShell parser and offline observer fixture, contract positives and 47 negative cases. Its Rust invocation ran only `native_reader_accepts_powershell_v3_compat3_fixture_and_rolling_window`; it did not run the acceptance CLI. |
| `cargo xtask check all` | Passed: static/security/contracts and supply-chain checks, full Rust gate, Desktop type/build checks, browser E2E, and Desktop-native checks. |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | Passed. |
| `git diff --check` | Passed. |

The aggregate architecture audit emitted 67 advisory warnings; checks still
passed. These include a module-size advisory for
`sky_dispatch_win32/src/input/tests/cleanup.rs` and the repository's other
recorded size/unsafe-boundary advisories. No warning was promoted to a gate
failure.

The R1–R4 baseline correction's first two Rust-gate attempts correctly exposed
stale test oracles, not flaky behavior: the lifecycle matrix expected cleanup
after immutable target invalidation, then the modifier test expected no
failed-release count after owner loss. The tests now assert withheld Up, zero
new sender calls, retained obligations, and failed-release quarantine.

### Final audit follow-up gate results

| Command / gate | Result |
| --- | --- |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features modifier_ -- --nocapture --test-threads=1` | Passed: 12 `sky_player` tests, covering 19.5/20 ms controlled QPC selection, typed Up/poll results, observer on/off accounting, wake attribution, existing mixed Up+Down final gates, and worker interrupt/focus paths. Six `rt_native_acceptance` binary unit tests also passed as tests only; the acceptance CLI was not invoked. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features hold_forensics -- --nocapture --test-threads=1` | Passed: 10 hold-forensics tests, including test-only anomaly detail and ring-overwrite accounting. |
| `cargo test --manifest-path rust/Cargo.toml --locked -p sky_player --all-features native_telemetry_schema_25_serialized_key_sets_are_pinned -- --nocapture --test-threads=1` | Passed: schema-25 serialized key fixture remains unchanged. |
| `cargo check --manifest-path rust/Cargo.toml --locked -p sky_player` | Passed with no warnings; non-test build excludes detailed anomaly records and their per-record field calculations. |
| `cargo xtask check rust` | Passed after the final source edit: `cargo fmt --check`, workspace/all-target Clippy with `-D warnings`, and full workspace all-features tests. |
| `bun run check` in `apps/desktop` | Passed during the single `cargo xtask check all` attempt. No desktop source changed afterward. |
| `bun run test:e2e` in `apps/desktop` | Passed in a standalone run: 155 tests; 4 opt-in screenshot evidence cases skipped (`GRAPHITE_FINAL_UI_CAPTURE=after`, `GRAPHITE_PACKAGE_C_EVIDENCE=1`, `PACKAGE_B_VISUAL_EVIDENCE=1`, and `GRAPHITE_WORKSPACE_CAPTURE=candidate`). E2E also completed successfully inside the final aggregate run. No E2E failure or flaky test was reported. |
| `cargo xtask check desktop-native` | Passed after the final source edit: desktop-runtime/default/all-features checks, tauri-test library tests, and generated binding comparison. Also passed as part of the completed aggregate run. |
| `cargo xtask check all` | First attempt was interrupted during E2E, so it was not counted as a pass. The subsequent complete run on the final source passed all static/security/contract/supply-chain, Rust, Bun check/E2E, and desktop-native stages. Architecture emitted 67 advisory warnings; no gate failed. |
| `git diff --check` | Passed after the final source and evidence edits. |

All final gate components passed on this correction. The first interrupted
`check all` attempt remains recorded as interrupted and is not represented as a
pass. No flaky test was observed.

Hosted Windows CI is reported against the exact pushed commit in the PR
handoff comment; it is not treated as passed until its exact-head required jobs
finish.

## Artifact status

Cargo/desktop and test builds produced ordinary ignored build outputs from the
correction source. No release package, installer, native qualification
manifest, or native gameplay artifact was produced or run. No native Final,
Canary, Smoke, Compare, ReceiveOnly acceptance, or real input was invoked. No
`f48e5a75` manifest/binary was used with changed source. CI-produced
compile/test artifacts, if any, are not native gameplay qualification
artifacts.
