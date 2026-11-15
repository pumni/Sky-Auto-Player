# Issue #661 M3 qualification checkpoint

## Scope and decision status

- Issue / draft PR: [#661](https://github.com/pumni/Sky-Auto-Player/issues/661) / [#662](https://github.com/pumni/Sky-Auto-Player/pull/662)
- Branch: `refactor/dispatch-runtime-cleanup`
- Accepted M2 source: `74c2b2618af50881f15eacd27ab722d316954743`, tree `902c772442706b326e020db6d36daedd60632b02`
- Native A/B baseline: `2277ac34a21a57a8d5ec313bba55ad78fc60794`, tree `9a8637129dbb7a05be4f9386da188413b45f9774`
- Native status: **NOT RUN**. The checked-in qualified profiles reject these identities. No manifest or binary was built and no ReceiveOnly session was started.
- This packet requests a separate #661 qualification identity before native work. It does not authorize shipping, policy changes, a full Compare/Final matrix, merge, or release.

## Offline HoL fixture

`sky_player::timing_model::analyze_dispatch_schedule_cancellable` is the existing desktop timing-advisory engine and runs the production `DispatchExecutor`. The test-only driver in `timing_model::tests::m3_offline_fixture_separates_down_cursor_hol_from_sender_occupancy` drives that same executor with an integer QPC clock and deterministic sender costs. It has no Win32 calls, input injection, or scheduler implementation of its own.

Fixture settings are 60 FPS with a 1.0-frame hold and 500 µs safety margin: `T = 16,667 µs`, `H = 17,167 µs`, `G = 17,167 µs`. The synthetic advisory uses 1 MHz QPC, 10 µs sender setup, 3 µs per input, and 5 µs post-send. The authored schedule is:

1. Down A + independent C at 0 µs.
2. Up A at 20,000 µs.
3. Repeat Down A at 34,334 µs; this meets the authored H+G cycle.
4. Independent Down B at 35,000 µs, behind A in the Down cursor.
5. Independent Up C at 36,000 µs.
6. Up A at 50,000 µs and Up B at 65,000 µs.

| Packet | Authored target | Expected / observed masks (Up, Down) | Modeled pre-call | Sender completion | Worker free |
|---|---:|---|---:|---:|---:|
| Initial chord | 0 | `(0x00, 0x05)` | 0 | 16 | 21 |
| Up A | 20,000 | `(0x01, 0x00)` | 20,000 | 20,013 | 20,018 |
| Up C | 36,000 | `(0x04, 0x00)` | 36,000 | 36,013 | 36,018 |
| Repeat Down A | 34,334 | `(0x00, 0x01)` | 37,180 | 37,193 | 37,198 |
| Down B | 35,000 | `(0x00, 0x02)` | 37,198 | 37,211 | 37,216 |
| Repeat Up A | 50,000 | `(0x01, 0x00)` | 54,360 | 54,373 | 54,378 |
| Up B | 65,000 | `(0x02, 0x00)` | 65,000 | 65,013 | 65,018 |

The Down cursor remains on A at the 37,180 µs G deadline; B is not emitted before A. C's independent Up is emitted at its 36,000 µs authored deadline while A is still G-blocked. One later Down chord (B) is affected by cursor HoL. Its 2,198 µs authored-to-pre-call delay decomposes into 2,180 µs from B's target to A's G eligibility, then 13 µs of A sender occupancy and 5 µs of post-send worker occupancy. The existing advisory row reports 2,198 µs as its sequential `hol_delay_lower_bound_ticks`; the fixture makes the cursor-only and worker-occupancy portions explicit without changing that DTO or metric schema.

H/G continue to use sender completion. Up A completes at 20,013 µs, so the repeated A Down is eligible at `20,013 + G = 37,180 µs`. Repeated Down A completes at 37,193 µs, so its authored Up at 50,000 µs is held until `37,193 + H = 54,360 µs`. No chord is reordered, dropped, retried, replayed, or shifted to improve the result.

This is one deterministic synthetic fixture, not a song or native distribution. It supports no p50/p95/p99 claim. Native timer overhead, actual SendInput envelope, CPU/spin, startup-to-Ready, observer loss, focus/sink integrity, and game receipt are unavailable here.

## Runner identity audit

There is no current native runner route that validates the #661 baseline and accepted M2 candidate:

| Location | Existing identity gate |
|---|---|
| `scripts/build_dispatch_core_harness.ps1` (`QualificationProfile`, `Get-SourceIdentity`, manifest writer) | `FrameSnapshot` pins base `8a800ff07a5dc362a5ee3aaa59204ff40b6ae327` / tree `48eec424c155e98fa1a81ad018bc832b6e6b964f`, branch `refactor/frame-snapshot-dispatch`, workload `frame-snapshot-v1`; `Scheduler` and historical `DispatchCore` use other pins. Build records clean source SHA/tree/branch, role, profile, `dist`, features, observer, target and binary SHA. |
| `scripts/dispatch_harness_manifest.psm1` (`Assert-FrameSnapshotManifestIdentity`, `Assert-ImmutableHarnessManifestIntegrity`) | Enforces FrameSnapshot role/branch/base/tree/workload/observer/margins/target and SHA shapes; baseline must equal the old pinned SHA/tree. Immutable integrity checks the manifest/binary paths, `SHA256SUMS.txt`, binary hash, copied builder hash, build log and clean log. |
| `scripts/qualify_dispatch_core.ps1` (`Assert-FrameSnapshotSourceRoot`, `Read-And-VerifyManifest`, stage checks) | Requires a clean source root whose HEAD/tree equals its manifest; candidate must be the current clean `FrameSnapshot` branch and descend from the old pinned base. Candidate manifest must equal the current runner checkout HEAD/tree. Role pairs must match compiler, target, profile, features, workload and observer. Canary/Smoke safety uses 500 µs; FrameSnapshot Compare/Final performance uses 0 µs. |
| `scripts/run_native_acceptance_case.ps1` (profile pins and harness manifest checks) | Rechecks profile base/tree/branch, role, source root cleanliness and SHA/tree, exact candidate checkout, workload version, feature set, target, profile, observer and binary hash. |
| `scripts/test_qualify_dispatch_core_contracts.ps1` | Positive FrameSnapshot role fixtures; negative branch, base SHA, source hash, role, dirty source and pinned baseline-tree cases; immutable-manifest integrity and workload/margin/profile checks. These tests protect the old identity and do not authorize a new one. |
| `scripts/dispatch_qualification_report.psm1`, `docs/playback-verification.md` | Report labels and runbook describe FrameSnapshot/Scheduler identities and protocols; neither adds a #661 route. |

`FrameSnapshot` is incompatible with #661 at baseline SHA/tree and branch. `Scheduler` pins another base/branch; `DispatchCore` is historical #646. No path passes all current checks for `2277ac34…` and `74c2b261…`. Reusing the #658 runner would misstate provenance, so native qualification is **NOT RUN**.

## Minimal patch proposal requiring Leader approval

Add a separate profile, for example `DispatchCleanupM3`, across the build, manifest, qualification, case-runner, reporting and contract-test files above. Keep all existing `FrameSnapshot`, `Scheduler` and `DispatchCore` constants, gates and behavior intact.

Pin these exact identities:

- Baseline role: SHA `2277ac34a21a57a8d5ec313bba55ad78fc60794f`, tree `9a8637129dbb7a05be4f9386da188413b45f9774`, detached and clean.
- Candidate role: accepted M2 SHA `74c2b2618af50881f15eacd27ab722d316954743`, tree `902c772442706b326e020db6d36daedd60632b02`, clean detached source worktree. Record candidate lineage as `refactor/dispatch-runtime-cleanup` and verify that this branch contains the pinned candidate SHA.
- Record qualification-runner SHA/tree separately from the harness source SHA/tree. The runner may be at a later PR HEAD because only M3 docs or qualification tools changed; its HEAD must never silently replace the pinned M2 candidate identity. The M2 binary must be built from the exact pinned M2 worktree.
- Use a distinct workload identity, e.g. `dispatch-runtime-cleanup-m3-v1`, for the five named workloads. Keep optimized `dist`, `x86_64-pc-windows-msvc`, `real-input-acceptance` (no `qpc-read-counting`), frozen guard, `qualification-on`, Scheduler V3 Compat3 observer and version. Performance margin is 0 µs; safety margin is 500 µs. Record all per-session FPS/H/G/margin/seed values.
- Preserve immutable manifests outside checkout with role, exact source SHA/tree, source root, branch lineage, Rust/Cargo identity, build flags/features, target, workload and observer versions, binary SHA-256 and manifest/checksum verification.

Files/functions to update: `build_dispatch_core_harness.ps1` profile table, source identity and manifest writer; `dispatch_harness_manifest.psm1` a new exact-role validator and integrity handoff; `qualify_dispatch_core.ps1` profile constants, detached candidate-root checks, exact M2 pin comparison, stage/margin rules, pair identity and reporting provenance; `run_native_acceptance_case.ps1` profile identity and manifest checks; `dispatch_qualification_report.psm1` labels; `test_qualify_dispatch_core_contracts.ps1` positive/negative contracts; and `docs/playback-verification.md` runbook. Keep the existing two profiles' behavior unchanged.

Positive contracts: exact detached baseline and exact detached M2 candidate pass with their matching role, base, branch lineage, tree, clean state, binary/checksum, `dist`, feature, observer, workload, margin and target identities. Negative contracts: wrong branch lineage; wrong base SHA or tree; candidate revision/tree differing from accepted M2 (including a docs-only newer HEAD); dirty source; wrong role; bad binary/manifest/checksum hash; wrong features/profile/target/observer/workload; branch not containing pinned M2; current runner HEAD substituted for the candidate; and evidence output root inside checkout. Retain all existing FrameSnapshot/Scheduler positive and negative contracts, plus explicit tests that the #661 identities do not pass the old FrameSnapshot validator.

## Native plan and session budget

No baseline/candidate manifest or binary identity is available from the current route. The current host reports Windows 11 Home build 26300 x64, PowerShell 7.6.10 and Rust/Cargo 1.99.1 MSVC; this does not overcome the identity failure. No native run is authorized by invalid provenance.

After the separate profile is approved and validated, the runbook's minimum candidate-only Canary is 1 quiet `canonical-single` session at 60 FPS / 500 µs. Smoke is up to 22 quiet safety sessions and stops on the first failure; it is not a performance comparison. For a later Compare decision, proposed scope is 72 sessions: `dense-alternating`, `mixed-up-down`, and `canonical-max-chord` × Quiet/Contention × six AB/BA pairs × two roles, 15 seconds each, at 60 FPS and 0 µs performance margin; safety uses 500 µs. This proposal needs explicit Leader approval before Compare. Do not start Final (166 sessions).

## Validation and regression status

The added test verifies the exact executor packet sequence, G/H from synthetic sender completion, one B chord affected by cursor HoL, independent Up C progress, the split between cursor wait and sender/post-send occupancy, and no data that could support native performance statistics.

| Invariant | Evidence | Result |
|---|---|---|
| Down cursor preserves authored chord order; independent musical Up stays eligible | `m3_offline_fixture_separates_down_cursor_hol_from_sender_occupancy`; executor cursor tests | PASS; fixture sequence and deadline assertions pass |
| H/G derive from sender completion | Same fixture; core and player floor/lifecycle tests | PASS; exact G and H completion-derived times asserted |
| Up precedes Down, masks disjoint, packet ≤ 15 inputs, one musical sender attempt | Core and Win32 packet/tracked sender suites | PASS; core 77, Win32 276 passed / 1 ignored |
| No retry/replay, focus/PID/generation/modifier/control/lease gates, focus grace/cleanup + G, terminal Up obligations | Player worker/focus/lifecycle and Win32 cleanup suites | PASS; player library 254 passed, Win32 cleanup/packet tests included |
| Invalid QPC evidence excluded from healthy timing distributions; transport journal survives failures | Player worker metrics/failure and Win32 tracked sender tests | PASS; player library 254 passed |
| Healthy realtime path remains allocation-free | `rt_dispatch_no_alloc` | PASS; 4 passed |
| Existing qualification profile identities remain protected; no old identity qualifies #661 | Immutable-manifest qualification contract tests | PASS for existing contracts; #661 identity remains incompatible and untested pending new profile approval |

Offline fixture breakdown uses one sample per row and does not support percentiles:

| Measurement | n | Observed synthetic value | Native evidence |
|---|---:|---:|---|
| Down B cursor-only HoL until A G eligibility | 1 | 2,180 µs | unavailable |
| A sender pre-call to completion | 1 | 13 µs | unavailable |
| A completion to worker-free (post-send) | 1 | 5 µs | unavailable |
| Advisory sequential delay for Down B | 1 | 2,198 µs | unavailable |

No native startup-to-Ready, timer-arm/return, deadline-miss, authored/eligibility-to-pre-call, sender envelope, CPU/spin, SendInput attempts/insertion, observer-loss, H/G, cleanup, or obligation distribution was collected. The fixture ends with all synthetic executor slots released; this is not native cleanup evidence. There were zero native SendInput attempts because no acceptance binary or sink session ran.

Validation on the checkpoint source:

| Command | Result |
|---|---|
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core` | exit 0; 75 unit + 2 property tests passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --features test-support` | exit 0; 276 passed, 1 ignored |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib` | exit 0; 254 passed |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | exit 0; 4 passed |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | exit 0 |
| `pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1` | exit 0; qualification/manifests contracts PASS |
| `pwsh -NoProfile -File scripts/test_evidence_contracts.ps1` | exit 0; evidence contracts PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | exit 0 |
| `git diff --check`; `git diff --cached --check` | exit 0; staged check covers both checkpoint files |
| `cargo xtask check all` | exit 0; static/security, workspace Rust, Bun, website E2E and desktop gates passed |
| `cargo xtask check static` | exit 0 after the final evidence summary edit |

The first targeted invocation used `-- --exact` without the module-qualified test name; it exited 0 but selected 0 tests. The corrected substring-filtered invocation selected and passed the new test (1 passed). No test failure occurred.

Required CI: to be checked on the pushed checkpoint HEAD.
