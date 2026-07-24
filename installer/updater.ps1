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
#   7. If either app executable is running from this folder: exit 4 unless -ForceClose.
#   8. Recover any durable `prepared` transaction before reading installed version.
#   9. Newer -> download zip + .sha256 (HTTPS allow-list only, bounded timeout).
#  10. Verify outer SHA256, then validate every zip entry before TEMP extraction.
#  11. Verify exact staged file set, manifest version, executable SHA256, and every
#       manifest-listed payload hash (mandatory, fail-closed before mutation).
#  12. Atomically write a durable journal + complete backup under the install root.
#  13. Copy the verified allow-list only; preserve config.json, .env, songs/, and logs/.
#  14. Verify copied hashes; commit journal. On failure/interruption, roll back.
#       Never delete the durable backup when any restore fails.
#  15. Atomically patch update.last_check_ts + update.last_notified_version.
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
}

function Initialize-Paths {
    if ($global:InstallRoot -and $global:ExePath -and $global:ConfigPath) {
        return  # already initialized
    }
    # Auto-detect paths only when this script is invoked as a script (``pwsh -File
    # updater.ps1``), not when dot-sourced for testing. Two reliable signals:
    #   * ``$PSCommandPath`` — set to the running script's full path under
    #     ``-File`` and ``-Command "& 'path'"`` invocation; ``$null`` when
    #     dot-sourced from a Pester test (the dot operator runs in the
    #     caller's scope, so ``$PSCommandPath`` belongs to the caller).
    #   * ``$MyInvocation.MyCommand.Path`` — set at *script body* scope, but
    #     ``$null`` inside a *function call* (PowerShell functions do not
    #     own a command path). The previous guard
    #     ``$MyInvocation.MyCommand.Path -eq $PSCommandPath`` always
    #     compared ``$null`` (inside this function) against the real path
    #     and never matched, leaving ``$global:InstallRoot`` empty and
    #     breaking every ``updater.bat`` invocation with
    #     ``Cannot bind argument to parameter 'Path' because it is an empty
    #     string`` at the ``Test-WriteAccess $InstallRoot`` gate.
    # Fix: drive auto-detection off ``$PSCommandPath`` only, which is the
    # reliable ``-File`` marker and is ``$null`` under dot-source (so the
    # Pester ``BeforeAll`` pre-set globals take precedence as designed).
    if ($PSCommandPath) {

        $global:ScriptDir   = Split-Path -Parent $PSCommandPath
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
# Compute a path relative to `$Base`, robust against 8.3 short-name path
# normalization mismatches between `$env:TEMP` (often returned as
# ``C:\\Users\\PE4CE_~1\\...`` under short-name-enabled volumes) and the long
# names that ``Get-ChildItem -Recurse`` emits (e.g. ``pe4cE_HOA``).
#
# ``.Substring($Base.Length)`` is **unsafe** under that mismatch: it
# truncates a *long* fullname using a *short* base length, leaving a leading
# fragment (we observed ``t\\`` showing up as a phantom top-level directory
# in the install root, which is exactly the leftover char of ``...\\extract``
# minus ``$Base.Length``). The bug here would otherwise let the v2.4.2
# cutover brick every install whose ``%TEMP%`` is short-named.
#
# Use FileSystemObject absolute-path normalization + case-insensitive prefix
# comparison; fall back to ``GetRelativePath`` for the residual edge cases.
function Get-RelativePathSafe([string]$Base, [string]$Full) {
    if ([string]::IsNullOrEmpty($Base) -or [string]::IsNullOrEmpty($Full)) { return $null }
    $baseFull = (New-Object -ComObject Scripting.FileSystemObject).GetAbsolutePathName($Base)
    $fullNorm = (New-Object -ComObject Scripting.FileSystemObject).GetAbsolutePathName($Full)
    # Preserve the full normalized strings. Truncating either side to legacy
    # MAX_PATH changes the relative path and can redirect a copied file.
    if ($fullNorm.Length -ge $baseFull.Length -and
        $fullNorm.Substring(0, $baseFull.Length).Equals($baseFull, [StringComparison]::OrdinalIgnoreCase)) {
        return $fullNorm.Substring($baseFull.Length).TrimStart('\', '/')
    }
    try {
        $rel = [System.IO.Path]::GetRelativePath($Base, $Full)
    } catch {
        return $null
    }
    if ($rel -eq '.' -or $rel -eq $Base) { return '' }
    return ($rel -replace '^[\\]?', '')
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

    $raw = Get-Content -Raw -LiteralPath $cfgPath -Encoding UTF8
    try {
        $parsed = $raw | ConvertFrom-Json
    } catch {
        Write-Log "Failed to parse config.json: $_"
        throw
    }

    $cfg = @{}
    if ($parsed -is [System.Management.Automation.PSCustomObject]) {
        $parsed.PSObject.Properties | ForEach-Object { $cfg[$_.Name] = $_.Value }
    } elseif ($parsed -is [System.Collections.IDictionary]) {
        foreach ($key in $parsed.Keys) { $cfg[$key] = $parsed[$key] }
    } else {
        throw 'config.json must contain a JSON object'
    }

    $update = @{}
    $existingUpdate = $cfg['update']
    if ($existingUpdate -is [System.Management.Automation.PSCustomObject]) {
        $existingUpdate.PSObject.Properties | ForEach-Object { $update[$_.Name] = $_.Value }
    } elseif ($existingUpdate -is [System.Collections.IDictionary]) {
        foreach ($key in $existingUpdate.Keys) { $update[$key] = $existingUpdate[$key] }
    } elseif ($null -ne $existingUpdate) {
        throw 'config.json update field must contain a JSON object'
    }

    $update['last_check_ts'] = $LastCheckTs
    $update['last_notified_version'] = $LastNotifiedVersion
    $cfg['update'] = $update

    $json = $cfg | ConvertTo-Json -Depth 100
    $token = [guid]::NewGuid().ToString('N')
    $tmpPath = "$cfgPath.tmp-$token"
    $replaceBackup = "$cfgPath.replace-backup-$token"
    try {
        [System.IO.File]::WriteAllText(
            $tmpPath,
            $json,
            (New-Object System.Text.UTF8Encoding($false))
        )
        [System.IO.File]::Replace($tmpPath, $cfgPath, $replaceBackup, $true)
        Remove-Item -LiteralPath $replaceBackup -Force -ErrorAction SilentlyContinue
    } finally {
        Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
    }
}

function Read-Sha256Sidecar {
    param(
        [string]$SidecarPath,
        [string]$ExpectedFileName
    )
    $sidecarText = (Get-Content -Raw -LiteralPath $SidecarPath -Encoding ASCII).Trim()
    $pattern = '^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($ExpectedFileName) + '$'
    $match = [regex]::Match($sidecarText, $pattern)
    if (-not $match.Success) {
        throw "SHA256 sidecar is not bound to expected asset $ExpectedFileName"
    }
    return $match.Groups[1].Value.ToLower()
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

function ConvertTo-SafeRelativePath([string]$RelativePath) {
    if ([string]::IsNullOrWhiteSpace($RelativePath)) {
        throw 'Relative path is empty.'
    }
    $normalized = $RelativePath.Replace('/', '\')
    if ([System.IO.Path]::IsPathRooted($normalized) -or $normalized.Contains(':')) {
        throw "Unsafe rooted or stream path: $RelativePath"
    }
    $segments = $normalized.Split('\')
    foreach ($segment in $segments) {
        if ([string]::IsNullOrWhiteSpace($segment) -or $segment -eq '.' -or $segment -eq '..') {
            throw "Unsafe path segment in: $RelativePath"
        }
        if ($segment.EndsWith('.') -or $segment.EndsWith(' ')) {
            throw "Ambiguous Windows path segment in: $RelativePath"
        }
    }
    return [string]::Join('\', $segments)
}

function Resolve-SafeChildPath([string]$Base, [string]$RelativePath) {
    $relative = ConvertTo-SafeRelativePath $RelativePath
    $baseFull = [System.IO.Path]::GetFullPath($Base).TrimEnd('\') + '\'
    $full = [System.IO.Path]::GetFullPath((Join-Path $Base $relative))
    if (-not $full.StartsWith($baseFull, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path escapes base directory: $RelativePath"
    }
    return [pscustomobject]@{ Relative = $relative; FullPath = $full }
}

function Assert-ZipArchiveSafe([string]$ZipPath) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
    $archive = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
    try {
        $seen = @{}
        foreach ($entry in $archive.Entries) {
            $raw = [string]$entry.FullName
            if ([string]::IsNullOrWhiteSpace($raw)) {
                throw 'Zip contains an empty entry name.'
            }
            $trimmed = $raw.TrimEnd([char[]]@('/', '\'))
            if ([string]::IsNullOrWhiteSpace($trimmed)) {
                throw "Zip contains an invalid root entry: $raw"
            }
            $relative = ConvertTo-SafeRelativePath $trimmed
            if ($seen.ContainsKey($relative)) {
                throw "Zip contains a duplicate or case-colliding entry: $relative"
            }
            $seen[$relative] = $true

            $unixFileType = (($entry.ExternalAttributes -shr 16) -band 0xF000)
            if ($unixFileType -eq 0xA000) {
                throw "Zip symbolic links are not allowed: $relative"
            }
        }
    } finally {
        $archive.Dispose()
    }
}

function Test-PreservedRelativePath([string]$RelativePath) {
    $rel = $RelativePath.Replace('/', '\')
    return (
        $rel -eq 'config.json' -or $rel -eq '.env' -or
        $rel -eq 'songs' -or $rel.StartsWith('songs\', [StringComparison]::OrdinalIgnoreCase) -or
        $rel -eq 'logs' -or $rel.StartsWith('logs\', [StringComparison]::OrdinalIgnoreCase)
    )
}

function Write-TransactionJournal {
    param(
        [string]$TransactionDir,
        [string]$State,
        [string[]]$BackedUp,
        [string[]]$NewFiles
    )
    $journalPath = Join-Path $TransactionDir 'journal.json'
    $tmpPath = "$journalPath.tmp-$([guid]::NewGuid().ToString('N'))"
    $payload = @{
        schema_version = 1
        state = $State
        backed_up = @($BackedUp)
        new_files = @($NewFiles)
    } | ConvertTo-Json -Depth 10
    try {
        [System.IO.File]::WriteAllText(
            $tmpPath,
            $payload,
            (New-Object System.Text.UTF8Encoding($false))
        )
        if (Test-Path -LiteralPath $journalPath) {
            $replaceBackup = "$journalPath.replace-backup-$([guid]::NewGuid().ToString('N'))"
            [System.IO.File]::Replace($tmpPath, $journalPath, $replaceBackup, $true)
            Remove-Item -LiteralPath $replaceBackup -Force -ErrorAction SilentlyContinue
        } else {
            [System.IO.File]::Move($tmpPath, $journalPath)
        }
    } finally {
        Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
    }
}

function Recover-InterruptedUpdate([string]$DestRoot) {
    $transactionDir = Join-Path $DestRoot '.sky-update-transaction'
    if (-not (Test-Path -LiteralPath $transactionDir)) { return $true }

    $journalPath = Join-Path $transactionDir 'journal.json'
    if (-not (Test-Path -LiteralPath $journalPath)) {
        # Copy-UpdateTree never mutates the install before journal.json is
        # atomically committed. A directory without a journal is preparation
        # debris only and can be removed safely.
        Remove-Item -LiteralPath $transactionDir -Recurse -Force -ErrorAction Stop
        return $true
    }

    try {
        $journal = Get-Content -Raw -LiteralPath $journalPath -Encoding UTF8 | ConvertFrom-Json
        if ($journal.schema_version -ne 1) { throw 'Unsupported update journal schema.' }
        if ($journal.state -eq 'committed') {
            Remove-Item -LiteralPath $transactionDir -Recurse -Force -ErrorAction Stop
            return $true
        }
        if ($journal.state -ne 'prepared') { throw 'Invalid update journal state.' }

        $backupRoot = Join-Path $transactionDir 'backup'
        foreach ($relativePath in @($journal.backed_up)) {
            $destInfo = Resolve-SafeChildPath -Base $DestRoot -RelativePath ([string]$relativePath)
            $backupInfo = Resolve-SafeChildPath -Base $backupRoot -RelativePath ([string]$relativePath)
            if (-not (Test-Path -LiteralPath $backupInfo.FullPath -PathType Leaf)) {
                throw "Backup is missing for $relativePath"
            }
            $destDir = Split-Path -Parent $destInfo.FullPath
            if (-not (Test-Path -LiteralPath $destDir)) {
                New-Item -ItemType Directory -Force -Path $destDir | Out-Null
            }
            Copy-Item -LiteralPath $backupInfo.FullPath -Destination $destInfo.FullPath -Force -ErrorAction Stop
            $backupHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $backupInfo.FullPath).Hash
            $restoredHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $destInfo.FullPath).Hash
            if ($backupHash -ne $restoredHash) {
                throw "Restored file hash mismatch: $relativePath"
            }
        }

        foreach ($relativePath in @($journal.new_files)) {
            $destInfo = Resolve-SafeChildPath -Base $DestRoot -RelativePath ([string]$relativePath)
            if (Test-Path -LiteralPath $destInfo.FullPath) {
                Remove-Item -LiteralPath $destInfo.FullPath -Force -ErrorAction Stop
            }
        }

        Remove-Item -LiteralPath $transactionDir -Recurse -Force -ErrorAction Stop
        Write-Log 'Recovered an interrupted update transaction.'
        return $true
    } catch {
        Write-Log "Interrupted update recovery failed; backup retained at $transactionDir : $_"
        Write-Host "Recovery failed. Backup retained at: $transactionDir"
        Write-Host "Resolve the file lock or permission error, then run updater.bat again. Details: $_"
        return $false
    }
}

function Copy-UpdateTree {
    param(
        [string]$StagingRoot,
        [string]$DestRoot,
        [string[]]$RelativePaths
    )

    $transactionDir = Join-Path $DestRoot '.sky-update-transaction'
    if (Test-Path -LiteralPath $transactionDir) {
        throw "Unresolved update transaction exists at $transactionDir"
    }

    $copyPaths = @($RelativePaths)
    if (-not $copyPaths) {
        $copyPaths = @(
            Get-ChildItem -LiteralPath $StagingRoot -Recurse -File | ForEach-Object {
                $relative = Get-RelativePathSafe -Base $StagingRoot -Full $_.FullName
                if ([string]::IsNullOrWhiteSpace($relative)) {
                    throw "Could not derive staging relative path for $($_.FullName)"
                }
                ConvertTo-SafeRelativePath $relative
            }
        )
    }

    $validPaths = @{}
    $filesToCopy = @()
    foreach ($relativePath in $copyPaths) {
        $sourceInfo = Resolve-SafeChildPath -Base $StagingRoot -RelativePath $relativePath
        if ($validPaths.ContainsKey($sourceInfo.Relative)) {
            throw "Duplicate staging path: $($sourceInfo.Relative)"
        }
        if (-not (Test-Path -LiteralPath $sourceInfo.FullPath -PathType Leaf)) {
            throw "Verified staging file is missing: $($sourceInfo.Relative)"
        }
        $validPaths[$sourceInfo.Relative] = $true
        $destInfo = Resolve-SafeChildPath -Base $DestRoot -RelativePath $sourceInfo.Relative
        $filesToCopy += [pscustomobject]@{
            Relative = $sourceInfo.Relative
            Source = $sourceInfo.FullPath
            Destination = $destInfo.FullPath
            Preserved = (Test-PreservedRelativePath $sourceInfo.Relative)
            Existed = (Test-Path -LiteralPath $destInfo.FullPath -PathType Leaf)
        }
    }

    $orphaned = @()
    $destFiles = Get-ChildItem -LiteralPath $DestRoot -Recurse -File -ErrorAction Stop
    foreach ($destFile in @($destFiles)) {
        $relative = Get-RelativePathSafe -Base $DestRoot -Full $destFile.FullName
        if ([string]::IsNullOrWhiteSpace($relative)) {
            throw "Could not derive install relative path for $($destFile.FullName)"
        }
        $relative = ConvertTo-SafeRelativePath $relative
        if ($relative.StartsWith('.sky-update-transaction\', [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        if (Test-PreservedRelativePath $relative) { continue }
        if (-not $validPaths.ContainsKey($relative)) {
            $orphaned += [pscustomobject]@{ Relative = $relative; FullPath = $destFile.FullName }
        }
    }

    $backupRoot = Join-Path $transactionDir 'backup'
    $backedUp = @()
    $newFiles = @()
    $backupSet = @{}
    try {
        New-Item -ItemType Directory -Force -Path $backupRoot | Out-Null

        foreach ($file in $filesToCopy) {
            if ($file.Preserved) { continue }
            if ($file.Existed) { $backupSet[$file.Relative] = $true } else { $newFiles += $file.Relative }
        }
        foreach ($orphan in $orphaned) { $backupSet[$orphan.Relative] = $true }

        foreach ($relativePath in $backupSet.Keys) {
            $sourceInfo = Resolve-SafeChildPath -Base $DestRoot -RelativePath $relativePath
            $backupInfo = Resolve-SafeChildPath -Base $backupRoot -RelativePath $relativePath
            $backupDir = Split-Path -Parent $backupInfo.FullPath
            if (-not (Test-Path -LiteralPath $backupDir)) {
                New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
            }
            Copy-Item -LiteralPath $sourceInfo.FullPath -Destination $backupInfo.FullPath -Force -ErrorAction Stop
            $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $sourceInfo.FullPath).Hash
            $backupHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $backupInfo.FullPath).Hash
            if ($sourceHash -ne $backupHash) {
                throw "Backup hash mismatch: $relativePath"
            }
            $backedUp += $relativePath
        }

        Write-TransactionJournal `
            -TransactionDir $transactionDir `
            -State 'prepared' `
            -BackedUp $backedUp `
            -NewFiles $newFiles

        foreach ($orphan in $orphaned) {
            Remove-Item -LiteralPath $orphan.FullPath -Force -ErrorAction Stop
        }

        foreach ($file in $filesToCopy) {
            if ($file.Preserved) { continue }
            $destDir = Split-Path -Parent $file.Destination
            if (-not (Test-Path -LiteralPath $destDir)) {
                New-Item -ItemType Directory -Force -Path $destDir | Out-Null
            }
            Copy-Item -LiteralPath $file.Source -Destination $file.Destination -Force -ErrorAction Stop
            $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.Source).Hash
            $destHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.Destination).Hash
            if ($sourceHash -ne $destHash) {
                throw "Post-copy hash mismatch: $($file.Relative)"
            }
        }

        Write-TransactionJournal `
            -TransactionDir $transactionDir `
            -State 'committed' `
            -BackedUp $backedUp `
            -NewFiles $newFiles
        Remove-Item -LiteralPath $transactionDir -Recurse -Force -ErrorAction Stop
    } catch {
        $originalError = $_
        Write-Log "Error during copy: $_. Rolling back..."
        Write-Host "Copy failed: $_. Rolling back files to pre-update state..."
        $recovered = Recover-InterruptedUpdate -DestRoot $DestRoot
        if (-not $recovered) {
            Write-Host "Automatic rollback was incomplete; the durable backup was NOT deleted."
        }
        throw $originalError
    }
}

function Get-VerifiedManifest {
    param(
        [string]$StagingRoot,
        [string]$ExpectedVersion,
        [string]$ExpectedExecutable
    )

    $manifestPath = Join-Path $StagingRoot 'MANIFEST.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw 'MANIFEST.json is missing from staging.'
    }

    $manifest = Get-Content -Raw -LiteralPath $manifestPath -Encoding UTF8 | ConvertFrom-Json
    if ($manifest.app -ne 'Sky-Auto-Player') {
        throw 'MANIFEST.json has an unexpected app identifier.'
    }
    if ([string]::IsNullOrWhiteSpace([string]$manifest.version)) {
        throw 'MANIFEST.json is missing version.'
    }
    if ($ExpectedVersion -and [string]$manifest.version -ne $ExpectedVersion) {
        throw "MANIFEST version $($manifest.version) does not match selected release $ExpectedVersion."
    }
    if (-not $manifest.files) {
        throw 'MANIFEST.json is missing a non-empty files array.'
    }

    $executable = [string]$manifest.executable
    if ($executable -ne 'Sky-Auto-Player.exe' -and $executable -ne 'Sky-Player.exe') {
        throw "Unexpected manifest executable: $executable"
    }
    if ($ExpectedExecutable -and $executable -ne $ExpectedExecutable) {
        throw "MANIFEST executable $executable does not match asset contract $ExpectedExecutable."
    }
    $executableHash = [string]$manifest.executable_sha256
    if ($executableHash -notmatch '^[0-9a-fA-F]{64}$') {
        throw 'MANIFEST executable_sha256 is missing or invalid.'
    }

    $expectedPaths = @{}
    $copyPaths = @()
    $exeInfo = Resolve-SafeChildPath -Base $StagingRoot -RelativePath $executable
    if (-not (Test-Path -LiteralPath $exeInfo.FullPath -PathType Leaf)) {
        throw "Manifest executable is missing: $executable"
    }
    $actualExeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exeInfo.FullPath).Hash.ToLower()
    if ($actualExeHash -ne $executableHash.ToLower()) {
        throw "Executable hash mismatch: $executable"
    }
    $expectedPaths[$exeInfo.Relative] = $true
    $copyPaths += $exeInfo.Relative

    foreach ($file in @($manifest.files)) {
        $relativePath = [string]$file.path
        $expectedHash = [string]$file.sha256
        if ($expectedHash -notmatch '^[0-9a-fA-F]{64}$') {
            throw "Invalid SHA256 for manifest path: $relativePath"
        }
        $fileInfo = Resolve-SafeChildPath -Base $StagingRoot -RelativePath $relativePath
        if ($expectedPaths.ContainsKey($fileInfo.Relative)) {
            throw "Duplicate manifest path: $($fileInfo.Relative)"
        }
        if (-not (Test-Path -LiteralPath $fileInfo.FullPath -PathType Leaf)) {
            throw "Manifest-listed file is missing: $($fileInfo.Relative)"
        }
        $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $fileInfo.FullPath).Hash.ToLower()
        if ($actualHash -ne $expectedHash.ToLower()) {
            throw "Manifest hash mismatch: $($fileInfo.Relative)"
        }
        $expectedPaths[$fileInfo.Relative] = $true
        $copyPaths += $fileInfo.Relative
    }

    $expectedPaths['MANIFEST.json'] = $true
    $copyPaths += 'MANIFEST.json'

    $actualPaths = @{}
    foreach ($item in (Get-ChildItem -LiteralPath $StagingRoot -Recurse -File)) {
        if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Reparse-point file is not allowed in staging: $($item.FullName)"
        }
        $relative = Get-RelativePathSafe -Base $StagingRoot -Full $item.FullName
        if ([string]::IsNullOrWhiteSpace($relative)) {
            throw "Could not derive staging relative path for $($item.FullName)"
        }
        $relative = ConvertTo-SafeRelativePath $relative
        if ($actualPaths.ContainsKey($relative)) {
            throw "Case-colliding or duplicate staged path: $relative"
        }
        $actualPaths[$relative] = $true
        if (-not $expectedPaths.ContainsKey($relative)) {
            throw "Unmanifested staged file: $relative"
        }
    }
    foreach ($relative in $expectedPaths.Keys) {
        if (-not $actualPaths.ContainsKey($relative)) {
            throw "Manifest path is absent from staged file set: $relative"
        }
    }

    return [pscustomobject]@{
        Manifest = $manifest
        CopyPaths = [string[]]$copyPaths
    }
}

function Test-ManifestIntegrity {
    param(
        [string]$StagingRoot,
        [string]$ExpectedVersion,
        [string]$ExpectedExecutable
    )
    try {
        $null = Get-VerifiedManifest `
            -StagingRoot $StagingRoot `
            -ExpectedVersion $ExpectedVersion `
            -ExpectedExecutable $ExpectedExecutable
        Write-Log 'MANIFEST.json verification passed (exact file set and executable hash).'
        return $true
    } catch {
        Write-Log "MANIFEST verification failed: $_"
        Write-Host "MANIFEST verification failed: $_"
        Write-Host 'Aborting before any install mutation.'
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
        exit 4
    }
}

# Recover a previous power-loss/process-kill transaction only after the app
# process gate is clear, and before trusting the installed MANIFEST version.
$pendingTransaction = Join-Path $InstallRoot '.sky-update-transaction'
if ($DryRun -and (Test-Path -LiteralPath $pendingTransaction)) {
    Write-Host "DryRun cannot validate an install with an unresolved transaction: $pendingTransaction"
    Write-Host 'Run updater.bat normally once to perform recovery, then retry DryRun.'
    exit 5
}
if (-not $DryRun -and -not (Recover-InterruptedUpdate -DestRoot $InstallRoot)) {
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

# --- GitHub Releases ---
$owner = 'pumni'
$repo  = 'Sky-Auto-Player'
$headers = @{ 'User-Agent' = 'sky-auto-player-updater'; 'Accept' = 'application/vnd.github.v3+json' }

try {
    if ($ch -eq 'beta') {
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
    
    $zipUrl = [string]$selected.ZipAsset.browser_download_url
    $shaUrl = [string]$selected.ShaAsset.browser_download_url
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
    Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing -TimeoutSec 60
    Invoke-WebRequest -Uri $shaUrl -OutFile $shaPath -UseBasicParsing -TimeoutSec 30
} catch {
    Write-Log "download failed: $_"
    Write-Host "Download failed: $_"
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 2
}

try {
    $expected = Read-Sha256Sidecar -SidecarPath $shaPath -ExpectedFileName $zipName
} catch {
    Write-Log "sidecar validation failed: $_"
    Write-Host "SHA256 sidecar is invalid: $_"
    Write-Host 'Aborting before any file mutation.'
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


# --- Validate every zip entry before extraction (Zip-Slip/symlink defense) ---
try {
    Assert-ZipArchiveSafe -ZipPath $zipPath
} catch {
    Write-Log "unsafe zip layout: $_"
    Write-Host "Update zip contains an unsafe path or link: $_"
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 5
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

# --- Verify exact staged file set and executable integrity ---
$expectedExecutable = if ($zipName -like 'Sky-Player-*') {
    'Sky-Player.exe'
} else {
    'Sky-Auto-Player.exe'
}
try {
    $verifiedManifest = Get-VerifiedManifest `
        -StagingRoot $StagingRoot `
        -ExpectedVersion $latestVersion `
        -ExpectedExecutable $expectedExecutable
    Write-Log 'MANIFEST.json verification passed (exact file set and executable hash).'
} catch {
    Write-Log "MANIFEST verification failed: $_"
    Write-Host "MANIFEST verification failed: $_"
    Write-Host 'Aborting before any install mutation.'
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 5
}

if ($DryRun) {
    Write-Host "DryRun passed: download, process, extraction, and manifest checks succeeded for $runningVersion -> $latestVersion"
    Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    exit 0
}

# --- Copy with transactional fallback (I16, I21, I22) ---
try {
    Copy-UpdateTree `
        -StagingRoot $StagingRoot `
        -DestRoot $InstallRoot `
        -RelativePaths $verifiedManifest.CopyPaths
} catch {
    Write-Log "copy failed: $_"
    Write-Host "Copy into install dir failed: $_. config.json and songs were not replaced."
    Write-Host 'If rollback was incomplete, the durable backup path was printed above; re-run after resolving the lock or permission issue.'
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
