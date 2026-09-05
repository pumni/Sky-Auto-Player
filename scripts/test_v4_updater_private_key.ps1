$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('sky-v4-updater-key-test-' + [guid]::NewGuid().ToString('N'))
$throwawayKeyPath = Join-Path $fixtureRoot 'throwaway.key'
$cargoShimPath = Join-Path $fixtureRoot 'cargo.cmd'
$passwordEnvName = 'SKY_V4_TEST_UPDATER_PASSWORD'
$passwordMarker = 'V4_TEST_ONLY_PASS_PHRASE_MARKER_7F3C91D2'

$savedPath = $env:Path
$savedActions = $env:GITHUB_ACTIONS
$savedPassword = [Environment]::GetEnvironmentVariable($passwordEnvName, 'Process')

function Assert-NoPasswordMarker {
    param(
        [string]$Name,
        [string]$Output
    )

    if ($Output.Contains($passwordMarker)) {
        throw "FAILED: $Name emitted the throwaway password marker"
    }
}

try {
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null

    # Generate only throwaway key material outside the repository.
    Push-Location (Join-Path $repoRoot 'desktop')
    try {
        & bun run tauri signer generate --ci --password '' --force -w $throwawayKeyPath *> $null
        if ($LASTEXITCODE -ne 0) {
            throw "Throwaway updater key generation failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }

    # Exercise the real verifier mismatch path while forcing the old CI-mask
    # branch, if present, to expose the marker in captured output.
    $env:GITHUB_ACTIONS = 'true'
    [Environment]::SetEnvironmentVariable($passwordEnvName, $passwordMarker, 'Process')
    $failureOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'verify_v4_updater_private_key.ps1') `
        -KeyPath $throwawayKeyPath `
        -PasswordEnv $passwordEnvName 2>&1 | Out-String
    $failureExitCode = $LASTEXITCODE

    if ($failureExitCode -eq 0) {
        throw 'FAILED: Throwaway updater key unexpectedly passed canonical-root verification'
    }
    Assert-NoPasswordMarker -Name 'Verifier mismatch path' -Output $failureOutput
    if ($failureOutput -notmatch '\[FAIL\]') {
        throw "FAILED: Verifier mismatch path did not report failure. Output:`n$failureOutput"
    }

    # Reach the wrapper success path without a production key. The temporary
    # cargo shim checks that the marker crossed the environment boundary and
    # returns success without printing it.
    $cargoShim = @"
@echo off
if "%TAURI_SIGNING_PRIVATE_KEY_PASSWORD%"=="$passwordMarker" exit /b 0
exit /b 1
"@
    Set-Content -LiteralPath $cargoShimPath -Value $cargoShim -Encoding ASCII
    $env:Path = "$fixtureRoot;$savedPath"

    $successOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'verify_v4_updater_private_key.ps1') `
        -KeyPath $throwawayKeyPath `
        -PasswordEnv $passwordEnvName 2>&1 | Out-String
    $successExitCode = $LASTEXITCODE

    Assert-NoPasswordMarker -Name 'Verifier success path' -Output $successOutput
    if ($successExitCode -ne 0) {
        throw "FAILED: Verifier success path failed with exit code $successExitCode. Output:`n$successOutput"
    }
    if ($successOutput -notmatch '\[PASS\]') {
        throw "FAILED: Verifier success path did not report success. Output:`n$successOutput"
    }

    Write-Host '[PASS] V4 updater private-key verifier secret-output regression passed'
} finally {
    $env:Path = $savedPath
    if ($null -ne $savedActions) {
        $env:GITHUB_ACTIONS = $savedActions
    } else {
        Remove-Item Env:GITHUB_ACTIONS -ErrorAction SilentlyContinue
    }
    if ($null -ne $savedPassword) {
        [Environment]::SetEnvironmentVariable($passwordEnvName, $savedPassword, 'Process')
    } else {
        [Environment]::SetEnvironmentVariable($passwordEnvName, $null, 'Process')
    }
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
}
