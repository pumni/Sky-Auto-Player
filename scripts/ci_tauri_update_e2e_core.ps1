[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$FixtureTargetDir,
  [string]$CandidateInstallerPath,
  [string]$CandidateSignaturePath,
  [string]$CandidateVersion,
  [string]$CandidatePublicKeyPath,
  [string]$EvidencePath,
  [switch]$KeepFixtureOnFailure
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$desktopRoot = Join-Path $repoRoot 'desktop'
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
$fixtureBundleRoot = Join-Path $fixtureTargetRoot 'dist/bundle/nsis'
$installRoot = Join-Path $fixtureRoot 'installed'
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
  if ($CandidateVersion -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
    throw "Provided-candidate updater qualification received a non-canonical SemVer: $CandidateVersion"
  }
}
$candidateVersion = if ($providedCandidate) { $CandidateVersion } else { '4.0.0-alpha.2' }
$cutoverVersion = '4.0.0-alpha.3'
$requestLogPath = Join-Path $fixtureRoot 'http-requests.jsonl'
$httpEvidencePath = if ([string]::IsNullOrWhiteSpace($EvidencePath)) {
  Join-Path $fixtureRoot 'fixture-http-evidence.json'
} else {
  [IO.Path]::GetFullPath($EvidencePath)
}
$manifestContract = [ordered]@{ status = 'not-checked' }
$candidateContract = [ordered]@{ status = 'not-checked' }
$preservationContract = [ordered]@{ status = 'not-checked' }
$fixtureStatus = 'FAIL'
$port = 0
$serverJob = $null
$previousInstallerCopy = Join-Path $fixtureRoot 'previous-v4-setup.exe'
$candidateArchive = $null
$candidateSignature = $null
$candidateCargoPath = Join-Path $desktopRoot 'src-tauri/Cargo.toml'
$lockPath = Join-Path $repoRoot 'rust/Cargo.lock'
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
  param([string]$Path, [int]$TimeoutSeconds = 180)
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
        endpoints = @("http://127.0.0.1:$port/stable")
      }
    }
  } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Path -Encoding utf8
}

function Invoke-FixtureBuild {
  param(
    [string]$ConfigPath,
    [string]$PrivateKeyPath,
    [string]$PublicRoots
  )
  $privateKey = ([IO.File]::ReadAllText($PrivateKeyPath)).Trim()
  if ([string]::IsNullOrWhiteSpace($privateKey)) {
    throw "Updater fixture private key is empty"
  }
  $env:SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS = $PublicRoots
  $env:TAURI_SIGNING_PRIVATE_KEY = $privateKey
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
  $oldCargoTargetDir = [Environment]::GetEnvironmentVariable('CARGO_TARGET_DIR', 'Process')
  $oldFixturePort = [Environment]::GetEnvironmentVariable('SKY_TAURI_UPDATE_FIXTURE_PORT', 'Process')
  try {
    [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $fixtureTargetRoot, 'Process')
    [Environment]::SetEnvironmentVariable('SKY_TAURI_UPDATE_FIXTURE_PORT', [string]$port, 'Process')
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
    if ([string]::IsNullOrEmpty($oldFixturePort)) {
      Remove-Item Env:SKY_TAURI_UPDATE_FIXTURE_PORT -ErrorAction SilentlyContinue
    } else {
      [Environment]::SetEnvironmentVariable('SKY_TAURI_UPDATE_FIXTURE_PORT', $oldFixturePort, 'Process')
    }
    Remove-Item Env:SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  }
}

try {
  $port = Get-DisposableLoopbackPort
  New-Item -ItemType Directory -Path $fixtureRoot, $installRoot -Force | Out-Null
  New-Item -ItemType File -Path $requestLogPath -Force | Out-Null
  New-Item -ItemType Directory -Path $fixtureBundleRoot -Force | Out-Null
  $fixtureBundleRoot = (Resolve-Path -LiteralPath $fixtureBundleRoot -ErrorAction Stop).Path

  foreach ($candidatePath in @($CandidateInstallerPath, $CandidateSignaturePath)) {
    if (-not [string]::IsNullOrWhiteSpace($candidatePath)) {
      $resolvedCandidatePath = [IO.Path]::GetFullPath($candidatePath)
      $fixturePrefix = $fixtureTargetRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
      if ($resolvedCandidatePath.StartsWith($fixturePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Downloaded candidate paths must remain outside the throwaway fixture target directory'
      }
    }
  }

  Push-Location $desktopRoot
  try {
    bun run tauri signer generate --ci --password '' --force -w $oldKeyPath *> $null
    bun run tauri signer generate --ci --password '' --force -w $newKeyPath *> $null
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
  Write-FixtureUpdaterConfig $bridgeConfigPath $oldPublicKey
  Write-FixtureUpdaterConfig $cutoverConfigPath $newPublicKey

  # Build only the throwaway bridge client with old+new roots. In production
  # qualification the candidate below is the exact installer and signature
  # downloaded from the release draft; it is never rebuilt by this fixture.
  Invoke-FixtureBuild $bridgeConfigPath $oldKeyPath "$oldPublicKey|$newPublicKey"
  $previousInstallers = @(Get-ChildItem -LiteralPath $fixtureBundleRoot -Filter '*4.0.0-alpha.1_x64-setup.exe' -File)
  if ($previousInstallers.Count -ne 1) {
    throw "Expected exactly one bridge-v4 installer, found $($previousInstallers.Count)"
  }
  Copy-Item -LiteralPath $previousInstallers[0].FullName -Destination $previousInstallerCopy -Force

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
    Invoke-FixtureBuild $cutoverConfigPath $newKeyPath $newPublicKey

    $candidateArchives = @(Get-ChildItem -LiteralPath $fixtureBundleRoot -Filter ("*" + $candidateVersion + "*-setup.exe") -File)
    if ($candidateArchives.Count -ne 1) {
      throw "Expected exactly one new-root candidate installer, found $($candidateArchives.Count)"
    }
    $candidateArchive = $candidateArchives[0]
    $candidateSignature = Get-Item -LiteralPath ($candidateArchive.FullName + '.sig') -ErrorAction Stop
  }
  $signatureText = ([IO.File]::ReadAllText($candidateSignature.FullName)).Trim()
  if ([string]::IsNullOrWhiteSpace($signatureText)) { throw 'Candidate updater signature is empty' }

  if (-not $providedCandidate) {
    # Sign the exact candidate bytes with the old root only. This detached
    # signature is used after cutover to prove the new-root-only client rejects
    # an old-root artifact through its real Update::download path.
    Copy-Item -LiteralPath $candidateArchive.FullName -Destination $candidateForOldSigningPath -Force
    Push-Location $desktopRoot
    try {
      bun run tauri signer sign --private-key-path $oldKeyPath --password '' $candidateForOldSigningPath *> $null
    } finally {
      Pop-Location
    }
    Move-Item -LiteralPath "$candidateForOldSigningPath.sig" -Destination $oldSignaturePath -Force
    $oldSignatureText = ([IO.File]::ReadAllText($oldSignaturePath)).Trim()
    if ([string]::IsNullOrWhiteSpace($oldSignatureText)) { throw 'Old-root updater signature is empty' }
  }

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
  if (-not $providedCandidate) {
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
  }

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
  $candidateResponse = Invoke-LoopbackHttpBytes $expectedCandidateUrl
  $candidateContract = [ordered]@{
    status = 'PASS'
    http = Assert-ExactHttpResponse $candidateResponse $candidateBytes 'application/octet-stream' 'fixture candidate artifact'
  }
  Write-Host "Fixture HTTP manifest contract: PASS (status=200; content-type=application/json; content-length=$($manifestHttp.content_length); body-sha256=$($manifestHttp.body_sha256))"
  Write-Host "Fixture HTTP candidate contract: PASS (status=200; content-type=application/octet-stream; content-length=$($candidateContract.http.content_length); body-sha256=$($candidateContract.http.body_sha256))"

  $installerRun = Start-Process -FilePath $previousInstallerCopy -ArgumentList @('/S', "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
  if ($installerRun.ExitCode -ne 0) { throw "Bridge-v4 installer exited with $($installerRun.ExitCode)" }
  $locationKey = 'HKCU:\Software\pumni\Sky Auto Player'
  New-Item -Path $locationKey -Force -Value $installRoot | Out-Null
  $appPath = Join-Path $installRoot 'sky_desktop_shell.exe'
  if (-not (Test-Path -LiteralPath $appPath)) { throw "Installed bridge app is missing: $appPath" }
  $bridgeBuiltinRoot = Join-Path $installRoot 'builtin-songs'
  & cargo xtask builtin-catalog verify-installed --root $bridgeBuiltinRoot
  if ($LASTEXITCODE -ne 0) { throw "Bridge installed built-in catalog verification failed with exit code $LASTEXITCODE" }

  $userSongBytes = [Text.Encoding]::UTF8.GetBytes('{"name":"Updater preserved user","songNotes":[{"time":0,"key":"1Key0"}]}')
  New-Item -ItemType Directory -Path $userSongsRoot -Force | Out-Null
  [IO.File]::WriteAllBytes($userSongPath, $userSongBytes)
  $userSongShaBefore = Get-ByteSha256 $userSongBytes
  if ((Get-ByteSha256 ([IO.File]::ReadAllBytes($userSongPath))) -ne $userSongShaBefore) {
    throw 'Could not establish the updater user-song preservation fixture'
  }
  [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $appDataRoot, 'Process')

  $appProcess = Start-Process -FilePath $appPath -ArgumentList @(
    '--selftest-desktop-update',
    '--selftest-update-marker', $markerPath,
    '--selftest-update-expected-version-file', $expectedVersionPath,
    '--selftest-update-safety-marker', $safetyPath
  ) -WindowStyle Hidden -PassThru
  Wait-Process -Id $appProcess.Id -Timeout 180
  Wait-ForPath -Path $markerPath
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
  $candidateBuiltinRoot = Join-Path $installRoot 'builtin-songs'
  & cargo xtask builtin-catalog verify-installed --root $candidateBuiltinRoot
  if ($LASTEXITCODE -ne 0) { throw "Candidate installed built-in catalog verification failed with exit code $LASTEXITCODE" }
  $candidateBuiltinManifest = Get-Content -LiteralPath (Join-Path $candidateBuiltinRoot 'manifest.json') -Raw | ConvertFrom-Json
  $candidateBuiltinCount = @($candidateBuiltinManifest.songs).Count
  if ($candidateBuiltinCount -le 0) {
    throw 'Candidate installed built-in catalog is empty after update'
  }
  $candidateBuiltinManifestSha = Get-ByteSha256 ([IO.File]::ReadAllBytes((Join-Path $candidateBuiltinRoot 'manifest.json')))
  $preservationContract = [ordered]@{
    status = 'PASS'
    transition = 'N-to-N+1'
    user_song = 'updater-preserved-user.json'
    user_song_sha256_before = $userSongShaBefore
    user_song_sha256_after = $userSongShaAfter
    built_in_manifest_sha256_after = $candidateBuiltinManifestSha
    built_in_song_count_after = $candidateBuiltinCount
  }
  Write-Host "Updater N-to-N+1 preservation: PASS (user_sha256=$userSongShaAfter; built_in_count=$candidateBuiltinCount; built_in_manifest_sha256=$candidateBuiltinManifestSha)"

  if ($providedCandidate) {
    "Packaged Tauri updater draft qualification: PASS (throwaway previous-v4 bridge applied the exact downloaded candidate $candidateVersion; user data preserved across N-to-N+1; built-in count=$candidateBuiltinCount; safety phases=$($requiredPhases -join ', '))" |
      Add-Content $summaryPath -Encoding UTF8
  } else {
    # Switch only the server manifest. The restarted candidate is the cutover
    # binary compiled with [new]; it must reject the old-only detached signature.
    Copy-Item -LiteralPath $oldManifestPath -Destination $manifestPath -Force
    $cutoverProcess = Start-Process -FilePath $appPath -ArgumentList @(
      '--selftest-desktop-update',
      '--selftest-update-marker', $cutoverMarkerPath
    ) -WindowStyle Hidden -PassThru
    Wait-Process -Id $cutoverProcess.Id -Timeout 180
    Wait-ForPath -Path $cutoverMarkerPath
    $cutoverResult = ([IO.File]::ReadAllText($cutoverMarkerPath)).Trim()
    if (-not $cutoverResult.StartsWith('update-failed:')) {
      throw "Cutover client accepted an old-root artifact: $cutoverResult"
    }
    "Packaged Tauri updater rotation: PASS (bridge [old,new] applied new-root-only $candidateVersion; user data preserved across N-to-N+1; built-in count=$candidateBuiltinCount; cutover [new] rejected old-root-only $cutoverVersion; safety phases=$($requiredPhases -join ', '))" |
      Add-Content $summaryPath -Encoding UTF8
  }
  $fixtureStatus = 'PASS'
} finally {
  if ($null -ne $serverJob) {
    New-Item -ItemType File -Path $stopPath -Force | Out-Null
    Stop-Job -Job $serverJob -ErrorAction SilentlyContinue
    Remove-Job -Job $serverJob -Force -ErrorAction SilentlyContinue
  }
  [IO.File]::WriteAllText($candidateCargoPath, $cargoSource, [Text.UTF8Encoding]::new($false))
  [IO.File]::WriteAllText($lockPath, $lockSource, [Text.UTF8Encoding]::new($false))
  if ([string]::IsNullOrEmpty($oldAppDataRoot)) {
    Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue
  } else {
    [Environment]::SetEnvironmentVariable('SKY_APP_DATA_ROOT', $oldAppDataRoot, 'Process')
  }
  Remove-Item Env:SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS -ErrorAction SilentlyContinue
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  Write-HttpEvidence $fixtureStatus
  if (-not $KeepFixtureOnFailure -and (Test-Path -LiteralPath $fixtureRoot)) {
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
  }
}
