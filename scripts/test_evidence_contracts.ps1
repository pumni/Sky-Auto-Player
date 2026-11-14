[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib/evidence_runner_contract.ps1')
. (Join-Path $PSScriptRoot 'lib/evidence_publication_contract.ps1')
. (Join-Path $PSScriptRoot 'lib/dispatch_lifecycle_report_contract.ps1')
Import-Module (Join-Path $PSScriptRoot 'dispatch_pair_screen.psm1') -Force

function Assert-Contract([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "Evidence contract self-test failed: $Message" }
}

$runnerCases = @(
    @{ EvidenceMode = 'baseline'; RunnerStage = 'baseline'; Prefix = 'dispatch-baseline-'; NeedsControl = $false },
    @{ EvidenceMode = 'release-barrier'; RunnerStage = 'release-barrier'; Prefix = 'lifecycle-release-barrier-'; NeedsControl = $false },
    @{ EvidenceMode = 'lease-age'; RunnerStage = 'lease-age'; Prefix = 'lifecycle-lease-age-'; NeedsControl = $true },
    @{ EvidenceMode = 'diagnostics'; RunnerStage = 'diagnostics'; Prefix = 'dispatch-diagnostics-'; NeedsControl = $false },
    @{ EvidenceMode = 'final'; RunnerStage = 'final'; Prefix = 'lifecycle-qualification-'; NeedsControl = $true }
)
foreach ($case in $runnerCases) {
    $contract = Get-EvidenceRunnerContract -EvidenceMode $case.EvidenceMode -HasControlRevision $case.NeedsControl
    Assert-Contract ($contract.RunIdPrefix -ceq $case.Prefix) "$($case.EvidenceMode) run prefix changed"
    Assert-Contract ($contract.RunnerStage -ceq $case.RunnerStage) "$($case.EvidenceMode) runner stage changed"
    Assert-Contract ($contract.RequiresControlRevision -eq $case.NeedsControl) "$($case.EvidenceMode) control requirement changed"
}

$historicalV1 = [ordered]@{
    schema_version = 1
    source_revision = 'historical-source'
    transport_kind = 'deterministic-mock'
}
$historicalResult = Test-DispatchLifecycleReport -Report $historicalV1
Assert-Contract ($historicalResult.EvidenceClass -ceq 'historical') 'schema v1 reports must remain readable as historical evidence'

function New-LiveAccountingStage {
    param([string]$Phase, [long]$Activated, [long]$Released, [long]$Cancelled, [long]$Active)
    return [ordered]@{
        phase = $Phase
        generation_accounting_owner = 'live_scheduler'
        total = 2
        scheduled = 0
        activated = $Activated
        released = $Released
        cancelled = $Cancelled
        active = $Active
    }
}

$liveV2 = [ordered]@{
    schema_version = 2
    report_kind = 'dispatch_lifecycle'
    result = 'PASS'
    provenance = [ordered]@{
        source_sha = 'a' * 40
        source_tree_sha = 'b' * 40
        execution = 'offline_test_support'
        transport_kind = 'deterministic_mock'
        native_sendinput_calls = 0
    }
    timing = [ordered]@{
        fps = 60
        frame_us = 16667
        hold_frames = 1
        timing_margin_us = 500
        min_hold_us = 17167
        min_release_gap_us = 17167
        qpc_frequency_hz = 10000003
        min_release_gap_ticks = 171671
    }
    live_scheduler = [ordered]@{
        generation_accounting_owner = 'live_scheduler'
        no_pause_control = [ordered]@{
            pause_requested = $false
            sender_attempts = 4
            final_release_obligation_mask = 0
            passed = $true
            generation_accounting_owner = 'live_scheduler'
            accounting_stages = @(
                (New-LiveAccountingStage 'after_first_down' 1 0 0 1),
                (New-LiveAccountingStage 'after_first_up' 1 1 0 0),
                (New-LiveAccountingStage 'after_final_up' 2 2 0 0)
            )
        }
        pause_cleanup_resume = [ordered]@{
            pause_requested = $true
            sender_attempts = 2
            final_release_obligation_mask = 0
            passed = $true
            generation_accounting_owner = 'live_scheduler'
            accounting_stages = @(
                (New-LiveAccountingStage 'after_pause_cleanup' 1 0 1 0),
                (New-LiveAccountingStage 'after_resumed_down' 2 0 1 1),
                (New-LiveAccountingStage 'after_final_cleanup' 2 0 2 0)
            )
            cleanup_completion_qpc_ticks = 100
            min_release_gap_us = 17167
            min_release_gap_ticks = 171671
            resume_barrier_qpc_ticks = 171771
            candidate_packet_not_before_qpc_ticks = 171771
            g_minus_one_decision = 'wait'
            exact_g_decision = 'send'
        }
    }
}
$liveResult = Test-DispatchLifecycleReport -Report $liveV2
Assert-Contract ($liveResult.EvidenceClass -ceq 'live_scheduler') 'schema v2 must prove the live scheduler owner'

$missingV2 = $liveV2 | ConvertTo-Json -Depth 12 | ConvertFrom-Json -AsHashtable
$missingV2.Remove('timing')
$missingRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $missingV2) } catch { $missingRejected = $true }
Assert-Contract $missingRejected 'schema v2 accepted a report missing timing evidence'

$mixedOwnerV2 = $liveV2 | ConvertTo-Json -Depth 12 | ConvertFrom-Json -AsHashtable
$mixedOwnerV2.live_scheduler.pause_cleanup_resume.accounting_stages[1].generation_accounting_owner = 'legacy_fixture'
$mixedOwnerRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $mixedOwnerV2) } catch { $mixedOwnerRejected = $true }
Assert-Contract $mixedOwnerRejected 'schema v2 accepted mixed accounting owners'

$wrongBarrierV2 = $liveV2 | ConvertTo-Json -Depth 12 | ConvertFrom-Json -AsHashtable
$wrongBarrierV2.live_scheduler.pause_cleanup_resume.resume_barrier_qpc_ticks++
$wrongBarrierRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $wrongBarrierV2) } catch { $wrongBarrierRejected = $true }
Assert-Contract $wrongBarrierRejected 'schema v2 accepted a barrier other than cleanup completion + G'

$legacyStages = @(('phase' + '1'), ('phase' + '2'), ('phase' + '-1'), ('r' + '3-final'))
foreach ($legacyStage in $legacyStages) {
    $rejected = $false
    try { [void](Get-EvidenceRunnerContract -EvidenceMode $legacyStage -HasControlRevision $false) }
    catch { $rejected = $true }
    Assert-Contract $rejected "legacy or unknown stage '$legacyStage' was accepted"
}

foreach ($stage in @('lease-age', 'final')) {
    $rejected = $false
    try { [void](Get-EvidenceRunnerContract -EvidenceMode $stage -HasControlRevision $false) }
    catch { $rejected = $true }
    Assert-Contract $rejected "$stage accepted a missing control revision"
}
foreach ($stage in @('baseline', 'release-barrier', 'diagnostics')) {
    $rejected = $false
    try { [void](Get-EvidenceRunnerContract -EvidenceMode $stage -HasControlRevision $true) }
    catch { $rejected = $true }
    Assert-Contract $rejected "$stage accepted a control revision"
}

$publicationCases = @(
    @{ Kind = 'baseline'; ExternalId = 'P0'; Stage = 'baseline'; Branch = 'rt-r3/p0-baseline' },
    @{ Kind = 'lifecycle_release_barrier'; ExternalId = 'P1'; Stage = 'release-barrier'; Branch = 'rt-r3/p1-lifecycle-release-barrier' },
    @{ Kind = 'lease_age_comparison'; ExternalId = 'P2'; Stage = 'lease-age'; Branch = 'rt-r3/p2-final-lease-age' },
    @{ Kind = 'timing_diagnostics'; ExternalId = 'P4'; Stage = 'diagnostics'; Branch = 'rt-r3/p4-diagnostics' },
    @{ Kind = 'final_qualification'; ExternalId = 'P5'; Stage = 'final'; Branch = 'rt-r3/p5-final-qualification' }
)
foreach ($case in $publicationCases) {
    $profile = Get-EvidencePublicationProfile -EvidenceKind $case.Kind
    Assert-Contract ($profile.ExternalEvidenceId -ceq $case.ExternalId) "$($case.Kind) external evidence identifier changed"
    Assert-Contract ($profile.RunnerStage -ceq $case.Stage) "$($case.Kind) runner route changed"
    Assert-Contract ($profile.ImplementationBranch -ceq $case.Branch) "$($case.Kind) implementation branch contract changed"
}

$unsupportedProfile = Get-EvidencePublicationProfile -EvidenceKind 'supplemental_characterization'
Assert-Contract ($unsupportedProfile.ExternalEvidenceId -ceq 'P3' -and $null -eq $unsupportedProfile.RunnerStage) 'P3 must remain rejected by the publisher route'
$legacyKinds = @('P0', 'P4', ('phase' + '4'), ('r' + '3-final'))
foreach ($legacyKind in $legacyKinds) {
    $rejected = $false
    try { [void](Get-EvidencePublicationProfile -EvidenceKind $legacyKind) }
    catch { $rejected = $true }
    Assert-Contract $rejected "legacy or unknown publication kind '$legacyKind' was accepted"
}

$invalidRunnerOutputRoot = Join-Path ([IO.Path]::GetTempPath()) ('sky-invalid-evidence-mode-' + [guid]::NewGuid().ToString('N'))
$publisherOutput = @(& pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'publish_rt_dispatch_evidence.ps1') -EvidenceKind 'P0' -HeadSha ('0' * 40) -EvidenceRoot $invalidRunnerOutputRoot 2>&1)
$publisherExitCode = $LASTEXITCODE
Assert-Contract ($publisherExitCode -ne 0) 'the evidence publisher accepted an external phase identifier as an evidence kind'
Assert-Contract (($publisherOutput -join "`n").Contains('ValidateSet')) 'legacy publisher rejection did not come from the parameter contract'

$disjointTailPairs = Get-DispatchPairLeaderReviewPrompt -AttemptedPairs 6 -AvailablePairs 6 -P95RegressionPairs 2 -P99RegressionPairs 2
Assert-Contract ($disjointTailPairs -eq $false) 'two p95 and two separate p99 regression pairs must not combine into a 4/6 prompt'
$p95TailPairs = Get-DispatchPairLeaderReviewPrompt -AttemptedPairs 6 -AvailablePairs 6 -P95RegressionPairs 4 -P99RegressionPairs 0
Assert-Contract ($p95TailPairs -eq $true) 'four p95 regression pairs must prompt leader review'
$p99TailPairs = Get-DispatchPairLeaderReviewPrompt -AttemptedPairs 6 -AvailablePairs 6 -P95RegressionPairs 0 -P99RegressionPairs 4
Assert-Contract ($p99TailPairs -eq $true) 'four p99 regression pairs must prompt leader review'
$incompletePairs = Get-DispatchPairLeaderReviewPrompt -AttemptedPairs 6 -AvailablePairs 5 -P95RegressionPairs 4 -P99RegressionPairs 4
Assert-Contract ($null -eq $incompletePairs) 'incomplete paired data must remain inconclusive'

Write-Output 'Evidence runner and publication contracts: PASS'
