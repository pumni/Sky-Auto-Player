[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$UpdaterPrivateKeyPath,

    [Parameter(Mandatory = $true)]
    [string[]]$StateRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
    throw "V4 release runner boundary failed closed: $Message"
}

function Get-FullPath([string]$Path, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Path)) { Fail "$Name is required" }
    try {
        return [IO.Path]::GetFullPath($Path)
    } catch {
        Fail "$Name is not a valid path"
    }
}

function Assert-OutsideWorkspace([string]$Path, [string]$Name, [string]$Workspace) {
    $workspacePrefix = $Workspace.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
    if ($Path.Equals($Workspace, [StringComparison]::OrdinalIgnoreCase) -or
        $Path.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "$Name must be outside the repository workspace"
    }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$workspace = if ([string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE)) {
    $repoRoot
} else {
    Get-FullPath $env:GITHUB_WORKSPACE "GITHUB_WORKSPACE"
}
$keyPath = Get-FullPath $UpdaterPrivateKeyPath "UpdaterPrivateKeyPath"
if (-not (Test-Path -LiteralPath $keyPath -PathType Leaf)) {
    Fail "updater private key path is not a file"
}
$keyPath = (Resolve-Path -LiteralPath $keyPath).Path
Assert-OutsideWorkspace $keyPath "updater private key" $workspace
Assert-OutsideWorkspace $keyPath "updater private key" $repoRoot

if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { Fail "RUNNER_TEMP is unavailable" }
$runnerTemp = Get-FullPath $env:RUNNER_TEMP "RUNNER_TEMP"
$runnerTempPrefix = $runnerTemp.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar

foreach ($root in $StateRoot) {
    $statePath = Get-FullPath $root "StateRoot"
    Assert-OutsideWorkspace $statePath "StateRoot" $workspace
    Assert-OutsideWorkspace $statePath "StateRoot" $repoRoot
    if (-not ($statePath.Equals($runnerTemp, [StringComparison]::OrdinalIgnoreCase) -or
        $statePath.StartsWith($runnerTempPrefix, [StringComparison]::OrdinalIgnoreCase))) {
        Fail "StateRoot must remain under RUNNER_TEMP"
    }
    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        Fail "StateRoot must be a directory"
    }
    if ((Test-Path -LiteralPath $statePath -PathType Container) -and
        @(Get-ChildItem -LiteralPath $statePath -Force).Count -ne 0) {
        Fail "StateRoot must be empty before release execution"
    }
    New-Item -ItemType Directory -Path $statePath -Force | Out-Null
}

$gitConfig = Join-Path $workspace ".git/config"
if (Test-Path -LiteralPath $gitConfig -PathType Leaf) {
    $gitConfigText = Get-Content -LiteralPath $gitConfig -Raw
    if ($gitConfigText -match "(?i)(http\.extraheader|x-access-token|authorization:)" ) {
        Fail "source checkout contains persisted GitHub credentials"
    }
}

Write-Host "V4 release runner boundary: PASS (dedicated runner paths and checkout credential hygiene verified)"
