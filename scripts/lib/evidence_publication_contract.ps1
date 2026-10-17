function Get-EvidencePublicationProfile {
    param([Parameter(Mandatory)][string]$EvidenceKind)

    switch -CaseSensitive ($EvidenceKind) {
        'baseline' {
            return [pscustomobject]@{ ExternalEvidenceId = 'P0'; RunnerStage = 'baseline'; ImplementationBranch = 'rt-r3/p0-baseline' }
        }
        'lifecycle_release_barrier' {
            return [pscustomobject]@{ ExternalEvidenceId = 'P1'; RunnerStage = 'release-barrier'; ImplementationBranch = 'rt-r3/p1-lifecycle-release-barrier' }
        }
        'lease_age_comparison' {
            return [pscustomobject]@{ ExternalEvidenceId = 'P2'; RunnerStage = 'lease-age'; ImplementationBranch = 'rt-r3/p2-final-lease-age' }
        }
        'supplemental_characterization' {
            return [pscustomobject]@{ ExternalEvidenceId = 'P3'; RunnerStage = $null; ImplementationBranch = $null }
        }
        'timing_diagnostics' {
            return [pscustomobject]@{ ExternalEvidenceId = 'P4'; RunnerStage = 'diagnostics'; ImplementationBranch = 'rt-r3/p4-diagnostics' }
        }
        'final_qualification' {
            return [pscustomobject]@{ ExternalEvidenceId = 'P5'; RunnerStage = 'final'; ImplementationBranch = 'rt-r3/p5-final-qualification' }
        }
        default { throw "Unsupported evidence kind: $EvidenceKind" }
    }
}
