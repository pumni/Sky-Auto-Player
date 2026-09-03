[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$PreviousInstaller,
  [Parameter(Mandatory = $true)]
  [string]$UpdaterConfigPath,
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
$safetyPath = Join-Path $fixtureRoot 'safety.txt'
$stopPath = Join-Path $fixtureRoot 'stop-server'
$candidateVersion = '4.0.0-alpha.2'
$port = 17845
$serverJob = $null
$previousInstallerCopy = Join-Path $fixtureRoot 'previous-v4-setup.exe'
$candidateCargoPath = Join-Path $desktopRoot 'src-tauri/Cargo.toml'
$lockPath = Join-Path $repoRoot 'rust/Cargo.lock'
$bundleRoot = Join-Path $repoRoot 'rust/target/dist/bundle/nsis'
$cargoSource = Get-Content -LiteralPath $candidateCargoPath -Raw
$lockSource = Get-Content -LiteralPath $lockPath -Raw
$candidateArchive = $null

function Wait-ForPath {
  param([string]$Path, [int]$TimeoutSeconds = 180)
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  while ([DateTime]::UtcNow -lt $deadline) {
    if (Test-Path -LiteralPath $Path) { return }
    Start-Sleep -Milliseconds 250
  }
  throw "Timed out waiting for $Path"
}

try {
  New-Item -ItemType Directory -Path $fixtureRoot, $installRoot | Out-Null
  if (-not (Test-Path -LiteralPath $PreviousInstaller)) {
    throw "Previous-v4 installer is missing: $PreviousInstaller"
  }
  Copy-Item -LiteralPath $PreviousInstaller -Destination $previousInstallerCopy

  $candidateArchive = Get-ChildItem -LiteralPath $bundleRoot -Filter '*.exe' -File |
    Where-Object { $_.Name -match [regex]::Escape($candidateVersion) } |
    Select-Object -First 1
  if ($null -eq $candidateArchive) {
    # Rebuild only the candidate package with the same test signing key. The
    # source and lockfile are restored in finally; no candidate version is
    # committed and no production version authority is changed.
    $cargoCandidate = $cargoSource -replace 'version = "4\.0\.0-alpha\.1"', ('version = "' + $candidateVersion + '"')
    $lockCandidate = [regex]::Replace(
      $lockSource,
      '(?s)(name = "sky_desktop_shell"\r?\nversion = ")4\.0\.0-alpha\.1("\r?\n)',
      '${1}' + $candidateVersion + '${2}',
      1
    )
    if ($lockCandidate -eq $lockSource) { throw 'Could not locate the desktop package in Cargo.lock' }
    Set-Content -LiteralPath $candidateCargoPath -Value $cargoCandidate -Encoding utf8
    Set-Content -LiteralPath $lockPath -Value $lockCandidate -Encoding utf8

    Push-Location $desktopRoot
    try {
      & bun run tauri build --ci --config $UpdaterConfigPath -- --profile dist --features tauri-update-fixture
      if ($LASTEXITCODE -ne 0) { throw "Candidate Tauri build failed with $LASTEXITCODE" }
    } finally {
      Pop-Location
    }

    $candidateArchive = Get-ChildItem -LiteralPath $bundleRoot -Filter '*.exe' -File |
      Where-Object { $_.Name -match [regex]::Escape($candidateVersion) } |
      Select-Object -First 1
  }

  if ($null -eq $candidateArchive) {
    throw "Candidate updater executable was not produced in $bundleRoot"
  }
  $candidateSignature = if ($null -ne $candidateArchive) {
    Get-Item -LiteralPath ($candidateArchive.FullName + '.sig') -ErrorAction Stop
  }
  if ($null -eq $candidateArchive -or $null -eq $candidateSignature) {
    throw "Candidate updater archive/signature was not produced in $bundleRoot"
  }
  if (-not (Test-Path -LiteralPath $PreviousInstaller)) {
    Copy-Item -LiteralPath $previousInstallerCopy -Destination $PreviousInstaller
  }

  $signatureText = (Get-Content -LiteralPath $candidateSignature.FullName -Raw).Trim()
  if ([string]::IsNullOrWhiteSpace($signatureText)) { throw 'Candidate updater signature is empty' }
  $manifest = [ordered]@{
    version = $candidateVersion
    notes = 'Deterministic packaged WO-03 candidate.'
    pub_date = '2026-09-04T00:00:00Z'
    platforms = [ordered]@{
      'windows-x86_64-nsis' = [ordered]@{
        signature = $signatureText
        url = 'http://127.0.0.1:17845/candidate/update.exe'
      }
    }
  }
  $manifestJson = ($manifest | ConvertTo-Json -Depth 8 -Compress)
  $archivePath = $candidateArchive.FullName

  $serverJob = Start-Job -ScriptBlock {
    param($Port, $ManifestJson, $ArchivePath, $StopPath)
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Parse('127.0.0.1'), $Port)
    $listener.Start()
    try {
      while (-not (Test-Path -LiteralPath $StopPath)) {
        $task = $listener.AcceptTcpClientAsync()
        if (-not $task.Wait(250)) { continue }
        $client = $task.Result
        $stream = $null
        try {
          $stream = $client.GetStream()
          $requestBytes = [byte[]]::new(8192)
          $read = $stream.Read($requestBytes, 0, $requestBytes.Length)
          $request = [Text.Encoding]::ASCII.GetString($requestBytes, 0, $read)
          $path = ($request -split "`r?`n", 2)[0].Split(' ')[1].Split('?')[0]
          if ($path -eq '/stable' -or $path -eq '/beta') {
            $bytes = [Text.Encoding]::UTF8.GetBytes($ManifestJson)
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
  } -ArgumentList $port, $manifestJson, $archivePath, $stopPath

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
  if ($installerRun.ExitCode -ne 0) { throw "Previous-v4 installer exited with $($installerRun.ExitCode)" }
  # Current-user NSIS stores this value during a normal interactive install.
  # The restricted CI runner can virtualize installer registry writes, so the
  # fixture makes the same value explicit before invoking the updater.
  $locationKey = 'HKCU:\Software\pumni\Sky Auto Player'
  New-Item -Path $locationKey -Force -Value $installRoot | Out-Null
  $appPath = Join-Path $installRoot 'sky_desktop_shell.exe'
  if (-not (Test-Path -LiteralPath $appPath)) { throw "Installed previous-v4 app is missing: $appPath" }

  $appProcess = Start-Process -FilePath $appPath -ArgumentList @(
    '--selftest-desktop-update',
    '--selftest-update-marker', $markerPath,
    '--selftest-update-safety-marker', $safetyPath
  ) -WindowStyle Hidden -PassThru
  Wait-Process -Id $appProcess.Id -Timeout 180
  Wait-ForPath -Path $markerPath
  $completion = (Get-Content -LiteralPath $markerPath -Raw).Trim()
  if ($completion -ne "update-complete:$candidateVersion") {
    throw "Packaged update did not restart into the candidate: $completion"
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
  "Packaged Tauri updater previous-v4 -> candidate-v4: PASS ($completion; safety phases=$($requiredPhases -join ', '))" |
    Add-Content $summaryPath -Encoding UTF8
} finally {
  if ($null -ne $serverJob) {
    New-Item -ItemType File -Path $stopPath -Force | Out-Null
    Stop-Job -Job $serverJob -ErrorAction SilentlyContinue
    Remove-Job -Job $serverJob -Force -ErrorAction SilentlyContinue
  }
  if ($null -ne $candidateArchive) {
    Remove-Item -LiteralPath $candidateArchive.FullName, ($candidateArchive.FullName + '.sig') -Force -ErrorAction SilentlyContinue
  }
  Set-Content -LiteralPath $candidateCargoPath -Value $cargoSource -Encoding utf8
  Set-Content -LiteralPath $lockPath -Value $lockSource -Encoding utf8
  if (-not $KeepFixtureOnFailure -and (Test-Path -LiteralPath $fixtureRoot)) {
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
  }
}
