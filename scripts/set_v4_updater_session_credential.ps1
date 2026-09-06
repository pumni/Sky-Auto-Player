# scripts/set_v4_updater_session_credential.ps1
# Secure operator provisioning script for Windows Credential Manager generic session credential.
# Governed by docs/v4-release-execution-topology.md, docs/v4-updater-key-custody.md, and SECURITY.md.

[CmdletBinding()]
param(
    # Non-interactive testing seam: only SecureString is accepted; no plaintext password CLI parameter exists.
    [Parameter(Mandatory = $false)]
    [System.Security.SecureString]$TestSecureString,

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
    $secureInput = if ($null -ne $TestSecureString) {
        $TestSecureString
    } else {
        Read-Host -Prompt "Enter V4 updater production private key passphrase" -AsSecureString
    }

    if ($null -eq $secureInput -or $secureInput.Length -eq 0) {
        Write-Host "[FAIL] Passphrase input cannot be empty"
        exit 1
    }

    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureInput)
    $passwordValue = $null
    try {
        $passwordValue = [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
        if ([string]::IsNullOrEmpty($passwordValue)) {
            Write-Host "[FAIL] Passphrase input cannot be empty"
            exit 1
        }
        [SkyAutoPlayer.V4UpdaterCredentialBroker]::WriteSessionCredential($Target, $passwordValue)
    } finally {
        if ($bstr -ne [IntPtr]::Zero) {
            [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
        }
        $passwordValue = $null
        Remove-Variable passwordValue -ErrorAction SilentlyContinue
    }

    Write-Host "[PASS] V4 updater session credential provisioned for target: $Target"
    exit 0
} catch {
    Write-Host "[FAIL] Failed to provision session credential: $($_.Exception.Message)"
    exit 1
}
