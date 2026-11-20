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

## PlaybackCoreLifecycle qualification profile

The checked-in **PlaybackCoreLifecycle** profile in
[scripts/dispatch_qualification_profiles.psm1](../scripts/dispatch_qualification_profiles.psm1)
is the sole current profile for this lifecycle refactor.

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

The profile preserves six alternating AB/BA pairs, each with 15-second samples,
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

The pinned baseline's native acceptance source at
[2544daa13eb23fdf03bd922e65b96fcffaba421c](https://github.com/pumni/Sky-Auto-Player/blob/2544daa13eb23fdf03bd922e65b96fcffaba421c/rust/crates/sky_player/src/bin/rt_native_acceptance.rs)
does not emit `lifecycle_measurement` or worker Armed QPC evidence. Control
fixtures must preserve that schema; candidate-only lifecycle fields on a
baseline row are invalid evidence. In each six-pair cell, the actual AB/BA role
order produces three adjacent candidate→candidate transitions and three
candidate→baseline transitions:

| Transition | Planned across 10 workload/load cells | Eligible | Available from worker QPC |
| --- | ---: | ---: | ---: |
| Candidate → candidate | 30 | 30 | 30 when identity and QPC evidence validate |
| Candidate → baseline | 30 | 0 | 0; the pinned baseline has no Armed field |

The Final run therefore measures at most 30 of 60 planned immediate-successor
transitions. It reports the other 30 as unavailable with the pinned-baseline
schema reason, and the mandatory `worker_joined_to_next_start_us` metric stays
**INCOMPLETE**. The partial candidate→candidate statistics remain visible but
do not make that required metric complete. The child-process launch diagnostic
does not replace missing playback Start evidence. A Leader/owner protocol
decision is required to accept candidate→candidate-only coverage, or to approve
a separate protocol that can observe baseline Start while preserving the
baseline's immutable source identity. Do not overlay the baseline binary or
fabricate lifecycle evidence for it.

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
