[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib/evidence_publication_contract.ps1')
. (Join-Path $PSScriptRoot 'lib/dispatch_lifecycle_report_contract.ps1')
Import-Module (Join-Path $PSScriptRoot 'dispatch_pair_screen.psm1') -Force

function Assert-Contract([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "Evidence contract self-test failed: $Message" }
}

$historicalV1 = [ordered]@{
    schema_version = 1
    source_revision = 'historical-source'
    transport_kind = 'deterministic-mock'
}
$historicalResult = Test-DispatchLifecycleReport -Report $historicalV1
Assert-Contract ($historicalResult.EvidenceClass -ceq 'historical') 'schema v1 reports must remain readable as historical evidence'

function New-LiveAccountingStage {
    param([string]$Phase, [long]$Scheduled, [long]$Activated, [long]$Released, [long]$Cancelled, [long]$Active)
    return [ordered]@{
        phase = $Phase
        generation_accounting_owner = 'live_scheduler'
        total = 2
        scheduled = $Scheduled
        activated = $Activated
        released = $Released
        cancelled = $Cancelled
        active = $Active
    }
}

function New-DispatchPacket {
    param([long]$UpMask, [long]$DownMask, [long]$Started)
    $mask = $UpMask -bor $DownMask
    return [ordered]@{
        up_mask = $UpMask
        down_mask = $DownMask
        requested_mask = $mask
        confirmed_mask = $mask
        first_inserted = 1
        attempts = 1
        started_qpc_ticks = $Started
        completed_qpc_ticks = $Started
        worker_outcome = 'dispatched'
    }
}

function New-DispatchCleanupCapture {
    param([long]$CompletionQpcTicks)
    return [ordered]@{
        attempts = 1
        up_attempts = 1
        down_attempts = 0
        up_events = 1
        down_events = 0
        completion_qpc_ticks = $CompletionQpcTicks
        packets = @([ordered]@{ scan_codes = @(30); key_up = $true })
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
            musical_sender_attempts = 4
            cleanup_sender_attempts = 0
            total_mock_send_attempts = 4
            final_release_obligation_mask = 0
            passed = $true
            generation_accounting_owner = 'live_scheduler'
            accounting_stages = @(
                (New-LiveAccountingStage 'before_first_down' 2 0 0 0 0),
                (New-LiveAccountingStage 'after_first_down' 1 1 0 0 1),
                (New-LiveAccountingStage 'after_first_up' 1 1 1 0 0),
                (New-LiveAccountingStage 'after_second_down' 0 2 1 0 1),
                (New-LiveAccountingStage 'after_final_up' 0 2 2 0 0)
            )
            packets = @(
                (New-DispatchPacket 0 1 100),
                (New-DispatchPacket 1 0 200),
                (New-DispatchPacket 0 1 300),
                (New-DispatchPacket 1 0 400)
            )
        }
        pause_cleanup_resume = [ordered]@{
            pause_requested = $true
            sender_attempts = 2
            musical_sender_attempts = 2
            cleanup_sender_attempts = 2
            total_mock_send_attempts = 4
            final_release_obligation_mask = 0
            passed = $true
            generation_accounting_owner = 'live_scheduler'
            accounting_stages = @(
                (New-LiveAccountingStage 'before_first_down' 2 0 0 0 0),
                (New-LiveAccountingStage 'after_initial_down' 1 1 0 0 1),
                (New-LiveAccountingStage 'after_pause_cleanup' 1 1 0 1 0),
                (New-LiveAccountingStage 'after_resumed_down' 0 2 0 1 1),
                (New-LiveAccountingStage 'after_final_cleanup' 0 2 0 2 0)
            )
            packets = @(
                (New-DispatchPacket 0 1 171771),
                (New-DispatchPacket 0 1 171772)
            )
            cleanup_captures = [ordered]@{
                pause = New-DispatchCleanupCapture 100
                final = New-DispatchCleanupCapture 171773
                total_attempts = 2
            }
            cleanup_completion_qpc_ticks = 100
            min_release_gap_us = 17167
            min_release_gap_ticks = 171671
            resume_barrier_qpc_ticks = 171771
            candidate_packet_not_before_qpc_ticks = 171771
            resumed_sender_start_qpc_ticks = 171771
            g_minus_one_decision = 'wait'
            exact_g_decision = 'send'
        }
    }
}
$liveResult = Test-DispatchLifecycleReport -Report $liveV2
Assert-Contract ($liveResult.EvidenceClass -ceq 'live_scheduler') 'schema v2 must prove the live scheduler owner'

$badAccountingV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$badFinalStage = $badAccountingV2.live_scheduler.pause_cleanup_resume.accounting_stages[4]
$badFinalStage.cancelled = 0
$badFinalStage.active = 2
$badAccountingRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $badAccountingV2) } catch { $badAccountingRejected = $true }
Assert-Contract $badAccountingRejected 'schema v2 accepted an active generation after final cleanup'

$duplicatePhaseV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$duplicatePhaseV2.live_scheduler.pause_cleanup_resume.accounting_stages[3].phase = 'after_pause_cleanup'
$duplicatePhaseRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $duplicatePhaseV2) } catch { $duplicatePhaseRejected = $true }
Assert-Contract $duplicatePhaseRejected 'schema v2 accepted a duplicate accounting phase'

$badTotalV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$badTotalV2.live_scheduler.no_pause_control.accounting_stages[4].total = 1
$badTotalRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $badTotalV2) } catch { $badTotalRejected = $true }
Assert-Contract $badTotalRejected 'schema v2 accepted accounting with total != scheduled + activated'

$fractionalHoldV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$fractionalHoldV2.timing.hold_frames = 1.25
$fractionalHoldRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $fractionalHoldV2) } catch { $fractionalHoldRejected = $true }
Assert-Contract $fractionalHoldRejected 'schema v2 accepted hold_frames=1.25 with the one-frame H value'

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

$earlyResumedSendV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$earlyResumedSendV2.live_scheduler.pause_cleanup_resume.resumed_sender_start_qpc_ticks = 171770
$earlyResumedSendRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $earlyResumedSendV2) } catch { $earlyResumedSendRejected = $true }
Assert-Contract $earlyResumedSendRejected 'schema v2 accepted resumed sender start before completion + G'

$cleanupDownV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$cleanupDownV2.live_scheduler.pause_cleanup_resume.cleanup_captures.pause.packets[0].key_up = $false
$cleanupDownRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $cleanupDownV2) } catch { $cleanupDownRejected = $true }
Assert-Contract $cleanupDownRejected 'schema v2 accepted a Down in the cleanup capture'

$extraCleanupUpV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$extraCleanupUpV2.live_scheduler.pause_cleanup_resume.cleanup_captures.pause.up_events = 2
$extraCleanupUpRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $extraCleanupUpV2) } catch { $extraCleanupUpRejected = $true }
Assert-Contract $extraCleanupUpRejected 'schema v2 accepted an extra cleanup Up outside the one-key lifecycle fixture'

$cleanupStampV2 = $liveV2 | ConvertTo-Json -Depth 20 | ConvertFrom-Json -AsHashtable
$cleanupStampV2.live_scheduler.pause_cleanup_resume.cleanup_captures.pause.completion_qpc_ticks = 99
$cleanupStampRejected = $false
try { [void](Test-DispatchLifecycleReport -Report $cleanupStampV2) } catch { $cleanupStampRejected = $true }
Assert-Contract $cleanupStampRejected 'schema v2 accepted cleanup capture with a completion stamp different from its resume barrier source'

Assert-DispatchReportSourceIdentity -Report $liveV2 -SourceSha ('a' * 40) -SourceTreeSha ('b' * 40)
$sourceMismatchRejected = $false
try { Assert-DispatchReportSourceIdentity -Report $liveV2 -SourceSha ('c' * 40) -SourceTreeSha ('b' * 40) }
catch { $sourceMismatchRejected = $true }
Assert-Contract $sourceMismatchRejected 'runner source identity validator accepted a report from a different HEAD'

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

Write-Output 'Lifecycle report and historical publication contracts: PASS'
