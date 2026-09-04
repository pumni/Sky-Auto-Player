Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Write-Host "Starting V4 production Authenticode contract tests..."

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$temporaryRoot = [IO.Path]::GetFullPath($temporaryRoot)
$fixtureRoot = Join-Path $temporaryRoot ('sky-v4-prod-signing-contract-' + [guid]::NewGuid().ToString('N'))
$envFile = Join-Path $fixtureRoot 'test-signing.env'
$targetExe = Join-Path $fixtureRoot 'target.exe'

# Find a valid PE file to use as target
$sourcePe = Get-ChildItem -LiteralPath (Join-Path $repoRoot 'rust\target') -Filter '*.exe' -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '\\deps\\' } |
    Select-Object -First 1

if ($null -eq $sourcePe) {
    # Fallback to xtask.exe or any system PE
    $sourcePe = Get-Item -LiteralPath (Join-Path $env:SystemRoot 'System32\notepad.exe')
}

try {
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
    Copy-Item -LiteralPath $sourcePe.FullName -Destination $targetExe -Force

    # Remove any existing signature on target
    $signToolCandidate = $null
    $kits = Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue
    if ($null -ne $kits -and -not [string]::IsNullOrWhiteSpace($kits.KitsRoot10)) {
        $matches = @(Get-ChildItem -LiteralPath (Join-Path $kits.KitsRoot10 'bin') -Filter signtool.exe -File -Recurse -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
            Sort-Object FullName -Descending)
        if ($matches.Count -gt 0) { $signToolCandidate = $matches[0].FullName }
    }
    if ($null -ne $signToolCandidate) {
        & $signToolCandidate remove /s $targetExe 2>$null | Out-Null
    }

    # Contract Test 1: Unconfigured production mode fails closed
    Write-Host "Contract Test 1: Unconfigured production mode fails closed..."
    $failedClosed = $false
    try {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot 'sign_v4_authenticode.ps1') `
            -Path $targetExe
        if ($LASTEXITCODE -ne 0) { $failedClosed = $true }
    } catch {
        $failedClosed = $true
    }
    if (-not $failedClosed) {
        throw "FAILED: sign_v4_authenticode did not fail closed when production provider is unconfigured"
    }
    Write-Host "Contract Test 1: PASS"

    # Set up ephemeral CI test certificate
    Write-Host "Setting up ephemeral test certificate..."
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'setup_v4_test_signing.ps1') `
        -EnvFile $envFile
    if ($LASTEXITCODE -ne 0) {
        throw "setup_v4_test_signing.ps1 failed"
    }
    Get-Content -LiteralPath $envFile | ForEach-Object {
        $line = [string]$_
        if ($line -match '^([^=]+)=(.*)$') {
            [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
        }
    }
    $testThumbprint = [string]$env:SKY_AUTHENTICODE_TEST_THUMBPRINT

    # Contract Test 2: Production signing rejects test credentials
    Write-Host "Contract Test 2: Production signing rejects test credentials..."
    $env:SKY_AUTHENTICODE_MODE = "production"
    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = "0123456789ABCDEF0123456789ABCDEF01234567"
    $env:SKY_AUTHENTICODE_PROVIDER = "custom"
    $env:SKY_AUTHENTICODE_PROVIDER_COMMAND = "echo signing %1"

    $rejectedCredentials = $false
    try {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot 'sign_v4_authenticode.ps1') `
            -Path $targetExe 2>$null
        if ($LASTEXITCODE -ne 0) { $rejectedCredentials = $true }
    } catch {
        $rejectedCredentials = $true
    }
    if (-not $rejectedCredentials) {
        throw "FAILED: Production signing accepted ephemeral CI test credentials"
    }
    Write-Host "Contract Test 2: PASS"

    # Contract Test 3: Sign with test mode, then verify production mode rejects it
    Write-Host "Contract Test 3: Production verification rejects CI test certificate..."
    $env:SKY_AUTHENTICODE_MODE = "test"
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'sign_v4_authenticode.ps1') `
        -Path $targetExe
    if ($LASTEXITCODE -ne 0) {
        throw "Test-mode signing failed"
    }

    # Verify test mode succeeds
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'verify_v4_authenticode.ps1') `
        -Mode test `
        -Artifact $targetExe
    if ($LASTEXITCODE -ne 0) {
        throw "Test-mode verification failed on signed target"
    }

    # Verify production mode rejects test certificate
    $rejectedInProduction = $false
    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = "0123456789ABCDEF0123456789ABCDEF01234567"
    try {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot 'verify_v4_authenticode.ps1') `
            -Mode production `
            -Artifact $targetExe 2>$null
        if ($LASTEXITCODE -ne 0) { $rejectedInProduction = $true }
    } catch {
        $rejectedInProduction = $true
    }
    if (-not $rejectedInProduction) {
        throw "FAILED: Production verification accepted test-signed binary"
    }
    Write-Host "Contract Test 3: PASS"

    # Contract Test 4: Production verification rejects test thumbprint
    Write-Host "Contract Test 4: Production verification rejects test thumbprint..."
    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = $testThumbprint
    $rejectedThumbprint = $false
    try {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot 'verify_v4_authenticode.ps1') `
            -Mode production `
            -Artifact $targetExe 2>$null
        if ($LASTEXITCODE -ne 0) { $rejectedThumbprint = $true }
    } catch {
        $rejectedThumbprint = $true
    }
    if (-not $rejectedThumbprint) {
        throw "FAILED: Production verification accepted test thumbprint"
    }
    Write-Host "Contract Test 4: PASS"

    # Contract Test 5: Production signing rejects mutually exclusive provider script and command
    Write-Host "Contract Test 5: Mutually exclusive provider script and command fails closed..."
    $env:SKY_AUTHENTICODE_MODE = "production"
    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = "1234567890ABCDEF1234567890ABCDEF12345678"
    $env:SKY_AUTHENTICODE_PROVIDER = "custom"
    $env:SKY_AUTHENTICODE_PROVIDER_COMMAND = "echo %1"
    $dummyScript = Join-Path $fixtureRoot "dummy_provider.ps1"
    Set-Content -LiteralPath $dummyScript -Value 'param([string]$Path) exit 0'
    $env:SKY_AUTHENTICODE_PROVIDER_SCRIPT = $dummyScript

    # Ensure no test credentials exist in env
    $env:SKY_AUTHENTICODE_TEST_PFX_PATH = ""
    $env:SKY_AUTHENTICODE_TEST_PFX_PASSWORD = ""
    $env:SKY_AUTHENTICODE_TEST_THUMBPRINT = ""

    $rejectedMutualExclusive = $false
    try {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot 'sign_v4_authenticode.ps1') `
            -Path $targetExe 2>$null
        if ($LASTEXITCODE -ne 0) { $rejectedMutualExclusive = $true }
    } catch {
        $rejectedMutualExclusive = $true
    }
    if (-not $rejectedMutualExclusive) {
        throw "FAILED: Production signing accepted mutually exclusive SCRIPT and COMMAND"
    }
    Write-Host "Contract Test 5: PASS"

    # Contract Test 6: Production signing executes structured provider script and enforces exit code
    Write-Host "Contract Test 6: Structured provider script failure propagation..."
    $env:SKY_AUTHENTICODE_PROVIDER_COMMAND = ""
    $failingScript = Join-Path $fixtureRoot "failing_provider.ps1"
    Set-Content -LiteralPath $failingScript -Value 'param([string]$Path) exit 42'
    $env:SKY_AUTHENTICODE_PROVIDER_SCRIPT = $failingScript

    $rejectedFailedProvider = $false
    try {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot 'sign_v4_authenticode.ps1') `
            -Path $targetExe 2>$null
        if ($LASTEXITCODE -ne 0) { $rejectedFailedProvider = $true }
    } catch {
        $rejectedFailedProvider = $true
    }
    if (-not $rejectedFailedProvider) {
        throw "FAILED: Production signing did not fail closed when provider script failed"
    }
    Write-Host "Contract Test 6: PASS"
} finally {
    # Clean up test certificate
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'cleanup_v4_test_signing.ps1')
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "[PASS] V4 Authenticode contract tests: CI test certificate cannot satisfy production mode"
exit 0
