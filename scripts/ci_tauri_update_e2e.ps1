[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$FixtureTargetDir,
  [string]$CandidateInstallerPath,
  [string]$CandidateSignaturePath,
  [string]$CandidateVersion,
  [string]$CandidatePublicKeyPath,
  [string]$BridgeRootPath,
  [string]$BridgeInstallerPath,
  [string]$BridgeSourceSha,
  [string]$BridgeVersion,
  [string]$BridgePublisher,
  [string]$BridgeIdentifier,
  [string]$BridgeSentinelId,
  [string]$BridgeSentinelSha256,
  [string]$EvidencePath,
  [switch]$KeepFixtureOnFailure
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$candidateCargoPath = Join-Path $repoRoot 'desktop/src-tauri/Cargo.toml'
$lockPath = Join-Path $repoRoot 'rust/Cargo.lock'
$identityHelperPath = Join-Path $PSScriptRoot 'updater_candidate_source_identity.ps1'
. $identityHelperPath
$previousVersion = '4.1.3'
$cargoSourceBytes = [IO.File]::ReadAllBytes($candidateCargoPath)
$lockSourceBytes = [IO.File]::ReadAllBytes($lockPath)

# The previous-v4 bridge keeps its explicit historical identity when the
# canonical candidate source moves to a later release version.
$syntheticCargo = "[package]`nname = `"sky_desktop_shell`"`nversion = `"4.0.0-rc.1`"`n"
$syntheticLock = "[[package]]`nname = `"sky_desktop_shell`"`nversion = `"4.0.0-rc.1`"`n"
$expectedCargo = "[package]`nname = `"sky_desktop_shell`"`nversion = `"$previousVersion`"`n"
$expectedLock = "[[package]]`nname = `"sky_desktop_shell`"`nversion = `"$previousVersion`"`n"
if ((Convert-FixtureCargoVersion -Source $syntheticCargo -Version $previousVersion) -ne $expectedCargo -or
    (Convert-FixtureLockVersion -Source $syntheticLock -Version $previousVersion) -ne $expectedLock) {
  throw 'Updater fixture previous-v4 version-independence self-test failed'
}

& (Join-Path $PSScriptRoot 'test_v4_updater_fixture_server.ps1')


$invokeArgs = @{ FixtureTargetDir = $FixtureTargetDir }
foreach ($name in @(
    'CandidateInstallerPath', 'CandidateSignaturePath', 'CandidateVersion', 'CandidatePublicKeyPath',
    'BridgeRootPath', 'BridgeInstallerPath', 'BridgeSourceSha', 'BridgeVersion', 'BridgePublisher', 'BridgeIdentifier',
    'BridgeSentinelId', 'BridgeSentinelSha256')) {
  if ($PSBoundParameters.ContainsKey($name)) {
    $invokeArgs[$name] = Get-Variable -Name $name -ValueOnly
  }
}
if ($PSBoundParameters.ContainsKey('EvidencePath')) {
  $invokeArgs['EvidencePath'] = $EvidencePath
}
if ($KeepFixtureOnFailure) {
  $invokeArgs['KeepFixtureOnFailure'] = $true
}

try {
  $providedBridge = $PSBoundParameters.ContainsKey('BridgeRootPath') -or
    $PSBoundParameters.ContainsKey('BridgeInstallerPath') -or
    $PSBoundParameters.ContainsKey('BridgeSourceSha') -or
    $PSBoundParameters.ContainsKey('BridgeVersion') -or
    $PSBoundParameters.ContainsKey('BridgePublisher') -or
    $PSBoundParameters.ContainsKey('BridgeIdentifier') -or
    $PSBoundParameters.ContainsKey('BridgeSentinelId') -or
    $PSBoundParameters.ContainsKey('BridgeSentinelSha256')
  $coreScriptPath = Join-Path $PSScriptRoot 'ci_tauri_update_e2e_core.ps1'
  $invokeCore = {
    param([hashtable]$CoreArguments)
    & $coreScriptPath @CoreArguments
  }.GetNewClosure()
  Invoke-CandidateUpdaterFixture `
    -RepoRoot $repoRoot `
    -CoreArguments $invokeArgs `
    -ProvidedBridge $providedBridge `
    -HistoricalBridgeVersion $previousVersion `
    -InvokeCore $invokeCore
} finally {
  [IO.File]::WriteAllBytes($candidateCargoPath, $cargoSourceBytes)
  [IO.File]::WriteAllBytes($lockPath, $lockSourceBytes)
  if ((Get-UpdaterFileSha256 -Bytes ([IO.File]::ReadAllBytes($candidateCargoPath))) -ne
    (Get-UpdaterFileSha256 -Bytes $cargoSourceBytes) -or
    (Get-UpdaterFileSha256 -Bytes ([IO.File]::ReadAllBytes($lockPath))) -ne
    (Get-UpdaterFileSha256 -Bytes $lockSourceBytes)) {
    throw 'Updater fixture wrapper failed to restore canonical Cargo manifest bytes'
  }
}
