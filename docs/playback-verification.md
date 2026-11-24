# Playback Verification

This is the current Windows runbook. It records how to verify the checked-out
source and how to build qualification manifests. The current behavior contract
is [playback-engine.md](playback-engine.md); security rules are in
[SECURITY.md](../SECURITY.md). Browser tests and PlanOnly runs are not native
timing or gameplay evidence.

Production dispatch executes pure **Control A** for all Down, Up, and mixed
actions with `ProcessQosPolicy` pinned to **Default** and a frozen pre-spin
window. Experimental B/C process QoS and runtime adaptive spin are not deployed.
Physical SendInput tests require interactive foreground authorization; in
unattended CI and automated runs, physical SendInput remains **NOT RUN**.

## Local Windows gates

Run from the repository root. Record the exact command, exit code, test count,
ignored or skipped tests, and any environmental limitation.

    cargo xtask check rust
    cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc -- --test-threads=1
    cargo test --locked --manifest-path rust/xtask/Cargo.toml audits::security::tests -- --test-threads=1
    cargo check --locked --manifest-path rust/Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance
    cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib
    cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- --test-threads=1
    cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- -D warnings
    cargo xtask check static --skip-supply-chain
    pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
    git diff --check

Run **cargo xtask check all** on the final source when the pinned supply-chain
tooling is available. Report its exact result. **check static --skip-supply-chain**
does not satisfy or waive the supply-chain gate.

For changed frontend or website sources, run the owning Bun checks from that
application directory. For shell lifecycle changes, also run the
**sky_desktop_shell** all-features test above. The relevant tests cover terminal
races, sender anomalies, session fencing, join and retirement ordering,
foreground recovery, bounded cleanup, quarantine, and watcher fallback.
Report focused command output rather than treating a browser mock as a native
sender test.

The current report projection uses report schema version 2 for
**DispatchCleanupM3**, **DispatchSessionRedesignM3**, **PlaybackCoreLifecycle**,
and **PlaybackCoreLifecycleV2** because these profiles share the projection
that omits the retired post-cleanup delay fields. Other profiles retain their
existing report schema version. This report version is separate from each
harness binary manifest schema; historical v2/v3 evidence readers and the
explicit lifecycle trace schema v4 remain unchanged.

Run the desktop check from the desktop package and the website checks from
the website package:

    Push-Location apps/desktop
    bun run check
    Pop-Location
    Push-Location apps/website
    bun run check
    bun run lint
    bun run format:check
    bun run test:unit
    Pop-Location

## PlaybackCoreLifecycle qualification profiles

The checked-in **PlaybackCoreLifecycle** profile in
[scripts/dispatch_qualification_profiles.psm1](../scripts/dispatch_qualification_profiles.psm1)
is the immutable v1 historical record. It is retained with its original
baseline and workload identity; its lockfile differs from the candidate and
its old paired measurements must not be described as a source-only delta.

- Baseline source:
  **2544daa13eb23fdf03bd922e65b96fcffaba421c**, tree
  **155882714df131ddc9b9985133a1f0611736b7cd**.
- Candidate source and tree: the exact clean, pushed candidate commit and its
  Git tree. Rebuild its manifest after the last relevant source or
  documentation change; a manifest from an earlier SHA is not reusable.
- Toolchain: Rust and Cargo 1.99.1, Windows MSVC, target
  **x86_64-pc-windows-msvc**, profile **dist**, feature
  **real-input-acceptance**, and locked dependencies.
- Manifest: records source and tree identities, toolchain, target, profile,
  features, Cargo.lock hash, binary SHA-256, runner provenance, and workload.
  Keep build outputs, manifests, and qualification evidence outside the
  checkout.

The v1 profile preserves six alternating AB/BA pairs, each with 15-second samples,
under Quiet and Contention. Workloads are **mixed-up-down**,
**dense-alternating**, **canonical-max-chord**, **near-minimum-retrigger**, and
**long-idle**. Performance statistics are p50, p95, p99, and max. Record
eligible-to-pre-call, wait-to-pre-call, pre-call-to-completion,
Stop-to-worker-verified-release, worker CPU, worker spin, raw-Up syscall count,
and worker-joined-to-next-playback-Start. Safety scenarios are focus loss, owner
mismatch, owner process termination, pause/resume, suspend/resume, Stop
cleanup, Skip cleanup, and supervisor lease expiry. Timing and safety margins
are both 500 microseconds.

Build exact baseline and candidate manifests from detached, clean source trees
with **scripts/build_dispatch_core_harness.ps1**, using profile
**PlaybackCoreLifecycle**, roles **Baseline** and **Candidate**, and separate
output roots outside the checkouts. The baseline is immutable. The candidate
manifest must bind the exact candidate SHA and tree for the run; a manifest
built from an earlier revision is not reusable.

Use the same pinned builder for each role:

    pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 -QualificationProfile PlaybackCoreLifecycle -SourceRole Baseline -SourceRoot <baseline-worktree> -OutputRoot <baseline-output>
    pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 -QualificationProfile PlaybackCoreLifecycle -SourceRole Candidate -SourceRoot <candidate-worktree> -OutputRoot <candidate-output>

Run the contract checks, then validate and render the final plan without
starting a native case:

    pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
    pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 -Stage Final -QualificationProfile PlaybackCoreLifecycle -CandidateManifest <candidate-manifest> -ControlManifest <baseline-manifest> -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 -PlanOnly -OutputRoot <evidence-root>

Run the qualification contract tests and PlanOnly dry checks with
**scripts/test_qualify_dispatch_core_contracts.ps1** and
**scripts/qualify_dispatch_core.ps1**. PlanOnly verifies identities and workload
configuration and reports zero native attempts. It does not execute the sink or
send input.

The runnable Final protocol keeps six alternating AB/BA pairs for each of the
five workloads in both Quiet and Contention, followed by the eight candidate
safety scenarios. It requires the explicit
**-OwnerApprovedReceiveOnlyRun** switch. Use that switch only after the
repository owner has approved the exact Windows machine, the project-owned
receiver, and the physical input scope. The runner rejects a missing or
misidentified receiver, including any target marked as a game. Do not run this
command as part of routine source verification.

    pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 -Stage Final -QualificationProfile PlaybackCoreLifecycle -CandidateManifest <candidate-manifest> -ControlManifest <baseline-manifest> -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 -OwnerApprovedReceiveOnlyRun -OutputRoot <evidence-root>

### PlaybackCoreLifecycleV2: strict same-lock qualification

Use **PlaybackCoreLifecycleV2** for current A/B qualification. It preserves v1
and pins the baseline to the clean PR-base commit **1d00c3b71fbf1f94003865bad6c4eba6d8dcd902**,
tree **25a20291b9cf84b1283804f451b33e3c4605507c**. Candidate and baseline
must have identical Cargo.lock bytes and hash; the runner checks both manifests
against the exact clean source trees and pushed candidate HEAD. Rust/Cargo,
MSVC target, profile, features, observer, loads, pair order, and sample duration
remain the same as v1.
The historical v1 profile retains its original candidate lineage
`refactor/playback-core-lifecycle`; v2 alone follows this PR branch.

The v2 paired workload contract checks the entire authored stream. The
candidate harness preserves the PR-base cadence for **dense-alternating** and
**near-minimum-retrigger**, and uses the same baseline cycle-count formula for
all five paired workloads. Frozen SHA-256 projections from the exact PR-base
source cover all five streams, including every cycle and tail. In particular,
`canonical-max-chord` keeps the baseline's 263 cycles; candidate-only trailing
padding cannot add a 264th cycle. `short-up-gap-stress` and
`pause-resume-active-cleanup` are ninth and tenth, candidate-only safety cases.
The latter pauses with a key active, requires verified cleanup before Resume,
and checks cleanup Up precedes the next same-key Down without an added pause.
For every paired run, the runner normalizes baseline
`authored_packet_targets` and candidate `authored_timestamp_group_targets` into
`(timestamp_us, kind, scan-mask)` events, then requires matching full-stream
and measured 15-second prefix hashes and counts. Tail event/group counts and
hashes remain in each row for diagnosis. A prefix or tail mismatch stops the
paired run before those measurements can be reported as comparable.

Build exact v2 manifests from detached, clean source trees. Keep outputs outside
the checkouts and rebuild the candidate manifest after every source or runner
change:

    pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 -QualificationProfile PlaybackCoreLifecycleV2 -SourceRole Baseline -SourceRoot <baseline-worktree-at-1d00c3b> -OutputRoot <baseline-output>
    pwsh -NoProfile -File scripts/build_dispatch_core_harness.ps1 -QualificationProfile PlaybackCoreLifecycleV2 -SourceRole Candidate -SourceRoot <candidate-worktree-at-pushed-head> -OutputRoot <candidate-output>

Check the profile contracts and validate the plan without starting a native
case:

    pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
    pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 -Stage Final -QualificationProfile PlaybackCoreLifecycleV2 -CandidateManifest <candidate-manifest> -ControlManifest <baseline-manifest> -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 -PlanOnly -OutputRoot <evidence-root>

`PlanOnly` verifies identity, same-lock and the v2 profile matrix, including
the candidate-only short-gap case. It does not execute the sink, build native
event projections, or send input; full-stream equality is asserted on each
paired run before its measurement is accepted. The runnable native protocol and
**-OwnerApprovedReceiveOnlyRun** approval requirements above also apply to v2.

### PlaybackCoreLifecycleV3: cookie-patched successor baseline

V3 is a separate requalification profile for the `cookie 0.18.3` lock update.
V1 and V2 manifests and evidence remain pinned to their original source trees;
V3 rejects them by profile name and schema. Its harness manifest schema is 3,
while the shared report projection remains schema 2 because its report fields
and lifecycle semantics are unchanged.

The V3 baseline successor is **4bc4a8409d3250da012be6e92f6681e721c17f3a**,
tree **8e2ce7c115458d95b9e13aa144b3d6cc16c083a4**. It has the exact V2 anchor
**1d00c3b71fbf1f94003865bad6c4eba6d8dcd902** as parent and changes only
`rust/Cargo.lock`, pinning `cookie 0.18.3`. Pin this commit at
`refs/heads/qualification/playback-core-lifecycle-v3-baseline`; the V3 runner
checks that the remote ref still points to the pinned SHA. V3 stores the
lineage anchor and successor SHA/tree/parent/lock hash separately. The PR candidate must descend
from the immutable V2 anchor, remain a sibling of the baseline successor, and
match the successor's Cargo.lock bytes. The runner rejects a successor with a
different parent/tree/patch, dirty source, mismatched lock hash, or candidate
that is not the exact pushed PR branch tip.

On Windows, create both V3 worktrees with LF checkout settings. From a clean
clone that has fetched the candidate branch, set the repository checkout
policy before adding either worktree:

```powershell
git -C <clone> config core.autocrlf false
git -C <clone> config core.eol lf
git -c core.autocrlf=false -c core.eol=lf -C <clone> worktree add --track -b refactor/playback-remove-release-gap <candidate-worktree> origin/refactor/playback-remove-release-gap
git -c core.autocrlf=false -c core.eol=lf -C <clone> worktree add --detach <baseline-worktree> 4bc4a8409d3250da012be6e92f6681e721c17f3a
```

Before building either manifest, verify the lockfile's byte hash in both
worktrees:

```powershell
$expectedLockSha256 = 'baaf6e57419a1f28d9a880d655a48863676fe2fe0ddcd9a1467153912af13b68'
foreach ($worktree in @('<candidate-worktree>', '<baseline-worktree>')) {
    $autocrlf = (git -C $worktree config --get core.autocrlf).Trim()
    $eol = (git -C $worktree config --get core.eol).Trim()
    if ($autocrlf -ne 'false' -or $eol -ne 'lf') { throw "Worktree ${worktree} must use core.autocrlf=false and core.eol=lf" }
    $actualLockSha256 = (Get-FileHash -LiteralPath (Join-Path $worktree 'rust/Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualLockSha256 -ne $expectedLockSha256) { throw "Unexpected Cargo.lock bytes in ${worktree}: $actualLockSha256" }
}
```

If a worktree was checked out with CRLF, restore only its working-tree copy
from the index under LF settings, then repeat the hash check:

```powershell
git -c core.autocrlf=false -c core.eol=lf -C <worktree> checkout-index --force -- rust/Cargo.lock
```

This does not change the pinned lock blob or source SHA/tree. The V3 builder
and manifest reader also reject CR bytes and any lock hash other than the
pinned LF-byte hash before accepting the checkout.

V3 keeps the V2 workload matrix, including 120 paired sessions and 10
candidate-only safety sessions (130 planned total), with the same target,
toolchain, `dist` profile, `real-input-acceptance` feature, authored-stream
parity contract, 500 us margins, observer, pair order, and 60 fps workload.
Native qualification remains subject to the same owner-approved ReceiveOnly
machine and input-scope requirements. A V3 `PlanOnly` run plans 130 sessions and records zero native attempts; it is not native gameplay evidence.

After an authorized run, **lifecycle-metrics.json** records candidate source,
tree, manifest and binary hashes with machine/boot and receiver identity.
Stop-to-release latency ends at a worker-side QPC sample taken after terminal
release verification, probe, and journal reconciliation. Worker join, a clean
journal, and validated ReceiveOnly Up evidence qualify the sample, but later
telemetry extraction and receiver event-log settling do not extend its
duration. The separate post-join report/receiver-settling QPC is diagnostic.

Worker-joined-to-next-Start uses the immediately following timed native case's
worker QPC observation after its Ready-to-Armed publication. The collector
binds both native session run IDs, scenario/source/manifest/binary identities,
machine and boot identity, and matching Windows QPC frequency. A parent-side
QPC sampled before launching the child process is retained only as
`worker_joined_to_runner_pre_next_process_launch_us` diagnostic data; it cannot
complete the playback Start metric. Missing successor Armed evidence, clock
mismatch, or identity mismatch leaves the required metric **INCOMPLETE**.

Immediate-successor coverage is versioned with the qualification profile. Keep
the v1 semantics for its historical baseline at
[2544daa13eb23fdf03bd922e65b96fcffaba421c](https://github.com/pumni/Sky-Auto-Player/blob/2544daa13eb23fdf03bd922e65b96fcffaba421c/rust/crates/sky_player/src/bin/rt_native_acceptance.rs):
that source emits no `lifecycle_measurement` or worker Armed QPC evidence. Its
control rows must preserve that schema; candidate-only lifecycle fields on a
v1 baseline row are invalid evidence. V1 measures candidate→candidate and keeps
candidate→baseline unavailable, so its required metric remains incomplete.

V2 pins baseline source
[1d00c3b71fbf1f94003865bad6c4eba6d8dcd902](https://github.com/pumni/Sky-Auto-Player/blob/1d00c3b71fbf1f94003865bad6c4eba6d8dcd902/rust/crates/sky_player/src/bin/rt_native_acceptance.rs),
which emits schema-2 worker Armed QPC evidence. V2 requires all 60 adjacent
candidate worker-join→successor Armed intervals: 30 candidate→candidate and
30 candidate→baseline. For each interval, validate frozen AB/BA adjacency,
source/tree/manifest/binary identities, row status and exit code, native run
ID/scenario, the collector's machine/boot identity, Windows QPC source and
matching frequency, and successor Armed QPC ≥ candidate worker-join QPC.
Missing or invalid evidence makes both the Start metric and overall report
`INCOMPLETE` with a reason. Do not overlay the baseline binary or fabricate
lifecycle evidence.

In each six-pair cell, the actual AB/BA role order produces three adjacent
candidate→candidate transitions and three candidate→baseline transitions:

| Profile | Transition | Planned across 10 workload/load cells | Eligible | Available from worker QPC |
| --- | --- | ---: | ---: | ---: |
| V1 | Candidate → candidate | 30 | 30 | 30 when identity and QPC evidence validate |
| V1 | Candidate → baseline | 30 | 0 | 0; pinned v1 baseline has no Armed field |
| V2 | Candidate → candidate | 30 | 30 | 30 when identity and QPC evidence validate |
| V2 | Candidate → baseline | 30 | 30 | 30 when baseline identity and Armed QPC evidence validate |

Under V1, a Final run therefore measures at most 30 of 60 planned
immediate-successor transitions. It reports the other 30 as unavailable with
the pinned-baseline schema reason, and the mandatory
`worker_joined_to_next_start_us` metric stays **INCOMPLETE**. The partial
candidate→candidate statistics remain visible but do not make that required
metric complete. The child-process launch diagnostic does not replace missing
playback Start evidence. V2 uses the pinned baseline's existing Armed evidence
to measure all 60; it must fail-stop as incomplete if any required sample is
missing or invalid. Keep these profile results separate because v1 cannot be
backfilled with fields that its source did not emit.

## Pinned Stage 1 Prepare capacity comparison

This comparison records Stage 1 evidence from the pinned pre-Stage 2 candidate.
Its second Prepare argument described a cleanup delay at that revision. Stage
2 removes that argument; the current contract requires verified cleanup and
admission checks without a cleanup-time floor.

The PR #679 memory fixture measures 10,000 one-key Down generations with the
same authored actions, QPC frequency (1 MHz), H (8,000 µs), and second Prepare
policy argument (16,667 µs) on the pinned pre-refactor baseline
`1d00c3b71fbf1f94003865bad6c4eba6d8dcd902` and the candidate. The baseline
interprets that argument as the release-gap minimum; the pinned Stage 1
candidate uses it for its separate resume cleanup delay. Each Down is 30,000
µs after the previous Down and its Up is 10,000 µs later, leaving a 20,000 µs
Up-to-next-Down gap so the legacy baseline can Prepare this workload.
The comparison reports calculated collection capacities for the compile phase
and for the schedule plus prepared plan while both coexist:

| Source | Compiler phase peak | Prepared plan capacity | Prepare overlap peak bound |
| --- | ---: | ---: | ---: |
| Pinned baseline `1d00c3b` | 8,160,774 bytes | 880,264 bytes | 8,160,774 bytes |
| PR #679 candidate | 6,720,718 bytes | 960,264 bytes | 6,720,718 bytes |

The modeled Prepare peak falls by 1,440,056 bytes (17.6%) for this fixture;
the candidate plan itself is 80,000 bytes larger. These values use Rust
collection capacities and the code's compile/Prepare lifetime model, with zero
additional caller-retained capacity in the fixture. They do not measure
allocator metadata, fragmentation, worker/application allocations, or process
RSS, and should not be described as the whole application's memory peak. The
focused measurement command on the candidate is:

    $env:SKY_MEMORY_CHORD_COUNT = '10000'
    cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core executor_memory_scaling_100_1000_10000_and_budget_proof -- --nocapture

To reproduce the baseline arm, use a separate clean worktree at
`1d00c3b71fbf1f94003865bad6c4eba6d8dcd902`. Its checked-in memory fixture has
20,000 µs Down cadence, so temporarily change that test's cadence multiplier to
`30_000` and add the same prepared-plan probe used by the candidate immediately
after its `dispatch_memory` output:

    drop(executor);
    let prepared = PreparedDispatchSchedule::prepare(
        &sched,
        std::num::NonZeroU64::new(1_000_000).expect("QPC frequency"),
        8_000,
        16_667,
        compile_memory.caller_retained_capacity_bytes,
    ).expect("prepared dispatch plan");
    let memory = prepared.memory_metrics();
    eprintln!("prepared_dispatch_memory chords={chord_count} compiler_phase_peak_capacity_bytes={} prepared_plan_capacity_bytes={} prepare_overlapping_peak_capacity_bytes={}", compile_memory.phase_peak_capacity_bytes, memory.plan_capacity_bytes, memory.overlapping_peak_capacity_bytes);

Then run the same `SKY_MEMORY_CHORD_COUNT=10000` cargo test command with
`--manifest-path (Join-Path $baseline 'rust/Cargo.toml')`. Restore only
`rust/crates/sky_dispatch_core/src/executor/memory_tests.rs` in that disposable
worktree after measurement. This edits test source only; keep the separately
built pinned-baseline qualification harness and manifest untouched.

## Current offline lifecycle evidence

The current `dispatch_lifecycle` evidence format is schema v4. The Rust example
[`dispatch_lifecycle_evidence.rs`](../rust/crates/sky_player/examples/dispatch_lifecycle_evidence.rs)
uses the live scheduler with deterministic mock senders and records zero native
SendInput calls. It checks that an Up transport commit permits the next
same-key Down at its authored target, then checks that Pause completes verified
cleanup, sends nothing while paused, and permits a fresh authored-due Down after
the Resume control commits. Its timing object records FPS, frame interval, hold
frames, Timing Margin, H, and QPC frequency; it has no cleanup-delay or
resume-barrier fields.

The evidence contract accepts v4 as current. Schema v2 and v3 readers remain
available for their pinned historical reports and retain their original
release-gap and cleanup-delay rules. The current telemetry key fixture is schema
27; schema 26 remains pinned as history. Check both evidence contracts with:

    pwsh -NoProfile -File scripts/test_evidence_contracts.ps1

These mock and contract checks do not run native input. Native ReceiveOnly and
gameplay evidence remain subject to the authorization and runbook above.

Raw Up syscall count sums
`TerminalSendAttemptEvidence.raw_calls` for the worker terminal cleanup. It
does not use `ReleaseAllOutcome.attempts`, which counts admitted cleanup
attempts before the sender call. A withheld attempt or a pre-send failure adds
zero raw calls; unavailable raw evidence remains missing instead of becoming
zero. Raw counts retain their stop-cleanup run IDs. Any unavailable mandatory
metric marks the lifecycle report **INCOMPLETE**; PlanOnly does not produce
native lifecycle measurements.

Native ReceiveOnly requires a later owner decision and an explicitly controlled,
safe receiver. Do not substitute gameplay for the ReceiveOnly harness.
PlanOnly, mocks, builds, and desktop tests are not gameplay acceptance.

Record native results only for work actually run. Until separately authorized
and performed, report:

    Native ReceiveOnly: NOT RUN
    Live Sky gameplay: NOT RUN

## Evidence interpretation

Keep sender timing separate from game sampling. **SendInput** completion does
not prove the game registered a note. Report missing or inconclusive measurements,
raw counts, skipped tests, and provenance gaps. Do not claim acceptance when a
required test, manifest identity, or native measurement is absent.
