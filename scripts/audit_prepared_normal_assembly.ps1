# Historical command name retained as a compatibility adapter. The audit now
# inspects DispatchExecutor::dispatch_staged_candidate, not a retired
# prepared-normal scheduler.
param(
    [string]$AssemblyPath = ''
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cargoArguments = @('--quiet', 'xtask', 'audit', 'assembly', 'prepared-normal')
if (-not [string]::IsNullOrWhiteSpace($AssemblyPath)) {
    $cargoArguments += @('--assembly-path', [System.IO.Path]::GetFullPath($AssemblyPath))
}

Push-Location -LiteralPath $repositoryRoot
$exitCode = 0
try {
    & cargo @cargoArguments
    $exitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}
exit $exitCode
