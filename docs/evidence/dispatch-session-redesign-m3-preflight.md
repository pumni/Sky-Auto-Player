# Dispatch Session Redesign M3 preflight

Status: Leader authorized preparation/instrumentation only. Native ReceiveOnly
and SendInput sessions remain on hold until the Leader approves the exact
mapping and both manifests on PR #668.

## Windows CI correction

The two failing Windows CI runs stopped in
`engine::tests::startup_admission::target_invalidated_before_scheduler_staging_finishes_without_input`
while waiting for `ArmedPublished`. A diagnostic snapshot identified terminal
trigger `focus_authority_invalidated_during_preparation` before the checkpoint.
The test used HWND 123/PID 77 and `require_focus=true` but omitted the foreground
test lock and explicit foreground HWND/PID setup. The production worker performs
a fresh foreground check before publishing Armed, as intended.

The fixture now takes `lock_foreground_window_for_test()`, installs scoped
cleanup which resets foreground window and owner state, and explicitly sets the
foreground HWND/PID to 123/77 before binding and arming. The diagnostic timeout
still reports the startup snapshot and terminal trigger. No production focus
check, admission rule, timeout, or checkpoint was changed.

The default workspace test build also exposed Desktop test fixtures that used
the existing `tauri-test` feature APIs without matching feature gates. The
control fixture now has a feature-neutral `player: None` construction; the
Tauri mock-app and callback-pause tests remain enabled under `tauri-test`.
These are test-only boundaries and do not alter production playback behavior.

Verification performed on the corrected fixture:

```text
cargo test --locked -p sky_player --all-features target_invalidated_before_scheduler_staging_finishes_without_input -- --nocapture
PASS: 1 test

for ($iteration = 1; $iteration -le 20; $iteration++) { Write-Host "[repeat $iteration/20]"; cargo test --locked -p sky_player --all-features target_invalidated_before_scheduler_staging_finishes_without_input --quiet; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } }
PASS: 20/20 repetitions

cargo xtask check rust
PASS: Windows Rust workspace fmt, locked all-target/all-feature Clippy with -D warnings, and workspace all-features tests
```

The final approval handoff records the exact candidate SHA and repeats the
Windows workspace command at that committed HEAD before any native execution.

## Baseline, candidate, build, and artifact paths

The immutable baseline is `d0f65e7c412041a6d502f81f469872198db7adc5`, tree
`19aae9e41bc667a2f991001ed105b6ffdebb8bc6`. Candidate source is the exact
pushed HEAD of `refactor/dispatch-session-redesign`, descendant of that
baseline. The runner and its provenance scripts are from that same pushed HEAD.
Historical `DispatchCleanupM3` source pins and artifacts are unchanged.

Both builds use Rust/Cargo 1.99.1, host/target `x86_64-pc-windows-msvc`, Cargo
profile `dist`, `--locked`, exactly the `real-input-acceptance` feature,
`SchedulerQueuedV3Compat3` observer schema/event schema 3, frozen waiter mode,
and 500 µs performance/safety margins. `qpc-read-counting` is disabled. Each
immutable manifest records source/runner identities, Cargo.lock path and
SHA-256, Cargo target path, build command, feature, observer identity, binary
hash, runner script provenance, and external artifact paths. The `Final` gate
rejects manifests whose Cargo.lock hashes differ.

From a clean pushed candidate checkout, use one new directory outside the
repository for worktrees and immutable build outputs:

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

Validate the two manifests and immutable artifact inventories before requesting
native-run approval:

```powershell
Import-Module scripts/dispatch_qualification_profiles.psm1 -Force
Import-Module scripts/dispatch_harness_manifest.psm1 -Force
$runnerIdentity = Get-DispatchSessionRedesignM3RunnerIdentity -RunnerRoot (Get-Location).Path
foreach ($manifestPath in @(
  (Join-Path $baselineOutput 'manifest.json'),
  (Join-Path $candidateOutput 'manifest.json')
)) {
  $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
  Assert-DispatchSessionRedesignM3ManifestIdentity -Manifest $manifest `
    -ExpectedRole $manifest.source_role -RepositoryRoot (Get-Location).Path | Out-Null
  Assert-DispatchSessionRedesignM3RunnerProvenance -Manifest $manifest `
    -RunnerIdentity $runnerIdentity -RunnerRoot (Get-Location).Path | Out-Null
  Assert-ImmutableHarnessManifestIntegrity -Manifest $manifest `
    -ManifestPath $manifestPath | Out-Null
}
```

Retain the output paths, manifest hashes, binary hashes, Cargo.lock hashes,
build logs, and `SHA256SUMS.txt` outside the checkout. Do not edit or regenerate
either manifest after Leader review; any source or runner change requires a new
candidate commit and new exact manifests.

## Prepared native case map

Only `DispatchSessionRedesignM3 -Stage Final` is accepted by the session
orchestrator. One command owns all phases and fail-stops between them:

| Order | Scenario | Workload | Load | FPS | Margin | Duration | Role | Count |
|---|---|---|---|---:|---:|---:|---|---:|
| 1 | `canonical-single` | `canonical-single` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 2 | `focus-loss` | `safety-smoke` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 3 | `stop-cleanup` | `safety-smoke` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 4 | `pause-resume` | `safety-smoke` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 5 | `owner-process-termination` | `safety-smoke` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 6 | `target-hwnd-change` | `safety-smoke` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 7 | `near-minimum-retrigger` | `near-minimum-retrigger` | Quiet | 60 | 500 µs | untimed | Candidate | 1 |
| 8 | `near-minimum-retrigger` | `near-minimum-retrigger` | Quiet | 120 | 500 µs | untimed | Candidate | 1 |
| 9–20 | `mixed-up-down` | `mixed-up-down` | Quiet | 60 | 500 µs | 15 s | Baseline + Candidate | 12 |
| 21–32 | `mixed-up-down` | `mixed-up-down` | Contention | 60 | 500 µs | 15 s | Baseline + Candidate | 12 |

Performance cells each contain six pairs, alternating AB/BA. The limit is one
canary, seven candidate safety sessions, and 24 paired sessions: 32 total, 20
Candidate and 12 Baseline. The runner stops on the first non-PASS case; a
failure or inconclusive result does not trigger a retry. Wrong-target attempts,
duplicate Down, chord split, H/G violations, unknown or unverified release,
residual release obligations, report/provenance mismatch, missing required
observer evidence, or sink identity/focus loss stop the run. Do not resume the
matrix after a stop.

The approval-gated command, after and only after Leader approval of the exact
mapping and both manifests, is:

```powershell
pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 `
  -Stage Final -QualificationProfile DispatchSessionRedesignM3 `
  -CandidateManifest (Join-Path $candidateOutput 'manifest.json') `
  -ControlManifest (Join-Path $baselineOutput 'manifest.json') `
  -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 `
  -OutputRoot (Join-Path $m3Root 'native-evidence')
```

This command starts real ReceiveOnly sessions. It has not been invoked during
preparation.

## Timing and evidence contract

The existing native trace and PowerShell report are reused; no timing or
safety-path optimization was inferred or applied. `D` is the authored physical
target in the QPC domain. `E` is the final `physical_not_before_qpc` after
H/G and other eligibility floors. `W` is the waiter-return/handoff QPC when
available. `S` is `pre_call_qpc`; `C` is `sendinput_completion_qpc`. The
existing QPC-derived metrics expose `E−D` as `floor_displacement`, `S−E` as
`residual_lateness`, `S−W` as `waiter_to_pre_call`, and `C−S` as
`sender_envelope`. The timer arm target, timer return, deadline-crossing QPC,
and wake QPC remain separately available with provenance and generation
matching; they are not substituted for a missing W sample.

The report keeps p50/p95/p99/max and sample counts; raw-zero, partial and
completed SendInput attempts; realized and configured H/G; chord/order,
duplicate, missing and split counters; cleanup and final release obligations;
worker/process CPU and spin availability; observer loss; QPC frequency and
missing-QPC counters; and unknown or unavailable values. It does not convert a
missing timestamp/release into zero or a successful verdict. The timing
observer is on for both roles to match the V3 Compat3 sink contract, and each
case records the observer setting. Observer-off comparison is not in this
32-session plan.

The fixed floors are one frame plus 500 µs: at 60 FPS, H=G=17,167 µs; at 120
FPS, H=G=8,834 µs. Both roles use the same frozen waiter policy, timer/event
wait configuration, `PriorityMode::Auto`, game sampling settings, and default
pre-roll behavior from the acceptance harness. Quiet has no contention worker;
Contention uses the existing two hidden `pwsh.exe` workers at BelowNormal
priority. The harness reports CPU/spin values and their availability.

## Environment and evidence limits

Initial Windows checks ran on host `x86_64-pc-windows-msvc`, Rust/Cargo 1.99.1.
The local runtime reports `Microsoft Windows 10.0.26300` while system product
metadata reports Windows 10 Home; this does not independently establish the
required Windows 11 identity. Resolve and record the approved Windows 11 host
identity before any native run. No SendInput, ReceiveOnly, Sky gameplay, focus
canary, or performance session has run as part of this preparation. Mock Rust
tests validate logic and fixtures only; they do not qualify live Windows
input, hardware timing, or Sky reception.
