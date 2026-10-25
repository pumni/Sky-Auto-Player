function Invoke-CandidateSourceIdentityTool {
  param(
    [Parameter(Mandatory = $true)] [string]$RepoRoot,
    [Parameter(Mandatory = $true)] [string]$CandidateVersion,
    [string]$SourceSha,
    [switch]$RequireWorktreeVersion
  )

  $arguments = @(
    '--quiet', 'xtask', 'ci', 'source-identity',
    '--repository-root', $RepoRoot,
    '--candidate-version', $CandidateVersion,
    '--format', 'json'
  )
  if (-not [string]::IsNullOrWhiteSpace($SourceSha)) {
    $arguments += @('--source-sha', $SourceSha)
  }
  if ($RequireWorktreeVersion) {
    $arguments += '--require-worktree-version'
  }

  Push-Location $RepoRoot
  try {
    $output = @(& cargo @arguments 2>&1)
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
      $diagnostic = ($output -join [Environment]::NewLine).Trim()
      if ([string]::IsNullOrWhiteSpace($diagnostic)) {
        throw "Rust candidate source identity failed with exit code $exitCode"
      }
      throw $diagnostic
    }
    try {
      return (($output -join [Environment]::NewLine) | ConvertFrom-Json -ErrorAction Stop)
    } catch {
      throw 'Rust candidate source identity returned invalid JSON'
    }
  } finally {
    Pop-Location
  }
}

function Assert-CandidateSourceIdentity {
  param(
    [Parameter(Mandatory = $true)] [string]$RepoRoot,
    [Parameter(Mandatory = $true)] [string]$CandidateVersion,
    [Parameter(Mandatory = $true)] [string]$SourceSha
  )

  if ([string]::IsNullOrWhiteSpace($CandidateVersion)) {
    throw 'Candidate source identity requires a version'
  }
  return Invoke-CandidateSourceIdentityTool `
    -RepoRoot $RepoRoot `
    -CandidateVersion $CandidateVersion `
    -SourceSha $SourceSha
}

function Get-CandidateSourceIdentity {
  param(
    [Parameter(Mandatory = $true)] [string]$RepoRoot,
    [Parameter(Mandatory = $true)] [string]$CandidateVersion
  )

  return Invoke-CandidateSourceIdentityTool `
    -RepoRoot $RepoRoot `
    -CandidateVersion $CandidateVersion `
    -RequireWorktreeVersion
}

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

function Get-UpdaterFileSha256 {
  param([Parameter(Mandatory = $true)] [byte[]]$Bytes)
  return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

function Invoke-WithHistoricalUpdaterBridgeSource {
  param(
    [Parameter(Mandatory = $true)] [string]$RepoRoot,
    [Parameter(Mandatory = $true)] [string]$BridgeVersion,
    [Parameter(Mandatory = $true)] [scriptblock]$Action
  )

  $cargoPath = Join-Path $RepoRoot 'desktop/src-tauri/Cargo.toml'
  $lockPath = Join-Path $RepoRoot 'rust/Cargo.lock'
  $cargoBytes = [IO.File]::ReadAllBytes($cargoPath)
  $lockBytes = [IO.File]::ReadAllBytes($lockPath)
  $encoding = [Text.UTF8Encoding]::new($false)
  $bridgeCargo = Convert-FixtureCargoVersion `
    -Source ($encoding.GetString($cargoBytes)) `
    -Version $BridgeVersion
  $bridgeLock = Convert-FixtureLockVersion `
    -Source ($encoding.GetString($lockBytes)) `
    -Version $BridgeVersion

  try {
    [IO.File]::WriteAllText($cargoPath, $bridgeCargo, $encoding)
    [IO.File]::WriteAllText($lockPath, $bridgeLock, $encoding)
    & $Action
  } finally {
    [IO.File]::WriteAllBytes($cargoPath, $cargoBytes)
    [IO.File]::WriteAllBytes($lockPath, $lockBytes)
    if ((Get-UpdaterFileSha256 -Bytes ([IO.File]::ReadAllBytes($cargoPath))) -ne
      (Get-UpdaterFileSha256 -Bytes $cargoBytes) -or
      (Get-UpdaterFileSha256 -Bytes ([IO.File]::ReadAllBytes($lockPath))) -ne
      (Get-UpdaterFileSha256 -Bytes $lockBytes)) {
      throw 'Updater fixture failed to restore canonical Cargo manifest bytes'
    }
  }
}

function Invoke-CandidateUpdaterFixture {
  param(
    [Parameter(Mandatory = $true)] [string]$RepoRoot,
    [Parameter(Mandatory = $true)] [hashtable]$CoreArguments,
    [Parameter(Mandatory = $true)] [bool]$ProvidedBridge,
    [Parameter(Mandatory = $true)] [string]$HistoricalBridgeVersion,
    [Parameter(Mandatory = $true)] [scriptblock]$InvokeCore
  )

  $preparedArguments = @{}
  foreach ($name in $CoreArguments.Keys) {
    $preparedArguments[$name] = $CoreArguments[$name]
  }
  if ($preparedArguments.ContainsKey('CandidateVersion') -and
    -not [string]::IsNullOrWhiteSpace([string]$preparedArguments['CandidateVersion'])) {
    $identity = Get-CandidateSourceIdentity `
      -RepoRoot $RepoRoot `
      -CandidateVersion ([string]$preparedArguments['CandidateVersion'])
    $preparedArguments['CandidateVersion'] = $identity.Version
    $preparedArguments['CandidateSourceSha'] = $identity.SourceSha
  }

  $invoke = { & $InvokeCore $preparedArguments }.GetNewClosure()
  if ($ProvidedBridge) {
    & $invoke
    return
  }
  Invoke-WithHistoricalUpdaterBridgeSource `
    -RepoRoot $RepoRoot `
    -BridgeVersion $HistoricalBridgeVersion `
    -Action $invoke
}
