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
