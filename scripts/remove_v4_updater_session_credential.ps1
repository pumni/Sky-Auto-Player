# scripts/remove_v4_updater_session_credential.ps1
# Secure operator cleanup script for Windows Credential Manager generic session credential.
# Governed by docs/v4-release-execution-topology.md, docs/v4-updater-key-custody.md, and SECURITY.md.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$Target = "SkyAutoPlayer/V4UpdaterProduction"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [System.OperatingSystem]::IsWindows()) {
    Write-Host "[FAIL] Windows Credential Manager is only supported on Windows"
    exit 1
}

. (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1")

try {
    Remove-V4UpdaterProductionCredential -Target $Target
    Write-Host "[PASS] V4 updater session credential removed for target: $Target"
    exit 0
} catch {
    Write-Host "[FAIL] Failed to remove session credential: $($_.Exception.Message)"
    exit 1
}
