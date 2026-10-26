# scripts/remove_v4_updater_session_credential.ps1
# Secure operator cleanup script for Windows Credential Manager generic session credential.
# Governed by docs/v4-release-execution-topology.md, docs/v4-updater-key-custody.md, and SECURITY.md.

[CmdletBinding()]
param(
    # Internal test seam only: non-production target isolation for regression tests.
    [Parameter(Mandatory = $false)]
    [string]$InternalTestTarget
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [System.OperatingSystem]::IsWindows()) {
    Write-Host "[FAIL] Windows Credential Manager is only supported on Windows"
    exit 1
}

$explicitTarget = $InternalTestTarget

. (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1")

try {
    if (-not [string]::IsNullOrWhiteSpace($explicitTarget)) {
        Remove-V4UpdaterProductionCredential -InternalTestTarget $explicitTarget
    } else {
        Remove-V4UpdaterProductionCredential
    }
    Write-Host "[PASS] V4 updater session credential removed"
    exit 0
} catch {
    Write-Host "[FAIL] Failed to remove session credential: $($_.Exception.Message)"
    exit 1
}
