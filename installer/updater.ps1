# License: GPL-3.0 (Sky Player project). No code ported from mpv; structural reference only.
# Sky Player external updater. See docs/2026-07-18_distribution-mpv-pattern-plan.md §Phase 2.
#
# Behaviour contract:
#   1. Set TLS 1.2/1.3 protocol bindings.
#   2. Verify write access to install root.
#   3. Read channel from -Channel or config.json update.channel (default stable).
#   4. Query GitHub Releases for that channel.
#   5. Compare candidate to running version (MANIFEST.json, else ProductVersion).
#   6. Same-or-older -> "Already up to date", exit 0.
#   7. Newer -> download zip + .sha256 (HTTPS allow-list only).
#   8. Verify SHA256; mismatch aborts before any install mutation.
#   9. If Sky-Player.exe is running from this folder: exit 4 unless -ForceClose.
#  10. Expand-Archive to TEMP staging.
#  11. Back up existing replaceable files.
#  12. Copy staging -> install, preserving config.json and completely skipping songs/.
#  13. On copy failure, roll back all backup files and clean up.
#  14. Patch update.last_check_ts (Unix int) + update.last_notified_version (handles missing keys).
#  15. Log one line; print DONE; do NOT relaunch unless -Restart (O3).
#
# Exit codes: 0 ok, 2 network/asset, 3 sha256, 4 process lock, 5 permission/extract/copy.

[CmdletBinding()]
param(
    [ValidateSet('stable','beta')]
    [string]$Channel,
    [switch]$DryRun,
    [switch]$ForceClose,
    [switch]$Restart
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# --- TLS Initialization (PS 5.1 compatibility) ---
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor [Net.SecurityProtocolType]::Tls13
} catch {
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    } catch {
        Write-Warning "Failed to explicitly set TLS 1.2 or TLS 1.3. Connection to GitHub may fail."
    }
}

# Smoke-test hook: when set, API/asset base (http://localhost:...). Production: unset.
$FakeRoot = $env:SKY_UPDATER_FAKE_ROOT

$ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Path
$InstallRoot = Split-Path -Parent $ScriptDir
$ExePath     = Join-Path $InstallRoot 'Sky-Player.exe'
$ConfigPath  = Join-Path $InstallRoot 'config.json'

$LogDir  = Join-Path $env:LOCALAPPDATA 'Sky-Player'
$LogFile = Join-Path $LogDir 'updater.log'
function Write-Log([string]$msg) {
    New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
    $line = '[{0:u}] {1}' -f (Get-Date).ToUniversalTime(), $msg
    try { Add-Content -Path $LogFile -Value $line -Encoding UTF8 } catch {}
}

function Assert-HttpsUrl([string]$Url) {
    if ($FakeRoot -and $Url.StartsWith($FakeRoot)) {
        if ($Url -notmatch '^https?://(localhost|127\.0\.0\.1)(:\d+)?/') {
            throw "Fake root must be localhost: $Url"
        }
        return
    }
    if ($Url -notmatch '^https://') {
        throw "Refusing non-HTTPS URL: $Url"
    }
    $okHosts = @(
        'api.github.com',
        'github.com',
        'objects.githubusercontent.com',
        'release-assets.githubusercontent.com'
    )
    $uri = [Uri]$Url
    if ($okHosts -notcontains $uri.Host) {
        throw "Refusing URL host not on allow-list: $($uri.Host)"
    }
}

function Test-WriteAccess([string]$Path) {
    $tempFile = Join-Path $Path (".write-test-" + [guid]::NewGuid().ToString('N'))
    try {
        [System.IO.File]::WriteAllText($tempFile, "test")
        Remove-Item -LiteralPath $tempFile -Force -ErrorAction SilentlyContinue
        return $true
    } catch {
        return $false
    }
}

function Read-ConfigObject {
    if (-not (Test-Path -LiteralPath $ConfigPath)) { return $null }
    try {
        return (Get-Content -Raw -LiteralPath $ConfigPath | ConvertFrom-Json)
    } catch { return $null }
}

function Write-UpdateFields {
    param(
        [int]$LastCheckTs,
        [string]$LastNotifiedVersion
    )
    if (-not (Test-Path -LiteralPath $ConfigPath)) { return }
    $text = Get-Content -Raw -LiteralPath $ConfigPath -Encoding UTF8

    # Patch last_check_ts, insert it inside "update" object if missing
    if ($text -match '"last_check_ts"\s*:\s*\d+') {
        $text = $text -replace '"last_check_ts"\s*:\s*\d+', "`"last_check_ts`": $LastCheckTs"
    } else {
        $text = $text -replace '("update"\s*:\s*\{\s*)', "`$1`n        `"last_check_ts`": $LastCheckTs,"
    }

    # Patch last_notified_version, insert it inside "update" object if missing
    if ($text -match '"last_notified_version"\s*:\s*"[^"]*"') {
        $text = $text -replace '"last_notified_version"\s*:\s*"[^"]*"', "`"last_notified_version`": `"$LastNotifiedVersion`""
    } else {
        $text = $text -replace '("update"\s*:\s*\{\s*)', "`$1`n        `"last_notified_version`": `"$LastNotifiedVersion`","
    }

    [System.IO.File]::WriteAllText($ConfigPath, $text, (New-Object System.Text.UTF8Encoding($false)))
}

function Get-RunningVersion {
    $manifest = Join-Path $InstallRoot 'MANIFEST.json'
    if (Test-Path -LiteralPath $manifest) {
        try {
            $m = Get-Content -Raw -LiteralPath $manifest | ConvertFrom-Json
            if ($m.version) { return [string]$m.version }
        } catch {}
    }
    $vi = (Get-Item -LiteralPath $ExePath -ErrorAction SilentlyContinue).VersionInfo
    if ($vi -and $vi.ProductVersion) { return [string]$vi.ProductVersion }
    return '0.0.0'
}

function Compare-Version([string]$a, [string]$b) {
    # +1 if a>b, -1 if a<b, 0 if equal. MUST use -split '\.' (literal dot) — G14.
    $av = ($a -split '[-+]', 2)[0]
    $bv = ($b -split '[-+]', 2)[0]
    $ax = @($av -split '\.' | ForEach-Object { [int]$_ })
    $bx = @($bv -split '\.' | ForEach-Object { [int]$_ })
    $n = [Math]::Max($ax.Count, $bx.Count)
    for ($i = 0; $i -lt $n; $i++) {
        $aa = if ($i -lt $ax.Count) { $ax[$i] } else { 0 }
        $bb = if ($i -lt $bx.Count) { $bx[$i] } else { 0 }
        if ($aa -gt $bb) { return 1 }
        if ($aa -lt $bb) { return -1 }
    }
    $aPre = $a.Contains('-')
    $bPre = $b.Contains('-')
    if (-not $aPre -and $bPre) { return 1 }
    if ($aPre -and -not $bPre) { return -1 }
    if ($aPre -and $bPre) { return [string]::CompareOrdinal($a, $b) }
    return 0
}

function Copy-UpdateTree([string]$StagingRoot, [string]$DestRoot) {
    $copiedFiles = @()
    $backedUpFiles = @()
    $backupDir = Join-Path $env:TEMP ('sky-backup-' + [guid]::NewGuid().ToString('N'))

    try {
        $filesToCopy = Get-ChildItem -LiteralPath $StagingRoot -Recurse -File
        
        # 1. Back up existing destination files that will be overwritten
        foreach ($file in $filesToCopy) {
            $rel = $file.FullName.Substring($StagingRoot.Length).TrimStart('\', '/')
            $dest = Join-Path $DestRoot $rel
            
            # Skip copying config.json or songs/ files entirely
            if ($rel -eq 'config.json' -or $rel -eq 'songs' -or $rel.StartsWith('songs/')) {
                continue
            }
            
            if (Test-Path -LiteralPath $dest) {
                if (-not (Test-Path -LiteralPath $backupDir)) {
                    New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
                }
                $relBackupPath = Join-Path $backupDir $rel
                $relBackupDir = Split-Path -Parent $relBackupPath
                if (-not (Test-Path -LiteralPath $relBackupDir)) {
                    New-Item -ItemType Directory -Force -Path $relBackupDir | Out-Null
                }
                Copy-Item -LiteralPath $dest -Destination $relBackupPath -Force | Out-Null
                $backedUpFiles += @{ Source = $dest; Backup = $relBackupPath }
            }
        }

        # 2. Copy files from staging to target
        foreach ($file in $filesToCopy) {
            $rel = $file.FullName.Substring($StagingRoot.Length).TrimStart('\', '/')
            $dest = Join-Path $DestRoot $rel
            
            if ($rel -eq 'config.json' -or $rel -eq 'songs' -or $rel.StartsWith('songs/')) {
                continue
            }
            
            $destDir = Split-Path -Parent $dest
            if (-not (Test-Path -LiteralPath $destDir)) {
                New-Item -ItemType Directory -Force -Path $destDir | Out-Null
            }
            Copy-Item -LiteralPath $file.FullName -Destination $dest -Force | Out-Null
            $copiedFiles += $dest
        }

        # Clean up backups on complete success
        if (Test-Path -LiteralPath $backupDir) {
            Remove-Item -Recurse -Force $backupDir -ErrorAction SilentlyContinue
        }
    } catch {
        Write-Log "Error during copy: $_. Rolling back..."
        Write-Host "Copy failed: $_. Rolling back files to pre-update state..."
        
        # Restore backed up original files
        foreach ($backup in $backedUpFiles) {
            try {
                Copy-Item -LiteralPath $backup.Backup -Destination $backup.Source -Force | Out-Null
            } catch {
                Write-Log "Failed to restore backup for $($backup.Source): $_"
            }
        }
        
        # Clean up newly copied files
        foreach ($copied in $copiedFiles) {
            $wasBackup = $false
            foreach ($backup in $backedUpFiles) {
                if ($backup.Source -eq $copied) {
                    $wasBackup = $true
                    break
                }
            }
            if (-not $wasBackup) {
                Remove-Item -LiteralPath $copied -Force -ErrorAction SilentlyContinue | Out-Null
            }
        }
        
        if (Test-Path -LiteralPath $backupDir) {
            Remove-Item -Recurse -Force $backupDir -ErrorAction SilentlyContinue
        }
        throw $_
    }
}

# --- Check Write Permissions ---
if (-not (Test-WriteAccess $InstallRoot)) {
    Write-Log "write access denied to $InstallRoot"
    Write-Host "Error: Write access is denied for the directory: $InstallRoot"
    Write-Host "Please close the application and run updater.bat as Administrator."
    exit 5
}

# --- Channel ---
$cfgObj = Read-ConfigObject
$updateCfg = if ($cfgObj) { $cfgObj.update } else { $null }
$ch = if ($Channel) {
    $Channel
} elseif ($updateCfg -and $updateCfg.channel) {
    [string]$updateCfg.channel
} else {
    'stable'
}
if ($ch -ne 'stable' -and $ch -ne 'beta') { $ch = 'stable' }

$runningVersion = Get-RunningVersion

# --- GitHub / fake root ---
$owner = 'pumni'
$repo  = 'Sky-Player'
$headers = @{ 'User-Agent' = 'sky-player-updater'; 'Accept' = 'application/vnd.github.v3+json' }

try {
    if ($FakeRoot) {
        $metaUrl = ($FakeRoot.TrimEnd('/') + '/release.json')
        Assert-HttpsUrl $metaUrl
        $candidate = Invoke-RestMethod -Uri $metaUrl -TimeoutSec 10
    } elseif ($ch -eq 'beta') {
        $apiBase = "https://api.github.com/repos/$owner/$repo/releases"
        Assert-HttpsUrl $apiBase
        $releases = Invoke-RestMethod -Uri $apiBase -Headers $headers -TimeoutSec 10
        # Iterate and pick the newest by Compare-Version
        $candidate = $null
        $best = $null
        foreach ($r in ($releases | Where-Object { -not $_.draft })) {
            $rt = [string]$r.tag_name; if ($rt -match '^v?(.+)$') { $rt = $Matches[1] }
            if (-not $best) { $best = $r; continue }
            $bt = [string]$best.tag_name; if ($bt -match '^v?(.+)$') { $bt = $Matches[1] }
            if ((Compare-Version $rt $bt) -gt 0) { $best = $r }
        }
        $candidate = $best
    } else {
        $apiLatest = "https://api.github.com/repos/$owner/$repo/releases/latest"
        Assert-HttpsUrl $apiLatest
        $candidate = Invoke-RestMethod -Uri $apiLatest -Headers $headers -TimeoutSec 10
    }
} catch {
    Write-Log "network error: $_"
    Write-Host "Network error: $_"
    exit 2
}

if (-not $candidate) {
    Write-Log "no release found for channel $ch"
    Write-Host "No release found for channel '$ch'."
    exit 2
}

$tagRaw = [string]$candidate.tag_name
if ($tagRaw -match '^v?(.+)$') { $latestVersion = $Matches[1] } else { $latestVersion = $tagRaw }

if ((Compare-Version $latestVersion $runningVersion) -le 0) {
    Write-Log "already up to date (running=$runningVersion latest=$latestVersion)"
    Write-Host "You are already using the latest version ($runningVersion)."
    exit 0
}

# --- Asset selection ---
$zipName = "Sky-Player-v$latestVersion.zip"
$shaName = "Sky-Player-v$latestVersion.zip.sha256"
if ($FakeRoot) {
    $zipUrl = ($FakeRoot.TrimEnd('/') + '/' + $zipName)
    $shaUrl = ($FakeRoot.TrimEnd('/') + '/' + $shaName)
    Assert-HttpsUrl $zipUrl
    Assert-HttpsUrl $shaUrl
} else {
    $zipAsset = $candidate.assets | Where-Object { $_.name -eq $zipName } | Select-Object -First 1
    $shaAsset = $candidate.assets | Where-Object { $_.name -eq $shaName } | Select-Object -First 1
    if (-not $zipAsset -or -not $shaAsset) {
        Write-Log "missing zip or sha256 asset for $latestVersion"
        Write-Host "Release v$latestVersion is missing the zip or sha256 sidecar. Aborting."
        exit 2
    }
    $zipUrl = [string]$zipAsset.browser_download_url
    $shaUrl = [string]$shaAsset.browser_download_url
    Assert-HttpsUrl $zipUrl
    Assert-HttpsUrl $shaUrl
}

$tmpDir = Join-Path $env:TEMP ('sky-update-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
$zipPath = Join-Path $tmpDir $zipName
$shaPath = Join-Path $tmpDir $shaName
$extractDir = Join-Path $tmpDir 'extract'
New-Item -ItemType Directory -Force -Path $extractDir | Out-Null

try {
    Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing
    Invoke-WebRequest -Uri $shaUrl -OutFile $shaPath -UseBasicParsing
} catch {
    Write-Log "download failed: $_"
    Write-Host "Download failed: $_"
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 2
}

$sidecarText = Get-Content -Raw -LiteralPath $shaPath
$expected = $null
if ($sidecarText -match '([0-9a-fA-F]{64})') { $expected = $Matches[1].ToLower() }
if (-not $expected) {
    Write-Log 'sidecar unparseable'
    Write-Host 'SHA256 sidecar could not be parsed. Aborting before any file mutation.'
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 3
}
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash.ToLower()
if ($actual -ne $expected) {
    Write-Log "sha256 mismatch: expected=$expected actual=$actual"
    Write-Host 'SHA256 mismatch. Aborting before any file mutation.'
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 3
}

if ($DryRun) {
    Write-Host "DryRun passed: would update $runningVersion -> $latestVersion"
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 0
}

# --- Process gate (G19) ---
$runningProcesses = Get-Process -Name 'Sky-Player' -ErrorAction SilentlyContinue
$targetProcess = $null
if ($runningProcesses) {
    foreach ($p in $runningProcesses) {
        try {
            if ($p.Path -and (Split-Path -Parent $p.Path) -eq $InstallRoot) {
                $targetProcess = $p
                break
            }
        } catch {}
    }
}

if ($targetProcess) {
    if (-not $ForceClose) {
        Write-Log 'Sky-Player.exe still running; refuse update'
        Write-Host 'Sky-Player.exe is still running in this directory. Close it, then re-run updater.bat.'
        Write-Host '(Advanced: updater.bat -ForceClose)'
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
        exit 4
    }
    Write-Host 'Stopping Sky-Player.exe (-ForceClose)...'
    $targetProcess | Stop-Process -Force
    Start-Sleep -Seconds 2
    
    $runningAgain = Get-Process -Name 'Sky-Player' -ErrorAction SilentlyContinue
    $stillRunning = $false
    if ($runningAgain) {
        foreach ($p in $runningAgain) {
            try {
                if ($p.Path -and (Split-Path -Parent $p.Path) -eq $InstallRoot) {
                    $stillRunning = $true
                    break
                }
            } catch {}
        }
    }
    if ($stillRunning) {
        Write-Log 'Sky-Player.exe still locked after ForceClose'
        Write-Host 'Could not stop Sky-Player.exe. Aborting.'
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
        exit 4
    }
}

# --- Stage extract (never onto install root) ---
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
    [System.IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $extractDir)
} catch {
    try {
        Expand-Archive -LiteralPath $zipPath -DestinationPath $extractDir -Force
    } catch {
        Write-Log "extract failed: $_"
        Write-Host "Extract failed: $_"
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
        exit 5
    }
}

$StagingRoot = $extractDir
$exeInExtract = Join-Path $extractDir 'Sky-Player.exe'
if (-not (Test-Path -LiteralPath $exeInExtract)) {
    $child = Get-ChildItem -LiteralPath $extractDir -Directory | Select-Object -First 1
    if ($child -and (Test-Path -LiteralPath (Join-Path $child.FullName 'Sky-Player.exe'))) {
        $StagingRoot = $child.FullName
    } else {
        Write-Log 'staging layout missing Sky-Player.exe'
        Write-Host "Update zip layout is unexpected (no Sky-Player.exe). Aborting."
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
        exit 5
    }
}

# --- Copy with transactional fallback (I16, I21, I22) ---
try {
    Copy-UpdateTree -StagingRoot $StagingRoot -DestRoot $InstallRoot
} catch {
    Write-Log "copy failed: $_"
    Write-Host "Copy into install dir failed: $_. User config.json and songs directory were restored. Re-run after resolving the issue."
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 5
}

# Unix epoch seconds as int (matches Python last_check_ts)
$epoch = [int][double]::Parse(
    (Get-Date -Date (Get-Date).ToUniversalTime() -UFormat %s),
    [System.Globalization.CultureInfo]::InvariantCulture
)
try {
    Write-UpdateFields -LastCheckTs $epoch -LastNotifiedVersion $latestVersion
} catch {
    Write-Log "config patch failed: $_"
    Write-Host "Warning: updated binaries but failed to patch config.json: $_"
}

Write-Log "updated $runningVersion -> $latestVersion"
Write-Host "DONE: updated to v$latestVersion."
if ($Restart) {
    Write-Host "Starting Sky-Player.exe (-Restart)..."
    try {
        Start-Process -FilePath $ExePath -WorkingDirectory $InstallRoot
    } catch {
        Write-Log "restart failed: $_"
        Write-Host "Restart failed (binaries updated successfully). Reopen Sky-Player.exe manually."
    }
} else {
    Write-Host "Reopen Sky-Player.exe to start the new version."
}
Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
exit 0
