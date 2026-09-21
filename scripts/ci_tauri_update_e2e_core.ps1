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
$desktopRoot = Join-Path $repoRoot 'desktop'
$tauriConfigPath = Join-Path $desktopRoot 'src-tauri/tauri.conf.json'
. (Join-Path $PSScriptRoot 'v4_nsis_smoke_boundary.ps1')
$smokeScope = $null
$runnerTemp = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
  [IO.Path]::GetTempPath()
} else {
  $env:RUNNER_TEMP
}
$summaryPath = if ([string]::IsNullOrWhiteSpace($env:GITHUB_STEP_SUMMARY)) {
  Join-Path $runnerTemp 'sky-auto-player-tauri-update-summary.md'
} else {
  $env:GITHUB_STEP_SUMMARY
}
$fixtureRoot = Join-Path $runnerTemp ('sky-auto-player-tauri-update-' + [guid]::NewGuid().ToString('N'))
$fixtureTargetRoot = [IO.Path]::GetFullPath($FixtureTargetDir)
$repoPrefix = $repoRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
if ($fixtureTargetRoot.Equals($repoRoot, [StringComparison]::OrdinalIgnoreCase) -or
  $fixtureTargetRoot.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Updater fixture target directory must be outside the repository workspace'
}
$bridgeTargetRoot = Join-Path $fixtureTargetRoot 'bridge'
$candidateTargetRoot = Join-Path $fixtureTargetRoot 'candidate'
$bridgeBundleRoot = Join-Path $bridgeTargetRoot 'dist/bundle/nsis'
$candidateBundleRoot = Join-Path $candidateTargetRoot 'dist/bundle/nsis'
$installRoot = Join-Path $fixtureRoot 'installed'
$preservedBridgeRoot = Join-Path $fixtureRoot 'preserved-bridge'
$appDataRoot = Join-Path $fixtureRoot 'app-data'
$userSongsRoot = Join-Path $appDataRoot 'songs'
$userSongPath = Join-Path $userSongsRoot 'updater-preserved-user.json'
$markerPath = Join-Path $fixtureRoot 'completion.txt'
$expectedVersionPath = Join-Path $fixtureRoot 'expected-installed-version.txt'
$cutoverMarkerPath = Join-Path $fixtureRoot 'cutover.txt'
$safetyPath = Join-Path $fixtureRoot 'safety.txt'
$stopPath = Join-Path $fixtureRoot 'stop-server'
$manifestPath = Join-Path $fixtureRoot 'manifest.json'
$oldManifestPath = Join-Path $fixtureRoot 'old-manifest.json'
$bridgeConfigPath = Join-Path $fixtureRoot 'bridge-updater.json'
$cutoverConfigPath = Join-Path $fixtureRoot 'cutover-updater.json'
$oldKeyPath = Join-Path $fixtureRoot 'old.key'
$newKeyPath = Join-Path $fixtureRoot 'new.key'
$oldSignaturePath = Join-Path $fixtureRoot 'old.sig'
$candidateForOldSigningPath = Join-Path $fixtureRoot 'candidate-for-old-signing.exe'
$providedBridge = -not [string]::IsNullOrWhiteSpace($BridgeRootPath) -or
  -not [string]::IsNullOrWhiteSpace($BridgeInstallerPath) -or
  -not [string]::IsNullOrWhiteSpace($BridgeSourceSha) -or
  -not [string]::IsNullOrWhiteSpace($BridgeVersion) -or
  -not [string]::IsNullOrWhiteSpace($BridgePublisher) -or
  -not [string]::IsNullOrWhiteSpace($BridgeIdentifier) -or
  -not [string]::IsNullOrWhiteSpace($BridgeSentinelId) -or
  -not [string]::IsNullOrWhiteSpace($BridgeSentinelSha256)
if ($providedBridge) {
  if ([string]::IsNullOrWhiteSpace($BridgeRootPath) -or
    [string]::IsNullOrWhiteSpace($BridgeInstallerPath) -or
    [string]::IsNullOrWhiteSpace($BridgeSourceSha) -or
    [string]::IsNullOrWhiteSpace($BridgeVersion) -or
    [string]::IsNullOrWhiteSpace($BridgePublisher) -or
    [string]::IsNullOrWhiteSpace($BridgeIdentifier) -or
    [string]::IsNullOrWhiteSpace($BridgeSentinelId) -or
    [string]::IsNullOrWhiteSpace($BridgeSentinelSha256)) {
    throw 'Provided-bridge updater qualification requires root, installer, source SHA, version, sentinel ID, and sentinel SHA'
  }
  if ($BridgeSourceSha -notmatch '^[0-9a-fA-F]{40}$' -or $BridgeSourceSha -match '^0{40}$') {
    throw 'Provided-bridge updater qualification received an invalid source SHA'
  }
  & cargo xtask version check --version $BridgeVersion --no-repo-match *> $null
  if ($LASTEXITCODE -ne 0) {
    throw "Provided-bridge updater qualification received a non-canonical SemVer: $BridgeVersion"
  }
  if ($BridgeSentinelSha256 -notmatch '^[0-9a-fA-F]{64}$') {
    throw 'Provided-bridge updater qualification received an invalid sentinel SHA'
  }
}
$providedCandidate = -not [string]::IsNullOrWhiteSpace($CandidateInstallerPath) -or
  -not [string]::IsNullOrWhiteSpace($CandidateSignaturePath) -or
  -not [string]::IsNullOrWhiteSpace($CandidateVersion) -or
  -not [string]::IsNullOrWhiteSpace($CandidatePublicKeyPath)
if ($providedCandidate) {
  if ([string]::IsNullOrWhiteSpace($CandidateInstallerPath) -or
    [string]::IsNullOrWhiteSpace($CandidateSignaturePath) -or
    [string]::IsNullOrWhiteSpace($CandidateVersion) -or
    [string]::IsNullOrWhiteSpace($CandidatePublicKeyPath)) {
    throw 'Provided-candidate updater qualification requires installer, signature, version, and public-key paths'
  }
  & cargo xtask version check --version $CandidateVersion --no-repo-match *> $null
  if ($LASTEXITCODE -ne 0) {
    throw "Provided-candidate updater qualification received a non-canonical SemVer: $CandidateVersion"
  }
}
$candidateVersion = if ($providedCandidate) { $CandidateVersion } else { '4.0.0-alpha.2' }
if ($providedCandidate -and $candidateVersion -cne '4.1.4') {
  throw "Package migration qualification requires the exact candidate 4.1.4, received $candidateVersion"
}

function Get-HigherSemVer([string]$Version) {
  $match = [regex]::Match($Version, '^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$')
  if (-not $match.Success) { throw "Cannot derive a higher synthetic updater version from $Version" }
  $patch = [int64]$match.Groups[3].Value
  if ($patch -eq [int64]::MaxValue) { throw 'Synthetic updater version patch component overflowed' }
  return "$($match.Groups[1].Value).$($match.Groups[2].Value).$($patch + 1)"
}

$cutoverVersion = Get-HigherSemVer $candidateVersion
$requestLogPath = Join-Path $fixtureRoot 'http-requests.jsonl'
$httpEvidencePath = if ([string]::IsNullOrWhiteSpace($EvidencePath)) {
  Join-Path $fixtureRoot 'fixture-http-evidence.json'
} else {
  [IO.Path]::GetFullPath($EvidencePath)
}
$manifestContract = [ordered]@{ status = 'not-checked' }
$candidateContract = [ordered]@{ status = 'not-checked' }
$preservationContract = [ordered]@{ status = 'not-checked' }
$migrationContract = [ordered]@{ status = 'not-checked' }
$fixtureStatus = 'FAIL'
$port = 0
$serverJob = $null
$previousInstallerCopy = Join-Path $fixtureRoot 'previous-v4-setup.exe'
$candidateArchive = $null
$candidateSignature = $null
$candidateCargoPath = Join-Path $desktopRoot 'src-tauri/Cargo.toml'
$lockPath = Join-Path $repoRoot 'rust/Cargo.lock'
$tauriConfigSource = [IO.File]::ReadAllText($tauriConfigPath)
$tauriConfig = $tauriConfigSource | ConvertFrom-Json
$permanentIdentifier = 'io.github.pumni.skyautoplayer'
$candidatePublisher = 'pumni'
$previousBridgeVersion = if ([string]::IsNullOrWhiteSpace($BridgeVersion)) { '4.1.3' } else { $BridgeVersion }
if ([string]$tauriConfig.identifier -cne $permanentIdentifier -or
  [string]$tauriConfig.bundle.publisher -cne $candidatePublisher) {
  throw 'Updater candidate source identity is not the exact 4.1.4 / pumni package contract'
}
$oldAppDataRoot = [Environment]::GetEnvironmentVariable('SKY_APP_DATA_ROOT', 'Process')
$cargoSource = [IO.File]::ReadAllText($candidateCargoPath)
$lockSource = [IO.File]::ReadAllText($lockPath)

function Get-DisposableLoopbackPort {
  $probe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
  try {
    $probe.Start()
    return ([Net.IPEndPoint]$probe.LocalEndpoint).Port
  } finally {
    $probe.Stop()
  }
}

function Get-ByteSha256([byte[]]$Bytes) {
  return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

$catalogManifestPath = Join-Path $repoRoot 'builtin-songs/manifest.json'
$catalogManifestSourceBytes = [IO.File]::ReadAllBytes($catalogManifestPath)
$catalogManifestDocument = Get-Content -LiteralPath $catalogManifestPath -Raw | ConvertFrom-Json
$catalogSentinelEntries = @($catalogManifestDocument.songs | Where-Object {
    [string]$_.path -like 'sheets/*.json'
  })
if ($catalogSentinelEntries.Count -lt 1) {
  throw 'Updater fixture requires at least one canonical JSON built-in song for the N sentinel'
}
$catalogSentinelEntry = $catalogSentinelEntries[0]
$catalogSentinelId = [string]$catalogSentinelEntry.id
$catalogSentinelRelativePath = ([string]$catalogSentinelEntry.path).Substring('sheets/'.Length)
if ([string]::IsNullOrWhiteSpace($catalogSentinelRelativePath) -or
  $catalogSentinelRelativePath.Contains('..')) {
  throw 'Updater fixture selected an invalid built-in sentinel path'
}
$catalogSongPath = Join-Path (Join-Path $repoRoot 'songs') $catalogSentinelRelativePath
if (-not (Test-Path -LiteralPath $catalogSongPath -PathType Leaf)) {
  throw "Updater fixture sentinel source is missing: $catalogSongPath"
}
$catalogSongSourceBytes = [IO.File]::ReadAllBytes($catalogSongPath)
$catalogSentinelSongBytes = [Text.Encoding]::UTF8.GetBytes(
  '{"name":"Updater bridge catalog sentinel","bpm":120,"songNotes":[{"time":0,"key":"1Key0"},{"time":333,"key":"1Key1"}]}'
)
$catalogSentinelSongSha = Get-ByteSha256 $catalogSentinelSongBytes
$catalogBridgeEvidence = $null
$catalogCandidateEvidence = $null
$catalogSourceRestoreStatus = if ($providedBridge) { 'PASS' } else { 'not-started' }
$catalogSourceRestoreError = $null

function Convert-FixturePublisher([string]$Source, [string]$Publisher) {
  $pattern = [regex]::new('(?m)^(\s*"publisher"\s*:\s*")[^"]+("\s*,?\s*$)')
  if ($pattern.Matches($Source).Count -ne 1) {
    throw 'Updater fixture could not uniquely locate the Tauri bundle publisher'
  }
  $converted = $pattern.Replace($Source, ('${1}' + $Publisher + '${2}'), 1)
  if ($Publisher -ceq 'github') {
    $hookPattern = [regex]::new('(?m)^\s*"installerHooks"\s*:\s*"[^"]+",?\r?\n')
    if ($hookPattern.Matches($converted).Count -ne 1) {
      throw 'Historical bridge build could not uniquely locate the migration installer hook'
    }
    $converted = $hookPattern.Replace($converted, '', 1)
    $commaPattern = [regex]::new('(?m)^(\s*"installMode"\s*:\s*"[^"]+"),\r?\n(\s*})')
    $converted = $commaPattern.Replace($converted, ('${1}' + [Environment]::NewLine + '${2}'), 1)
  }
  return $converted
}

function Set-CatalogBridgeSentinel {
  $manifest = Get-Content -LiteralPath $catalogManifestPath -Raw | ConvertFrom-Json
  $entries = @($manifest.songs | Where-Object { [string]$_.id -eq $catalogSentinelId })
  if ($entries.Count -ne 1 -or [string]$entries[0].path -ne [string]$catalogSentinelEntry.path) {
    throw 'Updater fixture could not locate its selected built-in catalog sentinel entry'
  }
  $entries[0].sha256 = $catalogSentinelSongSha
  $manifestBytes = [Text.Encoding]::UTF8.GetBytes(($manifest | ConvertTo-Json -Depth 8) + [Environment]::NewLine)
  [IO.File]::WriteAllBytes($catalogManifestPath, $manifestBytes)
  [IO.File]::WriteAllBytes($catalogSongPath, $catalogSentinelSongBytes)
  Write-Host "Updater fixture catalog-sentinel: bridge N uses stable ID $catalogSentinelId with sentinel content SHA $catalogSentinelSongSha"
}

function Restore-CanonicalBuiltinCatalog {
  [IO.File]::WriteAllBytes($catalogManifestPath, $catalogManifestSourceBytes)
  [IO.File]::WriteAllBytes($catalogSongPath, $catalogSongSourceBytes)
  $manifestSha = Get-ByteSha256 ([IO.File]::ReadAllBytes($catalogManifestPath))
  $songSha = Get-ByteSha256 ([IO.File]::ReadAllBytes($catalogSongPath))
  if ($manifestSha -ne (Get-ByteSha256 $catalogManifestSourceBytes) -or
    $songSha -ne (Get-ByteSha256 $catalogSongSourceBytes)) {
    throw 'Updater fixture failed to restore canonical built-in catalog source bytes'
  }
  $script:catalogSourceRestoreStatus = 'PASS'
}

function Get-InstalledBuiltinEvidence {
  param([Parameter(Mandatory = $true)] [string]$Root)

  $manifestPath = Join-Path $Root 'manifest.json'
  $manifestBytes = [IO.File]::ReadAllBytes($manifestPath)
  $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
  $entries = @($manifest.songs | Where-Object { [string]$_.id -eq $catalogSentinelId })
  if ($entries.Count -ne 1 -or [string]$entries[0].path -ne [string]$catalogSentinelEntry.path) {
    throw 'Installed built-in catalog does not preserve the selected sentinel identity/path'
  }
  $songPath = Join-Path $Root ([string]$entries[0].path)
  $songBytes = [IO.File]::ReadAllBytes($songPath)
  [pscustomobject]@{
    manifest_sha256 = Get-ByteSha256 $manifestBytes
    selected_id = [string]$entries[0].id
    selected_content_sha256 = Get-ByteSha256 $songBytes
    selected_manifest_sha256 = [string]$entries[0].sha256
  }
}

function Get-V4MigrationRegistryValue {
  param(
    [Parameter(Mandatory = $true)] [string]$Path,
    [Parameter(Mandatory = $true)] [string]$Name
  )
  $item = Get-ItemProperty -LiteralPath $Path -ErrorAction Stop
  $property = $item.PSObject.Properties[$Name]
  if ($null -eq $property) { return $null }
  return [string]$property.Value
}

function Get-V4MigrationUninstallEvidence {
  param([Parameter(Mandatory = $true)] [string]$ExpectedInstallRoot)

  $uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Sky Auto Player'
  if (-not (Test-Path -LiteralPath $uninstallKey)) {
    throw "Migration package did not create the permanent uninstall identity: $uninstallKey"
  }
  $duplicateUninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.pumni.skyautoplayer'
  if (Test-Path -LiteralPath $duplicateUninstallKey) {
    throw "Migration package created a duplicate identifier uninstall identity: $duplicateUninstallKey"
  }
  $installLocation = Get-V4MigrationRegistryValue -Path $uninstallKey -Name 'InstallLocation'
  $uninstallString = Get-V4MigrationRegistryValue -Path $uninstallKey -Name 'UninstallString'
  $normalizedInstallLocation = $installLocation.Trim().Trim('"')
  if ([string]::IsNullOrWhiteSpace($installLocation) -or
    [IO.Path]::GetFullPath($normalizedInstallLocation).TrimEnd('\') -ine [IO.Path]::GetFullPath($ExpectedInstallRoot).TrimEnd('\')) {
    throw "Migration uninstall identity points at an unexpected install root: $installLocation"
  }
  if ([string]::IsNullOrWhiteSpace($uninstallString) -or
    $uninstallString -notmatch [regex]::Escape((Join-Path $ExpectedInstallRoot 'uninstall.exe'))) {
    throw "Migration uninstall identity does not point at the installed uninstaller: $uninstallString"
  }
  return [ordered]@{
    key = $uninstallKey
    publisher = Get-V4MigrationRegistryValue -Path $uninstallKey -Name 'Publisher'
    display_version = Get-V4MigrationRegistryValue -Path $uninstallKey -Name 'DisplayVersion'
    install_location = $normalizedInstallLocation
    uninstall_string = $uninstallString
  }
}

function Assert-V4MigrationPublisherState {
  param(
    [Parameter(Mandatory = $true)] [string]$ExpectedPublisher,
    [Parameter(Mandatory = $true)] [string]$ExpectedVersion,
    [Parameter(Mandatory = $true)] [string]$ExpectedInstallRoot
  )
  $evidence = Get-V4MigrationUninstallEvidence -ExpectedInstallRoot $ExpectedInstallRoot
  if ([string]$evidence.publisher -cne $ExpectedPublisher -or
    [string]$evidence.display_version -cne $ExpectedVersion) {
    throw "Migration uninstall identity has unexpected publisher/version: publisher=$($evidence.publisher); version=$($evidence.display_version); expected=$ExpectedPublisher/$ExpectedVersion"
  }
  $legacyPublisherKey = 'HKCU:\Software\github\Sky Auto Player'
  if ($ExpectedPublisher -cne 'github' -and (Test-Path -LiteralPath $legacyPublisherKey)) {
    $legacyItem = Get-Item -LiteralPath $legacyPublisherKey -ErrorAction Stop
    $legacyDefault = $legacyItem.GetValue('', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if (-not [string]::IsNullOrWhiteSpace([string]$legacyDefault) -and
      [IO.Path]::GetFullPath([string]$legacyDefault).TrimEnd('\') -ieq [IO.Path]::GetFullPath($ExpectedInstallRoot).TrimEnd('\')) {
      throw "Migration left a historical publisher registry pointer at the new install root: $legacyPublisherKey"
    }
  }
  return $evidence
}

function Invoke-LoopbackHttpBytes([string]$Uri) {
  $handler = [Net.Http.HttpClientHandler]::new()
  $handler.UseProxy = $false
  $client = [Net.Http.HttpClient]::new($handler)
  try {
    $response = $client.GetAsync($Uri).GetAwaiter().GetResult()
    try {
      $bytes = $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
      $contentType = if ($null -ne $response.Content.Headers.ContentType) {
        [string]$response.Content.Headers.ContentType.MediaType
      } else {
        ''
      }
      $contentLength = $response.Content.Headers.ContentLength
      return [pscustomobject]@{
        status_code = [int]$response.StatusCode
        content_type = $contentType
        content_length = if ($null -eq $contentLength) { [int64]$bytes.Length } else { [int64]$contentLength }
        body_length = [int64]$bytes.Length
        body_sha256 = Get-ByteSha256 $bytes
        body = $bytes
      }
    } finally {
      $response.Dispose()
    }
  } finally {
    $client.Dispose()
    $handler.Dispose()
  }
}

function Assert-ExactHttpResponse {
  param(
    [Parameter(Mandatory = $true)] [object]$Response,
    [Parameter(Mandatory = $true)] [byte[]]$ExpectedBytes,
    [Parameter(Mandatory = $true)] [string]$ExpectedContentType,
    [Parameter(Mandatory = $true)] [string]$Name
  )
  $expectedHash = Get-ByteSha256 $ExpectedBytes
  if ($Response.status_code -ne 200 -or
    $Response.content_type -ne $ExpectedContentType -or
    [int64]$Response.content_length -ne [int64]$ExpectedBytes.Length -or
    [int64]$Response.body_length -ne [int64]$ExpectedBytes.Length -or
    [string]$Response.body_sha256 -ne $expectedHash) {
    throw "$Name HTTP response failed exact contract (status=$($Response.status_code); content_type=$($Response.content_type); content_length=$($Response.content_length); body_length=$($Response.body_length); body_sha256=$($Response.body_sha256))"
  }
  return [ordered]@{
    status = 'PASS'
    status_code = [int]$Response.status_code
    content_type = [string]$Response.content_type
    content_length = [int64]$Response.content_length
    body_length = [int64]$Response.body_length
    body_sha256 = [string]$Response.body_sha256
  }
}

function Write-HttpEvidence([string]$Status) {
  try {
    $requests = @()
    if (Test-Path -LiteralPath $requestLogPath -PathType Leaf) {
      $requests = @(Get-Content -LiteralPath $requestLogPath | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | ForEach-Object { $_ | ConvertFrom-Json })
    }
    $proxy = [ordered]@{
      http_proxy_set = @('HTTP_PROXY', 'http_proxy') | Where-Object {
        -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($_))
      } | Select-Object -First 1 | ForEach-Object { $true }
      https_proxy_set = @('HTTPS_PROXY', 'https_proxy') | Where-Object {
        -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($_))
      } | Select-Object -First 1 | ForEach-Object { $true }
      no_proxy_set = @('NO_PROXY', 'no_proxy') | Where-Object {
        -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($_))
      } | Select-Object -First 1 | ForEach-Object { $true }
      loopback_client_proxy_disabled = $true
    }
    foreach ($proxyName in @('http_proxy_set', 'https_proxy_set', 'no_proxy_set')) {
      if ($null -eq $proxy[$proxyName]) { $proxy[$proxyName] = $false }
    }
    $evidence = [ordered]@{
      schema_version = 1
      status = $Status
      port = [int]$port
      proxy_environment = $proxy
      manifest = $manifestContract
      candidate = $candidateContract
      preservation = $preservationContract
      migration = $migrationContract
      requests = $requests
    }
    $parent = Split-Path -Parent $httpEvidencePath
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    [IO.File]::WriteAllText($httpEvidencePath, ($evidence | ConvertTo-Json -Depth 12) + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
  } catch {
    Write-Warning 'Could not persist sanitized fixture HTTP evidence'
  }
}

function Wait-ForPath {
  param([string]$Path, [int]$TimeoutSeconds = 420)
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  while ([DateTime]::UtcNow -lt $deadline) {
    if (Test-Path -LiteralPath $Path) { return }
    Start-Sleep -Milliseconds 250
  }
  throw "Timed out waiting for $Path"
}

function Write-FixtureUpdaterConfig {
  param(
    [string]$Path,
    [string]$PublicKey
  )
  [ordered]@{
    plugins = [ordered]@{
      updater = [ordered]@{
        pubkey = $PublicKey
        dangerousInsecureTransportProtocol = $true
        # The fixture runtime supplies the actual port and trust roots. This
        # fixed placeholder only satisfies Tauri's bundle-time schema.
        endpoints = @("http://127.0.0.1:1/stable")
      }
    }
  } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Path -Encoding utf8
}

function Test-ProcessPathUnderRoots {
  param(
    [AllowNull()]
    [AllowEmptyString()]
    [string]$Path,
    [Parameter(Mandatory = $true)]
    [string[]]$ResolvedRoots
  )

  if ([string]::IsNullOrWhiteSpace($Path)) { return $false }
  try {
    $full = [IO.Path]::GetFullPath($Path)
  } catch {
    return $false
  }
  foreach ($root in $ResolvedRoots) {
    $prefix = $root + [IO.Path]::DirectorySeparatorChar
    if ($full.Equals($root, [StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
      return $true
    }
  }
  return $false
}

function Stop-UpdaterFixtureProcesses {
  param(
    [Parameter(Mandatory = $true)]
    [string[]]$Roots
  )

  $resolvedRoots = @(
    $Roots |
      Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
      ForEach-Object { [IO.Path]::GetFullPath($_).TrimEnd('\') }
  )

  $errors = [System.Collections.Generic.List[string]]::new()

  $processes = @(
    Get-CimInstance Win32_Process -ErrorAction Stop |
      Where-Object {
        Test-ProcessPathUnderRoots -Path ([string]$_.ExecutablePath) -ResolvedRoots $resolvedRoots
      }
  )

  foreach ($process in $processes) {
    try {
      Stop-Process -Id ([int]$process.ProcessId) -Force -ErrorAction Stop
      Wait-Process -Id ([int]$process.ProcessId) -Timeout 10 -ErrorAction SilentlyContinue
    } catch {
      $errors.Add("PID $($process.ProcessId) '$($process.ExecutablePath)': $($_.Exception.Message)")
    }
  }

  if ($errors.Count -gt 0) {
    throw ("Failed to stop updater fixture process(es): " + ($errors -join ' | '))
  }
}

function Clear-FixtureResourceStaging {
  param([Parameter(Mandatory = $true)] [string]$TargetRoot)
  $stagingRoot = Join-Path $TargetRoot 'dist/builtin-songs'
  if (Test-Path -LiteralPath $stagingRoot) {
    Remove-Item -LiteralPath $stagingRoot -Recurse -Force
  }
}

function Invoke-FixtureBuild {
  param(
    [string]$ConfigPath,
    [string]$PrivateKeyPath,
    [Parameter(Mandatory = $true)] [string]$TargetRoot
  )
  $privateKey = ([IO.File]::ReadAllText($PrivateKeyPath)).Trim()
  if ([string]::IsNullOrWhiteSpace($privateKey)) {
    throw "Updater fixture private key is empty"
  }
  $env:TAURI_SIGNING_PRIVATE_KEY = $privateKey
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
  $oldCargoTargetDir = [Environment]::GetEnvironmentVariable('CARGO_TARGET_DIR', 'Process')
  try {
    [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $TargetRoot, 'Process')
    Clear-FixtureResourceStaging -TargetRoot $TargetRoot
    Push-Location $desktopRoot
    try {
      & bun run tauri build --ci --config $ConfigPath -- --profile dist --features tauri-update-fixture
      if ($LASTEXITCODE -ne 0) { throw "Tauri updater fixture build failed with $LASTEXITCODE" }
    } finally {
      Pop-Location
    }
  } finally {
    if ([string]::IsNullOrEmpty($oldCargoTargetDir)) {
      Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    } else {
      [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $oldCargoTargetDir, 'Process')
    }
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  }
}

try {
  $port = Get-DisposableLoopbackPort
  New-Item -ItemType Directory -Path $fixtureRoot, $installRoot -Force | Out-Null
  New-Item -ItemType File -Path $requestLogPath -Force | Out-Null
  New-Item -ItemType Directory -Path $bridgeBundleRoot, $candidateBundleRoot -Force | Out-Null
  $bridgeBundleRoot = (Resolve-Path -LiteralPath $bridgeBundleRoot -ErrorAction Stop).Path
  $candidateBundleRoot = (Resolve-Path -LiteralPath $candidateBundleRoot -ErrorAction Stop).Path

  foreach ($candidatePath in @($CandidateInstallerPath, $CandidateSignaturePath, $BridgeInstallerPath)) {
    if (-not [string]::IsNullOrWhiteSpace($candidatePath)) {
      $resolvedCandidatePath = [IO.Path]::GetFullPath($candidatePath)
      $fixturePrefix = $fixtureTargetRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
      if ($resolvedCandidatePath.StartsWith($fixturePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Downloaded candidate and bridge paths must remain outside the throwaway fixture target directory'
      }
    }
  }
  if ($providedBridge) {
    & (Join-Path $repoRoot 'scripts/ci_validate_bridge.ps1') `
      -Mode Validate `
      -BridgeRoot $BridgeRootPath `
      -SourceSha $BridgeSourceSha `
      -Version $BridgeVersion `
      -Publisher $BridgePublisher `
      -Identifier $BridgeIdentifier `
      -SentinelId $BridgeSentinelId `
      -SentinelContentSha256 $BridgeSentinelSha256 `
      -RepositoryRoot $repoRoot
    $bridgeMetadata = Get-Content -LiteralPath (Join-Path $BridgeRootPath 'bridge.json') -Raw | ConvertFrom-Json
    $expectedBridgeInstaller = [IO.Path]::GetFullPath((Join-Path $BridgeRootPath ([string]$bridgeMetadata.installer)))
    if ([IO.Path]::GetFullPath($BridgeInstallerPath) -cne $expectedBridgeInstaller) {
      throw 'Provided bridge installer does not match bridge.json'
    }
    if ([string]$bridgeMetadata.sentinel_id -cne $catalogSentinelId -or
      [string]$bridgeMetadata.sentinel_content_sha256 -ine $catalogSentinelSongSha) {
      throw 'Provided bridge sentinel contract does not match the selected catalog sentinel'
    }
  }

  Push-Location $desktopRoot
  try {
    bun run tauri signer generate --ci --password '' --force -w $oldKeyPath *> $null
    if (-not $providedCandidate) {
      bun run tauri signer generate --ci --password '' --force -w $newKeyPath *> $null
    }
  } finally {
    Pop-Location
  }
  $oldPublicKey = ([IO.File]::ReadAllText("$oldKeyPath.pub")).Trim()
  $newPublicKey = if ($providedCandidate) {
    $candidatePublicKey = (Resolve-Path -LiteralPath $CandidatePublicKeyPath -ErrorAction Stop).Path
    $candidatePublicKeyItem = Get-Item -LiteralPath $candidatePublicKey -ErrorAction Stop
    if ($candidatePublicKeyItem.Length -le 0 -or $candidatePublicKeyItem.Length -gt 4096) {
      throw 'Provided candidate public key is empty or unbounded'
    }
    ([IO.File]::ReadAllText($candidatePublicKey)).Trim()
  } else {
    ([IO.File]::ReadAllText("$newKeyPath.pub")).Trim()
  }
  if ([string]::IsNullOrWhiteSpace($oldPublicKey) -or [string]::IsNullOrWhiteSpace($newPublicKey)) {
    throw 'Updater fixture public key generation failed'
  }
  if ($newPublicKey -match 'PRIVATE KEY' -or $newPublicKey.Length -gt 4096) {
    throw 'Updater fixture public key contains forbidden or unbounded material'
  }
  $candidatePublicKeyForRuntime = if ($providedCandidate) {
    [IO.Path]::GetFullPath($CandidatePublicKeyPath)
  } else {
    [IO.Path]::GetFullPath("$newKeyPath.pub")
  }
  $fixtureRuntimeArguments = @(
    '--selftest-update-fixture-port', [string]$port,
    '--selftest-update-fixture-public-key', [IO.Path]::GetFullPath("$oldKeyPath.pub"),
    '--selftest-update-fixture-public-key', $candidatePublicKeyForRuntime
  )
  if (-not $providedBridge) {
    Write-FixtureUpdaterConfig $bridgeConfigPath $oldPublicKey
    if (-not $providedCandidate) {
      Write-FixtureUpdaterConfig $cutoverConfigPath $newPublicKey
    }
  }

  # The ordinary updater consumer installs the exact prebuilt bridge. Only
  # the release/rehearsal fallback builds its throwaway bridge here.
  if ($providedBridge) {
    Copy-Item -LiteralPath $BridgeInstallerPath -Destination $previousInstallerCopy -Force
  } else {
    Set-CatalogBridgeSentinel
    [IO.File]::WriteAllText($tauriConfigPath, (Convert-FixturePublisher -Source $tauriConfigSource -Publisher 'github'), [Text.UTF8Encoding]::new($false))
    Invoke-FixtureBuild $bridgeConfigPath $oldKeyPath $bridgeTargetRoot
    $previousInstallers = @(Get-ChildItem -LiteralPath $bridgeBundleRoot -Filter ("*$previousBridgeVersion*_x64-setup.exe") -File)
    if ($previousInstallers.Count -ne 1) {
      throw "Expected exactly one bridge-v4 installer, found $($previousInstallers.Count)"
    }
    Copy-Item -LiteralPath $previousInstallers[0].FullName -Destination $previousInstallerCopy -Force
    Restore-CanonicalBuiltinCatalog
    [IO.File]::WriteAllText($tauriConfigPath, $tauriConfigSource, [Text.UTF8Encoding]::new($false))
  }
  if ($catalogSourceRestoreStatus -ne 'PASS') {
    throw 'Updater fixture did not restore the canonical source tree before building N+1'
  }

  if ($providedCandidate) {
    $candidateArchive = Get-Item -LiteralPath (Resolve-Path -LiteralPath $CandidateInstallerPath -ErrorAction Stop).Path -ErrorAction Stop
    $candidateSignature = Get-Item -LiteralPath (Resolve-Path -LiteralPath $CandidateSignaturePath -ErrorAction Stop).Path -ErrorAction Stop
    if ($candidateArchive.Extension.ToLowerInvariant() -ne '.exe' -or
      $candidateSignature.Name -ne "$($candidateArchive.Name).sig") {
      throw 'Provided candidate installer/signature names are not an exact pair'
    }
  } else {
    $cargoCandidate = $cargoSource -replace 'version = "4\.0\.0-alpha\.1"', ('version = "' + $candidateVersion + '"')
    $lockCandidate = [regex]::Replace(
      $lockSource,
      '(?s)(name = "sky_desktop_shell"\r?\nversion = ")4\.0\.0-alpha\.1("\r?\n)',
      '${1}' + $candidateVersion + '${2}',
      1
    )
    if ($lockCandidate -eq $lockSource) { throw 'Could not locate the desktop package in Cargo.lock' }
    [IO.File]::WriteAllText($candidateCargoPath, $cargoCandidate, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($lockPath, $lockCandidate, [Text.UTF8Encoding]::new($false))
    Invoke-FixtureBuild $cutoverConfigPath $newKeyPath $candidateTargetRoot

    $candidateArchives = @(Get-ChildItem -LiteralPath $candidateBundleRoot -Filter ("*" + $candidateVersion + "*-setup.exe") -File)
    if ($candidateArchives.Count -ne 1) {
      throw "Expected exactly one new-root candidate installer, found $($candidateArchives.Count)"
    }
    $candidateArchive = $candidateArchives[0]
    $candidateSignature = Get-Item -LiteralPath ($candidateArchive.FullName + '.sig') -ErrorAction Stop
  }
  $signatureText = ([IO.File]::ReadAllText($candidateSignature.FullName)).Trim()
  if ([string]::IsNullOrWhiteSpace($signatureText)) { throw 'Candidate updater signature is empty' }

  # Sign a disposable copy of the exact candidate bytes with the old root only.
  # The copied bytes are hashed before and after signing so the old-root
  # rejection path cannot quietly introduce a second current candidate.
  $candidateInstallerSha256 = Get-ByteSha256 ([IO.File]::ReadAllBytes($candidateArchive.FullName))
  Copy-Item -LiteralPath $candidateArchive.FullName -Destination $candidateForOldSigningPath -Force
  $oldSigningInstallerSha256 = Get-ByteSha256 ([IO.File]::ReadAllBytes($candidateForOldSigningPath))
  if ($oldSigningInstallerSha256 -ne $candidateInstallerSha256) {
    throw "Old-root rejection copy changed the current candidate bytes: candidate=$candidateInstallerSha256 old-signing-copy=$oldSigningInstallerSha256"
  }
  Push-Location $desktopRoot
  try {
    bun run tauri signer sign --private-key-path $oldKeyPath --password '' $candidateForOldSigningPath *> $null
  } finally {
    Pop-Location
  }
  $oldSigningInstallerSha256 = Get-ByteSha256 ([IO.File]::ReadAllBytes($candidateForOldSigningPath))
  if ($oldSigningInstallerSha256 -ne $candidateInstallerSha256) {
    throw "Old-root signing changed the disposable installer bytes: candidate=$candidateInstallerSha256 old-signing-copy=$oldSigningInstallerSha256"
  }
  Move-Item -LiteralPath "$candidateForOldSigningPath.sig" -Destination $oldSignaturePath -Force
  $oldSignatureText = ([IO.File]::ReadAllText($oldSignaturePath)).Trim()
  if ([string]::IsNullOrWhiteSpace($oldSignatureText)) { throw 'Old-root updater signature is empty' }

  $newManifest = [ordered]@{
    version = $candidateVersion
    notes = 'Deterministic bridge rotation candidate.'
    pub_date = '2026-09-04T00:00:00Z'
    platforms = [ordered]@{
      # Match the production release metadata manifest exactly. Tauri's
      # updater accepts this platform key and falls back from the bundle
      # specific target when the packaged runtime does not expose it.
      'windows-x86_64' = [ordered]@{
        signature = $signatureText
        url = "http://127.0.0.1:$port/candidate/update.exe"
      }
    }
  }
  $newManifest | ConvertTo-Json -Depth 8 -Compress | Set-Content -LiteralPath $manifestPath -Encoding utf8
  $oldManifest = [ordered]@{
    version = $cutoverVersion
    notes = 'Old-root rejection candidate.'
    pub_date = '2026-09-04T00:00:00Z'
    platforms = [ordered]@{
      'windows-x86_64' = [ordered]@{
        signature = $oldSignatureText
        url = "http://127.0.0.1:$port/candidate/update.exe"
      }
    }
  }
  $oldManifest | ConvertTo-Json -Depth 8 -Compress | Set-Content -LiteralPath $oldManifestPath -Encoding utf8

  $archivePath = $candidateArchive.FullName
  $serverJob = Start-Job -ScriptBlock {
    param($Port, $ManifestPath, $ArchivePath, $StopPath, $RequestLogPath)
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Parse('127.0.0.1'), $Port)
    $listener.Start()
    try {
      $acceptTask = $listener.AcceptTcpClientAsync()
      while (-not (Test-Path -LiteralPath $StopPath)) {
        if (-not $acceptTask.Wait(250)) { continue }
        $client = $acceptTask.Result
        $acceptTask = $listener.AcceptTcpClientAsync()
        $stream = $null
        try {
          $stream = $client.GetStream()
          $requestBytes = [byte[]]::new(8192)
          $read = $stream.Read($requestBytes, 0, $requestBytes.Length)
          $request = [Text.Encoding]::ASCII.GetString($requestBytes, 0, $read)
          $requestLine = ($request -split "`r?`n", 2)[0]
          $requestParts = $requestLine.Split(' ')
          $method = if ($requestParts.Count -gt 0) { $requestParts[0] } else { '' }
          $path = if ($requestParts.Count -gt 1) { $requestParts[1].Split('?')[0] } else { '' }
          if ($path -eq '/stable' -or $path -eq '/beta') {
            $bytes = [IO.File]::ReadAllBytes($ManifestPath)
            $contentType = 'application/json'
            $status = '200 OK'
          } elseif ($path -eq '/candidate/update.exe') {
            $bytes = [IO.File]::ReadAllBytes($ArchivePath)
            $contentType = 'application/octet-stream'
            $status = '200 OK'
          } else {
            $bytes = [Text.Encoding]::UTF8.GetBytes('not found')
            $contentType = 'text/plain'
            $status = '404 Not Found'
          }
          $requestEvidence = [ordered]@{
            method = $method
            path = $path
            status_code = [int]$status.Split(' ')[0]
            content_type = $contentType
            content_length = [int64]$bytes.Length
            body_sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
          }
          [IO.File]::AppendAllText($RequestLogPath, (($requestEvidence | ConvertTo-Json -Compress) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
          $header = [Text.Encoding]::ASCII.GetBytes("HTTP/1.1 $status`r`nContent-Type: $contentType`r`nContent-Length: $($bytes.Length)`r`nConnection: close`r`n`r`n")
          $stream.Write($header, 0, $header.Length)
          $stream.Write($bytes, 0, $bytes.Length)
          $stream.Flush()
        } finally {
          if ($null -ne $stream) { $stream.Close() }
          $client.Close()
        }
      }
    } finally {
      $listener.Stop()
    }
  } -ArgumentList $port, $manifestPath, $archivePath, $stopPath, $requestLogPath

  $serverReady = $false
  $serverDeadline = [DateTime]::UtcNow.AddSeconds(30)
  while ([DateTime]::UtcNow -lt $serverDeadline) {
    try {
      $response = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$port/stable" -TimeoutSec 2
      if ($response.StatusCode -eq 200) { $serverReady = $true; break }
    } catch { }
    Start-Sleep -Milliseconds 250
  }
  if (-not $serverReady) {
    $serverOutput = Receive-Job -Job $serverJob -Keep | Out-String
    throw "Local signed updater fixture did not become ready. Server job output: $serverOutput"
  }

  $manifestBytes = [IO.File]::ReadAllBytes($manifestPath)
  $manifestDocument = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
  $manifestPlatform = $manifestDocument.platforms.'windows-x86_64'
  $expectedCandidateUrl = "http://127.0.0.1:$port/candidate/update.exe"
  if ($null -eq $manifestPlatform -or
    [string]$manifestDocument.version -ne $candidateVersion -or
    [string]$manifestPlatform.url -ne $expectedCandidateUrl -or
    [string]::IsNullOrWhiteSpace([string]$manifestPlatform.signature)) {
    throw 'Fixture release manifest failed the Tauri-compatible schema contract'
  }
  $manifestResponse = Invoke-LoopbackHttpBytes "http://127.0.0.1:$port/stable"
  $manifestHttp = Assert-ExactHttpResponse $manifestResponse $manifestBytes 'application/json' 'fixture release manifest'
  $manifestContract = [ordered]@{
    status = 'PASS'
    schema_status = 'PASS'
    version = [string]$manifestDocument.version
    platform = 'windows-x86_64'
    candidate_url = [string]$manifestPlatform.url
    signature_present = $true
    http = $manifestHttp
  }
  $candidateBytes = [IO.File]::ReadAllBytes($candidateArchive.FullName)
  if ((Get-ByteSha256 $candidateBytes) -ne $candidateInstallerSha256) {
    throw 'Candidate bytes changed before loopback qualification'
  }
  $candidateResponse = Invoke-LoopbackHttpBytes $expectedCandidateUrl
  $candidateContract = [ordered]@{
    status = 'PASS'
    installer_sha256 = $candidateInstallerSha256
    n_to_n_plus_1_installer_sha256 = $candidateInstallerSha256
    old_root_rejection_copy_sha256 = $oldSigningInstallerSha256
    old_root_rejection_copy_matches = ($candidateInstallerSha256 -eq $oldSigningInstallerSha256)
    http = Assert-ExactHttpResponse $candidateResponse $candidateBytes 'application/octet-stream' 'fixture candidate artifact'
  }
  Write-Host "Fixture HTTP manifest contract: PASS (status=200; content-type=application/json; content-length=$($manifestHttp.content_length); body-sha256=$($manifestHttp.body_sha256))"
  Write-Host "Fixture HTTP candidate contract: PASS (status=200; content-type=application/octet-stream; content-length=$($candidateContract.http.content_length); body-sha256=$($candidateContract.http.body_sha256))"

  $smokeScope = Enter-V4NsisSmokeScope -InstallRoot $installRoot -ManageInstallRootCleanup:$false
  $migrationIdentityPaths = @(
    'HKCU:\Software\github\Sky Auto Player',
    'HKCU:\Software\pumni\Sky Auto Player',
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Sky Auto Player',
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.pumni.skyautoplayer'
  )
  $installerRun = Start-Process -FilePath $previousInstallerCopy -ArgumentList @('/S', '/NS', "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
  if ($installerRun.ExitCode -ne 0) { throw "Bridge-v4 installer exited with $($installerRun.ExitCode)" }
  $appPath = Join-Path $installRoot 'sky_desktop_shell.exe'
  if (-not (Test-Path -LiteralPath $appPath)) { throw "Installed bridge app is missing: $appPath" }
  $bridgeRegistryEvidence = Assert-V4MigrationPublisherState -ExpectedPublisher 'github' -ExpectedVersion $previousBridgeVersion -ExpectedInstallRoot $installRoot
  if (-not (Test-Path -LiteralPath (Join-Path $installRoot 'uninstall.exe') -PathType Leaf)) {
    throw 'Previous migration package did not install its uninstall executable in the bounded install root'
  }
  $bridgeBuiltinRoot = Join-Path $installRoot 'builtin-songs'
  & cargo xtask builtin-catalog verify-installed --root $bridgeBuiltinRoot
  if ($LASTEXITCODE -ne 0) { throw "Bridge installed built-in catalog verification failed with exit code $LASTEXITCODE" }
  $catalogBridgeEvidence = Get-InstalledBuiltinEvidence -Root $bridgeBuiltinRoot
  if ($catalogBridgeEvidence.selected_id -ne $catalogSentinelId -or
    $catalogBridgeEvidence.selected_content_sha256 -ne $catalogSentinelSongSha -or
    $catalogBridgeEvidence.selected_manifest_sha256 -ne $catalogSentinelSongSha) {
    throw 'Bridge installed built-in catalog did not contain the expected N sentinel bytes and hash'
  }
  $bridgeAppSha256 = Get-ByteSha256 ([IO.File]::ReadAllBytes($appPath))
  New-Item -ItemType Directory -Path $preservedBridgeRoot -Force | Out-Null
  foreach ($item in @(Get-ChildItem -LiteralPath $installRoot -Force)) {
    Copy-Item -LiteralPath $item.FullName -Destination $preservedBridgeRoot -Recurse -Force
  }
  $preservedBridgeAppPath = Join-Path $preservedBridgeRoot 'sky_desktop_shell.exe'
  if (-not (Test-Path -LiteralPath $preservedBridgeAppPath -PathType Leaf)) {
    throw "Preserved updater fixture bridge binary is missing: $preservedBridgeAppPath"
  }
  $preservedBridgeAppSha256 = Get-ByteSha256 ([IO.File]::ReadAllBytes($preservedBridgeAppPath))
  if ($preservedBridgeAppSha256 -ne $bridgeAppSha256) {
    throw "Preserved updater fixture bridge bytes changed: original=$bridgeAppSha256 preserved=$preservedBridgeAppSha256"
  }

  $userSongBytes = [Text.Encoding]::UTF8.GetBytes('{"name":"Updater preserved user","songNotes":[{"time":0,"key":"1Key0"}]}')
  New-Item -ItemType Directory -Path $userSongsRoot -Force | Out-Null
  [IO.File]::WriteAllBytes($userSongPath, $userSongBytes)
  $userSongShaBefore = Get-ByteSha256 $userSongBytes
  if ((Get-ByteSha256 ([IO.File]::ReadAllBytes($userSongPath))) -ne $userSongShaBefore) {
    throw 'Could not establish the updater user-song preservation fixture'
  }
  [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $appDataRoot, 'Process')

  $bridgeProcess = Start-Process -FilePath $appPath -ArgumentList (@(
    '--selftest-desktop-update',
    '--selftest-update-marker', $markerPath,
    '--selftest-update-expected-version-file', $expectedVersionPath,
    '--selftest-update-safety-marker', $safetyPath
  ) + $fixtureRuntimeArguments) -WindowStyle Hidden -PassThru
  $smokeScope.TrackedProcesses.Add($bridgeProcess)
  Wait-Process -Id $bridgeProcess.Id -Timeout 180

  # Fixture mode disables only the updater plugin's automatic restart. Wait
  # for the official NSIS transaction to finish, then launch the exact
  # installed candidate so the same packaged binary still proves the update
  # handoff without depending on a headless runner's restart behavior.
  $candidateRegistryEvidence = $null
  $candidateAppSha256 = $null
  $candidateInstallReady = $false
  $lastCandidateInstallStateError = $null
  $candidateInstallDeadline = [DateTime]::UtcNow.AddSeconds(180)
  while ([DateTime]::UtcNow -lt $candidateInstallDeadline) {
    try {
      $candidateRegistryEvidence = Assert-V4MigrationPublisherState -ExpectedPublisher $candidatePublisher -ExpectedVersion $candidateVersion -ExpectedInstallRoot $installRoot
      if (-not (Test-Path -LiteralPath $appPath -PathType Leaf)) {
        throw "Installed candidate app is missing: $appPath"
      }
      $candidateAppSha256 = Get-ByteSha256 ([IO.File]::ReadAllBytes($appPath))
      if ($candidateAppSha256 -ceq $bridgeAppSha256) {
        throw "Installed app still matches the bridge binary: $candidateAppSha256"
      }
      $candidateInstallReady = $true
      break
    } catch {
      $lastCandidateInstallStateError = $_.Exception.Message
      Start-Sleep -Milliseconds 250
    }
  }
  if (-not $candidateInstallReady) {
    throw "Timed out waiting for the official updater installer to publish candidate migration state and replace the installed executable: $lastCandidateInstallStateError"
  }
  Write-Host "Updater candidate install barrier: PASS (registry=$candidatePublisher/$candidateVersion; app_sha256=$candidateAppSha256; bridge_app_sha256=$bridgeAppSha256)"
  $candidateProcess = Start-Process -FilePath $appPath -ArgumentList (@(
    '--selftest-desktop-update',
    '--selftest-update-marker', $markerPath,
    '--selftest-update-expected-version-file', $expectedVersionPath,
    '--selftest-update-safety-marker', $safetyPath
  ) + $fixtureRuntimeArguments) -WindowStyle Hidden -PassThru
  $smokeScope.TrackedProcesses.Add($candidateProcess)
  $completedCandidateProcess = Wait-Process -Id $candidateProcess.Id -Timeout 180 -ErrorAction SilentlyContinue
  if ($null -eq $completedCandidateProcess) {
    throw "Candidate executable did not exit within 180 seconds: $appPath"
  }
  $candidateProcess.Refresh()
  if ($candidateProcess.ExitCode -ne 0) {
    throw "Candidate executable exited with code $($candidateProcess.ExitCode): $appPath"
  }
  Wait-ForPath -Path $markerPath -TimeoutSeconds 10
  $completion = ([IO.File]::ReadAllText($markerPath)).Trim()
  if ($completion -ne "update-complete:$candidateVersion") {
    throw "Bridge client did not apply the new-root candidate: $completion"
  }

  Wait-ForPath -Path $safetyPath
  $phases = @(Get-Content -LiteralPath $safetyPath)
  $requiredPhases = @('activity.quiesced', 'playback.keys_released', 'state.persisted', 'resources.closed')
  for ($index = 0; $index -lt $requiredPhases.Count; $index++) {
    $offset = [array]::IndexOf($phases, $requiredPhases[$index])
    if ($offset -lt 0) { throw "Missing updater shutdown safety phase: $($requiredPhases[$index])" }
    if ($index -gt 0 -and $offset -le $previousOffset) {
      throw 'Updater shutdown safety phases were not ordered'
    }
    $previousOffset = $offset
  }

  if (-not (Test-Path -LiteralPath $userSongPath -PathType Leaf)) {
    throw 'Updater removed the user song from application data'
  }
  $userSongShaAfter = Get-ByteSha256 ([IO.File]::ReadAllBytes($userSongPath))
  if ($userSongShaAfter -ne $userSongShaBefore) {
    throw "Updater changed the user song bytes: before=$userSongShaBefore after=$userSongShaAfter"
  }
  $candidateRegistryEvidence = Assert-V4MigrationPublisherState -ExpectedPublisher $candidatePublisher -ExpectedVersion $candidateVersion -ExpectedInstallRoot $installRoot
  $candidateBuiltinRoot = Join-Path $installRoot 'builtin-songs'
  & cargo xtask builtin-catalog verify-installed --root $candidateBuiltinRoot
  if ($LASTEXITCODE -ne 0) { throw "Candidate installed built-in catalog verification failed with exit code $LASTEXITCODE" }
  $candidateBuiltinManifest = Get-Content -LiteralPath (Join-Path $candidateBuiltinRoot 'manifest.json') -Raw | ConvertFrom-Json
  $candidateBuiltinCount = @($candidateBuiltinManifest.songs).Count
  if ($candidateBuiltinCount -le 0) {
    throw 'Candidate installed built-in catalog is empty after update'
  }
  $catalogCandidateEvidence = Get-InstalledBuiltinEvidence -Root $candidateBuiltinRoot
  if ($catalogBridgeEvidence.manifest_sha256 -eq $catalogCandidateEvidence.manifest_sha256) {
    throw 'Updater did not replace the installed built-in manifest bytes across N-to-N+1'
  }
  if ($catalogBridgeEvidence.selected_id -ne $catalogCandidateEvidence.selected_id) {
    throw 'Updater changed the selected built-in stable ID across N-to-N+1'
  }
  if ($catalogBridgeEvidence.selected_content_sha256 -eq $catalogCandidateEvidence.selected_content_sha256) {
    throw 'Updater did not replace the selected built-in content bytes across N-to-N+1'
  }
  if ($catalogCandidateEvidence.selected_id -ne $catalogSentinelId) {
    throw 'Candidate installed built-in catalog lost the selected stable identity'
  }
  if ($catalogSourceRestoreStatus -ne 'PASS') {
    throw 'Updater fixture source tree was not restored before candidate qualification'
  }
  $candidateBuiltinManifestSha = $catalogCandidateEvidence.manifest_sha256
  $preservationContract = [ordered]@{
    status = 'PASS'
    transition = 'N-to-N+1'
    user_song = 'updater-preserved-user.json'
    user_song_sha256_before = $userSongShaBefore
    user_song_sha256_after = $userSongShaAfter
    built_in_manifest_sha256_before = $catalogBridgeEvidence.manifest_sha256
    built_in_manifest_sha256_after = $catalogCandidateEvidence.manifest_sha256
    selected_builtin_id_before = $catalogBridgeEvidence.selected_id
    selected_builtin_id_after = $catalogCandidateEvidence.selected_id
    selected_builtin_content_sha256_before = $catalogBridgeEvidence.selected_content_sha256
    selected_builtin_content_sha256_after = $catalogCandidateEvidence.selected_content_sha256
    source_tree_restore = $catalogSourceRestoreStatus
    built_in_song_count_after = $candidateBuiltinCount
  }
  Write-Host "Updater N-to-N+1 resource replacement: PASS (user_sha256=$userSongShaAfter; built_in_manifest_sha256_before=$($catalogBridgeEvidence.manifest_sha256); built_in_manifest_sha256_after=$candidateBuiltinManifestSha; selected_id=$($catalogCandidateEvidence.selected_id); selected_content_sha256_before=$($catalogBridgeEvidence.selected_content_sha256); selected_content_sha256_after=$($catalogCandidateEvidence.selected_content_sha256); source_tree_restore=$catalogSourceRestoreStatus)"
  Write-Host "Updater N-to-N+1 preservation: PASS (user_sha256=$userSongShaAfter; built_in_count=$candidateBuiltinCount; built_in_manifest_sha256=$candidateBuiltinManifestSha)"

  $migrationContract = [ordered]@{
    status = 'PASS'
    previous = [ordered]@{
      version = $previousBridgeVersion
      publisher = [string]$bridgeRegistryEvidence.publisher
      identifier = $permanentIdentifier
      install_root = [string]$bridgeRegistryEvidence.install_location
      uninstall_identity = [string]$bridgeRegistryEvidence.key
    }
    candidate = [ordered]@{
      version = $candidateVersion
      publisher = [string]$candidateRegistryEvidence.publisher
      identifier = $permanentIdentifier
      install_root = [string]$candidateRegistryEvidence.install_location
      uninstall_identity = [string]$candidateRegistryEvidence.key
    }
    install_root_preserved = ([IO.Path]::GetFullPath($bridgeRegistryEvidence.install_location).TrimEnd('\') -ieq [IO.Path]::GetFullPath($candidateRegistryEvidence.install_location).TrimEnd('\'))
    uninstall_identity_preserved = ([string]$bridgeRegistryEvidence.key -ceq [string]$candidateRegistryEvidence.key)
    app_data_preserved = ($userSongShaAfter -ceq $userSongShaBefore)
    historical_publisher_registry = 'github -> pumni; stale github state absent'
  }
  Write-Host "Package publisher migration: PASS (previous=$previousBridgeVersion/github; candidate=$candidateVersion/$candidatePublisher; identifier=$permanentIdentifier; install_root_preserved=$($migrationContract.install_root_preserved); uninstall_identity_preserved=$($migrationContract.uninstall_identity_preserved); app_data_preserved=$($migrationContract.app_data_preserved))"

  if ([string]$candidateContract.http.body_sha256 -cne $candidateInstallerSha256) {
    throw "N-to-N+1 served candidate SHA differs from the candidate installer SHA: served=$($candidateContract.http.body_sha256) candidate=$candidateInstallerSha256"
  }
  $negativeRequestStart = @(Get-Content -LiteralPath $requestLogPath -ErrorAction Stop).Count
  # Switch only the server manifest. The preserved bridge is the same
  # tauri-update-fixture binary that accepted the candidate with old+new roots;
  # its fixture-only runtime seam now selects only the last (new) root.
  Copy-Item -LiteralPath $oldManifestPath -Destination $manifestPath -Force
  $cutoverProcess = Start-Process -FilePath $preservedBridgeAppPath -WorkingDirectory $preservedBridgeRoot -ArgumentList (@(
    '--selftest-desktop-update',
    '--selftest-update-marker', $cutoverMarkerPath,
    '--selftest-update-fixture-new-only'
  ) + $fixtureRuntimeArguments) -WindowStyle Hidden -PassThru
  $smokeScope.TrackedProcesses.Add($cutoverProcess)
  Wait-Process -Id $cutoverProcess.Id -Timeout 180
  Wait-ForPath -Path $cutoverMarkerPath
  $cutoverResult = ([IO.File]::ReadAllText($cutoverMarkerPath)).Trim()
  if (-not $cutoverResult.StartsWith('update-failed:')) {
    throw "Cutover client accepted an old-root artifact: $cutoverResult"
  }
  if ($cutoverResult.Length -gt 4096) {
    throw 'Cutover rejection marker is unexpectedly unbounded'
  }
  $negativeRequests = @(
    Get-Content -LiteralPath $requestLogPath -ErrorAction Stop |
      Select-Object -Skip $negativeRequestStart |
      Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } |
      ForEach-Object { $_ | ConvertFrom-Json }
  )
  $negativeManifestRequests = @($negativeRequests | Where-Object { [string]$_.path -eq '/stable' })
  $negativeCandidateRequests = @($negativeRequests | Where-Object { [string]$_.path -eq '/candidate/update.exe' })
  if ($negativeManifestRequests.Count -lt 1 -or $negativeCandidateRequests.Count -lt 1) {
    throw "Old-root rejection did not reach both fixture manifest and candidate download paths: manifest=$($negativeManifestRequests.Count) candidate=$($negativeCandidateRequests.Count)"
  }
  foreach ($request in $negativeCandidateRequests) {
    if ([string]$request.body_sha256 -cne $candidateInstallerSha256) {
      throw "Old-root rejection served a candidate SHA different from N-to-N+1: served=$($request.body_sha256) candidate=$candidateInstallerSha256"
    }
  }
  $candidateContract['old_root_rejection'] = [ordered]@{
    status = 'PASS'
    manifest_requests = $negativeManifestRequests.Count
    candidate_requests = $negativeCandidateRequests.Count
    served_candidate_sha256 = [string]$negativeCandidateRequests[0].body_sha256
    matches_n_to_n_plus_1 = ([string]$negativeCandidateRequests[0].body_sha256 -ceq [string]$candidateContract.n_to_n_plus_1_installer_sha256)
    update_result = $cutoverResult
  }
  if ($providedCandidate) {
    "Package migration + Tauri updater qualification: PASS (previous=$previousBridgeVersion/github; candidate=$candidateVersion/$candidatePublisher; identifier=$permanentIdentifier; install_root/uninstall_identity/app_data preserved; registry cleanup verified; N-to-N+1/old-root-rejection installer sha256=$candidateInstallerSha256; old-root manifest requests=$($negativeManifestRequests.Count); old-root candidate requests=$($negativeCandidateRequests.Count); built-in count=$candidateBuiltinCount; safety phases=$($requiredPhases -join ', '))" |
      Add-Content $summaryPath -Encoding UTF8
  } else {
    "Packaged Tauri updater rotation: PASS (preserved fixture bridge applied new-root-only $candidateVersion; N-to-N+1/old-root-rejection installer sha256=$candidateInstallerSha256; old-root manifest requests=$($negativeManifestRequests.Count); old-root candidate requests=$($negativeCandidateRequests.Count); synthetic higher version $cutoverVersion; user data preserved across N-to-N+1; built-in count=$candidateBuiltinCount; safety phases=$($requiredPhases -join ', '))" |
      Add-Content $summaryPath -Encoding UTF8
  }

  $uninstallerPath = Join-Path $installRoot 'uninstall.exe'
  Invoke-V4NsisUninstaller -UninstallerPath $uninstallerPath | Out-Null
  $uninstallDeadline = [DateTime]::UtcNow.AddSeconds(30)
  while ([DateTime]::UtcNow -lt $uninstallDeadline -and (Test-Path -LiteralPath $installRoot)) {
    Start-Sleep -Milliseconds 250
  }
  if (Test-Path -LiteralPath $installRoot) {
    throw "Migration uninstaller left the bounded install root in place: $installRoot"
  }
  if (-not (Test-Path -LiteralPath $userSongPath -PathType Leaf)) {
    throw 'Migration uninstaller removed application data owned by the user'
  }
  $postUninstallSongSha = Get-ByteSha256 ([IO.File]::ReadAllBytes($userSongPath))
  if ($postUninstallSongSha -ne $userSongShaBefore) {
    throw "Migration uninstaller changed application data bytes: before=$userSongShaBefore after=$postUninstallSongSha"
  }
  foreach ($identityPath in $migrationIdentityPaths) {
    if (Test-Path -LiteralPath $identityPath) {
      throw "Migration uninstaller left publisher or uninstall registry residue: $identityPath"
    }
  }
  $migrationContract['uninstall'] = [ordered]@{
    status = 'PASS'
    install_root_removed = $true
    app_data_preserved = ($postUninstallSongSha -ceq $userSongShaBefore)
    registry_publisher_state_clean = $true
    registry_scope_restore = 'deferred-to-finalizer'
    user_song_sha256_after_uninstall = $postUninstallSongSha
  }
  Write-Host "Package migration uninstall: PASS (install_root_removed=true; app_data_preserved=true; registry_publisher_state_clean=true)"
  $fixtureStatus = 'PASS'
} finally {
  $finalizerErrors = [System.Collections.Generic.List[string]]::new()

  # Step 1: Stop background server job
  try {
    if ($null -ne $serverJob) {
      New-Item -ItemType File -Path $stopPath -Force | Out-Null
      Stop-Job -Job $serverJob -ErrorAction SilentlyContinue
      Remove-Job -Job $serverJob -Force -ErrorAction SilentlyContinue
    }
  } catch {
    $finalizerErrors.Add("Failed to stop mock server job: $($_.Exception.Message)")
  }

  # Step 2: Restore source tree and project files
  try {
    Restore-CanonicalBuiltinCatalog
  } catch {
    $catalogSourceRestoreStatus = 'FAIL'
    $catalogSourceRestoreError = $_.Exception.Message
    $fixtureStatus = 'FAIL'
    $preservationContract.status = 'FAIL'
    $preservationContract.source_tree_restore = 'FAIL'
    $finalizerErrors.Add("Updater fixture source-tree restoration failed: $catalogSourceRestoreError")
  }

  try {
    [IO.File]::WriteAllText($candidateCargoPath, $cargoSource, [Text.UTF8Encoding]::new($false))
  } catch {
    $finalizerErrors.Add("Failed to restore Cargo.toml: $($_.Exception.Message)")
  }

  try {
    [IO.File]::WriteAllText($lockPath, $lockSource, [Text.UTF8Encoding]::new($false))
  } catch {
    $finalizerErrors.Add("Failed to restore Cargo.lock: $($_.Exception.Message)")
  }

  try {
    [IO.File]::WriteAllText($tauriConfigPath, $tauriConfigSource, [Text.UTF8Encoding]::new($false))
  } catch {
    $finalizerErrors.Add("Failed to restore tauri.conf.json: $($_.Exception.Message)")
  }

  # Step 3: Process environment restoration
  try {
    if ([string]::IsNullOrEmpty($oldAppDataRoot)) {
      Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue
    } else {
      [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $oldAppDataRoot, 'Process')
    }
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  } catch {
    $finalizerErrors.Add("Failed to restore environment variables: $($_.Exception.Message)")
  }

  # Step 4: Write HTTP evidence
  try {
    Write-HttpEvidence $fixtureStatus
  } catch {
    $finalizerErrors.Add("Failed to write HTTP evidence: $($_.Exception.Message)")
  }

  # Step 5: Exit NSIS smoke scope (registry, process, app data)
  if ($null -ne $smokeScope) {
    try {
      Exit-V4NsisSmokeScope -Scope $smokeScope
    } catch {
      $finalizerErrors.Add("Exit-V4NsisSmokeScope failed: $($_.Exception.Message)")
    }
  }

  try {
    Stop-UpdaterFixtureProcesses -Roots @($installRoot, $preservedBridgeRoot)
  } catch {
    $finalizerErrors.Add("Failed to stop updater fixture processes: $($_.Exception.Message)")
  }

  # Step 6: Fixture root cleanup
  try {
    if (-not $KeepFixtureOnFailure -and (Test-Path -LiteralPath $fixtureRoot)) {
      Remove-V4DirectoryWithRetry -Path $fixtureRoot
    }
  } catch {
    $finalizerErrors.Add("Failed to clean up fixture root: $($_.Exception.Message)")
  }

  if ($finalizerErrors.Count -gt 0) {
    throw ($finalizerErrors -join " | ")
  }
}
