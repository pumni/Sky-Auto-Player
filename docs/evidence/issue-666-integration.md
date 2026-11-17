# Issue #666 — Integrated correctness checkpoint

## Checkpoint scope

This is the post-M4 integration correction checkpoint requested for Issue #666
and draft PR #667. It is not M5 and is not a merge or milestone-acceptance
decision. The reviewed starting point was `f3fe3de7f622f94a9f548e0913bf4903dfefd5f2`
(tree `4f85d8deb0883911c5118102f9e35738129e2344`). CI workflow
`43790581987` passed on that reviewed SHA; it does not qualify the races fixed
here. The final checkpoint SHA, tree, parent, remote tip, CI state, and exact
verification ledger are recorded below after the commit is pushed.

No real `SendInput`, Sky gameplay, interactive focus, or suspend/resume
qualification was run. All packet and owner race regressions use the
production worker/harness with deterministic mock sender and identity seams.
The Windows native test budget remains unapproved.

## P1 — Musical Up remains bound to the Down origin

`dispatch_staged_candidate` now reads a frozen `ReleaseOwnerProof` from the
tracked release ledger for every musical Up bit before final admission. The
fresh final owner proof validates the current target and process authority;
the Up origin must match that proof by target generation and retained
authority identity. A scoped target writer permit serializes final target
publication/binding/close with the syscall and is held across the sender
boundary. HWND/PID equality alone is not used as process authority. Missing,
conflicted, mixed-origin, or drifted proofs fail closed before the sender and
leave the corresponding release obligation in the ledger.

| Scenario | Sender boundary and attempts | Obligation/scheduler result | Regression |
|---|---|---|---|
| Down(A), rebind B, authored Up(A) | Initial A Down: 1; Up on B: 0 | A mask `0x0001` retained; terminal diagnostic contains `musical Up origin owner differs` | `musical_up_from_a_is_withheld_after_rebind_to_b` |
| Down(A), republish same HWND/PID at a new generation, Up(A) | Initial Down: 1; Up across ABA: 0 | A mask retained; generation is part of the proof | `musical_up_from_a_is_withheld_after_same_hwnd_generation_aba` |
| Mixed Up(A)+Down(B) | Initial A Down: 1; mixed packet: 0 | A mask retained; B Down cursor remains current and unconsumed | `mixed_up_a_down_b_is_withheld_without_consuming_down_b` |
| Mixed packet is modifier-blocked, then staged as Up-only | Initial A Down: 1; mixed and fallback Up: 0 | A proof is re-read from the ledger, not rebuilt from B; A mask retained | `mixed_to_up_only_modifier_replan_keeps_origin_a_and_withholds_up_on_b` |
| Owner generation changes after continuity query and before permit | Initial Down: 1; pending Up: 0 | Permit/revalidation returns Replan; obligation retained | `generation_change_after_final_identity_query_fails_before_sender` |
| Same owner, independent Down/Up slots | Four calls for two Downs and two Ups | Each Up consumes only its own obligation; final mask `0x0000` | `same_owner_independent_ups_keep_separate_origin_obligations` |
| Missing or conflicted proof | No invalid Up call | Mask retained; terminal diagnostic identifies missing/conflicted owner proof | `missing_up_owner_proof_fails_closed_with_terminal_diagnostic`, `conflicted_up_owner_proof_fails_closed_with_terminal_diagnostic` |

The dispatcher's existing packet builder still emits Up before Down, keeps
Up and Down masks disjoint, and caps each prepared packet at 15 `INPUT`
records. Same-owner Up remains independent even when another slot has a
separate obligation.

## P2 — Stale owner polls no longer trigger desktop panic release

The engine now exposes `OwnerIdentityPollResult::{CurrentValid,
StaleSuperseded, CurrentInvalid}`. The old bool API remains as a compatibility
wrapper (`true` only for `CurrentValid`). The desktop monitor consumes the
typed result through `consume_owner_identity_poll_result`; only
`CurrentInvalid` marks that active binding invalid and calls `panic_release`,
once. The monitor's foreground hint remains a transition hint. It does not
authorize a musical packet, whose worker still performs fresh final admission.

`stale_owner_poll_failure_cannot_invalidate_rebinding_or_block_final_admission`
holds poll(A) at barriers, binds B, then completes A with a terminated-process
result. It observes `StaleSuperseded`, verifies B authority remains bound,
arms the actual production worker, and observes B's final admission and
dispatch succeed. The desktop consumer tests exercise the same branch helper
called by the monitor: stale and valid results call panic zero times; repeated
current-invalid polls call it exactly once. The current-owner invalidation
case also remains covered by
`current_owner_poll_failure_still_fails_closed_before_first_down`.
`late_authority_close_cannot_clear_a_newly_bound_generation` preserves the
late-close generation guard. Per-session invalidation state is reset with the
active session, so a poll result from an old binding cannot become the new
session's owner failure.

## P2 — Narrow proven raw-zero Down-only continuation

`proven_raw_zero_down_only` accepts only a Down-only packet with the exact
authored chord and mask, one raw sender attempt, `ZeroProgress`, raw inserted
count zero, no confirmed/skipped bits, no retry, matching Win32 error
metadata, and present ordered start/completion QPCs with no timing error.
The actual sender evidence carries the raw count; unavailable or synthetic
confirmed evidence cannot satisfy this predicate. `raw_zero_continuation_is_safe`
also requires the same target stamp, current control/focus state, an unexpired
lease, and no pause, stop, skip, panic, or suspend. The final owner permit
remains held through classification and journal reconciliation.

For a proven packet, the Down's scheduler identity is consumed as attempted
exactly once. Only that Down's in-flight/possibly-active journal residue is
cleared. No Up is sent for it, no completion time or cleanup/G barrier is
created, and previously confirmed H/G floors remain. The attempted anomaly,
raw sender evidence, counters, and terminal error latch remain sticky. Any
missing/contradictory evidence, short insertion, mixed or Up-bearing packet,
unsafe target/control state, or journal mismatch follows the existing
conservative reconciliation/cleanup path.

### A-hold / B-zero / C continuation trace

`proven_raw_zero_down_only_keeps_a_hold_and_dispatches_c_without_a_fake_floor`
uses the real production dispatch harness and captures the mock packet
boundary. The trace is:

| Packet | Requested masks (Up, Down) | Raw result / attempts | State after commit |
|---|---|---|---|
| A Down | `(0x0000, 0x0001)` | Complete; one attempt | A active, release mask `0x0001` |
| B Down | `(0x0000, 0x0002)` | ZeroProgress; raw inserted `0`; one attempt | A remains active; B is consumed as attempted; no B Up or fake completion/barrier |
| C Down | `(0x0000, 0x0004)` | Complete; one attempt | A and C active; C is evaluated and sent once |
| C Up | `(0x0004, 0x0000)` | Complete; one attempt | A remains active; C cleared |
| A Up | `(0x0001, 0x0000)` | Complete at A's authored Up time; one attempt | Release, active, possibly-active, failed-release, and in-flight masks all `0x0000` |

There are exactly five sender attempts: A=1, B=1 (raw count 0), C Down=1,
C Up=1, A Up=1. B is never replayed, is never reported as successfully
inserted, and receives no safety Up based on the zero count. Final session
outcome remains `Error` because B's attempted anomaly is sticky, even though
C dispatches and the final tracked masks are zero.

`proven_raw_zero_preserves_prior_h_and_g_floors_at_minus_exact_and_plus_one_tick`
checks H and G independently at −1 tick, exact, and +1 tick after B. At
−1, scheduler returns `Wait` at the actual retained floor; exact and +1 are
eligible. The floor is derived from A's confirmed physical send/Up; B creates
neither a completion timestamp nor a barrier. Conservative-path tests cover
short insertion and contradictory QPC (`raw_zero_classifier_rejects_short_packet_and_contradictory_qpc`),
mixed Up(A)+Down(B) (`mixed_up_a_down_b_raw_zero_uses_conservative_reconciliation`),
and concurrent Stop/Skip/lease expiry
(`raw_zero_with_concurrent_stop_skip_or_lease_expiry_cleans_up_and_does_not_continue`).
`raw_zero_candidate_owner_drift_after_identity_query_fails_before_sender`
configures a raw-zero sender but republishes the target after continuity query;
the permit rejects before the mock packet boundary, so there is no attempted
zero to consume. No post-query publication can cross the permit held from
admission through sender return and classification.

The tracked sender journal is checked on both sides of the exception by
`proven_raw_zero_journal_clear_preserves_another_confirmed_hold`: before the
proven-zero clear, A is active (`0x0001`), B is possibly active and in flight
(`0x0002`), and obligations are `0x0003`; after clearing only B, A remains
active and obligated (`0x0001`), while B's possible/in-flight bits are zero.
The conservative `deferred_down_zero_insert_retains_in_flight_until_verified_cleanup`
test separately proves ambiguous zero keeps the in-flight obligation until
verified safety cleanup.

## Staging, final admission, and journal simplification

The worker now shares one fresh current-owner proof across Down and Up
admission, compares each Up origin to it, and acquires one final writer permit
before the sender. The permit and process creation-time check are retained
because the ordinary atomic generation check alone cannot close the gap
between an OS identity query and the syscall. The lease, watchdog, suspend,
focus, and control checks remain at their established boundaries.

| Old path / helper | Status new | Caller real | Invariant kept | Evidence |
|---|---|---|---|---|
| `UpOnlyReplanAdmission` one-field wrapper | Removed | `stage_up_only_after_mixed_replan` callers in scheduler stage fallback and final mixed replan | Uses the same `SchedulerStageAdmission`; still rechecks control/focus and rebuilds the Up timing window from active scheduler slots | `mixed_to_up_only_modifier_replan_keeps_origin_a_and_withholds_up_on_b`; scheduler deadline regression |
| Separate scheduler `evaluate_scheduler_action` then `project_scheduler_action_for_staging` pass | Consolidated into `evaluate_scheduler_action_for_staging` | Worker scheduler staging | A `Wait` that has a future deadline is still evaluated at that exact deadline; the worker still evaluates again after an actual wait/wake because eligibility may change | `scheduler_deadline_reevaluation_carries_down_metadata_from_evaluation`; counted scheduler hook |
| Repeated foreground/owner/process admission reads (`final_down_fresh_target_admission` plus `fresh_process_identity_verdict`) | Consolidated to one fresh `ReleaseOwnerProof` query and process-identity proof per packet | `verify_final_down_admission` / `verify_final_musical_up_admission` through `dispatch_staged_candidate` | Fresh foreground PID and process creation-time proof remain; final permit revalidates current generation | Final admission operation hook; A→B and ABA regressions |
| Per-packet owner proof reconstruction for Up-only fallback | Removed as a target-derived substitute; retained-ledger proof is re-read | `dispatch_staged_candidate` on both mixed and fallback packet | Up origin remains frozen; fallback cannot transfer A's obligation to B | `mixed_to_up_only_modifier_replan_keeps_origin_a_and_withholds_up_on_b` |
| Full 15-slot `clear_unobligated_release_owners()` sweep on ordinary packet commit | Consolidated to relevant packet-mask slots | `commit_physical_packet`, packet outcome cleanup, proven-zero journal clear | Clears only slots touched by packet; terminal zero-obligation cleanup intentionally retains a bounded full clear | `measured_final_admission_operations_for_healthy_packet_shapes`; terminal cleanup tests |
| `NativeDispatchSession::snapshot` and `snapshot_lite` repeated common field reads/projection | Consolidated into `SessionSnapshotProjection` | Desktop/player full and lite status consumers | Shared lifecycle, elapsed/pre-roll/paused state, common metrics, compatibility constants, errors, and outcomes use one captured source; public full/lite DTO shapes stay unchanged | `assert_snapshot_common_projection`; schema-25 key fixture; desktop-native tests |
| Target generation checks | Retained | Stage, pre-wait, final admission, permit, and cleanup | Stale staged candidates cannot become authority; same-HWND generation change remains distinguishable | Generation counts and ABA/permit regressions |
| Process creation-time proof (`GetProcessTimes`) | Retained, consolidated to one final process check per healthy packet | `verify_final_musical_packet_admission` via the retained authority | PID/HWND reuse does not stand in for process identity | `verify_final_process_identity`; source and owner race tests |
| Watchdog lease, control, and power/suspend protections | Retained | Worker wait and final admission; desktop owner monitor | Stale monitor results do not grant syscall authority; owner loss still fails closed | Lease/Stop/Skip/suspend regressions and desktop consumer tests |
| Musical final owner permit, including the nested authority check | Corrected: writer `try_lock` plus owner-authority `try_lock` | `dispatch_staged_candidate` through `verify_final_musical_packet_admission` | Fresh generation/HWND/PID/`Arc::ptr_eq` proof remains required; either busy lock returns Replan before syscall; writer permit remains held through sender | `musical_down_final_permit_replans_without_waiting_for_owner_authority_mutex`, `musical_up_final_permit_replans_without_waiting_then_sends_once`, owner-generation publication contention regression |
| Serialized release permit | Retained | `verify_safety_cleanup_target` for terminal/resumable tracked cleanup via `lock_current_release_owner_proof` | Cleanup still verifies the frozen origin and serializes proof with target publication through cleanup syscall | `terminal_tracked_cleanup_uses_owner_proof_without_foreground_requirement`; owner-origin cleanup regression suite |

### Final permit mutex contention correction

Leader review at `e55e9e06526c1ecaba6c5bd7d5cc93a97b09111f` identified that
`try_lock_current_release_owner_proof` acquired `writer` with `try_lock`, then
called `release_owner_proof_is_current_locked`, whose nested
`owner_authority.lock()` could block if the supervisor held the authority
mutex or was preempted while holding it. A nonblocking writer lock alone did
not make the musical final permit nonblocking.

The musical permit now runs in this order: fresh current-target and process
creation-time proof; `writer.try_lock()`; `owner_authority.try_lock()`; then
stable HWND/generation, owner-status/PID, retained generation and
`Arc::ptr_eq` checks while the authority guard is held. Failure to acquire
either lock or match proof returns pre-syscall Replan. The authority guard is
dropped after validation; the writer guard remains held through physical
sender return, serializing publish/bind/close. No proof is cached across
replan. The safety-cleanup caller retains `writer.lock()` followed by
`owner_authority.lock()` in `release_owner_proof_is_current_serialized`; that
verified cleanup path is deliberately allowed to use serialized blocking
synchronization. The new seam `lock_owner_authority_for_test` is `#[cfg(test)]`
and adds no production lock accounting or allocation.

The channel-controlled regressions use the actual dispatch worker harness. The
final-gate hook signals only after fresh foreground/PID/process verification;
the supervisor thread then holds the owner-authority mutex before releasing
the worker into final permit. For Down, the worker returns Replan while the
mutex is still held: sender count remains zero, chord cursor remains 0,
active/possible/in-flight/release masks remain zero. After unlock and fresh
restage, one Down is sent. For Up(A), the first attempt sends nothing and
retains active/release mask `0x0001`; after unlock and fresh restage, one
same-owner Up clears the mask. The timeout only bounds deadlock detection; the
hook/channel establishes the interleaving.

The generation control starts B publication while the authority mutex is
held. A test-only publication hook pauses the publisher after it acquires the
writer and before it resets the retained authority; final admission replans
without a packet. After releasing the mutex, B is published at generation 1
and bound. A newly staged Up(A) under B is rejected by the frozen-origin
comparison, leaves A's obligation `0x0001`, and produces no additional
sender call. Existing A→B and same-HWND ABA tests remain unchanged. Same-owner
Down/Up and healthy mixed dispatch still pass. The positive terminal cleanup
test proves the serialized safety path can complete.

`EnginePollStatus` has no public `Blocked` variant. A scheduler wait or onset
block continues to project the existing lifecycle status (`Playing` while
active); no status value or wire field was introduced. A blocked onset is
covered as a dispatch/admission case, not represented as a new public status.

## Healthy proof-operation accounting

The “before” column is a manual call-path census of the reviewed source at
`f3fe3de`, not a baseline latency benchmark. The “after” column is from
`measured_final_admission_operations_for_healthy_packet_shapes`, which resets
the test counters after setup and exercises the production stage, pre-wait,
final-gate, journal, and mock sender paths. The measurement is per one
eligible packet; mixed-to-Up-only includes the blocked mixed attempt and the
successful Up fallback. Blocked-replan is per one modifier-blocked candidate.
Worker outer-loop/waiter QPC reads and setup are excluded. Prepared native
sender's two QPC samples per physical call are reported separately because
the mock sender does not execute the Win32 sender.

| Operation | Down-only before → after | Up-only before → after | Mixed before → after | Mixed → Up fallback before → after | Modifier-blocked onset before → after |
|---|---:|---:|---:|---:|---:|
| Scheduler evaluations | 1 → 1 | 1 → 1 | 1 → 1 | 1 → 1 | 1 → 1 |
| QPC reads for one due worker iteration (outer loop + final suffix + native sender) | 4 → 4 | 4 → 4 | 4 → 4 | 4 → 4 | 1 → 1 |
| `GetForegroundWindow` queries | 2 → 1 | 2 → 1 | 2 → 1 | 2 → 1 | 0 → 0 |
| Foreground owner/PID checks | 2 → 1 | 2 → 1 | 2 → 1 | 2 → 1 | 0 → 0 |
| Process identity checks / `GetProcessTimes` calls | 2 / 2 → 1 / 1 | 2 / 2 → 1 / 1 | 2 / 2 → 1 / 1 | 2 / 2 → 1 / 1 | 0 / 0 → 0 / 0 |
| Authority `Arc` clones (target + release proof) | 2 → 2 | 1 → 2 | 2 → 3 | 1 → 3 | 0 → 0 |
| Owner-authority mutex acquisitions | 2 → 2 | 1 → 2 | 2 → 2 | 1 → 2 | 0 → 0 |
| Target-generation revalidations | 11 → 11 | 8 → 10 | 11 → 11 | 11 → 13 | 4 → 4 |
| Owner-ledger slot visits | 16 → 2 | 15 → 2 | 16 → 4 | 15 → 3 | 0 → 0 |
| Physical sender syscalls by production source path (mock boundary count in test) | 1 → 1 | 1 → 1 | 1 → 1 | 1 → 1 | 0 → 0 |
| Healthy worker dispatch allocations | 0 → 0 | 0 → 0 | 0 → 0 | 0 → 0 | 0 → 0 |

The QPC row decomposes into one outer worker-loop sample, one final lease-age
sample for an admitted packet, and two native sender samples (pre-call and
completion). The measurement hook prints the final-suffix count (`1` for a
healthy packet, `0` for a modifier block); outer-loop and native sender counts
are one each by source at this single due iteration. A post-identity
generation-race attempt adds one fresh final-gate QPC sample to the outer-loop
sample, then the permit rejects before the sender (two total QPC reads, zero
sender calls in that race probe).

The accounting uses the actual production branches: old final admission made
two foreground-owner and two process-identity calls (`final_down_fresh_target_admission`
and `fresh_process_identity_verdict`), then Down-bearing packets separately
loaded their release proof. That path acquired the owner-authority mutex twice
for Down-bearing packets (fresh process check and retained release proof) and
once for Up-only packets (fresh process check); current admission does one
fresh proof acquisition plus one permit validation. Current counter hooks cover foreground/PID,
identity, `GetProcessTimes`, target mutex/Arc/writer permit/generation checks,
scheduler evaluations, QPC, release-proof Arc clones, and ledger visits. The
current harness installs a synthetic foreground/process authority, so its
`real_GetForegroundWindow` and `GetProcessTimes` counters are zero even though
the seam counts one query/check; the corresponding real Windows source call
counts are the source-audited `1` shown above. The test's printed
`allocations=0` is a fixed label, not an allocator measurement; the zero claim
comes from the dedicated `rt_dispatch_no_alloc` gate.

The extra Arc clone and generation checks on Up paths retain a frozen origin
proof and a final permit. They are deliberate safety costs. Normal packet
ledger work visits only relevant mask bits rather than clearing all 15 slots.
The one scheduler evaluation remains unchanged for an already-eligible
candidate; a future-deadline Wait still takes the necessary deadline
reevaluation, and the worker still reevaluates after wake. The final
generation-race harness separately reports a new fail-before-sender permit
path; the reviewed implementation had no equivalent final permit.

## Shared snapshot projection and compatibility

Both snapshot APIs now capture common lifecycle/status, elapsed time,
pre-roll, paused and startup state, common metrics, compatibility constants,
last/terminal errors, and terminal outcome through `SessionSnapshotProjection`.
Each public DTO then adds its existing shape. No DTO, fingerprint, schema, or
serialized key changed; telemetry remains schema 25. The shared projection
parity assertions cover running/finished dispatch, idle scheduler wait,
manual pause/resume, startup failure/cancellation outcomes, and error
projection. `EnginePollStatus` has no Blocked status as described above.
`native_telemetry_schema_25_serialized_key_sets_are_pinned` verifies the
schema-25 serialized key fixture. Existing null/unavailable QPC provenance,
zero-valued compatibility fields, terminal outcome/error, and desktop
playback-status consumer behavior remain covered by the telemetry and
desktop-native suites.

## Verification ledger

Platform/toolchain: Windows x64 MSVC, Rust/Cargo 1.99.1.

| Command | Exit | Result |
|---|---:|---|
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib dispatch_loop::tests::` (first parallel run) | 1 | 44 passed, 1 failed: legacy `stale_target_replans_mixed_candidate_to_independent_up_only_packet` expected an Up to cross into a new target generation. Updated the regression to assert fail-closed origin ownership. |
| Same dispatch-loop focused command (parallel follow-up) | 1 | 43 passed, 2 failed under concurrent test load: authored-boundary evaluation jitter in `up_only_missing_completion_qpc_without_timing_error_still_latches_error` and cleanup-barrier timing jitter in the 40-anomaly regression. Both passed in the serial run below. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib dispatch_loop::tests:: -- --test-threads=1` | 0 | 46 passed; includes A→B, same-HWND ABA, mixed/fallback, missing/conflicted proof, raw-zero, owner drift, H/G ±1, and conservative-path regressions. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support --lib proven_raw_zero_journal_clear_preserves_another_confirmed_hold` | 0 | 1 passed; exact pre/post journal masks for B zero while A remains held. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib stale_owner_poll_failure_cannot_invalidate_rebinding_or_block_final_admission` | 0 | 1 passed; 301 filtered. Barrier-held A poll does not invalidate B, cancel Arm, or prevent B final admission. |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --features tauri-test --lib owner_identity_poll_consumer_tests::` | 0 | 3 passed; 341 filtered. Actual desktop monitor consumer seam: stale/current-valid panic count 0; repeated current-invalid panic count 1. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib native_telemetry_schema_25_serialized_key_sets_are_pinned` | 0 | 1 passed; schema remains 25 with pinned serialized key sets. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib engine::tests::telemetry_metrics::` (parallel run before drift assertion update) | 1 | 43 passed, 1 failed: full/lite elapsed samples differed by 3 µs because each API independently samples QPC. Comparator now bounds only elapsed/pre-roll sample drift at 5 ms; lifecycle, status, metrics, and compatibility fields remain exact. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib engine::tests::telemetry_metrics:: -- --test-threads=1` | 0 | 44 passed; full/lite common projection, running/paused/finished/error, startup, and terminal cases pass. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib proven_raw_zero_preserves_prior_h_and_g_floors_at_minus_exact_and_plus_one_tick -- --nocapture` | 0 | 1 passed; H/G −1, exact, +1 tick matrix passed. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features "test-support qpc-read-counting" --lib measured_final_admission_operations_for_healthy_packet_shapes -- --nocapture --test-threads=1` | 0 | 1 passed; exact after-counts above; no mock/native qualification claim. |
| `git diff --check` (before final evidence updates) | 0 | No whitespace errors at that point; final check is recorded after all edits. |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` (initial) | 1 | Reported rustfmt diffs; formatted Rust workspace and desktop manifest before rerunning. |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` (initial) | 1 | Reported the same workspace rustfmt diffs; both formatter commands applied successfully. |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` (initial) | 1 | Rejected `raw_zero_continuation_is_safe` for too many arguments; changed it to consume existing `FinalControlSignals`. |
| Same clippy command (rerun) | 0 | Clean; no warnings. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | 80 unit + 1 executor property + 1 golden + 2 property tests passed; 0 failed/ignored; 0 doctests selected. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | 0 | 278 passed, 1 ignored child-process helper; 0 failed; 0 bin/doctests selected. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` (earlier integration checkpoint, before final permit correction) | 0 | 302 passed, 1 ignored waiter-only A/B benchmark; 0 failed; nested no-allocation target 4 passed; 0 doctests selected. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | 4 passed; no healthy dispatch allocations across 1/5/15-key and mixed packets, including G boundary cases. |
| Focused desktop consumer suite above | 0 | 3 passed. |
| `cargo xtask check desktop-native` | 0 | PASS. Three `cargo check` variants (desktop-runtime/no-default, default, all-features), Tauri-test lib suite, and binding check completed; subcommand output was summarized by xtask. |
| `cargo xtask check all` (final serial run, after journal and owner-drift regressions) | 0 | PASS. Static checks: 67 advisory architecture warnings, 0 security findings; Rust format, workspace Clippy, and all-features tests passed; Bun frozen-lockfile install/check and browser E2E passed; three desktop-native cargo-check variants, Tauri-test lib, and bindings passed. No xtask retry or skipped stage was reported. Xtask captured child output, so it did not expose aggregate workspace test counts; package-focused counts are recorded above. The separate ignored waiter-only A/B benchmark remains ignored and is not gameplay qualification. |
| Source SHA for final permit correction gates | — | `dcac1263e8ad923d5293d96a3ede5b4f018a2d14`, tree `92b3c87be40de23e6f672704bd99401c50b8cb97`, parent reviewed SHA `e55e9e06526c1ecaba6c5bd7d5cc93a97b09111f`. Final correction source was fully gated at this SHA; subsequent edits in this evidence update document the results and the owning contract. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib final_permit_replans_without_waiting -- --test-threads=1` | 0 | 2 passed: Down and Up both return Replan with `owner_authority` held after fresh identity proof; each dispatches exactly once after unlock and fresh restaging. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib owner_generation_publication_during_authority_contention_never_borrows_new_proof -- --test-threads=1` (first assertion) | 1 | Test expected generation 2; fixture publishes its first HWND transition as generation 1. Corrected the assertion and reran below; no product failure. |
| Same owner-generation contention test (rerun) | 0 | 1 passed; B publication is held at the owner-authority reset while the mutex is owned; A's old Up proof remains withheld after B binds. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib dispatch_loop::tests:: -- --test-threads=1` | 0 | 49 passed, 0 failed/ignored; includes the three new permit-contention/publication regressions and the existing A→B, same-HWND ABA, mixed, raw-zero, and healthy sender cases. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib engine::target::tests:: -- --test-threads=1` | 0 | 10 passed, 0 failed/ignored. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib terminal_tracked_cleanup_uses_owner_proof_without_foreground_requirement -- --test-threads=1` | 0 | 1 passed; verified terminal safety cleanup remains usable through its serialized path. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features "test-support qpc-read-counting" --lib measured_final_admission_operations_for_healthy_packet_shapes -- --nocapture --test-threads=1` (rerun after mutex fix) | 0 | 1 passed; healthy packet proof-operation counts are unchanged. Owner-authority acquisitions remain Down 2, Up 2, Mixed 2, Mixed→Up 2; the permit now uses a nonblocking authority acquisition. |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` on correction source SHA | 0 | Rust workspace formatting clean. |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` on correction source SHA | 0 | Desktop Tauri formatting clean. |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` on correction source SHA | 0 | No warnings. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` on correction source SHA | 0 | 84 passed (80 unit, 1 executor property, 1 golden, 2 property); 0 failed/ignored; 0 doctests. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` on correction source SHA | 0 | 278 passed, 1 ignored child-process helper; 0 failed; 0 bin/doctests. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` on correction source SHA | 0 | 305 passed, 1 ignored waiter-only A/B benchmark; 0 failed; nested no-allocation target 4 passed; 0 doctests. |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` on correction source SHA | 0 | 4 passed, 0 failed/ignored. |
| `cargo xtask check desktop-native` on correction source SHA | 0 | Three desktop cargo-check variants and the Tauri-test lib stage passed. |
| `cargo xtask check all` on documentation-updated checkpoint `7a4bcf1f779431073f928adde6e5b5a0e9459480` | 0 | PASS, serial. Static checks reported 67 advisory architecture warnings and 0 security findings. Rust workspace format, Clippy, and all-features tests passed; Bun frozen-lockfile install/check and browser E2E passed; three desktop-native checks and Tauri test passed. No failed/retried/skipped stage. Xtask captured child output, so aggregate test counts are not exposed; package counts above are exact. |
| CI on first code checkpoint push (`51789df4271d7e6a8baf0e1b8cbd0ba8ba12f2d6`) | cancelled | Workflow `43807337966` was initially observed in progress on that SHA; after the subsequent PR update, GitHub reported it cancelled. It did not qualify the final correction. |
| CI after full correction push (`a4e4a4e1a1b632b901f19f8c92ace525a983e084`) | queued / check pending | `gh pr checks 667 --repo pumni/Sky-Auto-Player` reported `Classify validation layers` pending; workflow `43814350737` (`https://github.com/pumni/Sky-Auto-Player/actions/runs/43814350737`) reported `status=queued`, `headSha=a4e4a4e1a1b632b901f19f8c92ace525a983e084`. CI was not awaited. Reviewed-base workflow `43790581987` passed only on `f3fe3de7f622f94a9f548e0913bf4903dfefd5f2`. |

Other intermediate results: the H/G boundary regression needed a missing
`AtomicU64` import and then an owned `Arc` in its sender closure; the operation
counter test needed its `PacketCapture` import and a non-borrowing mixed
candidate setup. Those compile failures were fixed before the passing focused
runs recorded above. The first final Clippy run rejected a 10-argument raw-zero
predicate; after it reused `FinalControlSignals`, Clippy passed. The initial
format checks reported rustfmt diffs; both manifest formatters were applied,
and final format checks passed. The first permit test implementation attempted
to move the production harness into a worker thread; compilation correctly
rejected its non-Send MMCSS guard. The final test keeps the production worker
on its harness thread and uses a separate supervisor thread to hold the
authority mutex. An unused first lock seam was removed before full gates. The
generation test's initial expected value was corrected from 2 to 1 after
confirming the fixture's initial generation. No failed or unrun gate is
reported as passed.

## Final checkpoint identity

| Item | Value |
|---|---|
| Branch | `refactor/prepared-playback-runtime` |
| Source code SHA / tree / parent on which correction gates ran | `dcac1263e8ad923d5293d96a3ede5b4f018a2d14` / `92b3c87be40de23e6f672704bd99401c50b8cb97` / `e55e9e06526c1ecaba6c5bd7d5cc93a97b09111f` |
| Final docs follow-up HEAD / tree / parent | Reported in the checkpoint handoff; this evidence file avoids self-referencing its own hash. Product source is unchanged from the fully gated source SHA above. |
| Remote branch tip after push / worktree state | Reported in the checkpoint handoff. |
| Final-SHA CI | Queried after the final push and reported in the checkpoint handoff; CI was not awaited. |
| Leader integration verdict | Await `ACCEPT INTEGRATION @<SHA>`; no merge or PR-ready action is authorized by CI alone |
