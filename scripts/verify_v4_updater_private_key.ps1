[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$KeyPath,

    [Parameter(Mandatory = $false)]
    [string]$Password = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$desktopRoot = Join-Path $repoRoot "desktop"

$keyFile = if (-not [string]::IsNullOrWhiteSpace($KeyPath)) {
    (Resolve-Path -LiteralPath $KeyPath -ErrorAction Stop).Path
} elseif (-not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
    (Resolve-Path -LiteralPath $env:TAURI_SIGNING_PRIVATE_KEY_PATH -ErrorAction Stop).Path
} else {
    $null
}

$cleanupTempKey = $false
$tempKeyPath = $null

try {
    if ($null -eq $keyFile) {
        if (-not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
            $runnerTemp = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
                [IO.Path]::GetTempPath()
            } else {
                $env:RUNNER_TEMP
            }
            $tempKeyPath = Join-Path $runnerTemp ("sky-v4-keycheck-" + [guid]::NewGuid().ToString("N") + ".key")
            [IO.File]::WriteAllText($tempKeyPath, $env:TAURI_SIGNING_PRIVATE_KEY.Trim(), [Text.UTF8Encoding]::new($false))
            $keyFile = $tempKeyPath
            $cleanupTempKey = $true
        } else {
            throw "No updater private key specified. Supply -KeyPath or set TAURI_SIGNING_PRIVATE_KEY_PATH or TAURI_SIGNING_PRIVATE_KEY"
        }
    }

    if (-not (Test-Path -LiteralPath $keyFile -PathType Leaf)) {
        throw "Private key file does not exist: $keyFile"
    }

    $passwordArg = if ($null -ne $Password) { $Password } else { "" }

    # Mask password if running in GitHub Actions
    if (-not [string]::IsNullOrWhiteSpace($passwordArg)) {
        Write-Output "::add-mask::$passwordArg"
    }

    $prevPwd = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $passwordArg
    try {
        & cargo xtask updater-trust verify-private-key --key-file $keyFile
        if ($LASTEXITCODE -ne 0) {
            Write-Host "[FAIL] Local updater private key does not match canonical production v4 root"
            exit 1
        }
        Write-Host "[PASS] Local updater private key matches canonical production v4 root (Key ID: F6355260A0C663D5)"
        exit 0
    } finally {
        if ($null -ne $prevPwd) {
            $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $prevPwd
        } else {
            Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
        }
    }
} catch {
    Write-Host "[FAIL] Updater private key verification failed: $($_.Exception.Message)"
    exit 1
} finally {
    if ($cleanupTempKey -and (-not [string]::IsNullOrWhiteSpace($tempKeyPath)) -and (Test-Path -LiteralPath $tempKeyPath)) {
        Remove-Item -LiteralPath $tempKeyPath -Force -ErrorAction SilentlyContinue
    }
}
