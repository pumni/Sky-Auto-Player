[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\.benchmarks\physical-key-study'),
    [ValidateRange(100, 100000)]
    [int]$Iterations = 300,
    [ValidateRange(1, 1000000)]
    [int]$DueUs = 5000
)

$ErrorActionPreference = 'Stop'
throw 'BLOCKED-LEGACY-PHYSICAL-STUDY: the coordinator benchmark was removed. A production-equivalent DispatchExecutor study contract requires Leader review.'