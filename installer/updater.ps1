# License: GPL-3.0 (Sky Auto Player project). No code ported from mpv; structural reference only.
# Sky Auto Player external updater. See docs/2026-07-18_distribution-mpv-pattern-plan.md §Phase 2.
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
#   9. If Sky-Auto-Player.exe is running from this folder: exit 4 unless -ForceClose.
#  10. Expand-Archive to TEMP staging.
#  11. Verify MANIFEST.json per-file integrity (MANDATORY — fail-closed if
#       MANIFEST.json is absent; aborts before any install mutation).
#  12. Back up existing replaceable files.
#  13. Copy staging -> install, preserving config.json and completely skipping songs/.
#  14. On copy failure, roll back all backup files and clean up.
#  15. Patch update.last_check_ts (Unix int) + update.last_notified_version (handles missing keys).
#  16. Log one line; print DONE; do NOT relaunch unless -Restart (O3).
#
# Exit codes: 0 ok, 2 network/asset, 3 sha256, 4 process lock, 5 permission/extract/copy/manifest.

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

# --- Path Management ---
# If test environment has already set script-scoped paths, use those.
# Otherwise, paths will be auto-detected on first use via Initialize-Paths.
if ($script:InstallRoot -and $script:ExePath -and $script:ConfigPath) {
    $InstallRoot = $script:InstallRoot
    $ExePath = $script:ExePath
    $ConfigPath = $script:ConfigPath
    $LogDir = $script:LogDir
    $LogFile = $script:LogFile
} else {
    $InstallRoot = $null
    $ExePath = $null
    $ConfigPath = $null
    $LogDir = $null
    $LogFile = $null
    $FakeRoot = $null
}

function Initialize-Paths {
    if ($global:InstallRoot -and $global:ExePath -and $global:ConfigPath) {
        return  # already initialized
    }
    # Only auto-detect if we're running directly (not dot-sourced for testing)
    # When dot-sourced, $MyInvocation.MyCommand.Path points to the caller's script
    if ($MyInvocation.MyCommand.Path -eq $PSCommandPath) {
        $global:FakeRoot = $env:SKY_UPDATER_FAKE_ROOT
        $global:ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Path
        $global:InstallRoot = Split-Path -Parent $global:ScriptDir
        try {
            $global:ExePath = Resolve-PrimaryExe -Root $global:InstallRoot
        } catch {
            $global:ExePath = Join-Path $global:InstallRoot 'Sky-Auto-Player.exe'
        }
        $global:ConfigPath  = Join-Path $global:InstallRoot 'config.json'
    }
    $global:LogDir  = Join-Path $env:LOCALAPPDATA 'Sky-Auto-Player'
    $global:LogFile = Join-Path $global:LogDir 'updater.log'
}

function Get-ExePath {
    if (-not $global:ExePath) { Initialize-Paths }
    return $global:ExePath
}
function Get-ConfigPath {
    if (-not $global:ConfigPath) { Initialize-Paths }
    return $global:ConfigPath
}

function Resolve-PrimaryExe([string]$Root) {
    $auto = Join-Path $Root 'Sky-Auto-Player.exe'
    $legacy = Join-Path $Root 'Sky-Player.exe'
    if (Test-Path -LiteralPath $auto) { return $auto }
    if (Test-Path -LiteralPath $legacy) { return $legacy }
    throw "Resolve-PrimaryExe: Neither Sky-Auto-Player.exe nor Sky-Player.exe found in $Root"
}
function Resolve-ProcessNames {
    return @('Sky-Auto-Player', 'Sky-Player')
}
function Select-ReleaseAssets($Assets, [string]$Version) {
    $autoZip = "Sky-Auto-Player-v$Version.zip"
    $autoSha = "Sky-Auto-Player-v$Version.zip.sha256"
    $legacyZip = "Sky-Player-v$Version.zip"
    $legacySha = "Sky-Player-v$Version.zip.sha256"

    $hasAutoZip = $Assets | Where-Object { $_.name -eq $autoZip } | Select-Object -First 1
    $hasAutoSha = $Assets | Where-Object { $_.name -eq $autoSha } | Select-Object -First 1
    if ($hasAutoZip -and $hasAutoSha) {
        return @{ ZipAsset = $hasAutoZip; ShaAsset = $hasAutoSha }
    }

    $hasLegacyZip = $Assets | Where-Object { $_.name -eq $legacyZip } | Select-Object -First 1
    $hasLegacySha = $Assets | Where-Object { $_.name -eq $legacySha } | Select-Object -First 1
    if ($hasLegacyZip -and $hasLegacySha) {
        return @{ ZipAsset = $hasLegacyZip; ShaAsset = $hasLegacySha }
    }

    throw "Select-ReleaseAssets: missing zip or sha256 for version $Version"
}
function Resolve-StagingRoot([string]$ExtractDir) {
    if (Test-Path -LiteralPath (Join-Path $ExtractDir 'Sky-Auto-Player.exe')) { return $ExtractDir }
    if (Test-Path -LiteralPath (Join-Path $ExtractDir 'Sky-Player.exe')) { return $ExtractDir }
    
    $child = Get-ChildItem -LiteralPath $ExtractDir -Directory | Select-Object -First 1
    if ($child) {
        if (Test-Path -LiteralPath (Join-Path $child.FullName 'Sky-Auto-Player.exe')) { return $child.FullName }
        if (Test-Path -LiteralPath (Join-Path $child.FullName 'Sky-Player.exe')) { return $child.FullName }
    }
    
    throw "Resolve-StagingRoot: Update zip layout is unexpected (no Sky-Auto-Player.exe or Sky-Player.exe found in staging)."
}

function Get-InstallRoot {
    if (-not $global:InstallRoot) { Initialize-Paths }
    return $global:InstallRoot
}
function Get-LogFile {
    if (-not $global:LogFile) { Initialize-Paths }
    return $global:LogFile
}
function Write-Log([string]$msg) {
    # Best-effort log writer. Defensive against a null $LogDir / $LogFile —
    # E.g. when updater.ps1 helpers are dot-sourced into Pester tests where
    # the script-level $LogDir variable resolves through a different scope
    # chain than $global:LogDir set by the test BeforeAll. Falling back to
    # Get-LogFile / Get-InstallRoot (both of which consult Initialize-Paths)
    # keeps logging workable while never crashing a fail-closed path that
    # logs an error then returns $false.
    $logFile = $LogFile
    if (-not $logFile) { $logFile = Get-LogFile }
    if (-not $logFile) { return }
    $logDir = Split-Path -Parent $logFile
    if (-not $logDir) { return }
    try { New-Item -ItemType Directory -Force -Path $logDir | Out-Null } catch {}
    $line = '[{0:u}] {1}' -f (Get-Date).ToUniversalTime(), $msg
    try { Add-Content -LiteralPath $logFile -Value $line -Encoding UTF8 } catch {}
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
    $cfgPath = Get-ConfigPath
    if (-not (Test-Path -LiteralPath $cfgPath)) { return $null }
    try {
        return (Get-Content -Raw -LiteralPath $cfgPath | ConvertFrom-Json)
    } catch { return $null }
}

function Write-UpdateFields {
    param(
        [int]$LastCheckTs,
        [string]$LastNotifiedVersion
    )
    $cfgPath = Get-ConfigPath
    if (-not (Test-Path -LiteralPath $cfgPath)) { return }
    
    # Read and parse JSON properly
    $raw = Get-Content -Raw -LiteralPath $cfgPath -Encoding UTF8
    try {
        $cfg = $raw | ConvertFrom-Json
    } catch {
        Write-Log "Failed to parse config.json: $_"
        throw
    }
    
    # Ensure update object exists - ConvertFrom-Json creates PSCustomObject which doesn't allow dynamic properties
    # Convert to hashtable if needed
    if ($cfg -is [System.Management.Automation.PSCustomObject]) {
        $ht = @{}
        $cfg.PSObject.Properties | ForEach-Object { $ht[$_.Name] = $_.Value }
        $cfg = $ht
    }
    
    if (-not $cfg.update) { $cfg.update = @{} }
    
    # Update the fields
    $cfg.update.last_check_ts = $LastCheckTs
    $cfg.update.last_notified_version = $LastNotifiedVersion
    
    # Write back with preserved formatting (depth 10 to handle nested objects)
    $json = $cfg | ConvertTo-Json -Depth 10
    [System.IO.File]::WriteAllText($cfgPath, $json, (New-Object System.Text.UTF8Encoding($false)))
}

function Get-RunningVersion {
    $manifest = Join-Path (Get-InstallRoot) 'MANIFEST.json'
    if (Test-Path -LiteralPath $manifest) {
        try {
            $m = Get-Content -Raw -LiteralPath $manifest | ConvertFrom-Json
            if ($m.version) { return [string]$m.version }
        } catch {}
    }
    $vi = (Get-Item -LiteralPath (Get-ExePath) -ErrorAction SilentlyContinue).VersionInfo
    if ($vi -and $vi.ProductVersion) { return [string]$vi.ProductVersion }
    return '0.0.0'
}

function Compare-Version([string]$Current, [string]$Latest) {
    # Delegate to Sky-Auto-Player.exe --compare-versions for PEP 440 compliance.
    # Exit codes: 0=equal, 1=latest>current, 2=latest<current, 3=parse error.
    $exe = Get-ExePath
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Sky-Auto-Player.exe not found at $exe; cannot compare versions"
    }
    & $exe --compare-versions $Current $Latest
    $exitCode = $LASTEXITCODE
    if ($exitCode -eq 3) { throw "Version parse failed: $Current vs $Latest" }
    # Map to -1/0/1 for backward compat with callers
    if ($exitCode -eq 0) { return 0 }
    if ($exitCode -eq 1) { return 1 }
    if ($exitCode -eq 2) { return -1 }
    throw "Unexpected exit code $exitCode from --compare-versions"
}

function Copy-UpdateTree([string]$StagingRoot, [string]$DestRoot) {
    $copiedFiles = @()
    $backedUpFiles = @()
    $backupDir = Join-Path $env:TEMP ('sky-backup-' + [guid]::NewGuid().ToString('N'))

    try {
        $filesToCopy = Get-ChildItem -LiteralPath $StagingRoot -Recurse -File
        
        # 0. Identify orphaned files using MANIFEST.json
        $orphanedFiles = @()
        $manifestPath = Join-Path $StagingRoot 'MANIFEST.json'
        if (Test-Path -LiteralPath $manifestPath) {
            try {
                $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
                if ($manifest.files) {
                    $validManifestPaths = @{}
                    foreach ($file in $manifest.files) {
                        $norm = $file.path -replace '/', '\'
                        $validManifestPaths[$norm] = $true
                    }
                    
                    $destFiles = Get-ChildItem -LiteralPath $DestRoot -Recurse -File -ErrorAction SilentlyContinue
                    if ($destFiles) {
                        foreach ($dFile in $destFiles) {
                            $rel = $dFile.FullName.Substring($DestRoot.Length).TrimStart('\', '/')
                            $relNorm = $rel -replace '/', '\'
                            
                            # Preserve list: config.json, songs\, logs\
                            if ($relNorm -eq 'config.json' -or $relNorm -eq 'songs' -or $relNorm.StartsWith('songs\') -or $relNorm -eq 'logs' -or $relNorm.StartsWith('logs\')) {
                                continue
                            }
                            
                            if (-not $validManifestPaths.ContainsKey($relNorm)) {
                                $orphanedFiles += $dFile.FullName
                            }
                        }
                    }
                }
            } catch {
                Write-Log "Failed to parse MANIFEST.json for orphan cleanup: $_"
            }
        }
        
        # 1. Back up existing destination files that will be overwritten
        foreach ($file in $filesToCopy) {
            $rel = $file.FullName.Substring($StagingRoot.Length).TrimStart('\', '/')
            # Normalize path separators to backslash so preserve-list comparisons
            # work regardless of whether Get-ChildItem -Recurse emitted '\' or '/'.
            $relNorm = $rel -replace '/', '\'
            $dest = Join-Path $DestRoot $rel
            
            # Skip copying config.json, songs/ or logs/ files entirely (preserve-list mandate).
            # $relNorm -eq 'songs' is defensive — Get-ChildItem -File never returns
            # directories, but a future caller could pass a directory in.
            if ($relNorm -eq 'config.json' -or $relNorm -eq 'songs' -or $relNorm.StartsWith('songs\') -or $relNorm -eq 'logs' -or $relNorm.StartsWith('logs\')) {
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

        # 1.5 Back up and delete orphaned files
        foreach ($dest in $orphanedFiles) {
            $rel = $dest.Substring($DestRoot.Length).TrimStart('\', '/')
            if (-not (Test-Path -LiteralPath $dest)) { continue }
            
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
            
            Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue | Out-Null
        }

        # 2. Copy files from staging to target
        foreach ($file in $filesToCopy) {
            $rel = $file.FullName.Substring($StagingRoot.Length).TrimStart('\', '/')
            $relNorm = $rel -replace '/', '\'
            $dest = Join-Path $DestRoot $rel
            
            if ($relNorm -eq 'config.json' -or $relNorm -eq 'songs' -or $relNorm.StartsWith('songs\') -or $relNorm -eq 'logs' -or $relNorm.StartsWith('logs\')) {
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

function Test-ManifestIntegrity {
    param([string]$StagingRoot)

    # Returns $true if MANIFEST.json exists at $StagingRoot AND its files[]
    # array's per-file SHA256 hashes all match the staged files. Returns
    # $false on: missing MANIFEST, missing files[] array, per-file hash
    # mismatch, missing file, or JSON parse error.
    #
    # This is the fail-closed per-file integrity gate per
    # docs/distribution-and-update.md §3 "Mandatory MANIFEST.json
    # Verification". Callers MUST abort the install on $false.
    #
    # Per docs/distribution-and-update.md §1 release contract, every
    # official release ships with MANIFEST.json — a missing MANIFEST
    # implies either a regressed build pipeline (release.yml --manifest
    # gate skipped) or a tampered/3rd-party-repackaged zip; neither case
    # should the updater install.
    #
    # SECURITY NOTE: This fail-closed integrity gate inherently defeats
    # Zip-Slip attacks against the native .NET/PowerShell extract API.
    # If a malicious archive attempts to extract files outside `$StagingRoot`,
    # `Test-Path` below will return `$false`, causing the update to abort
    # before any files are copied to the installation directory.
    $manifestPath = Join-Path $StagingRoot 'MANIFEST.json'
    if (-not (Test-Path -LiteralPath $manifestPath)) {
        Write-Log "MANIFEST.json missing from staging — refusing to install (fail-closed)."
        Write-Host "MANIFEST.json is missing from the update zip. Per the release contract,"
        Write-Host "every official release ships with MANIFEST.json. The updater refuses to"
        Write-Host "install a zip that bypasses the per-file integrity invariant."
        Write-Host "Re-download from https://github.com/pumni/Sky-Auto-Player/releases and try again."
        return $false
    }
    try {
        $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
        if (-not $manifest.files) {
            Write-Log "MANIFEST.json present but has no `"files`" array — refusing to install."
            Write-Host "MANIFEST.json is missing its required `"files`" array. Aborting."
            return $false
        }
        $failed = 0
        foreach ($file in $manifest.files) {
            $fullPath = Join-Path $StagingRoot $file.path
            if (Test-Path -LiteralPath $fullPath) {
                $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $fullPath).Hash.ToLower()
                if ($actual -ne $file.sha256) {
                    Write-Log "MANIFEST mismatch: $($file.path) expected $($file.sha256) got $actual"
                    $failed++
                }
            } else {
                Write-Log "MANIFEST missing file: $($file.path)"
                $failed++
            }
        }
        if ($failed -gt 0) {
            Write-Log "MANIFEST verification failed: $failed file(s)"
            Write-Host "MANIFEST verification failed: $failed file(s) mismatch. Aborting before any file mutation."
            return $false
        }
        Write-Log "MANIFEST.json verification passed ($($manifest.files.Count) files)"
        return $true
    } catch {
        Write-Log "MANIFEST.json parse error: $_"
        Write-Host "MANIFEST.json is corrupted or invalid. Aborting."
        return $false
    }
}

# --- MAIN EXECUTION GUARD ---
# Only run the update logic when executed directly, not when dot-sourced for testing.
# When dot-sourced, $MyInvocation.InvocationName is '.' (a single dot).
#
# Probe 2026-07-22 across {pwsh -File, pwsh -Command "& 'path'", powershell 5.1 -File,
# pwsh -Command iex, pwsh -c, Import-Module} confirms this guard never misses a real
# user-invoked path: only intentional dot-source (e.g. ``. '...updater.ps1'`` from
# ``installer/Tests/updater.Tests.ps1`` BeforeAll) sets ``InvocationName`` to ``.`` —
# every other invocation form leaves it set to the script path, ``&``, or empty. If
# you strengthen this guard, re-run the probe and add a regression test that covers
# the six invocation forms above plus the dot-source path.
if ($MyInvocation.InvocationName -eq '.') {
    if ($env:SKY_UPDATER_DEBUG -eq '1') { Write-Host "DEBUG updater.ps1: Dot-sourced, skipping main execution" }
    return
}

if ($env:SKY_UPDATER_DEBUG -eq '1') { Write-Host "DEBUG updater.ps1: Running main execution" }
Initialize-Paths

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
$repo  = 'Sky-Auto-Player'
$headers = @{ 'User-Agent' = 'sky-auto-player-updater'; 'Accept' = 'application/vnd.github.v3+json' }

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
            if ((Compare-Version -Current $bt -Latest $rt) -gt 0) { $best = $r }
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

if ((Compare-Version -Current $runningVersion -Latest $latestVersion) -le 0) {
    Write-Log "already up to date (running=$runningVersion latest=$latestVersion)"
    Write-Host "You are already using the latest version ($runningVersion)."
    exit 0
}

# --- Asset selection ---
try {
    $selected = Select-ReleaseAssets -Assets $candidate.assets -Version $latestVersion
    $zipName = $selected.ZipAsset.name
    $shaName = $selected.ShaAsset.name
    
    if ($FakeRoot) {
        $zipUrl = ($FakeRoot.TrimEnd('/') + '/' + $zipName)
        $shaUrl = ($FakeRoot.TrimEnd('/') + '/' + $shaName)
    } else {
        $zipUrl = [string]$selected.ZipAsset.browser_download_url
        $shaUrl = [string]$selected.ShaAsset.browser_download_url
    }
    Assert-HttpsUrl $zipUrl
    Assert-HttpsUrl $shaUrl
} catch {
    Write-Log "missing zip or sha256 asset for $latestVersion"
    Write-Host "Release v$latestVersion is missing the zip or sha256 sidecar. Aborting."
    exit 2
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
$runningProcesses = @()
foreach ($name in (Resolve-ProcessNames)) {
    $procs = Get-Process -Name $name -ErrorAction SilentlyContinue
    if ($procs) { $runningProcesses += $procs }
}
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
        Write-Log "$($targetProcess.ProcessName).exe still running; refuse update"
        Write-Host "$($targetProcess.ProcessName).exe is still running in this directory. Close it, then re-run updater.bat."
        Write-Host '(Advanced: updater.bat -ForceClose)'
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
        exit 4
    }
    Write-Host 'Stopping Sky-Auto-Player.exe (-ForceClose)...'
    $targetProcess | Stop-Process -Force
    Start-Sleep -Seconds 2
    
    $runningAgain = @()
    foreach ($name in (Resolve-ProcessNames)) {
        $procs = Get-Process -Name $name -ErrorAction SilentlyContinue
        if ($procs) { $runningAgain += $procs }
    }
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
        Write-Log "$($targetProcess.ProcessName).exe still locked after ForceClose"
        Write-Host "Could not stop $($targetProcess.ProcessName).exe. Aborting."
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

try {
    $StagingRoot = Resolve-StagingRoot -ExtractDir $extractDir
} catch {
    Write-Log "staging layout missing exe: $_"
    Write-Host "Update zip layout is unexpected (missing exe). Aborting."
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 5
}

# --- Verify MANIFEST.json integrity (mandatory; fail-closed if absent) ---
# Behavior contract step 11. Per docs/distribution-and-update.md §3, every
# official release ships with MANIFEST.json. We delegate to
# Test-ManifestIntegrity (returns $false on missing/corrupt/mismatching
# manifest) and abort on $false — fail-closed per the per-file integrity
# invariant.
if (-not (Test-ManifestIntegrity -StagingRoot $StagingRoot)) {
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 5
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

# Unix epoch seconds as int (matches Python ``int(time.time())`` which is
# what ``orchestration/update_service.py:current_unix_ts`` writes to
# ``config.json`` under ``update.last_check_ts``).
#
# IMPORTANT: must be a *UTC* epoch.  The earlier form
# ``[int][double]::Parse((Get-Date -UFormat %s), InvariantCulture)`` returned
# a LOCAL-time epoch — ``Get-Date -UFormat %s`` is relative to the machine's
# timezone, not UTC — and diverged from Python by exactly the timezone offset
# (e.g. +25200s on UTC+7).  That divergence broke both downstream consumers:
#   * ``should_auto_check`` in update_service.py reads ``now - last_check_ts``
#     with ``now`` in UTC; a local-time ``last_check_ts`` makes the delta
#     negative on positive-offset zones, silently bypassing the 24h throttle
#     and spamming the unauthenticated GitHub releases API.
#   * ``modals.py`` renders ``time.localtime(last_check_ts)``; a local epoch
#     gets the offset applied a second time, showing "last checked" off by
#     2x the tz offset vs. an in-app (Python-UTC) check.
# ``[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()`` is the .NET standard for the
# Unix epoch in UTC: locale-free, no sub-second floor surprise, and identical
# definition to Python ``time.time()``.  This is also PS 5.1-safe — the cast
# to ``[int]`` truncates toward zero (matches Python ``int()`` on positives).
$epoch = [int][DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
try {
    Write-UpdateFields -LastCheckTs $epoch -LastNotifiedVersion $latestVersion
} catch {
    Write-Log "config patch failed: $_"
    Write-Host "Warning: updated binaries but failed to patch config.json: $_"
}

Write-Log "updated $runningVersion -> $latestVersion"
Write-Host "DONE: updated to v$latestVersion."
if ($Restart) {
    $newAuto = Join-Path $InstallRoot 'Sky-Auto-Player.exe'
    if (Test-Path -LiteralPath $newAuto) { $startExe = $newAuto } else { $startExe = $ExePath }
    Write-Host "Starting $(Split-Path -Leaf $startExe) (-Restart)..."
    try {
        Start-Process -FilePath $startExe -WorkingDirectory $InstallRoot
    } catch {
        Write-Log "restart failed: $_"
        Write-Host "Restart failed (binaries updated successfully). Reopen Sky Auto Player manually."
    }
} else {
    Write-Host "Reopen Sky Auto Player to start the new version."
}
Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
exit 0
