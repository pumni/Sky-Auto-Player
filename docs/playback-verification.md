# Playback Verification and Qualification

Current playback behavior is defined in [playback-engine.md](playback-engine.md).
This document is the sole qualification runbook. Source and tests at the checked
out SHA establish implementation behavior; archived run evidence is revision-bound.

## Native ReceiveOnly qualification

The native acceptance harness targets a ReceiveOnly WinForms window hosted in an
independent `pwsh.exe` process by `scripts/native_acceptance_sink.ps1`. The sink
identity binds HWND, PID, process start time, and executable path. The interactive
desktop must be unlocked and the exact sink window foreground. It records input
and never forwards input to a game. Headless or identity-incomplete runs are
INCONCLUSIVE, not PASS.

Scheduler qualification uses the immutable baseline and candidate manifests with
`-QualificationProfile Scheduler`. Both must identify the V3 Compat3 observer,
schema 3, and pass observer, target, foreground, sequence, queue, and loss
validation. Baseline source identity is pinned; candidate source identity must
match the clean checked-out head. Build manifests bind role, source SHA/tree,
toolchain, target, profile, features, workload version, and binary hash. Keep
all generated evidence under an `OutputRoot` outside the checkout.

The opt-in `FrameSnapshot` profile uses workload `frame-snapshot-v1`, the same
Scheduler V3 Compat3 observer, and the pinned baseline
`8a800ff07a5dc362a5ee3aaa59204ff40b6ae327` / tree
`48eec424c155e98fa1a81ad018bc832b6e6b964f`. Its candidate must be a clean
descendant on `refactor/frame-snapshot-dispatch`. The role-specific manifests
also bind source and binary hashes, the exact source identity, target, and both
margins. Canary/Smoke and candidate safety use a 500 µs margin; Compare/Final
performance uses margin 0 for both roles. Existing Scheduler and DispatchCore
profile semantics are unchanged.

## DispatchCleanupM3 runner-contract profile

`DispatchCleanupM3` is a separate profile for issue #661. Its workload identity
is `dispatch-runtime-cleanup-m3-v1`; its native observer remains Scheduler V3
Compat3 with schema 3. The profile does not change the `FrameSnapshot`,
`Scheduler`, or `DispatchCore` pins or validation rules.

Build the baseline and candidate from clean detached worktrees at these exact
source identities:

| Role | Commit | Tree |
|---|---|---|
| Baseline | `2277ac34a21a57a8d5ec313bba55ad78fc60794f` | `9a8637129dbb7a05be4f9386da188413b45f9774` |
| Candidate, accepted M2 | `74c2b2618af50881f15eacd27ab722d316954743` | `902c772442706b326e020db6d36daedd60632b02` |

The candidate commit must be an ancestor of the pushed
`refactor/dispatch-runtime-cleanup` branch. The clean runner checkout must match
that branch tip. The runner commit/tree and captured runner script hashes are
recorded separately from the candidate source identity. A later runner HEAD is
never substituted for the accepted M2 candidate source. The detached worktrees,
runner checkout, and external evidence output must retain their actual Git
identity; do not put a branch name into the detached source identity.

M3 manifests require Rust and Cargo 1.99.1, the Windows MSVC host and
`x86_64-pc-windows-msvc` target, optimized `dist`, exactly the
`real-input-acceptance` feature, and no `qpc-read-counting`. Performance margin
is 0 µs and safety margin is 500 µs. Cargo target output, binaries, manifests,
runner provenance copies, checksums, and logs all live outside the checkouts.
The checksum inventory covers the manifest, binary, build scripts/logs, and
captured M3 runner source files. Per-session reports capture FPS, margin, the
configured H/G floors, and workload version. These workload definitions accept
no PRNG seed; reports record a null seed and that deterministic seed mode.

M3 per-session H/G reporting follows the native acceptance scenario: the
ordinary hold uses `ACCEPTANCE_HOLD_FRAMES = 1`; `hol-interleaved` explicitly
uses two hold frames; G always uses one frame. A frame is
`ceil(1,000,000 / FPS)` µs and the selected margin is added to both floors.
At 60 FPS / 500 µs, `canonical-single` therefore reports H/G 17,167/17,167 µs.
The report validator checks run-config H/G against its scenario, FPS, and
margin before including a session row. Canary is one candidate-only Quiet
`canonical-single` safety session with no timed performance sample. Smoke is a
candidate-only Quiet safety sequence. Compare reports its AB/BA performance
pairs. Final includes paired performance sessions, candidate-only safety
sessions, and 60/120 FPS boundary sessions; its aggregate H/G, FPS, and margin
are explicitly `varies by session`, with actual values retained per row in
`sessions.csv`.

From the clean, pushed runner checkout, add two detached worktrees outside the
runner directory and use new empty output directories outside both worktrees:

```powershell
git worktree add --detach <baseline-worktree> 2277ac34a21a57a8d5ec313bba55ad78fc60794f
git worktree add --detach <candidate-worktree> 74c2b2618af50881f15eacd27ab722d316954743
pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 `
  -QualificationProfile DispatchCleanupM3 -SourceRole Baseline `
  -SourceRoot <baseline-worktree> -OutputRoot <baseline-output>
pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 `
  -QualificationProfile DispatchCleanupM3 -SourceRole Candidate `
  -SourceRoot <candidate-worktree> -OutputRoot <candidate-output>
```

Each build verifies the remote runner tip, accepted M2 lineage, detached source
SHA/tree, toolchain and output locations before emitting its immutable manifest.
Verify both with `Assert-DispatchCleanupM3ManifestIdentity` and
`Assert-ImmutableHarnessManifestIntegrity` before requesting any native session.

Offline manifest builds and integrity checks do not start the acceptance
runner. The candidate-only Quiet `canonical-single` Canary at 60 FPS and 500 µs
safety margin was separately authorized and run once. Its reporting correction
is documented append-only in
[`docs/evidence/issue-661-m3/canary-reporting-erratum.md`](evidence/issue-661-m3/canary-reporting-erratum.md).
The reporting-correction checkpoint authorizes zero further native sessions;
Smoke, Compare, and Final require separate Leader decisions.

## Issue #666 M4 waiter-only guard A/B

The opt-in offline A/B uses the production Windows `HybridWaiter` and the
current startup-calibration selector from one clean candidate checkout and one
`sky_player` test binary. Arm A is an **experimental fixed 1,000 µs** guard;
Arm B uses the current calibrated selector/fallback. Both pay the same six
sample calibration probe. Each Quiet and Contention cell has one warm-up per
arm and six paired runs, alternating AB/BA order. Every run waits on 900
60-FPS deadlines (15 seconds at 16,667 µs frame intervals). Percentiles use
nearest rank `sorted[ceil(p*n)-1]`.

Use a new output path outside the checkout. The wrapper refuses a dirty source
tree, a different branch, or an existing/in-repository output directory; it
records the exact source SHA/tree, Windows/CPU/toolchain, lockfile hash, test
binary hash, test command/exit code, raw files, and result JSON:

```powershell
pwsh -NoProfile -File scripts/run_issue666_m4_guard_ab_offline.ps1 `
  -OutputRoot "$env:TEMP\issue-666-m4-ab-<candidate-short-sha>"
```

The output contains `wait-samples.csv`, `wait-runs.csv`, `result.json`,
`cargo-test.log`, and `run-manifest.json`. It reports wake-to-deadline QPC
lateness, return-to-observation gap (explicitly **not** sender pre-call),
waiter spin, worker/process CPU, and contention-worker CPU. It has no executor,
focus/owner gate, sender, packet, H/G dispatch floor, cleanup, or Prepare/Arm
timing, so those measures remain null/not measured. It neither calls
`SendInput` nor qualifies game delivery. Do not describe this same-revision
guard experiment as an exact-base-versus-candidate A/B; the M1
baseline-equivalent structural benchmark is a separate evidence class.

Native ReceiveOnly and optional gameplay planning is in
[`issue-666-m4-native-qualification.json`](evidence/issue-666-m4-native-qualification.json).
It is planning only and explicitly awaits separate Leader authorization; no
native input, interactive focus, real suspend/resume, or Sky gameplay session
is implied by running the offline waiter experiment.

## Final workload

The Final stage runs these five performance workloads:

`dense-alternating`, `mixed-up-down`, `canonical-max-chord`,
`near-minimum-retrigger`, and `long-idle`.

Each workload runs Quiet and Contention cells, with six alternating AB/BA pairs
per cell and both Baseline and Candidate roles in every pair: 5 × 2 × 6 × 2 =
120 sessions. Each session is 15 seconds at 60 FPS. Scheduler profile sessions
use 500 µs; FrameSnapshot performance sessions use 0 µs on both roles.

After performance, the runner executes all 22 native safety scenarios under
Quiet and Contention (44 cases), then the fixed 60 FPS / margin 0 and 120 FPS /
margin 500 boundary cases. FrameSnapshot safety scenarios use the candidate at
500 µs; including the two boundary cases, its Final stage is 166 native
sessions. Do not run Compare and Final sequentially by default: obtain a
separate approved session budget and one-stage plan before native qualification.
The existing fail-stop sequence stops on the first failed native case. Do not
retry failures to complete a matrix. Smoke, Compare, the six AB/BA pairs, metric
filters, thresholds, and drain timeout keep their defined behavior.

## Issue #666 M3 session ownership checks

The Windows `test-support` suite covers target-generation authority lifetime,
fresh retained process-creation proof at the musical gate, owner disappearance
through watchdog wake and tracked cleanup, callback-epoch rundown, presentation
stall isolation, and bounded join/retirement. Run it with the M3 command set in
[`issue-666-m3.md`](evidence/issue-666-m3.md). These deterministic mock tests
do not call real `SendInput`, suspend Windows, or qualify gameplay reception.
The M3 evidence report maps each lifecycle invariant to its test and states
which paths still need OS/native qualification.

## Issue #666 integration correction checkpoint

The integrated correctness corrections are recorded in
[`issue-666-integration.md`](evidence/issue-666-integration.md). The regression
suite exercises musical Up origin matching at the production worker boundary,
the desktop supervisor's typed stale-poll consumer, and the narrow proven
Down-only raw-zero continuation case. These are deterministic Windows tests
with mock packet senders. They do not call real `SendInput`, manipulate the
interactive foreground, perform system suspend/resume, or qualify gameplay.

The raw-zero exception is valid only when the sender evidence proves one actual
Down-only attempt with exact inserted count zero, consistent status/masks,
ordered QPC evidence, and unchanged target/control state. It consumes that
attempt without a replay, safety Up, or synthetic H/G completion. Prior
confirmed H/G floors remain in force. Mixed/Up packets and incomplete, short,
or contradictory evidence keep the conservative cleanup path and its
owner-origin policy. Every anomaly remains a terminal error in the final
outcome, even when safe unattempted notes continue.

Snapshot full/lite parity and schema-25 key-set tests remain separate from the
playback timing and native qualification profiles. Snapshot compatibility does
not increase the healthy dispatch allocation budget. The operation counts in
the evidence are test-hook counts for the actual worker/harness boundary;
sender QPC and `GetProcessTimes` counts are separately identified from
production source because the mock does not call those native APIs.

## Offline lifecycle and manifest checks

The current offline scheduler lifecycle report is produced and validated with:

```powershell
pwsh -NoProfile -File scripts/run_dispatch_lifecycle_evidence.ps1
```

It runs the schema-v2 probe through the live `DispatchExecutor` with deterministic
test-support senders, validates generation accounting, captured musical packets,
cleanup Up captures, and the completion + G resume barrier, then writes the report
outside the checkout. It performs no native `SendInput` calls and does not qualify
foreground delivery or gameplay reception. The P0-P5 publication helper is
historical metadata only; it is not a producer or validator for this report.

For an offline FrameSnapshot binary/manifest build, first use a detached worktree
at the pinned baseline and the clean candidate checkout, each with a new empty
output directory outside both checkouts:

```powershell
pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 `
  -QualificationProfile FrameSnapshot -SourceRole Baseline `
  -SourceRoot <detached-pinned-baseline> -OutputRoot <new-baseline-output>
pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 `
  -QualificationProfile FrameSnapshot -SourceRole Candidate `
  -SourceRoot <clean-candidate-checkout> -OutputRoot <new-candidate-output>
```

The build emits the binary and immutable manifest but does not execute the
acceptance harness or send keys. Verify each manifest offline with
`Assert-FrameSnapshotManifestIdentity` for its role and
`Assert-ImmutableHarnessManifestIntegrity` before any later approved native
qualification. Physical qualification uses `qualify_dispatch_core.ps1` and its
`run_native_acceptance_case.ps1` route only after a separately approved session
budget; building or validating a manifest is not a native test.

## Verdict and evidence

A cell is complete only with six valid pairs and complete provenance. Report
p50/p95/p99/max and available authored-to-pre-call, eligibility-to-pre-call,
deadline-crossing, pre-call-to-completion, CPU, spin, missing-sample, H/G,
chord, SendInput, final key-state, release-obligation, and target-identity
evidence. Apply the runner's existing tail regression review prompts without
converting them into automatic PASS.

Distinguish FAIL from INCONCLUSIVE. Unsafe Down, healthy-path chord split,
musical H/G violation, unresolved cleanup obligation, invalid provenance, or
missing required coverage prevents PASS. Missing observer samples remain
missing; they are not reconstructed. The 22-case suite is described in the
scenario inventory and case contracts in
[`scripts/qualify_dispatch_core.ps1`](../scripts/qualify_dispatch_core.ps1) and
[`scripts/test_qualify_dispatch_core_contracts.ps1`](../scripts/test_qualify_dispatch_core_contracts.ps1).

The Rust no-allocation check is separate from native sink qualification:

```powershell
cargo test --locked --manifest-path rust/Cargo.toml `
  -p sky_player --features test-support --test rt_dispatch_no_alloc
```

The required CI gate must pass on the exact source head before native Final
qualification. A Final sink PASS does not establish gameplay correctness.
Gameplay validation is a separate operator-run check with the actual game and
song, and must record expected/observed onsets and repeated missing notes. A
fully inserted `SendInput` count alone cannot establish that the game sampled a
note.
