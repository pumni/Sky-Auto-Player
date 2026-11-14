[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$EvidenceMode,
    [Parameter(Mandatory)]
    [string]$OutputRoot,
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ControlRevision
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib/evidence_runner_contract.ps1')
[void](Get-EvidenceRunnerContract -EvidenceMode $EvidenceMode -HasControlRevision $PSBoundParameters.ContainsKey('ControlRevision'))
throw 'BLOCKED-LEGACY-DISPATCH-EVIDENCE: this runner still has coordinator-owned accounting and precision report semantics. Historical reports remain pinned to their source SHA. A DispatchExecutor report contract and acceptance oracle require Leader review.'