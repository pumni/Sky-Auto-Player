[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$KeyPath,

    [Parameter(Mandatory = $false)]
    [string]$PasswordEnv = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",

    [Parameter(Mandatory = $false)]
    [switch]$UseCredentialBroker
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

function Redact-UpdaterVerifierOutput {
    param(
        [AllowEmptyString()]
        [string]$Output,
        [string]$Password
    )

    $redacted = $Output
    if (-not [string]::IsNullOrEmpty($Password)) {
        $redacted = $redacted.Replace($Password, '[REDACTED]')
    }
    return $redacted
}

# 1. Resolve key path: strictly require a file path (no raw key in env vars or command lines)
try {
    $keyFile = if (-not [string]::IsNullOrWhiteSpace($KeyPath)) {
        (Resolve-Path -LiteralPath $KeyPath -ErrorAction Stop).Path
    } elseif (-not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
        (Resolve-Path -LiteralPath $env:TAURI_SIGNING_PRIVATE_KEY_PATH -ErrorAction Stop).Path
    } else {
        throw "No updater private key path specified"
    }
} catch {
    throw "Unable to resolve updater private key file"
}

if (-not (Test-Path -LiteralPath $keyFile -PathType Leaf)) {
    throw "Private key file does not exist"
}

# 2. Delegate broker-backed verification to Rust. Rust reads and deletes the
#    session credential and passes it only to this child process environment.
if ($UseCredentialBroker) {
    Push-Location $repoRoot
    try {
        & cargo xtask credential run-action --action verify-private-key --key-path $keyFile
        exit $LASTEXITCODE
    } finally {
        Pop-Location
    }
}

# Resolve password for local non-broker verification: environment or secure prompt.
$envVal = [Environment]::GetEnvironmentVariable($PasswordEnv)
$passwordValue = if (-not [string]::IsNullOrWhiteSpace($envVal)) {
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
    # Redact only cryptographic secret material at this boundary. A filesystem
    # path is runner configuration, not a cryptographic secret.
    $verificationOutput = & cargo xtask updater-trust verify-private-key --key-file $keyFile 2>&1 | Out-String
    $verificationOutput = Redact-UpdaterVerifierOutput -Output $verificationOutput -Password $passwordValue
    Write-Output $verificationOutput.TrimEnd()
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[FAIL] Local updater private key does not match canonical production v4 root"
        exit 1
    }
    Write-Host "[PASS] Local updater private key matches canonical production v4 root"
    exit 0
} catch {
    $errorMessage = Redact-UpdaterVerifierOutput -Output $_.Exception.Message -Password $passwordValue
    Write-Host "[FAIL] Updater private key verification failed: $errorMessage"
    exit 1
} finally {
    if ($null -ne $prevPwd) {
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $prevPwd
    } else {
        Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
    }
}
