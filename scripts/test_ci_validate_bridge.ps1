[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$tempBase = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { [IO.Path]::GetTempPath() } else { $env:RUNNER_TEMP }
$tempRoot = Join-Path $tempBase ("sky-auto-player-bridge-contract-test-" + [guid]::NewGuid().ToString("N"))
$bundle = Join-Path $tempRoot "bundle"
$output = Join-Path $tempRoot "output"
$sourceSha = "1234567890abcdef1234567890abcdef12345678"
$sentinelSha = "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
$validator = Join-Path $root "scripts/ci_validate_bridge.ps1"

try {
    New-Item -ItemType Directory -Path $bundle -Force | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $bundle "Sky-Auto-Player_4.0.0-alpha.1_x64-setup.exe"), [byte[]](1, 2, 3, 4))
    & $validator -Mode Create -BundleDir $bundle -OutputRoot $output -SourceSha $sourceSha `
        -Version "4.0.0-alpha.1" -SentinelId "catalog-sentinel" `
        -SentinelContentSha256 $sentinelSha -RepositoryRoot $root | Out-Null
    $files = @(Get-ChildItem -LiteralPath $output -File)
    if ($files.Count -ne 2 -or -not (Test-Path -LiteralPath (Join-Path $output "bridge.json"))) {
        throw "bridge Create self-test did not produce exactly bridge.json and one installer"
    }
    & $validator -Mode Validate -BridgeRoot $output -SourceSha $sourceSha `
        -Version "4.0.0-alpha.1" -SentinelId "catalog-sentinel" `
        -SentinelContentSha256 $sentinelSha -RepositoryRoot $root | Out-Null
    [IO.File]::WriteAllText((Join-Path $output "leaked-private.key"), "PRIVATE KEY")
    try {
        & $validator -Mode Validate -BridgeRoot $output -SourceSha $sourceSha -RepositoryRoot $root | Out-Null
        throw "bridge validator accepted an unexpected private-key artifact"
    } catch {
        if ($_.Exception.Message -notmatch "exactly two files") { throw }
    }
    Write-Output "CI updater-bridge contract self-tests: PASS"
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
