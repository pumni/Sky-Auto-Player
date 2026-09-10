[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot,
    [Parameter(Mandatory = $true)]
    [string]$TargetRoot,
    [Parameter(Mandatory = $true)]
    [string]$SourceSha,
    [string]$RepositoryRoot = (Get-Location).Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$bridgeVersion = "4.0.0-alpha.1"
$repoRoot = (Resolve-Path -LiteralPath $RepositoryRoot -ErrorAction Stop).Path
$desktopRoot = Join-Path $repoRoot "desktop"
$cargoPath = Join-Path $desktopRoot "src-tauri/Cargo.toml"
$lockPath = Join-Path $repoRoot "rust/Cargo.lock"
$catalogManifestPath = Join-Path $repoRoot "builtin-songs/manifest.json"
$runnerTemp = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    throw "RUNNER_TEMP is required for the disposable bridge build"
} else {
    (Resolve-Path -LiteralPath $env:RUNNER_TEMP -ErrorAction Stop).Path
}
$outputPath = [IO.Path]::GetFullPath($OutputRoot)
$targetPath = [IO.Path]::GetFullPath($TargetRoot)
$runnerPrefix = $runnerTemp.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
foreach ($path in @($outputPath, $targetPath)) {
    if (-not $path.StartsWith($runnerPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Bridge build paths must remain under RUNNER_TEMP: $path"
    }
}
if ($outputPath.Equals($targetPath, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Bridge output and target directories must be distinct"
}

function Get-BytesSha256([byte[]]$Bytes) {
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

function Assert-SourceSha([string]$Value) {
    if ($Value -notmatch '^[0-9a-fA-F]{40}$' -or $Value -match '^0{40}$') {
        throw "Bridge source SHA must be a non-zero 40-character commit SHA"
    }
    $actual = (& git -C $repoRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $actual -ine $Value) {
        throw "Bridge source SHA does not match the checked-out source: expected=$Value actual=$actual"
    }
    return $Value.ToLowerInvariant()
}

function Convert-FixtureCargoVersion([string]$Source) {
    $pattern = [regex]::new('(?m)^(version = ")[^"]+("(?=\r?$))')
    if ($pattern.Matches($Source).Count -ne 1) {
        throw "Bridge build could not uniquely locate the desktop Cargo package version"
    }
    return $pattern.Replace($Source, ('${1}' + $bridgeVersion + '${2}'), 1)
}

function Convert-FixtureLockVersion([string]$Source) {
    $pattern = [regex]::new('(?s)(\[\[package\]\]\r?\nname = "sky_desktop_shell"\r?\nversion = ")[^"]+("(?=\r?\n))')
    if ($pattern.Matches($Source).Count -ne 1) {
        throw "Bridge build could not uniquely locate the desktop package in Cargo.lock"
    }
    return $pattern.Replace($Source, ('${1}' + $bridgeVersion + '${2}'), 1)
}

function Set-CatalogSentinel {
    param(
        [Parameter(Mandatory = $true)] [string]$SentinelId,
        [Parameter(Mandatory = $true)] [string]$SentinelRelativePath,
        [Parameter(Mandatory = $true)] [byte[]]$SentinelBytes,
        [Parameter(Mandatory = $true)] [string]$SentinelSha256
    )
    $manifest = Get-Content -LiteralPath $catalogManifestPath -Raw | ConvertFrom-Json
    $entries = @($manifest.songs | Where-Object { [string]$_.id -eq $SentinelId })
    if ($entries.Count -ne 1 -or [string]$entries[0].path -ne "sheets/$SentinelRelativePath") {
        throw "Bridge build could not locate its selected catalog sentinel"
    }
    $entries[0].sha256 = $SentinelSha256
    $manifestBytes = [Text.Encoding]::UTF8.GetBytes(($manifest | ConvertTo-Json -Depth 8) + [Environment]::NewLine)
    [IO.File]::WriteAllBytes($catalogManifestPath, $manifestBytes)
    [IO.File]::WriteAllBytes((Join-Path (Join-Path $repoRoot "songs") $SentinelRelativePath), $SentinelBytes)
}

function Write-BridgeBuildConfig([string]$Path, [string]$BuildPublicKey) {
    [ordered]@{
        plugins = [ordered]@{
            updater = [ordered]@{
                # This key only satisfies Tauri's bundle-time config. The
                # fixture binary receives qualification roots at runtime.
                pubkey = $BuildPublicKey
                dangerousInsecureTransportProtocol = $true
                endpoints = @("http://127.0.0.1:1/stable")
            }
        }
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Path -Encoding utf8
}

$sourceSha = Assert-SourceSha $SourceSha
$cargoSource = [IO.File]::ReadAllBytes($cargoPath)
$lockSource = [IO.File]::ReadAllBytes($lockPath)
$manifestSource = [IO.File]::ReadAllBytes($catalogManifestPath)
$manifest = Get-Content -LiteralPath $catalogManifestPath -Raw | ConvertFrom-Json
$sentinel = @($manifest.songs | Where-Object { [string]$_.path -like "sheets/*.json" }) | Select-Object -First 1
if ($null -eq $sentinel) { throw "Bridge build requires a canonical JSON catalog sentinel" }
$sentinelId = [string]$sentinel.id
$sentinelRelativePath = ([string]$sentinel.path).Substring("sheets/".Length)
if ([string]::IsNullOrWhiteSpace($sentinelId) -or [string]::IsNullOrWhiteSpace($sentinelRelativePath) -or
    $sentinelRelativePath.Contains("..") -or $sentinelRelativePath.Contains("\")) {
    throw "Bridge build selected an invalid catalog sentinel"
}
$sentinelPath = Join-Path (Join-Path $repoRoot "songs") $sentinelRelativePath
$songSource = [IO.File]::ReadAllBytes($sentinelPath)
$sentinelBytes = [Text.Encoding]::UTF8.GetBytes(
    '{"name":"Updater bridge catalog sentinel","bpm":120,"songNotes":[{"time":0,"key":"1Key0"},{"time":333,"key":"1Key1"}]}')
$sentinelSha256 = Get-BytesSha256 $sentinelBytes
$keyPath = Join-Path $runnerTemp ("sky-auto-player-updater-bridge-" + [guid]::NewGuid().ToString("N") + ".key")
$configPath = Join-Path $runnerTemp ("sky-auto-player-updater-bridge-" + [guid]::NewGuid().ToString("N") + ".json")
$oldCargoTargetDir = [Environment]::GetEnvironmentVariable("CARGO_TARGET_DIR", "Process")
$oldSigningKey = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY", "Process")
$oldSigningPassword = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")
$oldAuthenticodeMode = [Environment]::GetEnvironmentVariable("SKY_AUTHENTICODE_MODE", "Process")

try {
    [IO.File]::WriteAllBytes($cargoPath, [Text.Encoding]::UTF8.GetBytes((Convert-FixtureCargoVersion ([Text.Encoding]::UTF8.GetString($cargoSource)))))
    [IO.File]::WriteAllBytes($lockPath, [Text.Encoding]::UTF8.GetBytes((Convert-FixtureLockVersion ([Text.Encoding]::UTF8.GetString($lockSource)))))
    Set-CatalogSentinel -SentinelId $sentinelId -SentinelRelativePath $sentinelRelativePath -SentinelBytes $sentinelBytes -SentinelSha256 $sentinelSha256

    Push-Location $desktopRoot
    try {
        & bun run tauri signer generate --ci --password "" --force -w $keyPath *> $null
        if ($LASTEXITCODE -ne 0) { throw "Bridge signing-key generation failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
    $buildPublicKey = ([IO.File]::ReadAllText("$keyPath.pub")).Trim()
    $privateKey = ([IO.File]::ReadAllText($keyPath)).Trim()
    if ([string]::IsNullOrWhiteSpace($buildPublicKey) -or [string]::IsNullOrWhiteSpace($privateKey)) {
        throw "Bridge disposable signing key was not generated"
    }
    Write-BridgeBuildConfig $configPath $buildPublicKey
    if (Test-Path -LiteralPath $outputPath) { Remove-Item -LiteralPath $outputPath -Recurse -Force }
    New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
    if (Test-Path -LiteralPath $targetPath) { Remove-Item -LiteralPath $targetPath -Recurse -Force }
    New-Item -ItemType Directory -Path $targetPath -Force | Out-Null
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", $targetPath, "Process")
    [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY", $privateKey, "Process")
    [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "", "Process")
    [Environment]::SetEnvironmentVariable("SKY_AUTHENTICODE_MODE", "unsigned-zero-budget", "Process")
    Push-Location $desktopRoot
    try {
        & bun run tauri build --ci --config $configPath -- --profile dist --features tauri-update-fixture
        if ($LASTEXITCODE -ne 0) { throw "Bridge Tauri build failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
    $bundleDir = Join-Path $targetPath "dist/bundle/nsis"
    $installers = @(Get-ChildItem -LiteralPath $bundleDir -Filter ("*" + $bridgeVersion + "*_x64-setup.exe") -File)
    if ($installers.Count -ne 1) {
        throw "Bridge build must produce exactly one NSIS installer; found $($installers.Count)"
    }
    Copy-Item -LiteralPath $installers[0].FullName -Destination (Join-Path $outputPath $installers[0].Name)
    $contractPath = Join-Path $runnerTemp ("sky-auto-player-updater-bridge-contract-" + [guid]::NewGuid().ToString("N"))
    & (Join-Path $PSScriptRoot "ci_validate_bridge.ps1") `
        -Mode Create `
        -BundleDir $outputPath `
        -OutputRoot $contractPath `
        -SourceSha $sourceSha `
        -Version $bridgeVersion `
        -SentinelId $sentinelId `
        -SentinelContentSha256 $sentinelSha256 `
        -RepositoryRoot $repoRoot
    if (-not (Test-Path -LiteralPath $contractPath -PathType Container)) {
        throw "Bridge contract validator did not produce its validation root"
    }
    Remove-Item -LiteralPath $outputPath -Recurse -Force
    Move-Item -LiteralPath $contractPath -Destination $outputPath
    Write-Host "Updater bridge producer: PASS (source=$sourceSha; version=$bridgeVersion; sentinel=$sentinelId; sentinel_sha256=$sentinelSha256)"
} finally {
    [IO.File]::WriteAllBytes($cargoPath, $cargoSource)
    [IO.File]::WriteAllBytes($lockPath, $lockSource)
    [IO.File]::WriteAllBytes($catalogManifestPath, $manifestSource)
    [IO.File]::WriteAllBytes($sentinelPath, $songSource)
    if ((Get-BytesSha256 ([IO.File]::ReadAllBytes($cargoPath))) -ne (Get-BytesSha256 $cargoSource) -or
        (Get-BytesSha256 ([IO.File]::ReadAllBytes($lockPath))) -ne (Get-BytesSha256 $lockSource) -or
        (Get-BytesSha256 ([IO.File]::ReadAllBytes($catalogManifestPath))) -ne (Get-BytesSha256 $manifestSource) -or
        (Get-BytesSha256 ([IO.File]::ReadAllBytes($sentinelPath))) -ne (Get-BytesSha256 $songSource)) {
        throw "Bridge producer failed to restore tracked source bytes exactly"
    }
    if ([string]::IsNullOrEmpty($oldCargoTargetDir)) { Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue } else { [Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", $oldCargoTargetDir, "Process") }
    if ([string]::IsNullOrEmpty($oldSigningKey)) { Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue } else { [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY", $oldSigningKey, "Process") }
    if ([string]::IsNullOrEmpty($oldSigningPassword)) { Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue } else { [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $oldSigningPassword, "Process") }
    if ([string]::IsNullOrEmpty($oldAuthenticodeMode)) { Remove-Item Env:SKY_AUTHENTICODE_MODE -ErrorAction SilentlyContinue } else { [Environment]::SetEnvironmentVariable("SKY_AUTHENTICODE_MODE", $oldAuthenticodeMode, "Process") }
    Remove-Item -LiteralPath $keyPath, "$keyPath.pub", $configPath -Force -ErrorAction SilentlyContinue
}
