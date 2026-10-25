# Windows regression entry point for the Rust-owned V4 updater credential boundary.
# All live Credential Manager operations use generated unit-test targets only.

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not [System.OperatingSystem]::IsWindows()) {
    throw "Credential Manager regression tests require Windows"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ownerSource = Get-Content -LiteralPath (Join-Path $repoRoot "rust/crates/sky_windows_security/src/credential.rs") -Raw
$nativeSource = Get-Content -LiteralPath (Join-Path $repoRoot "rust/crates/sky_windows_security/src/win32.rs") -Raw
$provisionSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot "set_v4_updater_session_credential.ps1") -Raw
$brokerAdapter = Get-Content -LiteralPath (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1") -Raw
$removeAdapter = Get-Content -LiteralPath (Join-Path $PSScriptRoot "remove_v4_updater_session_credential.ps1") -Raw

if ($nativeSource -notmatch "CredReadW" -or $nativeSource -notmatch "CredWriteW" -or $nativeSource -notmatch "CredDeleteW" -or
    $nativeSource -notmatch "CRED_PERSIST_SESSION") {
    throw "Rust Credential Manager owner is missing its native operations"
}
if ($ownerSource -notmatch "SkyAutoPlayer/V4UpdaterProduction" -or
    $ownerSource -notmatch "run_one_shot" -or
    $ownerSource -notmatch "reject_ambient_password" -or
    $ownerSource -notmatch "test_target") {
    throw "Rust Credential Manager owner is missing its target or lifecycle contract"
}
if ($brokerAdapter -match "Add-Type|PSTypeName|DllImport|CREDENTIAL_WRITE") {
    throw "PowerShell credential adapter still contains an embedded native broker"
}
if ($removeAdapter -match '\$script:CanonicalCredentialTarget' -or
    $removeAdapter -notmatch 'Remove-V4UpdaterProductionCredential\s*\r?\n') {
    throw "Credential removal adapter must use Rust's canonical default target"
}
if ($provisionSource -notmatch "RedirectStandardInput" -or
    $provisionSource -notmatch 'StandardInput\.Write\(\$passwordValue\)' -or
    $provisionSource -notmatch "ZeroFreeBSTR" -or
    $provisionSource -match '\[string\]\$Password|Write-Host.*passwordValue|Write-Output.*passwordValue') {
    throw "Provisioning adapter does not preserve the hidden stdin-only secret boundary"
}

# Exercise the retained PowerShell delete adapter only with an isolated target.
$testTarget = "SkyAutoPlayer/V4UpdaterUnitTest_$([guid]::NewGuid().ToString('N'))"
$sentinel = "BENIGN_CREDENTIAL_ADAPTER_SENTINEL"
$writer = $null
try {
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = "cargo"
    $startInfo.WorkingDirectory = $repoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in @("xtask", "credential", "write-stdin", "--test-target", $testTarget)) {
        [void]$startInfo.ArgumentList.Add($argument)
    }
    $writer = [System.Diagnostics.Process]::new()
    $writer.StartInfo = $startInfo
    if (-not $writer.Start()) { throw "Rust isolated-target writer did not start" }
    $stdoutTask = $writer.StandardOutput.ReadToEndAsync()
    $stderrTask = $writer.StandardError.ReadToEndAsync()
    $writer.StandardInput.Write($sentinel)
    $writer.StandardInput.Close()
    $writer.WaitForExit()
    $writerOutput = $stdoutTask.GetAwaiter().GetResult() + $stderrTask.GetAwaiter().GetResult()
    if ($writer.ExitCode -ne 0 -or $writerOutput.Contains($sentinel)) {
        throw "Rust isolated-target setup failed or exposed its sentinel"
    }

    Push-Location $repoRoot
    try {
        $removeOutput = (& pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "remove_v4_updater_session_credential.ps1") `
            -InternalTestTarget $testTarget 2>&1 | Out-String)
        $removeExitCode = $LASTEXITCODE
        if ($removeExitCode -ne 0) { throw "PowerShell isolated-target removal failed: $removeOutput" }
        $statusOutput = (& cargo xtask credential status --test-target $testTarget 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0 -or $statusOutput -notmatch "ABSENT") {
            throw "PowerShell removal adapter left its isolated target present: $statusOutput"
        }
    } finally {
        Pop-Location
    }
} finally {
    if ($null -ne $writer) { $writer.Dispose() }
    Push-Location $repoRoot
    try { & cargo xtask credential delete --test-target $testTarget | Out-Null }
    finally { Pop-Location }
}

Push-Location (Join-Path $repoRoot "rust")
try {
    & cargo test --package sky_windows_security
    if ($LASTEXITCODE -ne 0) { throw "Rust Credential Manager regression suite failed" }
    & cargo test --package sky_xtask credential_cli::tests
    if ($LASTEXITCODE -ne 0) { throw "Rust credential output secrecy regression failed" }
    & cargo test --package sky_xtask --test credential_cli_windows
    if ($LASTEXITCODE -ne 0) { throw "Rust credential CLI lifecycle regression failed" }
} finally {
    Pop-Location
}
Write-Host "V4 updater credential regression: PASS (isolated test target; canonical target untouched)"
