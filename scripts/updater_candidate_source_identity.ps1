function Get-CargoPackageVersion {
  param([Parameter(Mandatory = $true)] [string]$ManifestSource)

  $packageSection = [regex]::Match(
    $ManifestSource,
    '(?ms)^\[package\]\r?\n(?<section>.*?)(?=^\[|\z)'
  )
  if (-not $packageSection.Success) {
    throw 'Candidate source manifest has no package section'
  }
  $versions = [regex]::Matches(
    $packageSection.Groups['section'].Value,
    '(?m)^version\s*=\s*"([^"]+)"\s*$'
  )
  if ($versions.Count -ne 1) {
    throw 'Candidate source manifest does not declare one literal package version'
  }
  return $versions[0].Groups[1].Value
}

function Get-HeadSourceManifest {
  param(
    [Parameter(Mandatory = $true)] [string]$RepoRoot,
    [Parameter(Mandatory = $true)] [string]$SourceSha
  )

  if ($SourceSha -notmatch '^[0-9a-fA-F]{40}$' -or $SourceSha -match '^0{40}$') {
    throw 'Candidate source identity received an invalid Git HEAD SHA'
  }
  Push-Location $RepoRoot
  try {
    $headSha = (& git rev-parse --verify HEAD 2>$null).Trim()
    if ($LASTEXITCODE -ne 0 -or $headSha -cne $SourceSha) {
      throw 'Candidate source identity SHA does not match the checked-out Git HEAD'
    }
    $manifestLines = & git show ($SourceSha + ':desktop/src-tauri/Cargo.toml') 2>$null
    if ($LASTEXITCODE -ne 0) {
      throw 'Candidate source identity could not read the desktop manifest from Git HEAD'
    }
    return ($manifestLines -join [Environment]::NewLine)
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
  $manifestSource = Get-HeadSourceManifest -RepoRoot $RepoRoot -SourceSha $SourceSha
  $sourceVersion = Get-CargoPackageVersion -ManifestSource $manifestSource
  Push-Location $RepoRoot
  try {
    & cargo xtask version check --version $CandidateVersion --no-repo-match *> $null
    if ($LASTEXITCODE -ne 0 -or $CandidateVersion -cne $sourceVersion) {
      throw "Candidate version $CandidateVersion does not match the release manifest at Git HEAD ($sourceVersion)"
    }
  } finally {
    Pop-Location
  }
  return [pscustomobject]@{ Version = $sourceVersion; SourceSha = $SourceSha }
}

function Get-CandidateSourceIdentity {
  param(
    [Parameter(Mandatory = $true)] [string]$RepoRoot,
    [Parameter(Mandatory = $true)] [string]$CandidateVersion
  )

  Push-Location $RepoRoot
  try {
    $sourceSha = (& git rev-parse --verify HEAD 2>$null).Trim()
    if ($LASTEXITCODE -ne 0 -or $sourceSha -notmatch '^[0-9a-fA-F]{40}$') {
      throw 'Candidate source identity could not resolve the checked-out Git HEAD SHA'
    }
    $identity = Assert-CandidateSourceIdentity `
      -RepoRoot $RepoRoot `
      -CandidateVersion $CandidateVersion `
      -SourceSha $sourceSha
    & cargo xtask version check --version $CandidateVersion *> $null
    if ($LASTEXITCODE -ne 0) {
      throw "Candidate version $CandidateVersion does not match the checked-out release source and lockfile"
    }
    return $identity
  } finally {
    Pop-Location
  }
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
