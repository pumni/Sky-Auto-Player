[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

$ErrorActionPreference = "Stop"
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\")).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$work = Join-Path $tempRoot ("sky-pyinstaller-bootloader-" + [guid]::NewGuid().ToString("N"))
# Native uv argument parsing on the Windows 2025 runner can discard
# backslashes from absolute paths passed through an argument array. Forward
# slashes are accepted by uv and preserve the project/env-file boundary.
$uvProjectRoot = $projectRoot -replace '\\', '/'
$uvEnvFile = (Join-Path $projectRoot ".env") -replace '\\', '/'

function Invoke-Checked([string]$Program, [string[]]$Arguments, [string]$WorkingDirectory) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE"
    }
}

try {
    $version = (& uv run --project $uvProjectRoot "--env-file=$uvEnvFile" python -c "import importlib.metadata; print(importlib.metadata.version('pyinstaller'))").Trim()
    if ($version -notmatch '^\d+\.\d+\.\d+$') {
        throw "Could not resolve an exact PyInstaller version from the locked environment."
    }
    $tag = "v$version"
    $expectedSourceCommits = @{
        # Keep this allowlist aligned with the exact PyInstaller version in
        # uv.lock. A tag alone is not sufficient provenance for a release
        # bootloader because a mutable tag could resolve to another commit.
        "6.22.2" = "19f42e7f13d56cd880a4ced8bb3594875e5227c6"
    }
    $expectedSourceCommit = $expectedSourceCommits[$version]
    if (-not $expectedSourceCommit) {
        throw "PyInstaller $version has no allowlisted source commit. Update the bootloader provenance allowlist first."
    }
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    $source = Join-Path $work "pyinstaller"
    Invoke-Checked "git" @(
        "clone", "--depth", "1", "--branch", $tag,
        "https://github.com/pyinstaller/pyinstaller.git", $source
    ) $work
    $actualSourceCommit = (& git -C $source rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $actualSourceCommit -ne $expectedSourceCommit) {
        throw "PyInstaller source commit mismatch: expected $expectedSourceCommit, got $actualSourceCommit"
    }
    Push-Location (Join-Path $source "bootloader")
    try {
        Invoke-Checked "uv" @(
            "run", "--project", $uvProjectRoot, "--env-file=$uvEnvFile",
            "python", "waf", "all"
        ) (Get-Location).Path
    } finally {
        Pop-Location
    }

    $candidates = @(Get-ChildItem (Join-Path $source "PyInstaller\bootloader") -Recurse -Filter "run.exe" -File |
        Where-Object { $_.FullName -match "Windows-64bit-intel" })
    if ($candidates.Count -ne 1) {
        throw "Expected exactly one Windows-64bit-intel source-built run.exe; found $($candidates.Count)."
    }
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    Copy-Item -LiteralPath $candidates[0].FullName -Destination (Join-Path $output "run.exe") -Force
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $output "run.exe")).Hash.ToLowerInvariant()
    [ordered]@{
        pyinstaller_version = $version
        source_tag = $tag
        source_commit = $actualSourceCommit
        bootloader = "Windows-64bit-intel/run.exe"
        sha256 = $hash
        built_utc = [DateTime]::UtcNow.ToString("o")
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output "metadata.json") -Encoding UTF8
    Write-Host "Source-built PyInstaller bootloader: $output\run.exe ($hash)"
} finally {
    if (Test-Path -LiteralPath $work) {
        Remove-Item -LiteralPath $work -Recurse -Force
    }
}
