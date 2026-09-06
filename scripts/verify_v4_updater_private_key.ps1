[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$KeyPath,

    [Parameter(Mandatory = $false)]
    [string]$PasswordEnv = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",

    [Parameter(Mandatory = $false)]
    [switch]$UseCredentialBroker,

    [Parameter(Mandatory = $false)]
    [string]$CredentialTarget = "SkyAutoPlayer/V4UpdaterProduction"
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

# 1. Resolve key path: strictly require a file path (no raw key in env vars or command lines)
$keyFile = if (-not [string]::IsNullOrWhiteSpace($KeyPath)) {
    (Resolve-Path -LiteralPath $KeyPath -ErrorAction Stop).Path
} elseif (-not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
    (Resolve-Path -LiteralPath $env:TAURI_SIGNING_PRIVATE_KEY_PATH -ErrorAction Stop).Path
} else {
    throw "No updater private key path specified. Provide -KeyPath <path> or set TAURI_SIGNING_PRIVATE_KEY_PATH environment variable."
}

if (-not (Test-Path -LiteralPath $keyFile -PathType Leaf)) {
    throw "Private key file does not exist: $keyFile"
}

# 2. Resolve password: prefer credential broker if requested, or specified environment variable, or secure interactive prompt
$envVal = [Environment]::GetEnvironmentVariable($PasswordEnv)
$passwordValue = if ($UseCredentialBroker) {
    . (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1")
    Get-V4UpdaterProductionCredential -Target $CredentialTarget
} elseif (-not [string]::IsNullOrWhiteSpace($envVal)) {
    $envVal
} elseif ([Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
    Write-Host "Enter updater private key passphrase (press Enter if unencrypted): " -NoNewline
    $securePrompt = Read-Host -AsSecureString
    if ($securePrompt.Length -gt 0) {
        $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($securePrompt)
        try {
            [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
        } finally {
            [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
        }
    } else {
        ""
    }
} else {
    ""
}

$prevPwd = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $passwordValue
try {
    # The child owns verification and emits only sanitized status. Keep a final
    # redaction guard around the boundary so this process never forwards a
    # password if a dependency unexpectedly includes it in diagnostic output.
    $verificationOutput = & cargo xtask updater-trust verify-private-key --key-file $keyFile 2>&1 | Out-String
    if (-not [string]::IsNullOrEmpty($passwordValue)) {
        $verificationOutput = $verificationOutput.Replace($passwordValue, "[REDACTED]")
    }
    Write-Output $verificationOutput.TrimEnd()
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[FAIL] Local updater private key does not match canonical production v4 root"
        exit 1
    }
    Write-Host "[PASS] Local updater private key matches canonical production v4 root"
    exit 0
} catch {
    $errorMessage = $_.Exception.Message
    if (-not [string]::IsNullOrEmpty($passwordValue)) {
        $errorMessage = $errorMessage.Replace($passwordValue, "[REDACTED]")
    }
    Write-Host "[FAIL] Updater private key verification failed: $errorMessage"
    exit 1
} finally {
    if ($null -ne $prevPwd) {
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $prevPwd
    } else {
        Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
    }
}
