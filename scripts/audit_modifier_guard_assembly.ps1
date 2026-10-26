param(
    [Parameter(Mandatory)]
    [string]$SkyPlayerAssemblyPath,
    [Parameter(Mandatory)]
    [string]$Win32AssemblyPath
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cargoArguments = @(
    '--quiet',
    'xtask',
    'audit',
    'assembly',
    'modifier-guard',
    '--sky-player-assembly-path',
    [System.IO.Path]::GetFullPath($SkyPlayerAssemblyPath),
    '--win32-assembly-path',
    [System.IO.Path]::GetFullPath($Win32AssemblyPath)
)

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
