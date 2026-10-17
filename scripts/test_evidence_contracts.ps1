[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib/evidence_runner_contract.ps1')
. (Join-Path $PSScriptRoot 'lib/evidence_publication_contract.ps1')

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

$runnerSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'run_rt_dispatch_evidence.ps1') -Raw
Assert-Contract ($runnerSource -match '\$runnerStage = \$evidenceModeContract\.RunnerStage') 'the runner must consume its validated semantic stage'
Assert-Contract ($runnerSource -notmatch '\$Stage\b') 'the runner retained a call site for the removed Stage parameter'
Assert-Contract ($runnerSource -notmatch 'production_changes_permitted_by_phase') 'the runner retained a phase-specific output identifier'

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
$runnerOutput = @(& pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'run_rt_dispatch_evidence.ps1') -EvidenceMode ('phase' + '4') -OutputRoot $invalidRunnerOutputRoot 2>&1)
$runnerExitCode = $LASTEXITCODE
Assert-Contract ($runnerExitCode -ne 0) 'the evidence runner accepted a legacy phase argument'
Assert-Contract (($runnerOutput -join "`n").Contains('unsupported evidence mode')) 'legacy runner rejection did not report the evidence mode contract'

$publisherOutput = @(& pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'publish_rt_dispatch_evidence.ps1') -EvidenceKind 'P0' -HeadSha ('0' * 40) -EvidenceRoot $invalidRunnerOutputRoot 2>&1)
$publisherExitCode = $LASTEXITCODE
Assert-Contract ($publisherExitCode -ne 0) 'the evidence publisher accepted an external phase identifier as an evidence kind'
Assert-Contract (($publisherOutput -join "`n").Contains('ValidateSet')) 'legacy publisher rejection did not come from the parameter contract'

Write-Output 'Evidence runner and publication contracts: PASS'
