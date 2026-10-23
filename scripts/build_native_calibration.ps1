[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$manifestPath = Join-Path $repoRoot "rust/Cargo.toml"
$targetTriple = "x86_64-pc-windows-msvc"

function Fail([string]$Message) {
    throw "Native calibration package handoff failed: $Message"
}

function Get-PeMachine([string]$Path) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($stream.Length -lt 0x40) { Fail "built calibration binary is not a complete PE file" }
        $stream.Position = 0
        if ([Text.Encoding]::ASCII.GetString($reader.ReadBytes(2)) -cne "MZ") {
            Fail "built calibration binary is missing the DOS header"
        }
        $stream.Position = 0x3c
        $peOffset = $reader.ReadInt32()
        if ($peOffset -lt 0x40 -or $peOffset -gt ($stream.Length - 6)) {
            Fail "built calibration binary has an invalid PE header offset"
        }
        $stream.Position = $peOffset
        $signature = @($reader.ReadByte(), $reader.ReadByte(), $reader.ReadByte(), $reader.ReadByte())
        if ($signature[0] -ne 0x50 -or $signature[1] -ne 0x45 -or
            $signature[2] -ne 0 -or $signature[3] -ne 0) {
            Fail "built calibration binary is missing the PE signature"
        }
        return [int]$reader.ReadUInt16()
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    Fail "Rust workspace manifest is missing: $manifestPath"
}

& cargo build --locked --manifest-path $manifestPath --package sky_dispatch_win32 --bin native_calibration --target $targetTriple --profile dist
if ($LASTEXITCODE -ne 0) {
    Fail "pinned x86_64 dist build failed with exit code $LASTEXITCODE"
}

$metadataOutput = @(& cargo metadata --locked --no-deps --format-version 1 --manifest-path $manifestPath 2>&1)
if ($LASTEXITCODE -ne 0) {
    Fail "Cargo target-directory discovery failed with exit code $LASTEXITCODE"
}
try {
    $cargoMetadata = ($metadataOutput -join [Environment]::NewLine) | ConvertFrom-Json
} catch {
    Fail "Cargo target-directory discovery returned invalid metadata"
}
if ([string]::IsNullOrWhiteSpace([string]$cargoMetadata.target_directory)) {
    Fail "Cargo metadata did not provide a target directory"
}
$sourceBinary = Join-Path ([string]$cargoMetadata.target_directory) "$targetTriple/dist/native_calibration.exe"
if (-not (Test-Path -LiteralPath $sourceBinary -PathType Leaf)) {
    Fail "expected x86_64 dist binary is missing: $sourceBinary"
}
$sourceItem = Get-Item -LiteralPath $sourceBinary -ErrorAction Stop
if (($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $sourceItem.Length -le 0) {
    Fail "built calibration binary must be a non-empty regular file"
}
if ((Get-PeMachine $sourceItem.FullName) -ne 0x8664) {
    Fail "built calibration binary is not an x86_64 PE executable"
}

$resourcesRoot = Join-Path $repoRoot "desktop/src-tauri/resources"
if (-not (Test-Path -LiteralPath $resourcesRoot -PathType Container)) {
    New-Item -ItemType Directory -Path $resourcesRoot -Force | Out-Null
}
$resourcesItem = Get-Item -LiteralPath $resourcesRoot -ErrorAction Stop
if (($resourcesItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    Fail "Tauri resource staging directory must not be a reparse point"
}
$stagedBinary = Join-Path $resourcesRoot "native_calibration.exe"
if (Test-Path -LiteralPath $stagedBinary) {
    $stagedItem = Get-Item -LiteralPath $stagedBinary -ErrorAction Stop
    if ($stagedItem.PSIsContainer -or (($stagedItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        Fail "Tauri calibration resource must be a regular file before replacement"
    }
    Remove-Item -LiteralPath $stagedBinary -Force
}
Copy-Item -LiteralPath $sourceItem.FullName -Destination $stagedBinary
$stagedItem = Get-Item -LiteralPath $stagedBinary -ErrorAction Stop
$sha256 = (Get-FileHash -LiteralPath $stagedBinary -Algorithm SHA256).Hash.ToLowerInvariant()
if ($stagedItem.Length -ne $sourceItem.Length -or (Get-PeMachine $stagedItem.FullName) -ne 0x8664) {
    Fail "staged calibration resource differs from the validated x86_64 build"
}

Write-Output "Native calibration package handoff: PASS (target=$targetTriple; size=$($stagedItem.Length); sha256=$sha256; path=$stagedBinary)"
