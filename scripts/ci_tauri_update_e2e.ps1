[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$BundleDir,
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
$installRoot = Join-Path $fixtureRoot 'installed'
$markerPath = Join-Path $fixtureRoot 'completion.txt'
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
$candidateVersion = '4.0.0-alpha.2'
$cutoverVersion = '4.0.0-alpha.3'
$port = 17845
$serverJob = $null
$previousInstallerCopy = Join-Path $fixtureRoot 'previous-v4-setup.exe'
$candidateArchive = $null
$candidateSignature = $null
$candidateCargoPath = Join-Path $desktopRoot 'src-tauri/Cargo.toml'
$lockPath = Join-Path $repoRoot 'rust/Cargo.lock'
$cargoSource = [IO.File]::ReadAllText($candidateCargoPath)
$lockSource = [IO.File]::ReadAllText($lockPath)

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
  try {
    Push-Location $desktopRoot
    try {
      & bun run tauri build --ci --config $ConfigPath -- --profile dist --features tauri-update-fixture
      if ($LASTEXITCODE -ne 0) { throw "Tauri updater fixture build failed with $LASTEXITCODE" }
    } finally {
      Pop-Location
    }
  } finally {
    Remove-Item Env:SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  }
}

try {
  New-Item -ItemType Directory -Path $fixtureRoot, $installRoot -Force | Out-Null
  New-Item -ItemType Directory -Path $BundleDir -Force | Out-Null
  $bundleRoot = (Resolve-Path -LiteralPath $BundleDir -ErrorAction Stop).Path

  Push-Location $desktopRoot
  try {
    bun run tauri signer generate --ci --password '' --force -w $oldKeyPath | Out-Null
    bun run tauri signer generate --ci --password '' --force -w $newKeyPath | Out-Null
  } finally {
    Pop-Location
  }
  $oldPublicKey = ([IO.File]::ReadAllText("$oldKeyPath.pub")).Trim()
  $newPublicKey = ([IO.File]::ReadAllText("$newKeyPath.pub")).Trim()
  if ([string]::IsNullOrWhiteSpace($oldPublicKey) -or [string]::IsNullOrWhiteSpace($newPublicKey)) {
    throw 'Updater fixture public key generation failed'
  }
  Write-FixtureUpdaterConfig $bridgeConfigPath $oldPublicKey
  Write-FixtureUpdaterConfig $cutoverConfigPath $newPublicKey

  # Build the actual bridge client with old+new roots. The candidate build
  # below is rebuilt with only the new root, so the installed binaries exercise
  # the same Rust/Tauri transition as production rotation.
  Invoke-FixtureBuild $bridgeConfigPath $oldKeyPath "$oldPublicKey|$newPublicKey"
  $previousInstallers = @(Get-ChildItem -LiteralPath $bundleRoot -Filter '*4.0.0-alpha.1_x64-setup.exe' -File)
  if ($previousInstallers.Count -ne 1) {
    throw "Expected exactly one bridge-v4 installer, found $($previousInstallers.Count)"
  }
  Copy-Item -LiteralPath $previousInstallers[0].FullName -Destination $previousInstallerCopy -Force

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

  $candidateArchives = @(Get-ChildItem -LiteralPath $bundleRoot -Filter "*${candidateVersion}*-setup.exe" -File)
  if ($candidateArchives.Count -ne 1) {
    throw "Expected exactly one new-root candidate installer, found $($candidateArchives.Count)"
  }
  $candidateArchive = $candidateArchives[0]
  $candidateSignature = Get-Item -LiteralPath ($candidateArchive.FullName + '.sig') -ErrorAction Stop
  $signatureText = ([IO.File]::ReadAllText($candidateSignature.FullName)).Trim()
  if ([string]::IsNullOrWhiteSpace($signatureText)) { throw 'Candidate updater signature is empty' }

  # Sign the exact candidate bytes with the old root only. This detached
  # signature is used after cutover to prove the new-root-only client rejects
  # an old-root artifact through its real Update::download path.
  Copy-Item -LiteralPath $candidateArchive.FullName -Destination $candidateForOldSigningPath -Force
  Push-Location $desktopRoot
  try {
    bun run tauri signer sign --private-key-path $oldKeyPath --password '' $candidateForOldSigningPath | Out-Null
  } finally {
    Pop-Location
  }
  Move-Item -LiteralPath "$candidateForOldSigningPath.sig" -Destination $oldSignaturePath -Force
  $oldSignatureText = ([IO.File]::ReadAllText($oldSignaturePath)).Trim()
  if ([string]::IsNullOrWhiteSpace($oldSignatureText)) { throw 'Old-root updater signature is empty' }

  $newManifest = [ordered]@{
    version = $candidateVersion
    notes = 'Deterministic bridge rotation candidate.'
    pub_date = '2026-09-04T00:00:00Z'
    platforms = [ordered]@{
      'windows-x86_64-nsis' = [ordered]@{
        signature = $signatureText
        url = "http://127.0.0.1:$port/candidate/update.exe"
      }
    }
  }
  $oldManifest = [ordered]@{
    version = $cutoverVersion
    notes = 'Old-root rejection candidate.'
    pub_date = '2026-09-04T00:00:00Z'
    platforms = [ordered]@{
      'windows-x86_64-nsis' = [ordered]@{
        signature = $oldSignatureText
        url = "http://127.0.0.1:$port/candidate/update.exe"
      }
    }
  }
  $newManifest | ConvertTo-Json -Depth 8 -Compress | Set-Content -LiteralPath $manifestPath -Encoding utf8
  $oldManifest | ConvertTo-Json -Depth 8 -Compress | Set-Content -LiteralPath $oldManifestPath -Encoding utf8

  $archivePath = $candidateArchive.FullName
  $serverJob = Start-Job -ScriptBlock {
    param($Port, $ManifestPath, $ArchivePath, $StopPath)
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
          $path = ($request -split "`r?`n", 2)[0].Split(' ')[1].Split('?')[0]
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
      $listener.Close()
    }
  } -ArgumentList $port, $manifestPath, $archivePath, $stopPath

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

  $installerRun = Start-Process -FilePath $previousInstallerCopy -ArgumentList @('/S', "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
  if ($installerRun.ExitCode -ne 0) { throw "Bridge-v4 installer exited with $($installerRun.ExitCode)" }
  $locationKey = 'HKCU:\Software\pumni\Sky Auto Player'
  New-Item -Path $locationKey -Force -Value $installRoot | Out-Null
  $appPath = Join-Path $installRoot 'sky_desktop_shell.exe'
  if (-not (Test-Path -LiteralPath $appPath)) { throw "Installed bridge app is missing: $appPath" }

  $appProcess = Start-Process -FilePath $appPath -ArgumentList @(
    '--selftest-desktop-update',
    '--selftest-update-marker', $markerPath,
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
  "Packaged Tauri updater rotation: PASS (bridge [old,new] applied new-root-only $candidateVersion; cutover [new] rejected old-root-only $cutoverVersion; safety phases=$($requiredPhases -join ', '))" |
    Add-Content $summaryPath -Encoding UTF8
} finally {
  if ($null -ne $serverJob) {
    New-Item -ItemType File -Path $stopPath -Force | Out-Null
    Stop-Job -Job $serverJob -ErrorAction SilentlyContinue
    Remove-Job -Job $serverJob -Force -ErrorAction SilentlyContinue
  }
  [IO.File]::WriteAllText($candidateCargoPath, $cargoSource, [Text.UTF8Encoding]::new($false))
  [IO.File]::WriteAllText($lockPath, $lockSource, [Text.UTF8Encoding]::new($false))
  Remove-Item Env:SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS -ErrorAction SilentlyContinue
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  if (-not $KeepFixtureOnFailure -and (Test-Path -LiteralPath $fixtureRoot)) {
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
  }
}
