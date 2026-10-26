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
