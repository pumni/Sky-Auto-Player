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

## DispatchSessionRedesignM3 preflight (native runs on hold)

`DispatchSessionRedesignM3` reuses the immutable harness builder, manifest
validator, per-case runner, observer reader, and paired report path. It leaves
the historical `DispatchCleanupM3` pins unchanged. The baseline is
`d0f65e7c412041a6d502f81f469872198db7adc5` / tree
`19aae9e41bc667a2f991001ed105b6ffdebb8bc6`. The candidate source is the exact
clean HEAD of `refactor/dispatch-session-redesign`; the runner checkout must be
that same pushed branch tip. Both roles use Rust/Cargo 1.99.1, MSVC host and
target `x86_64-pc-windows-msvc`, `dist`, exactly `real-input-acceptance`,
`Cargo.lock` with `--locked`, Frozen waiter behavior, 500 µs performance and
safety margins, the V3 Compat3 ReceiveOnly observer, timer and event waits,
`PriorityMode::Auto`, ring telemetry capacity 1,024, and the same 60 FPS mixed
workload. The two generated manifests record Cargo.lock path/hash; Final rejects
different lock hashes. Neither `qpc-read-counting` nor a separate observer-off
run is part of this plan. Timing-observer on/off remains recorded per
invocation; this profile uses observer-on for both roles.

Use a Windows 11 interactive host with an unlocked desktop, the verified sink
foreground, and the exact pinned toolchain. Before any native session, confirm
that OS and target details identify the approved machine. Put detached
worktrees, Cargo targets, manifests, binaries, logs, and evidence under a new
directory outside the checkout. From a clean, pushed branch tip, prepare the
source checkouts and immutable harnesses:

```powershell
$m3Root = Join-Path $env:TEMP 'sky-auto-player-dispatch-session-redesign-m3'
$baselineSource = Join-Path $m3Root 'baseline-source'
$candidateSource = Join-Path $m3Root 'candidate-source'
$baselineOutput = Join-Path $m3Root 'baseline-harness'
$candidateOutput = Join-Path $m3Root 'candidate-harness'
git worktree add --detach $baselineSource d0f65e7c412041a6d502f81f469872198db7adc5
$candidateRevision = (git rev-parse HEAD).Trim()
git worktree add --detach $candidateSource $candidateRevision
pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 `
  -QualificationProfile DispatchSessionRedesignM3 -SourceRole Baseline `
  -SourceRoot $baselineSource -OutputRoot $baselineOutput
pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 `
  -QualificationProfile DispatchSessionRedesignM3 -SourceRole Candidate `
  -SourceRoot $candidateSource -OutputRoot $candidateOutput
```

Before requesting authorization, verify both `manifest.json` files with
`Assert-DispatchSessionRedesignM3ManifestIdentity`,
`Assert-DispatchSessionRedesignM3RunnerProvenance`, and
`Assert-ImmutableHarnessManifestIntegrity`; compare their Cargo.lock hashes and
record each manifest and binary SHA-256. The builder performs immutable checks
as part of each build. Candidate source must equal the current pushed branch
HEAD; baseline and candidate source trees must be clean detached worktrees.

The executable plan is one guarded `Final` stage. Its mapping is:

| Order | Scenario / workload | Load | FPS | Margin | Sample | Role | Sessions |
|---|---|---|---:|---:|---:|---|---:|
| 1 | `canonical-single` / `canonical-single` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 2–6 | `focus-loss`, `stop-cleanup`, `pause-resume`, `owner-process-termination`, `target-hwnd-change` / `safety-smoke` | Quiet | 60 | 500 µs | untimed | Candidate | 5 |
| 7 | `near-minimum-retrigger` / `near-minimum-retrigger` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 8 | `near-minimum-retrigger` / `near-minimum-retrigger` | Quiet | 120 | 500 µs | untimed | Candidate | 1 |
| 9–32 | `mixed-up-down` / `mixed-up-down` | Quiet, Contention | 60 | 500 µs | 15 s | Baseline + Candidate | 24 |

Within each performance cell, six pairs alternate AB then BA; both roles finish
before a pair is counted. The runner completes the canary first, runs safety
cases sequentially and stops at the first non-PASS result, then begins paired
performance only if all preceding candidate cases pass. The cap is 32 sessions
(20 Candidate, 12 Baseline). A failure, inconclusive result, wrong-target
attempt, timing-floor violation, chord/order anomaly, unknown release, or
nonzero residual release obligation stops the matrix immediately. Do not retry
a failed case or continue to another phase.

The command below is **approval-gated**. Do not run it until the Leader has
approved the exact mapping and both validated manifests on the PR. That command
starts the first ReceiveOnly canary and can start native sessions:

```powershell
pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 `
  -Stage Final -QualificationProfile DispatchSessionRedesignM3 `
  -CandidateManifest (Join-Path $candidateOutput 'manifest.json') `
  -ControlManifest (Join-Path $baselineOutput 'manifest.json') `
  -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 `
  -OutputRoot (Join-Path $m3Root 'native-evidence')
```

Timing uses QPC values and preserves availability/provenance. `D` is the
authored physical target, `E` the final physical not-before point after H/G,
`W` the waiter return/handoff when present, `S` the sender pre-call sample, and
`C` the SendInput completion sample. Existing trace metrics provide `E−D`
(`floor_displacement`), `S−E` (`residual_lateness`), `S−W`
(`waiter_to_pre_call`), and `C−S` (`sender_envelope`); timer arm/return and
deadline-crossing samples remain separate. The report retains p50/p95/p99/max,
raw and short SendInput outcomes, H/G, chord/order and cleanup counters, final
release obligations, worker/process CPU, spin sample availability, observer
loss, QPC frequency, missing-QPC counts, timer provenance and unknown values.
Missing W/QPC or observer loss is counted and is not imputed as zero. Stop to
verified release is represented by the Stop cleanup case and its final release
obligation and sink reconciliation evidence.

The startup regression was caused by the test fixture omitting foreground
authority setup while production correctly rechecked foreground before
`ArmedPublished`. The fixture now locks the foreground test seam, sets HWND/PID
123/77 explicitly, and resets it with scoped cleanup; production focus checks
are unchanged. The Windows-specific mock and ReceiveOnly suites do not establish
real Sky gameplay reception, and this preparation has run no native sessions.

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

The final musical permit also has deterministic authority-mutex contention
coverage. A hook pauses after fresh foreground/PID/process verification; a
supervisor test thread acquires the retained-authority mutex before allowing
the worker to enter the permit. Down and Up return `Replan` without a packet,
cursor advance, or obligation clear, then send once after fresh restaging.
Concurrent target publication and same-HWND ABA keep the old Up origin
fail-closed. Verified terminal cleanup continues to use its serialized path.
These tests exercise the production worker/harness with mock senders and a
test-only lock seam; they do not call real `SendInput`.

## Issue #666 post-integration replan timing correction

The deterministic `sky_player` dispatch tests exercise the actual scheduler,
staging, final admission, waiter, mock sender, and journal commit helpers. The
permit-contention regression also runs the same outer replan/wait policy used
by the worker loop. It holds the retained authority mutex only after fresh
foreground, PID, and process-continuity admission, then verifies the immediate
retry and interruptible 1 ms backoff. A separate H/G trace checks floor minus
one tick, exact floor, and plus one tick, including future-deadline projection
and wake selection. These tests use a mock sender and do not call native
`SendInput`, change interactive focus, suspend the system, or measure game
reception. The trace timings are individual QPC observations from deterministic
tests, not latency distributions or gameplay qualification. See
[`issue-666-replan-timing.md`](evidence/issue-666-replan-timing.md) for the
baseline/candidate operation-count comparison, race trace, and verification
ledger.

## Issue #666 mid-playback preflight and code-quality correction

The production worker regressions keep a validated A-session preflight stamp
across early owner-proof Busy and final writer/owner-authority Busy while a
second authored Down is pending. The probe reports A physically held after the
first Down, so any accidental all-keys rescan would fail. The tests assert no
second sender call or journal/cursor mutation during Busy, then verify one B
attempt, the retained A-origin proof, and A's authored Up. Separate controls
cover same-HWND generation ABA, focus recovery, and lease expiry. H/G tick-edge
checks use supplied QPC values; a separate real waiter smoke test covers a
future deadline wake. These are mock-sender tests and do not establish Sky
gameplay reception. See
[`issue-666-preflight-code-quality.md`](evidence/issue-666-preflight-code-quality.md)
for the exact cases, warning cleanup, and verification ledger.

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
