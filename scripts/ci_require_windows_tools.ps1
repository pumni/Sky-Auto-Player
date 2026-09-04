[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string[]]$Tool
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$missing = @(
    foreach ($name in $Tool) {
        if ($null -eq (Get-Command $name -ErrorAction SilentlyContinue)) {
            $name
        }
    }
)
if ($missing.Count -gt 0) {
    throw "Required Windows CI tool(s) missing before expensive steps: $($missing -join ', ')"
}

foreach ($name in $Tool) {
    $command = Get-Command $name -ErrorAction Stop
    Write-Host ("dependency: {0} -> {1}" -f $name, $command.Source)
}
Write-Host ("Windows CI dependency preflight: PASS ({0} tool(s))" -f $Tool.Count)
