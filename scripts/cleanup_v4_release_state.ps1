[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string[]]$StateRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
    throw "V4 release state cleanup failed closed: $Message"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$workspace = if ([string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE)) {
    $repoRoot
} else {
    [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
}
if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { Fail "RUNNER_TEMP is unavailable" }
$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
$runnerTempPrefix = $runnerTemp.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
$workspacePrefix = $workspace.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
$repoPrefix = $repoRoot.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar

foreach ($root in $StateRoot) {
    if ([string]::IsNullOrWhiteSpace($root)) { Fail "StateRoot is required" }
    $statePath = [IO.Path]::GetFullPath($root)
    if ($statePath.Equals($workspace, [StringComparison]::OrdinalIgnoreCase) -or
        $statePath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $statePath.Equals($repoRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $statePath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "refusing to remove a path inside the repository workspace"
    }
    if (-not ($statePath.Equals($runnerTemp, [StringComparison]::OrdinalIgnoreCase) -or
        $statePath.StartsWith($runnerTempPrefix, [StringComparison]::OrdinalIgnoreCase))) {
        Fail "refusing to remove a path outside RUNNER_TEMP"
    }
    if (Test-Path -LiteralPath $statePath -PathType Leaf) {
        Fail "refusing to remove a state file where a directory is required"
    }
    if (Test-Path -LiteralPath $statePath -PathType Container) {
        Remove-Item -LiteralPath $statePath -Recurse -Force
    }
}

Write-Host "V4 release state cleanup: PASS (runner-local state roots removed)"
