function Get-EvidenceRunnerContract {
    param(
        [Parameter(Mandatory)][string]$EvidenceMode,
        [Parameter(Mandatory)][bool]$HasControlRevision
    )

    $contract = switch -CaseSensitive ($EvidenceMode) {
        'baseline' {
            [pscustomobject]@{ RunIdPrefix = 'dispatch-baseline-'; RunnerStage = 'baseline'; RequiresControlRevision = $false; IsDiagnostics = $false; IsFinalQualification = $false; IsLeaseAgeComparison = $false }
        }
        'release-barrier' {
            [pscustomobject]@{ RunIdPrefix = 'lifecycle-release-barrier-'; RunnerStage = 'release-barrier'; RequiresControlRevision = $false; IsDiagnostics = $false; IsFinalQualification = $false; IsLeaseAgeComparison = $false }
        }
        'lease-age' {
            [pscustomobject]@{ RunIdPrefix = 'lifecycle-lease-age-'; RunnerStage = 'lease-age'; RequiresControlRevision = $true; IsDiagnostics = $false; IsFinalQualification = $false; IsLeaseAgeComparison = $true }
        }
        'diagnostics' {
            [pscustomobject]@{ RunIdPrefix = 'dispatch-diagnostics-'; RunnerStage = 'diagnostics'; RequiresControlRevision = $false; IsDiagnostics = $true; IsFinalQualification = $false; IsLeaseAgeComparison = $false }
        }
        'final' {
            [pscustomobject]@{ RunIdPrefix = 'lifecycle-qualification-'; RunnerStage = 'final'; RequiresControlRevision = $true; IsDiagnostics = $false; IsFinalQualification = $true; IsLeaseAgeComparison = $false }
        }
        default { throw "BLOCKED-STAGE: unsupported evidence mode '$EvidenceMode'" }
    }

    if ($contract.RequiresControlRevision -ne $HasControlRevision) {
        if ($contract.RequiresControlRevision) {
            throw "BLOCKED-STAGE: $EvidenceMode requires the accepted control revision"
        }
        throw "ControlRevision is reserved for lease-age and final qualification stages"
    }

    return $contract
}
