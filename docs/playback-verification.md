# Playback Verification

This is the current Windows runbook. It records how to verify the checked-out
source and how to build qualification manifests. The current behavior contract
is [playback-engine.md](playback-engine.md); security rules are in
[SECURITY.md](../SECURITY.md). Browser tests and PlanOnly runs are not native
timing or gameplay evidence.

Production dispatch executes pure **Control A** for all Down, Up, and mixed
actions over the onset-only chord plan with completion-anchored releases
(`release_not_before = completion_qpc + H_ticks`), `ProcessQosPolicy` pinned to
**Default**, and a frozen pre-spin window. Experimental B/C process QoS and
runtime adaptive spin are not deployed. Physical SendInput tests require
interactive foreground authorization; in unattended CI and automated runs,
physical SendInput remains **NOT RUN**.

In `rt-native-acceptance`, isolated safety/cleanup fixtures (`Hold`,
`CleanupFullRelease`, `StopCleanup`, `SkipCleanup`,
`PauseResumeActiveCleanup`, `SuspendResume`, `ModifierHeldAfterOwned`) use
explicit scenario-local fixed long `H` values recorded in the report
(`scenario_hold_us`, `scenario_uses_isolated_long_hold`) so control triggers
reach actively held keys without altering production defaults. Fixed-pulse
candidate workloads carry distinct identities
(`mixed-up-down-fixed-pulse-v1`, `canonical-max-chord-fixed-pulse-v1`) and do
not claim identity with immutable historical 50 ms / 40 ms variable-hold
baselines. Historical V1/V2/V3 qualification profiles remain immutable and
branch-pinned.

## Evidence reachability

A test of a helper or hand-constructed state proves only that isolated behavior.
For claims about production scheduling, admission, dispatch, or retirement,
exercise the corresponding production entry path with reachable inputs and
controlled effects, or explicitly bound the claim. Prefer the existing
`ProductionDispatchTestHarness`, fault injection, and independent golden or
reference invariants. A passing synthetic state is not proof of a reachable
production transition. Native timing and physical-input claims still require
the separate qualification below.

## Local Windows gates

Run from the repository root. Record the exact command, exit code, test count,
ignored or skipped tests, and any environmental limitation.

    node scripts/toolchain-pin.mjs exec -- cargo xtask check rust
    node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc -- --test-threads=1
    node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path xtask/Cargo.toml audits::security::tests -- --test-threads=1
    node scripts/toolchain-pin.mjs exec -- cargo check --locked --manifest-path Cargo.toml -p sky_player -p sky_dispatch_win32 --all-targets --features sky_player/real-input-acceptance
    cargo check --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib
    node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- --test-threads=1
    node scripts/toolchain-pin.mjs exec -- cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --lib --all-features -- -D warnings
    node scripts/toolchain-pin.mjs exec -- cargo xtask check static --skip-supply-chain
    pwsh -NoProfile -File scripts/test_qualify_dispatch_core_contracts.ps1
    git diff --check

For final merge readiness, run **node scripts/toolchain-pin.mjs exec -- cargo xtask check all**
on final relevant source. If pinned supply-chain tooling is unavailable, report the blocker
rather than qualifying a partial check. **check static --skip-supply-chain**
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
    cargo test --locked --manifest-path Cargo.toml -p sky_dispatch_core executor_memory_scaling_100_1000_10000_and_budget_proof -- --nocapture

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
[`dispatch_lifecycle_evidence.rs`](../crates/sky_player/examples/dispatch_lifecycle_evidence.rs)
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

### CompletionAnchoredPulseV1 nonphysical plan

Generate the nonphysical qualification plan from a clean checkout matching the pushed `origin/refactor/completion-anchored-pulse-engine` tip with:

    Import-Module ./scripts/dispatch_qualification_profiles.psm1 -Force
    New-CompletionAnchoredPulseV1NonphysicalPlan -RepositoryRoot . -PlanOnly -ParityEvidence $parityEvidence

Limitations of `-PlanOnly`:

- `native_build_status` is `NOT BUILT`, `native_manifest_status` is `NOT VERIFIED`, and `binary_sha256` is `NOT_BUILT` (no harness build or execution occurs).
- `receiver_started` and `harness_started` are `false` with `no_native_invocation = true`.
- Paired session eligibility (72 sessions: 3 workloads x 2 loads x 6 AB/BA pairs x 2 arms) is conditional on **caller-supplied** authored-stream parity evidence (`-ParityEvidence`) for `dense-alternating`, `near-minimum-retrigger` and `long-idle`. The plan does not execute the parity test; it only binds the supplied records to the identities it verified itself (below). Unbound, malformed, stale, mismatched or duplicated workloads are excluded with an explicit per-workload `reason`; unknown or null entries are listed in `parity_evidence_rejections`. Status values are `EVIDENCE_BOUND_ALL_PAIRED_WORKLOADS`, `PARTIAL_EVIDENCE_BOUND` and `UNVERIFIED_NO_PARITY_EVIDENCE_SUPPLIED`; the plan records `parity_evidence_origin = caller_supplied_test_output` and `parity_executed_by_plan = false`.
- `mixed-up-down` and `canonical-max-chord` are excluded from paired performance comparison and retained as candidate-only fixed-pulse geometry checks.
- Safety rows (`safety_scenarios`, 16) carry the effective hold from `scenarios.rs` (`17167` us acceptance floor; `450000` for `pause-resume-active-cleanup` and `suspend-resume`; `9950000` for `stop-cleanup`, `skip-cleanup` and `cleanup-full-release`; `100000` for `modifier-held-after-owned`), the trigger, obligation at trigger, expected mask, verdict contract, `evidence_class` (`planned native`, `controlled mock` or `conditional`) and the Rust source reference. `owner-mismatch`, `owner-process-termination`, `supervisor-lease-expiry`, `target-hwnd-change`, `modifier-held-final-boundary` and `preflight-user-held` are pre-Down fail-closed cases with zero sink events and a zero obligation. `suspend-resume` injects a synthetic suspend notification only: it proves terminal cleanup of the held key and blocked Down, not cleanup before a real OS suspend completes, and exercises no resume phase. Ordinary `pause-resume` pauses between notes (zero obligation) and is distinct from `pause-resume-active-cleanup`. `modifier-held-*` use a test-support modifier seam, not physical input; the modifier is never journal-owned. `ambiguous-packet` is `Inconclusive` unless the mixed-packet injection is reached. `scripts/test_qualify_dispatch_core_contracts.ps1` parses `scenarios.rs`, `generation_stress.rs`, `owner_identity.rs` and `modifier_guard.rs` and fails when a row's hold, pre-Down status or contract token drifts. These are plans for a later owner-authorized ReceiveOnly run, not results.

#### Parity evidence object

One object per paired workload, flat properties, all required:

| Property | Contract |
| --- | --- |
| `workload` | one of the three paired workloads; duplicates exclude that workload |
| `evidence_origin` | nonempty string naming the producer, e.g. `caller-supplied-test-output` |
| `game_fps`, `timing_margin_us`, `configured_H_us`, `sample_seconds` | integers equal to `60`, `500`, `17167`, `15` |
| `baseline_revision`, `candidate_revision` | 40 lowercase hex; must equal the commit the plan verified (profile baseline; clean pushed candidate tip) |
| `baseline_tree`, `candidate_tree` | 40 lowercase hex; must equal the verified trees |
| `baseline_lock_sha256`, `candidate_lock_sha256` | 64 lowercase hex; must equal the verified Cargo.lock hashes |
| `baseline_verified`, `candidate_verified` | boolean `true` (the authored-stream test passed in that tree) |
| `<role>_event_count`, `<role>_group_count` | integers >= 1, group <= event |
| `<role>_first_us`, `<role>_last_us` | integers >= 0, last >= first |
| `<role>_sha256` | 64 lowercase hex |

### ChordWidthPilotV1 nonphysical preflight and offline analyzer

The bounded chord-width pilot (`ChordWidthPilotV1`, workload `chord-width-pilot-v1`) prepares a quiet-only three-session ReceiveOnly characterization across chord widths `1`, `5`, and `15` at `60` FPS, `500` us timing margin, `H = 17167` us, `100000` us cadence, `150` onsets (`15` s sample), Control A admission, fixed `1000 us` production spin threshold (startup wake calibration removed), and the qualification observer enabled.

- **Observer protocol**: Uses the existing `SchedulerQueuedV3Compat3` (`scheduler-receiveonly-queue-v3-compat3`) protocol with event schema `3`, QPC timestamps (`observed_qpc_ticks`), ready schema `3`, summary schema `1`, and queue capacity `32768`. Historical V1/V2/V3/M3 profiles remain untouched.
- **Preflight entry point**: `scripts/prepare_chord_width_pilot_v1.ps1` enforces a clean checkout (tracked and untracked) on `refactor/completion-anchored-pulse-engine` descending from `e7293877a6ce58a387cb8edfe0a88399a85d2b15`, matching `origin/refactor/completion-anchored-pulse-engine`, frozen LF `rust/Cargo.lock` (`9b9ba6c03131d96029421bfa9f6f69f9ce331e003e32e095eac54a714e73021e`), pinned `1.99.1` toolchain, external non-reparse `OutputRoot` (and fresh nonexistent external `target` dir for `-BuildHarness` with post-build HEAD/tree/remote-tip/lock recheck), PE `MZ` header, and full manifest/script hashes. In `-Mode PreflightOnly`, it runs `scripts/run_native_acceptance_case.ps1 -QualificationProfile ChordWidthPilotV1 ... -PreflightOnly` for every planned session without launching a receiver, harness, or `SendInput`. Direct case-runner entry enforces the same frozen lock, remote tip, and script-hash provenance checks.
- **Provenance boundary**: `-BuildHarness` records `build_provenance = fresh_pinned_dist_build` (`binary_provenance_verified_by_fresh_build = true`) with exact command and exit code. Reusing `-HarnessManifestPath` verifies schema, PE header, and hashes only, recording `build_provenance = reused_manifest_hash_verified_only_not_rebuilt` (`binary_provenance_verified_by_fresh_build = false`, `authorized_to_execute = false`).
- **Offline analyzer**: `node scripts/analyze_chord_reception.mjs --report <rt-native-acceptance.jsonl> --invocation <invocation.json> --sink-ready <sink-ready.json> --sink-events <sink-events.jsonl> --observer-summary <sink-observer-summary.json> --manifest <manifest.json> --width <1|5|15>` consumes the actual Rust JSONL report (`status: "PASS"`, flat `details`) and case `invocation.json`, enforces lossless 64-bit FILETIME identity matching, literal alternating `Down(i)/Up(i)` geometry with exact checked-ceil authored offsets (`50000` us + `100000` us cadence), exact same-domain epoch consistency, completion-anchored hold floor `S_up >= C_down + ceil(H_ticks)`, exact epoch-relative `UpSlotTrace.up_not_before_offset_ticks == (C_down + ceil(H_ticks)) - epoch`, and cross-thread `R + 1 >= S`; any invalid run returns `valid = false`, `accepted_for_aggregation = false`, `metrics = null`, and exit code `1`.
- **Execution entry gate**: `scripts/prepare_chord_width_pilot_v1.ps1 -Mode Execute` remains unconditionally `LOCKED` (preparation only, never a second execution authority). `scripts/run_native_acceptance_case.ps1 -QualificationProfile ChordWidthPilotV1` (without `-PreflightOnly`) enforces a fail-closed entry gate before any directory, receiver, or harness creation, requiring explicit `-OwnerApprovedReceiveOnlyRun`, `-ApprovedPreflightPlanPath`, and all five independently supplied `-ExpectedApproved*` hashes (`SourceRevision`, `SourceTree`, `ManifestSha256`, `BinarySha256`, `PreflightPlanSha256`) bound to a `fresh_pinned_dist_build` plan with 3/3 passing `CASE_PREFLIGHT_ONLY PASS` results in width order `15 -> 1 -> 5`, plus immediate pre-launch rechecks. Hash agreement is integrity binding, not proof of an independently trusted authority. These branch-bound pilot scripts and analyzer are preserved as revision-specific reproducibility tools, not a new general native qualification workflow on `main`.

#### Exploratory ReceiveOnly results at `ffaec326`

Owner-supervised ReceiveOnly runs were executed at immutable source `ffaec3266d9f11d82db8ed86bfe2a8d6fda429bd` (tree `f1a29c1d3fa8fe55a12902e3972cf5891192475b`, frozen `rust/Cargo.lock` `9b9ba6c03131d96029421bfa9f6f69f9ce331e003e32e095eac54a714e73021e`, `rt-native-acceptance.exe` SHA-256 `0988195440d8e4fd360c93feed9943fe58748e452888d9972d98f319714c7915`, manifest `8e0af747b939c21298c8e897a039ef9be2dc33c0330b489aa90bfaadfd11eda8`, preflight plan `c04996d43104ef790a76fe27a81779891921cc6bc2ee2c09cf9b3672165cf802`) on one Windows 11 Home `10.0.26300` host (`AMD Ryzen 5 5500U`, `10 MHz` QPC, Balanced power plan, quiet load). Full per-session inventories and hashes live in PR #685 (`#issuecomment-7189883687`, `#issuecomment-7190327830`, `#issuecomment-7191033015`, `#issuecomment-7191460106`).

- **D1 (`width=15`, active IME, `native-case-20261126T174338-3d9c1bf7`)**: **FAIL** (`valid = false`, `metrics = null`, `3,362 / 4,500` events written before close, `1,138` unobserved with upstream location `UNKNOWN`). Raw `sink-events.jsonl` recorded `1,234` `VK_PROCESSKEY (0xE5)` letter `WM_KEYDOWN` events (`0` on punctuation/Up); censored diagnostic `down_skew` `p50` was `111.07 ms` with `+43.25 ms/chord` lag accumulation. Preserved as `FAIL`.
- **D2 (`width=15`, `native-case-20261126T183202-ac825e90`, ENG US, pre-send receiver-thread HKL `NOT RECORDED`, post-run `0` `0xE5` and `4500/4500` US VK)** and **D3 (`width=1` `native-case-20261126T185602-a05915ee`, `width=5` `native-case-20261126T185738-0ae84ae7`, both pre-send receiver-thread HKL `0x04090409`)**: exploratory **PASS** (`valid = true`, `total_invalid_count = 0`, `6,300 / 6,300` events, `900 / 900` packets, `3,150 / 3,150` `UpSlotTrace`, zero drops, clean retirement):

| Run / Width | Sender `S - D` `p50 / max` (µs) | Down `C - S` `p50 / max` (µs) | `down_skew` `p50 / max` (µs) | `common_chord_hold` `min / p50` (µs) | `per_key_hold` `min / p50` (µs) | `< 1/60 s` chords | Model 60 Hz `P_full_state` / `P_same_sample_down` mean |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| D3-1 / `1` | `28.8 / 264.5` | `347.6 / 723.5` | `N/A (0)` | `13,498.3 / 17,520.0` | `13,498.3 / 17,520.0` | `2 / 150` | `0.9985` / `0.9985` |
| D3-2 / `5` | `30.1 / 207.7` | `1,435.5 / 2,338.1` | `261.4 / 2,626.9` | `15,278.2 / 18,271.8` | `16,222.4 / 18,607.1` | `2 / 150` | `0.9992` / `0.9791` |
| D2 / `15` | `29.5 / 457.2` | `3,391.4 / 4,545.1` | `2,393.9 / 5,091.2` | `14,839.5 / 18,249.3` | `17,271.8 / 20,493.4` | `14 / 150` | `0.9970` / `0.8409` |

- **Limitations & scope**: These runs bind to `ffaec326` (not later documentation commits) and are exploratory one-run-per-width ReceiveOnly WndProc measurements on one host — not Sky gameplay acceptance, paired baseline comparison, or rare-tail qualification. The 60 Hz uniform-phase model uses each chord's raw WndProc intervals (`100 ms` cadence equals exactly 6 ideal 60 Hz frames, so phases correlate across a song and means are expectations rather than independent Bernoulli trials). The 72-session `CompletionAnchoredPulseV1` paired parity/native matrix and native safety scenarios remain **NOT REGENERATED / NOT RUN**.



Each role's digest and geometry must also equal the immutable expected stream in the profile (`ExpectedAuthoredStreams`, the digests asserted by `benchmark_schedule_tests.rs::paired_workloads_match_the_immutable_pr_base_authored_streams`). A missing field is never compared as equal.

Generate the evidence by running the existing no-send test in a clean checkout of each exact source tree, with the pinned toolchain:

For a current root-workspace candidate, use its root manifest:

    $workspaceManifest = 'Cargo.toml'
    node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path $workspaceManifest -p sky_player --features real-input-acceptance --bin rt-native-acceptance paired_workloads_match_the_immutable_pr_base_authored_streams

An immutable pre-root-workspace baseline uses `rust/Cargo.toml` only in that
revision's own checkout. Keep that historical path explicit and bind its use
to the recorded baseline revision; current source has no nested-workspace
fallback.

The test asserts the expected digest and geometry itself and sends no input; record the command, exit code, revision, tree and lock hash per role, and write the object file outside tracked source. Use `ConvertFrom-Json` on that file as `$parityEvidence`. Any new commit invalidates the candidate record; rerun it before reusing the evidence.

Record native results only for work actually run. Until separately authorized
and performed, report:

    Native ReceiveOnly: NOT RUN
    Live Sky gameplay: NOT RUN

## Evidence interpretation

Keep sender timing separate from game sampling. **SendInput** completion does
not prove the game registered a note. Report missing or inconclusive measurements,
raw counts, skipped tests, and provenance gaps. Do not claim acceptance when a
required test, manifest identity, or native measurement is absent.
