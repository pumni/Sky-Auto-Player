[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string[]]$Tool
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$toolNames = @(
    foreach ($entry in $Tool) {
        foreach ($name in ($entry -split ',')) {
            $candidate = $name.Trim()
            if (-not [string]::IsNullOrWhiteSpace($candidate)) {
                $candidate
            }
        }
    }
)

$missing = @(
    foreach ($name in $toolNames) {
        if ($null -eq (Get-Command $name -ErrorAction SilentlyContinue)) {
            $name
        }
    }
)
if ($missing.Count -gt 0) {
    throw "Required Windows CI tool(s) missing before expensive steps: $($missing -join ', ')"
}

foreach ($name in $toolNames) {
    $command = Get-Command $name -ErrorAction Stop
    Write-Host ("dependency: {0} -> {1}" -f $name, $command.Source)
}
Write-Host ("Windows CI dependency preflight: PASS ({0} tool(s))" -f $toolNames.Count)
