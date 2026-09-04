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

$resolvedTools = @{}
$missing = @(
    foreach ($name in $toolNames) {
        $command = Get-Command $name -ErrorAction SilentlyContinue
        $source = if ($null -ne $command) { $command.Source }

        if ([string]::IsNullOrWhiteSpace($source) -and $name -ieq 'signtool.exe') {
            $kits = Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue
            if ($null -ne $kits -and -not [string]::IsNullOrWhiteSpace($kits.KitsRoot10)) {
                $matches = @(Get-ChildItem -LiteralPath (Join-Path $kits.KitsRoot10 'bin') -Filter signtool.exe -File -Recurse -ErrorAction SilentlyContinue |
                    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
                    Sort-Object FullName -Descending)
                if ($matches.Count -gt 0) {
                    $source = $matches[0].FullName
                }
            }
        }

        if ([string]::IsNullOrWhiteSpace($source)) {
            $name
        } else {
            $resolvedTools[$name] = $source
        }
    }
)
if ($missing.Count -gt 0) {
    throw "Required Windows CI tool(s) missing before expensive steps: $($missing -join ', ')"
}

foreach ($name in $toolNames) {
    Write-Host ("dependency: {0} -> {1}" -f $name, $resolvedTools[$name])
}
Write-Host ("Windows CI dependency preflight: PASS ({0} tool(s))" -f $toolNames.Count)
