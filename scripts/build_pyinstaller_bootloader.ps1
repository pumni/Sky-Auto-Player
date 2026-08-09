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

function Invoke-Checked([string]$Program, [string[]]$Arguments, [string]$WorkingDirectory) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE"
    }
}

try {
    $version = (& uv run --project $projectRoot --env-file (Join-Path $projectRoot ".env") python -c "import importlib.metadata; print(importlib.metadata.version('pyinstaller'))").Trim()
    if ($version -notmatch '^\d+\.\d+\.\d+$') {
        throw "Could not resolve an exact PyInstaller version from the locked environment."
    }
    $tag = "v$version"
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    $source = Join-Path $work "pyinstaller"
    Invoke-Checked "git" @(
        "clone", "--depth", "1", "--branch", $tag,
        "https://github.com/pyinstaller/pyinstaller.git", $source
    ) $work
    Push-Location (Join-Path $source "bootloader")
    try {
        Invoke-Checked "uv" @(
            "run", "--project", $projectRoot, "--env-file", (Join-Path $projectRoot ".env"),
            "python", "waf", "all"
        ) (Get-Location).Path
    } finally {
        Pop-Location
    }

    $candidates = @(Get-ChildItem (Join-Path $source "bootloader\build") -Recurse -Filter "run.exe" -File |
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
