[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$BundleDir,
  [string]$CandidateInstallerPath,
  [string]$CandidateSignaturePath,
  [string]$CandidateVersion,
  [string]$CandidatePublicKeyPath,
  [switch]$KeepFixtureOnFailure
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$candidateCargoPath = Join-Path $repoRoot 'desktop/src-tauri/Cargo.toml'
$lockPath = Join-Path $repoRoot 'rust/Cargo.lock'
$previousVersion = '4.0.0-alpha.1'
$cargoSource = [IO.File]::ReadAllText($candidateCargoPath)
$lockSource = [IO.File]::ReadAllText($lockPath)

function Convert-FixtureCargoVersion {
  param(
    [Parameter(Mandatory = $true)] [string]$Source,
    [Parameter(Mandatory = $true)] [string]$Version
  )
  $pattern = [regex]::new('(?m)^(version = ")[^"]+("(?=\r?$))')
  if ($pattern.Matches($Source).Count -ne 1) {
    throw 'Updater fixture could not uniquely locate the desktop Cargo package version'
  }
  return $pattern.Replace($Source, ('${1}' + $Version + '${2}'), 1)
}

function Convert-FixtureLockVersion {
  param(
    [Parameter(Mandatory = $true)] [string]$Source,
    [Parameter(Mandatory = $true)] [string]$Version
  )
  $pattern = [regex]::new('(?s)(\[\[package\]\]\r?\nname = "sky_desktop_shell"\r?\nversion = ")[^"]+("(?=\r?\n))')
  if ($pattern.Matches($Source).Count -ne 1) {
    throw 'Updater fixture could not uniquely locate the desktop Cargo.lock package version'
  }
  return $pattern.Replace($Source, ('${1}' + $Version + '${2}'), 1)
}

# Regression for the release-prep transition: the previous-v4 bridge must stay
# pinned to alpha.1 even when the canonical candidate source has already moved
# to an RC (or any later canonical SemVer).
$syntheticCargo = "[package]`nname = `"sky_desktop_shell`"`nversion = `"4.0.0-rc.1`"`n"
$syntheticLock = "[[package]]`nname = `"sky_desktop_shell`"`nversion = `"4.0.0-rc.1`"`n"
$expectedCargo = "[package]`nname = `"sky_desktop_shell`"`nversion = `"$previousVersion`"`n"
$expectedLock = "[[package]]`nname = `"sky_desktop_shell`"`nversion = `"$previousVersion`"`n"
if ((Convert-FixtureCargoVersion -Source $syntheticCargo -Version $previousVersion) -ne $expectedCargo -or
    (Convert-FixtureLockVersion -Source $syntheticLock -Version $previousVersion) -ne $expectedLock) {
  throw 'Updater fixture previous-v4 version-independence self-test failed'
}

$invokeArgs = @{ BundleDir = $BundleDir }
foreach ($name in @('CandidateInstallerPath', 'CandidateSignaturePath', 'CandidateVersion', 'CandidatePublicKeyPath')) {
  if ($PSBoundParameters.ContainsKey($name)) {
    $invokeArgs[$name] = Get-Variable -Name $name -ValueOnly
  }
}
if ($KeepFixtureOnFailure) {
  $invokeArgs['KeepFixtureOnFailure'] = $true
}

try {
  $bridgeCargo = Convert-FixtureCargoVersion -Source $cargoSource -Version $previousVersion
  $bridgeLock = Convert-FixtureLockVersion -Source $lockSource -Version $previousVersion
  [IO.File]::WriteAllText($candidateCargoPath, $bridgeCargo, [Text.UTF8Encoding]::new($false))
  [IO.File]::WriteAllText($lockPath, $bridgeLock, [Text.UTF8Encoding]::new($false))

  & (Join-Path $PSScriptRoot 'ci_tauri_update_e2e_core.ps1') @invokeArgs
} finally {
  [IO.File]::WriteAllText($candidateCargoPath, $cargoSource, [Text.UTF8Encoding]::new($false))
  [IO.File]::WriteAllText($lockPath, $lockSource, [Text.UTF8Encoding]::new($false))
}
