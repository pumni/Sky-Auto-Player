[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$KeyPath,

    [Parameter(Mandatory = $false)]
    [string]$PasswordEnv = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
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

# 2. Resolve password: prefer specified environment variable or secure interactive prompt
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

# 3. Mask password if running in GitHub Actions to prevent log leakage
if (-not [string]::IsNullOrWhiteSpace($passwordValue) -and $env:GITHUB_ACTIONS -eq "true") {
    Write-Output "::add-mask::$passwordValue"
}

$prevPwd = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $passwordValue
try {
    & cargo xtask updater-trust verify-private-key --key-file $keyFile
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[FAIL] Local updater private key does not match canonical production v4 root"
        exit 1
    }
    Write-Host "[PASS] Local updater private key matches canonical production v4 root (Key ID: F6355260A0C663D5)"
    exit 0
} catch {
    Write-Host "[FAIL] Updater private key verification failed: $($_.Exception.Message)"
    exit 1
} finally {
    if ($null -ne $prevPwd) {
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $prevPwd
    } else {
        Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
    }
}
