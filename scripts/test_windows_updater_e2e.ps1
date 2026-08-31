[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$FromPackage,
    [Parameter(Mandatory = $true)]
    [string]$ToPackage,
    [string]$GitHubTargetVersion = "3.4.6",
    [string]$ExpectedFromVersion = "3.4.5",
    [string]$ExpectedToVersion = "3.4.5",
    [string]$SyntheticTargetVersion = "3.4.6",
    [switch]$RunGitHubSmoke,
    [switch]$KeepEvidence,
    [switch]$SelfTestResultPolling,
    [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = "Stop"
$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$script:Timestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssZ")
$script:EvidenceRoot = Join-Path $script:RepoRoot "artifacts\updater-e2e\$script:Timestamp"
$script:SandboxRoot = Join-Path ([IO.Path]::GetTempPath()) "sky-updater-e2e-$script:Timestamp-$PID"
$script:Results = [ordered]@{}
$script:AllPassed = $false
$script:PreviousLocalAppData = $env:LOCALAPPDATA
$script:DefenderBefore = $null
$script:DefenderAfter = $null
$script:DefenderExclusionsUnchanged = $false
$script:DefenderThreatCount = $null
$script:DefenderRunStart = $null
$script:HarnessElevated = $false
$script:DefenderSnapshotElevated = $false

if (-not ('SkyUpdaterE2E.NativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace SkyUpdaterE2E {
    public static class NativeMethods {
        [DllImport("user32.dll", EntryPoint = "PostMessageW", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    }
}
'@
}

function Write-EvidenceJson {
    param([string]$Name, [object]$Value)
    $path = Join-Path $script:EvidenceRoot $Name
    $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $path -Encoding utf8
}

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Invoke-ElevatedDefenderSnapshot {
    param(
        [string]$Phase,
        [DateTime]$SinceUtc
    )
    $helper = Join-Path $script:RepoRoot "scripts\capture_defender_evidence.ps1"
    $output = Join-Path $script:EvidenceRoot "defender-$Phase.json"
    if (-not (Test-Path -LiteralPath $helper -PathType Leaf)) {
        throw "Defender evidence helper is missing: $helper"
    }
    $arguments = @(
        "-NoProfile"
        "-File"
        $helper
        "-OutputPath"
        $output
        "-SinceUtc"
        $SinceUtc.ToUniversalTime().ToString("o")
    )
    $hosts = @(
        (Join-Path $PSHOME "pwsh.exe")
        (Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe")
    ) | Select-Object -Unique
    $lastFailure = $null
    foreach ($hostPath in $hosts) {
        try {
            $process = Start-Process -FilePath $hostPath -Verb RunAs -Wait -PassThru `
                -ArgumentList $arguments -WindowStyle Hidden -ErrorAction Stop
            if ($process.ExitCode -eq 0 -and (Test-Path -LiteralPath $output -PathType Leaf)) {
                $script:DefenderSnapshotElevated = $true
                return Get-Content -LiteralPath $output -Raw | ConvertFrom-Json
            }
            $lastFailure = "exit code $($process.ExitCode) from $hostPath"
        }
        catch {
            $lastFailure = $_.Exception.Message
        }
    }
    throw "Defender evidence helper failed during ${Phase}: $lastFailure"
}

function Assert-DefenderEnabled {
    param([object]$Evidence, [string]$Phase)
    if (-not $Evidence.antivirus_enabled) {
        throw "Defender antivirus is not enabled $Phase"
    }
    if (-not $Evidence.realtime_protection_enabled) {
        throw "Defender real-time protection is not enabled $Phase"
    }
}

function Test-DefenderExclusionsUnchanged {
    param([object]$Before, [object]$After)
    $beforePaths = @($Before.exclusions | ForEach-Object { [string]$_ }) | Sort-Object
    $afterPaths = @($After.exclusions | ForEach-Object { [string]$_ }) | Sort-Object
    return @(Compare-Object -ReferenceObject $beforePaths -DifferenceObject $afterPaths).Count -eq 0
}

function Get-RelativeFilePath {
    param([string]$Root, [string]$Path)
    [IO.Path]::GetRelativePath($Root, $Path).Replace("\", "/")
}

function Get-FileHashes {
    param([string]$Root)
    $hashes = [ordered]@{}
    foreach ($file in Get-ChildItem -LiteralPath $Root -Recurse -File) {
        $relative = Get-RelativeFilePath $Root $file.FullName
        $hashes[$relative] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return ,$hashes
}

function Get-ManifestObject {
    param([string]$Root)
    Get-Content -LiteralPath (Join-Path $Root "MANIFEST.json") -Raw | ConvertFrom-Json
}

function Save-ManifestWithCurrentHashes {
    param([string]$Root)
    $manifest = Get-ManifestObject $Root
    $entries = @(
        foreach ($file in Get-ChildItem -LiteralPath $Root -Recurse -File) {
            $relative = Get-RelativeFilePath $Root $file.FullName
            if ($relative -eq "MANIFEST.json") { continue }
            [ordered]@{
                path = $relative
                size = $file.Length
                sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        }
    )
    $manifest.files = $entries
    $manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $Root "MANIFEST.json") -Encoding utf8
}

function Test-PreservedPath {
    param([string]$Relative)
    return $Relative -eq "config.json" -or $Relative -eq ".env" `
        -or $Relative.StartsWith("songs/", [StringComparison]::OrdinalIgnoreCase) `
        -or $Relative.StartsWith("logs/", [StringComparison]::OrdinalIgnoreCase)
}

function Assert-ManagedManifestFiles {
    param([string]$Root, [object]$Manifest)
    $hashes = Get-FileHashes $Root
    foreach ($entry in @($Manifest.files)) {
        if (Test-PreservedPath $entry.path) { continue }
        if (-not $hashes.Contains($entry.path)) {
            throw "manifest file is missing: $($entry.path)"
        }
        $path = Join-Path $Root ($entry.path.Replace("/", "\"))
        if ((Get-Item -LiteralPath $path).Length -ne [int64]$entry.size) {
            throw "manifest size mismatch: $($entry.path)"
        }
        if ($hashes[$entry.path] -ne $entry.sha256.ToLowerInvariant()) {
            throw "manifest hash mismatch: $($entry.path)"
        }
    }
    return ,$hashes
}

function Get-PackageRoot {
    param([string]$Archive, [string]$Destination)
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    Expand-Archive -LiteralPath $Archive -DestinationPath $Destination -Force
    $children = @(Get-ChildItem -LiteralPath $Destination -Force)
    if ($children.Count -eq 1 -and $children[0].PSIsContainer) {
        return $children[0].FullName
    }
    return $Destination
}

function Copy-TreeContents {
    param([string]$Source, [string]$Destination)
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    Copy-Item -Path (Join-Path $Source "*") -Destination $Destination -Recurse -Force
}

function Add-PreservedFixture {
    param([string]$Root)
    # Keep the fixture schema-valid and disable the app's automatic update
    # worker. The restart acceptance must test updater preservation, not let a
    # freshly restarted app rewrite its own update-notification state.
    $config = @'
{
    "e2e": "preserve",
    "schema_version": 3,
    "default_hold_frames": 1.0,
    "theme": "aurora",
    "ui_background_mode": "transparent",
    "default_tempo_scale": 1.0,
    "game_fps": 60,
    "telemetry_enabled_by_default": false,
    "verbose_hud": false,
    "hotkeys": {
        "pause": "f8",
        "skip": "f9",
        "quit": "f10",
        "refocus": "f6",
        "panic": "ctrl+alt+backspace"
    },
    "safety": {
        "prompt_on_medium_risk": true,
        "prompt_on_high_risk": true
    },
    "songs_dir": "songs",
    "sky_process_names": [
        "Sky.exe",
        "Sky Children of the Light.exe"
    ],
    "allow_title_fallback": false,
    "update": {
        "auto_check": false,
        "channel": "stable",
        "skip_version": "",
        "check_interval_s": 86400,
        "last_check_ts": 0,
        "last_error_ts": 0,
        "last_notified_version": "",
        "legacy_old_dir_sweep_pending": false
    }
}
'@
    $configText = [regex]::Replace($config.Trim(), '\r?\n', [Environment]::NewLine)
    [IO.File]::WriteAllText((Join-Path $Root "config.json"), $configText)
    [IO.File]::WriteAllText((Join-Path $Root ".env"), "E2E_PRESERVED=1`r`n")
    New-Item -ItemType Directory -Path (Join-Path $Root "songs") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $Root "logs") -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $Root "songs\user.skysheet"), "user song`r`n")
    [IO.File]::WriteAllText((Join-Path $Root "logs\user.log"), "user log`r`n")
}

function New-Scenario {
    param([string]$Name, [string]$FromRoot)
    $root = Join-Path $script:SandboxRoot $Name
    $local = Join-Path $root "localappdata"
    $install = Join-Path $root "install"
    Copy-TreeContents $FromRoot $install
    Add-PreservedFixture $install
    New-Item -ItemType Directory -Path $local -Force | Out-Null
    $script:ScenarioLocalAppData = $local
    $env:LOCALAPPDATA = $local
    $beforeHashes = Get-FileHashes $install
    $beforeManifest = Get-ManifestObject $install
    Write-EvidenceJson "$Name-before-manifest.json" $beforeManifest
    Write-EvidenceJson "$Name-before-hashes.json" $beforeHashes
    [pscustomobject]@{
        Name = $Name
        Root = $root
        Local = $local
        Install = $install
        BeforeHashes = $beforeHashes
        BeforeManifest = $beforeManifest
    }
}

function New-RunDirectory {
    param([string]$Candidate)
    $runs = Join-Path $env:LOCALAPPDATA "Sky-Auto-Player\update-runs"
    $run = Join-Path $runs ("run-" + ([guid]::NewGuid().ToString("N")))
    New-Item -ItemType Directory -Path $run -Force | Out-Null
    Copy-Item -LiteralPath $Candidate -Destination (Join-Path $run "Sky-Auto-Player-Updater.exe") -Force
    return $run
}

function Quote-ProcessArgument {
    param([string]$Value)
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Start-ParentFixture {
    param([string]$Install)
    $primary = Join-Path $Install "Sky-Auto-Player.exe"
    try {
        $process = Start-Process -FilePath $primary -WorkingDirectory $Install -PassThru
        Start-Sleep -Milliseconds 750
        return $process
    }
    catch {
        return $null
    }
}

function Stop-ProcessIfRunning {
    param([object]$Process)
    if ($null -ne $Process) {
        try {
            if (-not $Process.HasExited) { Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue }
        } catch { }
    }
}

function Start-UpdaterProcess {
    param(
        [string]$Install,
        [string]$Candidate,
        [string]$CurrentVersion,
        [string]$TargetVersion,
        [string]$ReleaseDir,
        [uint32]$ParentPid = 1,
        [string]$FailAt,
        [string]$PauseAt,
        [string]$ResumeFile,
        [switch]$Restart,
        [switch]$CleanupOnly,
        [switch]$KeepPaused,
        [switch]$RequireProgressWindow,
        [switch]$FailRestart
    )
    $run = New-RunDirectory $Candidate
    $updater = Join-Path $run "Sky-Auto-Player-Updater.exe"
    $stdout = Join-Path $run "stdout.txt"
    $stderr = Join-Path $run "stderr.txt"
    $arguments = @(
        "--install-root", $Install,
        "--parent-pid", [string]$ParentPid,
        "--current-version", $CurrentVersion,
        "--target-version", $TargetVersion,
        "--channel", "stable"
    )
    if ($Restart) { $arguments += "--restart" }
    if ($FailRestart) { $arguments += "--fail-restart" }
    if ($ReleaseDir) { $arguments += @("--release-dir", $ReleaseDir) }
    if ($FailAt) { $arguments += @("--fail-at", $FailAt) }
    if ($PauseAt) { $arguments += @("--pause-at", $PauseAt) }
    if ($ResumeFile) { $arguments += @("--resume-file", $ResumeFile) }
    if ($CleanupOnly) { $arguments += "--cleanup-only" }
    $argumentLine = (($arguments | ForEach-Object { Quote-ProcessArgument ([string]$_) }) -join " ")
    $process = Start-Process -FilePath $updater -ArgumentList $argumentLine -WorkingDirectory $Install `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    if (-not $KeepPaused) {
        Start-Sleep -Milliseconds 1000
    }
    if ($RequireProgressWindow -and -not (Wait-ForProgressWindow $process.Id)) {
        throw "native updater progress window was not visible"
    }
    [pscustomobject]@{
        Process = $process
        Run = $run
        Updater = $updater
        ParentPid = $ParentPid
        Stdout = $stdout
        Stderr = $stderr
    }
}

function Read-UpdaterResult {
    param([object]$Scenario)
    $resultPath = Join-Path $Scenario.Local "Sky-Auto-Player\update-state\last-result.json"
    if (-not (Test-Path -LiteralPath $resultPath -PathType Leaf)) { return $null }
    return [pscustomobject]@{
        ResultPath = $resultPath
        Result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    }
}

function Wait-ForUpdaterResult {
    param(
        [object]$Scenario,
        [object]$RunInfo,
        [string]$ExpectedStatus,
        [string]$ExpectedErrorCode,
        [int]$TimeoutSeconds = 45
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $observed = Read-UpdaterResult $Scenario
        if ($observed) {
            $RunInfo.Process.Refresh()
            $statusMatches = [string]::IsNullOrEmpty($ExpectedStatus) -or
                [string]$observed.Result.status -eq $ExpectedStatus
            $errorMatches = [string]::IsNullOrEmpty($ExpectedErrorCode) -or
                [string]$observed.Result.error_code -eq $ExpectedErrorCode
            if ($statusMatches -and $errorMatches) {
                return [pscustomobject]@{
                    Result = $observed.Result
                    ResultPath = $observed.ResultPath
                    ProcessAliveAfterResult = -not $RunInfo.Process.HasExited
                }
            }
            if ($RunInfo.Process.HasExited) {
                throw "updater exited with an unexpected result: status=$($observed.Result.status), code=$($observed.Result.error_code)"
            }
        }
        if ($RunInfo.Process.HasExited) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "updater did not write a result while running: $($Scenario.Name)"
}

function Close-TerminalUpdaterWindow {
    param([object]$Process)
    $Process.Refresh()
    $hwnd = $Process.MainWindowHandle
    if ($hwnd -eq [IntPtr]::Zero) {
        throw "terminal updater window has no main HWND"
    }
    $wmCommand = [uint32]0x0111
    $idClose = [IntPtr]1004
    if (-not [SkyUpdaterE2E.NativeMethods]::PostMessage($hwnd, $wmCommand, $idClose, [IntPtr]::Zero)) {
        throw "could not send terminal updater Close command"
    }
}

function Invoke-UpdaterExpectTerminal {
    param(
        [object]$Scenario,
        [string]$Candidate,
        [string]$CurrentVersion,
        [string]$TargetVersion,
        [string]$ReleaseDir,
        [string]$FailAt,
        [string]$ExpectedStatus,
        [string]$ExpectedErrorCode,
        [switch]$Restart,
        [switch]$FailRestart
    )
    $parent = Start-ParentFixture $Scenario.Install
    $parentPid = if ($parent) { [uint32]$parent.Id } else { [uint32]1 }
    $runInfo = Start-UpdaterProcess -Install $Scenario.Install -Candidate $Candidate `
        -ParentPid $parentPid -CurrentVersion $CurrentVersion -TargetVersion $TargetVersion `
        -ReleaseDir $ReleaseDir -FailAt $FailAt -Restart:$Restart `
        -RequireProgressWindow -FailRestart:$FailRestart
    Stop-ProcessIfRunning $parent
    $observed = Wait-ForUpdaterResult $Scenario $runInfo $ExpectedStatus $ExpectedErrorCode
    if (-not $observed.ProcessAliveAfterResult) {
        throw "terminal updater exited before its result window could be inspected"
    }
    if (-not (Wait-ForProgressWindow $runInfo.Process.Id)) {
        throw "terminal updater window was not visible after result write"
    }
    Start-Sleep -Milliseconds 500
    $runInfo.Process.Refresh()
    if ($runInfo.Process.HasExited) {
        throw "terminal updater auto-closed before the approved Close command"
    }
    Close-TerminalUpdaterWindow $runInfo.Process
    if (-not $runInfo.Process.WaitForExit(15000)) {
        Stop-ProcessIfRunning $runInfo.Process
        throw "terminal updater did not exit after the approved Close command"
    }
    [pscustomobject]@{
        Scenario = $Scenario
        Run = $runInfo.Run
        ExitCode = $runInfo.Process.ExitCode
        Result = $observed.Result
        ResultPath = $observed.ResultPath
        TerminalWindowHeld = $true
        Stdout = if (Test-Path $runInfo.Stdout) { Get-Content $runInfo.Stdout -Raw } else { "" }
        Stderr = if (Test-Path $runInfo.Stderr) { Get-Content $runInfo.Stderr -Raw } else { "" }
    }
}

function Resume-UpdaterAndReadResult {
    param(
        [object]$Scenario,
        [object]$RunInfo,
        [string]$ResumeFile,
        [int]$WaitSeconds = 180
    )
    [IO.File]::WriteAllText($ResumeFile, "resume`r`n")
    if (-not $RunInfo.Process.WaitForExit($WaitSeconds * 1000)) {
        Stop-ProcessIfRunning $RunInfo.Process
        throw "resumable updater timed out: $($Scenario.Name)"
    }
    $observed = Read-UpdaterResult $Scenario
    if (-not $observed) { throw "resumable updater did not write a result: $($Scenario.Name)" }
    [pscustomobject]@{
        Scenario = $Scenario
        Run = $RunInfo.Run
        ParentPid = $RunInfo.ParentPid
        ExitCode = $RunInfo.Process.ExitCode
        Result = $observed.Result
        ResultPath = $observed.ResultPath
        Stdout = if (Test-Path $RunInfo.Stdout) { Get-Content $RunInfo.Stdout -Raw } else { "" }
        Stderr = if (Test-Path $RunInfo.Stderr) { Get-Content $RunInfo.Stderr -Raw } else { "" }
    }
}

function Invoke-Updater {
    param(
        [object]$Scenario,
        [string]$Candidate,
        [string]$CurrentVersion,
        [string]$TargetVersion,
        [string]$ReleaseDir,
        [string]$FailAt,
        [string]$PauseAt,
        [switch]$Restart,
        [switch]$RequireProgressWindow,
        [switch]$FailRestart
    )
    $parent = Start-ParentFixture $Scenario.Install
    $parentPid = if ($parent) { [uint32]$parent.Id } else { [uint32]1 }
    $runInfo = Start-UpdaterProcess -Install $Scenario.Install -Candidate $Candidate `
        -ParentPid $parentPid -CurrentVersion $CurrentVersion -TargetVersion $TargetVersion -ReleaseDir $ReleaseDir `
        -FailAt $FailAt -PauseAt $PauseAt -Restart:$Restart `
        -RequireProgressWindow:$RequireProgressWindow -FailRestart:$FailRestart
    Start-Sleep -Milliseconds 500
    Stop-ProcessIfRunning $parent
    if (-not $runInfo.Process.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-ProcessIfRunning $runInfo.Process
        throw "updater timed out: $($Scenario.Name)"
    }
    $resultPath = Join-Path $Scenario.Local "Sky-Auto-Player\update-state\last-result.json"
    $result = $null
    if (Test-Path -LiteralPath $resultPath) {
        $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    }
    [pscustomobject]@{
        Scenario = $Scenario
        Run = $runInfo.Run
        ParentPid = $parentPid
        ExitCode = $runInfo.Process.ExitCode
        Result = $result
        ResultPath = $resultPath
        Stdout = if (Test-Path $runInfo.Stdout) { Get-Content $runInfo.Stdout -Raw } else { "" }
        Stderr = if (Test-Path $runInfo.Stderr) { Get-Content $runInfo.Stderr -Raw } else { "" }
    }
}

function Wait-ForProgressWindow {
    param(
        [int]$ProcessId,
        [int]$TimeoutSeconds = 15
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        try {
            $process = Get-Process -Id $ProcessId -ErrorAction Stop
            if ($process.MainWindowTitle -eq "Sky Auto Player Updater") {
                return $true
            }
        } catch { return $false }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Wait-ForEmergencyBackup {
    param(
        [string]$Install,
        [int]$TimeoutSeconds = 45
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $backup = Get-ChildItem -LiteralPath $Install -Filter ".sky-update-*.bak" -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -notlike "*.reconcile.bak" } |
            Select-Object -First 1
        if ($backup) { return $backup.FullName }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    return $null
}

function Get-CanonicalAppProcesses {
    param([string]$Install)
    $primary = [IO.Path]::GetFullPath((Join-Path $Install "Sky-Auto-Player.exe"))
    $matches = @(
        foreach ($process in Get-Process) {
            try {
                if ($process.Path -and [IO.Path]::GetFullPath($process.Path) -ieq $primary) {
                    $process
                }
            } catch { }
        }
    )
    return ,$matches
}

function Wait-ForPrimaryEmergencyBackup {
    param(
        [object]$Scenario,
        [string]$ExpectedOriginalFilename,
        [string]$ExpectedFileVersion,
        [string]$ExpectedProductVersion,
        [string]$ExpectedSha256,
        [int]$TimeoutSeconds = 45
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $candidates = @(Get-ChildItem -LiteralPath $Scenario.Install -Filter ".sky-update-*.bak" -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -notlike "*.reconcile.bak" })
        foreach ($candidate in $candidates) {
            $info = $candidate.VersionInfo
            $hash = (Get-FileHash -LiteralPath $candidate.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($info.OriginalFilename -eq $ExpectedOriginalFilename -and
                $info.FileVersion -eq $ExpectedFileVersion -and
                $info.ProductVersion -eq $ExpectedProductVersion -and
                $hash -eq $ExpectedSha256) {
                return [pscustomobject]@{
                    Path = $candidate.FullName
                    OriginalFilename = [string]$info.OriginalFilename
                    FileVersion = [string]$info.FileVersion
                    ProductVersion = [string]$info.ProductVersion
                    Sha256 = $hash
                    SourceSha256 = $ExpectedSha256
                }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    return $null
}

function Read-RunHandoff {
    param([string]$Run)
    $path = Join-Path $Run "handoff.json"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }
    return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
}

function Wait-ForReadyHandoff {
    param(
        [object]$RunInfo,
        [string]$TargetVersion,
        [int]$TimeoutSeconds = 15
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $RunInfo.Process.Refresh()
        if ($RunInfo.Process.HasExited) {
            throw "updater exited before READY handoff was published"
        }
        $handoff = $null
        try {
            $handoff = Read-RunHandoff $RunInfo.Run
        } catch {
            # Atomic handoff publication may race this observation. Retry until
            # the bounded READY deadline instead of treating an early read as a
            # lifecycle failure.
        }
        if ($handoff -and $handoff.state -eq "ready" -and
            $handoff.updater_pid -eq $RunInfo.Process.Id -and
            $handoff.target_version -eq $TargetVersion) {
            return $handoff
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "updater did not publish matching READY handoff within $TimeoutSeconds seconds"
}

function Wait-ForActiveUpdateState {
    param(
        [object]$RunInfo,
        [string]$Path,
        [int]$TimeoutSeconds = 15
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $RunInfo.Process.Refresh()
        if ($RunInfo.Process.HasExited) {
            throw "updater exited before active-update state was published"
        }
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            try {
                return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
            } catch {
                # Retry if the observation overlaps an atomic state update.
            }
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "updater did not publish active-update state within $TimeoutSeconds seconds"
}

function Assert-RestartObserved {
    param([string]$Install)
    $primary = [IO.Path]::GetFullPath((Join-Path $Install "Sky-Auto-Player.exe"))
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $found = $false
        foreach ($process in Get-Process) {
            try {
                if ($process.Path -and [IO.Path]::GetFullPath($process.Path) -ieq $primary) {
                    $found = $true
                    break
                }
            } catch { }
        }
        if ($found) { return $true }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Wait-ForExactlyOneRestartedApp {
    param(
        [string]$Install,
        [uint32]$OriginalParentPid = 0,
        [int]$TimeoutSeconds = 20
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $matches = @(Get-CanonicalAppProcesses $Install)
        $new = @($matches | Where-Object { $_.Id -ne $OriginalParentPid })
        if ($new.Count -eq 1) {
            return $new[0]
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return $null
}

function Stop-RestartedApp {
    param([string]$Install)
    $primary = [IO.Path]::GetFullPath((Join-Path $Install "Sky-Auto-Player.exe"))
    foreach ($process in Get-Process) {
        try {
            if ($process.Path -and [IO.Path]::GetFullPath($process.Path) -ieq $primary) {
                Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            }
        } catch { }
    }
}

function Wait-ForPreparedJournal {
    param(
        [string]$Install,
        [int]$TimeoutSeconds = 30
    )
    $journal = Join-Path $Install ".sky-update-transaction\journal.json"
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path -LiteralPath $journal -PathType Leaf) {
            return $true
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Assert-PreservedState {
    param([object]$Scenario)
    $paths = @("config.json", ".env", "songs/user.skysheet", "logs/user.log")
    foreach ($relative in $paths) {
        $before = $Scenario.BeforeHashes[$relative]
        $after = (Get-FileHash -LiteralPath (Join-Path $Scenario.Install ($relative.Replace("/", "\"))) -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($before -ne $after) { throw "preserved path changed: $relative" }
    }
}

function Assert-NoInstallMutation {
    param([object]$Scenario)
    $after = Get-FileHashes $Scenario.Install
    if ($after.Count -ne $Scenario.BeforeHashes.Count) {
        throw "installation file set changed unexpectedly: $($Scenario.Name)"
    }
    foreach ($relative in $Scenario.BeforeHashes.Keys) {
        if (-not $after.Contains($relative) -or $after[$relative] -ne $Scenario.BeforeHashes[$relative]) {
            throw "installation file changed unexpectedly: $relative"
        }
    }
}

function Save-ScenarioEvidence {
    param([string]$Name, [object]$Scenario, [object]$Run, [object]$Manifest)
    $afterManifest = Get-ManifestObject $Scenario.Install
    $afterHashes = Get-FileHashes $Scenario.Install
    Write-EvidenceJson "$Name-after-manifest.json" $afterManifest
    Write-EvidenceJson "$Name-after-hashes.json" $afterHashes
    if ($Name -eq "canonical-v345-to-v346") {
        Copy-Item -LiteralPath (Join-Path $script:EvidenceRoot "$Name-before-manifest.json") `
            -Destination (Join-Path $script:EvidenceRoot "before-manifest.json") -Force
        Copy-Item -LiteralPath (Join-Path $script:EvidenceRoot "$Name-before-hashes.json") `
            -Destination (Join-Path $script:EvidenceRoot "before-hashes.json") -Force
        Copy-Item -LiteralPath (Join-Path $script:EvidenceRoot "$Name-after-manifest.json") `
            -Destination (Join-Path $script:EvidenceRoot "after-manifest.json") -Force
        Copy-Item -LiteralPath (Join-Path $script:EvidenceRoot "$Name-after-hashes.json") `
            -Destination (Join-Path $script:EvidenceRoot "after-hashes.json") -Force
    }
    Write-EvidenceJson "$Name-result.json" $Run.Result
    if ($Run.ResultPath -and (Test-Path $Run.ResultPath)) {
        Copy-Item -LiteralPath $Run.ResultPath -Destination (Join-Path $script:EvidenceRoot "$Name-result.json") -Force
    }
    Assert-ManagedManifestFiles $Scenario.Install $Manifest | Out-Null
    Assert-PreservedState $Scenario
    if (Test-Path -LiteralPath (Join-Path $Scenario.Install ".sky-update-transaction")) {
        throw "transaction directory remains after successful scenario: $Name"
    }
    return [pscustomobject]@{
        status = $Run.Result.status
        exit_code = $Run.ExitCode
        restart_verified = Assert-RestartObserved $Scenario.Install
        transaction_removed = $true
    }
}

function Assert-CanonicalSuccess {
    param([object]$Scenario, [object]$Run)
    $activePath = Join-Path $Scenario.Local "Sky-Auto-Player\update-state\active-update.json"
    if (Test-Path -LiteralPath $activePath) {
        throw "canonical success left active-update.json behind"
    }
    $reserved = @(
        Get-ChildItem -LiteralPath $Scenario.Install -Recurse -Force -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like ".sky-update-*" }
    )
    if ($reserved.Count -ne 0) {
        throw "canonical success left updater-owned artifacts: $($reserved.FullName -join ', ')"
    }
    $restarted = Wait-ForExactlyOneRestartedApp $Scenario.Install $Run.ParentPid
    if ($null -eq $restarted) {
        throw "canonical success did not produce exactly one live restarted primary process"
    }
    $processes = @(Get-CanonicalAppProcesses $Scenario.Install)
    if ($processes.Count -ne 1 -or $processes[0].Id -ne $restarted.Id) {
        throw "canonical success observed more than one live primary process"
    }
    return $restarted
}

function Save-UpdaterLog {
    param([string]$Name, [object]$Scenario)
    $source = Join-Path $Scenario.Local "Sky-Auto-Player\update-state\updater.log"
    $destination = Join-Path $script:EvidenceRoot "$Name-updater.log"
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination $destination -Force
        Add-Content -LiteralPath (Join-Path $script:EvidenceRoot "updater.log") `
            -Value ("[$Name]`r`n" + (Get-Content -LiteralPath $source -Raw))
    }
}

function Build-SyntheticLocalRelease {
    param([string]$ToRoot, [string]$Version)
    $release = Join-Path $script:SandboxRoot "synthetic-release"
    Copy-TreeContents $ToRoot $release
    # Keep the updater bytes from the exact package template. The E2E
    # executor is only the local-source/fault-injection process; it must never
    # replace the packaged production updater in the payload under test.
    [IO.File]::WriteAllText(
        (Join-Path $release "e2e-v$Version.marker"),
        "synthetic target $Version`r`n"
    )
    $manifest = Get-ManifestObject $release
    $manifest.version = $Version
    $manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $release "MANIFEST.json") -Encoding utf8
    Save-ManifestWithCurrentHashes $release
    $zipName = "Sky-Auto-Player-v$Version.zip"
    $zip = Join-Path $release $zipName
    $bundle = Join-Path $script:SandboxRoot "synthetic-bundle"
    New-Item -ItemType Directory -Path $bundle -Force | Out-Null
    Compress-Archive -Path (Join-Path $release "*") -DestinationPath $zip -Force
    $zipHash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $release "$zipName.sha256"), "$zipHash  $zipName`r`n", [Text.Encoding]::ASCII)
    # LocalReleaseSource reads the bundle directory, while the ZIP itself is
    # the exact archive that is hash-verified and extracted by the updater.
    return $release
}

function Build-CorruptSidecarRelease {
    param([string]$ValidRelease)
    $release = Join-Path $script:SandboxRoot "corrupt-sidecar-release"
    Copy-TreeContents $ValidRelease $release
    $sidecar = Get-ChildItem -LiteralPath $release -Filter "*.zip.sha256" -File | Select-Object -First 1
    if (-not $sidecar) { throw "synthetic release sidecar is missing" }
    $zipName = $sidecar.Name.Substring(0, $sidecar.Name.Length - ".sha256".Length)
    [IO.File]::WriteAllText($sidecar.FullName, ("0" * 64) + "  " + $zipName + "`r`n", [Text.Encoding]::ASCII)
    return $release
}

function Record-Failure {
    param([string]$Name, [object]$ErrorRecord)
    $script:Results[$Name] = [ordered]@{ status = "FAIL"; error = $ErrorRecord.ToString() }
}

function Invoke-ResultPollingSelfTest {
    $root = Join-Path ([IO.Path]::GetTempPath()) ("sky-updater-result-polling-" + [guid]::NewGuid().ToString("N"))
    $local = Join-Path $root "localappdata"
    $state = Join-Path $local "Sky-Auto-Player\update-state"
    $resultPath = Join-Path $state "last-result.json"
    $finalPath = Join-Path $root "final-result.json"
    $writerScript = Join-Path $root "write-final-result.ps1"
    $holderScript = Join-Path $root "hold-process.ps1"
    $writer = $null
    $holder = $null
    try {
        New-Item -ItemType Directory -Path $state -Force | Out-Null
        [IO.File]::WriteAllText($resultPath, '{"status":"success","error_code":null}')
        [IO.File]::WriteAllText($finalPath, '{"status":"failure","error_code":"RESTART_FAILED"}')
        [IO.File]::WriteAllText($writerScript, @'
param([string]$ResultPath, [string]$FinalPath)
Start-Sleep -Milliseconds 250
Copy-Item -LiteralPath $FinalPath -Destination $ResultPath -Force
'@)
        [IO.File]::WriteAllText($holderScript, @'
param([int]$Seconds)
Start-Sleep -Seconds $Seconds
'@)
        $hostPath = [Environment]::ProcessPath
        if ([string]::IsNullOrWhiteSpace($hostPath)) {
            throw "could not determine the current PowerShell host path"
        }
        # Keep child PowerShell processes from inheriting the harness capture
        # pipes.  Python's communicate() waits for those pipes to close, and a
        # hosted Windows runner can otherwise retain them after this script's
        # five-second polling contract has completed.
        $writerStdout = Join-Path $root "writer-stdout.txt"
        $writerStderr = Join-Path $root "writer-stderr.txt"
        $holderStdout = Join-Path $root "holder-stdout.txt"
        $holderStderr = Join-Path $root "holder-stderr.txt"
        $writerArguments = @(
            "-NoProfile", "-File", $writerScript, "-ResultPath", $resultPath, "-FinalPath", $finalPath
        )
        $writerLine = ($writerArguments | ForEach-Object { Quote-ProcessArgument ([string]$_) }) -join " "
        $writer = Start-Process -FilePath $hostPath -ArgumentList $writerLine -WindowStyle Hidden `
            -RedirectStandardOutput $writerStdout -RedirectStandardError $writerStderr -PassThru
        $holderArguments = @("-NoProfile", "-File", $holderScript, "-Seconds", "3")
        $holderLine = ($holderArguments | ForEach-Object { Quote-ProcessArgument ([string]$_) }) -join " "
        $holder = Start-Process -FilePath $hostPath -ArgumentList $holderLine -WindowStyle Hidden `
            -RedirectStandardOutput $holderStdout -RedirectStandardError $holderStderr -PassThru
        $scenario = [pscustomobject]@{ Name = "result-polling-self-test"; Local = $local }
        $runInfo = [pscustomobject]@{ Process = $holder }
        $observed = Wait-ForUpdaterResult $scenario $runInfo "failure" "RESTART_FAILED" 5
        if ($observed.Result.status -ne "failure" -or
            $observed.Result.error_code -ne "RESTART_FAILED" -or
            -not $observed.ProcessAliveAfterResult) {
            throw "result polling self-test accepted the wrong result"
        }
    }
    finally {
        Stop-ProcessIfRunning $writer
        Stop-ProcessIfRunning $holder
        if (Test-Path -LiteralPath $root) {
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

if ($SelfTestResultPolling) {
    try {
        Invoke-ResultPollingSelfTest
        exit 0
    }
    catch {
        Write-Error $_
        exit 1
    }
}

try {
    New-Item -ItemType Directory -Path $script:EvidenceRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $script:SandboxRoot -Force | Out-Null
    $script:HarnessElevated = Test-IsAdministrator
    if ($script:HarnessElevated) {
        throw "Updater acceptance must run from a non-elevated PowerShell session"
    }
    $script:DefenderRunStart = Get-Date
    $script:DefenderBefore = Invoke-ElevatedDefenderSnapshot "before" $script:DefenderRunStart
    Assert-DefenderEnabled $script:DefenderBefore "before acceptance"
    Write-EvidenceJson "defender-before.json" $script:DefenderBefore
    $fromRoot = Get-PackageRoot (Resolve-Path $FromPackage).Path (Join-Path $script:SandboxRoot "from-package")
    $toRoot = Get-PackageRoot (Resolve-Path $ToPackage).Path (Join-Path $script:SandboxRoot "to-package")
    $fromManifest = Get-ManifestObject $fromRoot
    $toManifest = Get-ManifestObject $toRoot
    if ($fromManifest.version -ne $ExpectedFromVersion) { throw "rollout acceptance requires a v$ExpectedFromVersion source package; got $($fromManifest.version)" }
    if ($toManifest.version -ne $ExpectedToVersion) { throw "rollout acceptance requires a v$ExpectedToVersion exact package template; got $($toManifest.version)" }
    if ($fromManifest.version -ne $toManifest.version) { throw "source and exact target template must have the same v3.4.5 package version" }
    if ($SyntheticTargetVersion -eq $fromManifest.version) { throw "synthetic target version must be newer than source" }

    $cargoManifest = Join-Path $script:RepoRoot "rust\Cargo.toml"
    & cargo build --manifest-path $cargoManifest -p sky_updater --bin sky_updater --profile dist
    if ($LASTEXITCODE -ne 0) { throw "production updater build failed" }
    & cargo build --manifest-path $cargoManifest -p sky_updater --bin sky_updater_e2e `
        --features e2e-local-source,e2e-fault-injection --profile dist
    if ($LASTEXITCODE -ne 0) { throw "E2E updater build failed" }
    $productionCandidate = Join-Path $script:RepoRoot "rust\target\dist\sky_updater.exe"
    $e2eCandidate = Join-Path $script:RepoRoot "rust\target\dist\sky_updater_e2e.exe"
    if (-not (Test-Path $productionCandidate) -or -not (Test-Path $e2eCandidate)) { throw "updater candidates missing" }
    $packagedUpdater = Join-Path $toRoot "Sky-Auto-Player-Updater.exe"
    $candidateHashes = [ordered]@{
        packaged_updater = (Get-FileHash $packagedUpdater -Algorithm SHA256).Hash.ToLowerInvariant()
        production_candidate = (Get-FileHash $productionCandidate -Algorithm SHA256).Hash.ToLowerInvariant()
        e2e_executor = (Get-FileHash $e2eCandidate -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    @(
        "packaged_updater $($candidateHashes.packaged_updater)"
        "production_candidate $($candidateHashes.production_candidate)"
        "e2e_executor $($candidateHashes.e2e_executor)"
    ) | Set-Content (Join-Path $script:EvidenceRoot "candidate-sha256.txt") -Encoding ascii

    # The mandatory rollout proof starts with the v3.4.5 package and installs
    # a test-only synthetic v3.4.6 payload. Its updater bytes remain the exact
    # packaged v3.4.5 updater; only the test marker/manifest version changes.
    $syntheticRelease = Build-SyntheticLocalRelease $toRoot $SyntheticTargetVersion
    $syntheticManifest = Get-ManifestObject $syntheticRelease
    $corruptSidecarRelease = Build-CorruptSidecarRelease $syntheticRelease

    $scenario = New-Scenario "canonical-v345-to-v346" $fromRoot
    $run = Invoke-Updater $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $syntheticRelease `
        -Restart -RequireProgressWindow
    if ($run.Result.status -ne "success" -or $run.ExitCode -ne 0) { throw "canonical v3.4.5 to synthetic v3.4.6 result was not success" }
    $h1 = Save-ScenarioEvidence "canonical-v345-to-v346" $scenario $run $syntheticManifest
    Save-UpdaterLog "canonical-v345-to-v346" $scenario
    $installedUpdaterHash = (Get-FileHash (Join-Path $scenario.Install "Sky-Auto-Player-Updater.exe") -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($installedUpdaterHash -ne $candidateHashes.packaged_updater) { throw "canonical install updater hash is not the exact packaged updater hash" }
    if (-not $h1.restart_verified) { throw "H1 restart was not observed" }
    $canonicalRestart = Assert-CanonicalSuccess $scenario $run
    Write-EvidenceJson "canonical-v345-to-v346-result.json" ([ordered]@{
        status = "PASS"
        source_version = $fromManifest.version
        target_version = $syntheticManifest.version
        packaged_updater_sha256 = $candidateHashes.packaged_updater
        installed_updater_sha256 = $installedUpdaterHash
        restarted_pid = $canonicalRestart.Id
        exactly_one_live_restart_process = $true
        active_state_removed = $true
        transaction_removed = $true
        reserved_artifacts_removed = $true
    })
    $script:Results.canonical_v345_to_v346 = [ordered]@{
        status = "PASS"
        installed_updater_sha256 = $installedUpdaterHash
        restarted_pid = $canonicalRestart.Id
        restart_verified = $true
        exactly_one_live_restart_process = $true
        active_state_removed = $true
        transaction_removed = $true
        reserved_artifacts_removed = $true
    }
    Stop-RestartedApp $scenario.Install

    if ($RunGitHubSmoke) {
        $scenario = New-Scenario "happy-github" $fromRoot
        $run = Invoke-Updater $scenario $productionCandidate $fromManifest.version $GitHubTargetVersion $null -Restart
        if ($run.Result.status -ne "success" -or $run.ExitCode -ne 0) { throw "H2 result was not success" }
        $h2Manifest = Get-ManifestObject $scenario.Install
        $h2 = Save-ScenarioEvidence "happy-github" $scenario $run $h2Manifest
        Save-UpdaterLog "happy-github" $scenario
        if (-not $h2.restart_verified) { throw "H2 restart was not observed" }
        $script:Results.happy_github = [ordered]@{ status = "PASS"; target_version = $h2Manifest.version; restart_verified = $true; transaction_removed = $true }
        Stop-RestartedApp $scenario.Install
    } else {
        # H2 requires a published, non-draft GitHub release. It is a
        # post-publish smoke test and must not gate pre-release local acceptance.
        $script:Results.happy_github = [ordered]@{
            status = "NOT_RUN"
            reason = "post-publish GitHub smoke is disabled for pre-release acceptance"
        }
        Write-EvidenceJson "happy-github-result.json" $script:Results.happy_github
    }

    $scenario = New-Scenario "one-click-launcher" $fromRoot
    Copy-Item -LiteralPath $e2eCandidate `
        -Destination (Join-Path $scenario.Install "Sky-Auto-Player-Updater.exe") -Force
    Save-ManifestWithCurrentHashes $scenario.Install
    $previousHandshakeMode = $env:SKY_AUTO_PLAYER_E2E_HANDSHAKE_ONLY
    $previousOneClickRoot = $env:SKY_ONE_CLICK_INSTALL_ROOT
    $previousOneClickCurrent = $env:SKY_ONE_CLICK_CURRENT_VERSION
    $previousOneClickTarget = $env:SKY_ONE_CLICK_TARGET_VERSION
    try {
        $env:SKY_AUTO_PLAYER_E2E_HANDSHAKE_ONLY = "1"
        $env:SKY_ONE_CLICK_INSTALL_ROOT = $scenario.Install
        $env:SKY_ONE_CLICK_CURRENT_VERSION = $fromManifest.version
        $env:SKY_ONE_CLICK_TARGET_VERSION = $syntheticManifest.version
        & uv run --env-file .env python -m pytest tests/test_windows_one_click_launcher.py -q
        if ($LASTEXITCODE -ne 0) { throw "real Python one-click launcher acceptance failed" }
    } finally {
        $env:SKY_AUTO_PLAYER_E2E_HANDSHAKE_ONLY = $previousHandshakeMode
        $env:SKY_ONE_CLICK_INSTALL_ROOT = $previousOneClickRoot
        $env:SKY_ONE_CLICK_CURRENT_VERSION = $previousOneClickCurrent
        $env:SKY_ONE_CLICK_TARGET_VERSION = $previousOneClickTarget
    }
    Write-EvidenceJson "one-click-launcher-result.json" ([ordered]@{
        status = "PASS"
        boundary = "python launch_update -> native ready handoff"
        app_exit_unit_test = "tests/test_textual_update_modals.py::test_ready_update_exits_exactly_once"
    })
    $script:Results.one_click_launcher = $true

    $scenario = New-Scenario "ready-visible" $fromRoot
    $readyResume = Join-Path $scenario.Root "ready-visible.resume"
    $readyRun = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate `
        -CurrentVersion $fromManifest.version -TargetVersion $syntheticManifest.version `
        -ReleaseDir $syntheticRelease -PauseAt "after-lock" -ResumeFile $readyResume `
        -KeepPaused -RequireProgressWindow
    $readyHandoff = Wait-ForReadyHandoff -RunInfo $readyRun -TargetVersion $syntheticManifest.version
    $readyActivePath = Join-Path $scenario.Local "Sky-Auto-Player\update-state\active-update.json"
    if (-not (Test-Path -LiteralPath $readyActivePath -PathType Leaf)) {
        Stop-ProcessIfRunning $readyRun.Process
        throw "ready-visible active state is missing"
    }
    $readyActive = Get-Content -LiteralPath $readyActivePath -Raw | ConvertFrom-Json
    $readyLock = @(Get-ChildItem (Join-Path $scenario.Local "Sky-Auto-Player\update-locks") -Filter "*.lock" -ErrorAction SilentlyContinue)
    if ($readyActive.updater_pid -ne $readyRun.Process.Id -or
        $readyActive.run_id -ne $readyHandoff.run_id -or
        $readyActive.target_version -ne $syntheticManifest.version -or
        $readyLock.Count -eq 0 -or -not (Wait-ForProgressWindow $readyRun.Process.Id)) {
        Stop-ProcessIfRunning $readyRun.Process
        throw "ready-visible did not prove lock, active state, handoff, and UI ownership together"
    }
    $readyCompleted = Resume-UpdaterAndReadResult $scenario $readyRun $readyResume
    if ($readyCompleted.Result.status -ne "success" -or $readyCompleted.ExitCode -ne 0) {
        throw "ready-visible resumed update did not complete successfully"
    }
    Write-EvidenceJson "ready-visible-result.json" ([ordered]@{
        status = "PASS"
        handoff = $readyHandoff
        active = $readyActive
        lock_owned = $true
        progress_window_visible = $true
        result = $readyCompleted.Result
        resumed_exit_code = $readyCompleted.ExitCode
    })
    Save-UpdaterLog "ready-visible" $scenario
    Stop-RestartedApp $scenario.Install
    $script:Results.ready_visible = $true

    $scenario = New-Scenario "locked-primary" $fromRoot
    $primaryLock = [IO.File]::Open((Join-Path $scenario.Install "Sky-Auto-Player.exe"), [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $run = Invoke-UpdaterExpectTerminal $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $syntheticRelease `
            -ExpectedStatus "failure" -ExpectedErrorCode "INSTALL_TARGET_BUSY"
    } finally {
        $primaryLock.Dispose()
    }
    if ($run.Result.error_code -ne "INSTALL_TARGET_BUSY" -or -not $run.TerminalWindowHeld -or
        (Test-Path (Join-Path $scenario.Install ".sky-update-transaction"))) { throw "locked-primary safety check failed" }
    Assert-NoInstallMutation $scenario
    $script:Results.locked_primary_safe = $true
    Write-EvidenceJson "locked-primary-result.json" $run.Result
    Save-UpdaterLog "locked-primary" $scenario

    $scenario = New-Scenario "integrity-failure" $fromRoot
    $run = Invoke-UpdaterExpectTerminal $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $corruptSidecarRelease `
        -ExpectedStatus "failure" -ExpectedErrorCode "CHECKSUM_MISMATCH"
    if ($run.Result.status -ne "failure" -or $run.Result.error_code -ne "CHECKSUM_MISMATCH" -or
        -not $run.TerminalWindowHeld -or (Test-Path (Join-Path $scenario.Install ".sky-update-transaction"))) {
        throw "corrupt sidecar did not produce a held integrity failure before mutation"
    }
    Assert-NoInstallMutation $scenario
    Assert-PreservedState $scenario
    Write-EvidenceJson "integrity-failure-result.json" $run.Result
    $script:Results.integrity_failure_ui = $true
    Save-UpdaterLog "integrity-failure" $scenario

    $scenario = New-Scenario "concurrent" $fromRoot
    $concurrentResume = Join-Path $scenario.Root "concurrent.resume"
    $first = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate -CurrentVersion $fromManifest.version `
        -TargetVersion $syntheticManifest.version -ReleaseDir $syntheticRelease -PauseAt "after-lock" `
        -ResumeFile $concurrentResume -KeepPaused
    $lockDeadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $lockFiles = @(Get-ChildItem (Join-Path $scenario.Local "Sky-Auto-Player\update-locks") -Filter "*.lock" -ErrorAction SilentlyContinue)
        if ($lockFiles.Count -gt 0) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $lockDeadline)
    $activeBeforeDuplicate = Wait-ForActiveUpdateState -RunInfo $first `
        -Path (Join-Path $scenario.Local "Sky-Auto-Player\update-state\active-update.json")
    $second = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate -CurrentVersion $fromManifest.version `
        -TargetVersion $syntheticManifest.version -ReleaseDir $syntheticRelease
    $second.Process.WaitForExit(15000) | Out-Null
    $secondHandoff = Read-RunHandoff $second.Run
    $activeAfterDuplicate = Get-Content -LiteralPath (Join-Path $scenario.Local "Sky-Auto-Player\update-state\active-update.json") -Raw | ConvertFrom-Json
    if ($second.Process.ExitCode -ne 0 -or $secondHandoff.state -ne "rejected" -or
        $secondHandoff.error_code -ne "UPDATE_ALREADY_RUNNING" -or
        $activeBeforeDuplicate.install_id -ne $activeAfterDuplicate.install_id -or
        $activeBeforeDuplicate.run_id -ne $activeAfterDuplicate.run_id -or
        $activeBeforeDuplicate.updater_pid -ne $activeAfterDuplicate.updater_pid -or
        $activeBeforeDuplicate.target_version -ne $activeAfterDuplicate.target_version -or
        (Test-Path (Join-Path $scenario.Install ".sky-update-transaction"))) { throw "concurrent updater ownership check failed" }
    $concurrentCompleted = Resume-UpdaterAndReadResult $scenario $first $concurrentResume
    if ($concurrentCompleted.Result.status -ne "success" -or $concurrentCompleted.ExitCode -ne 0) { throw "original updater did not complete after duplicate rejection" }
    [ordered]@{ status = $secondHandoff.error_code; state = $secondHandoff.state; exit_code = $second.Process.ExitCode; active_unchanged = $true; original_result = $concurrentCompleted.Result } |
        ConvertTo-Json | Set-Content (Join-Path $script:EvidenceRoot "concurrent-result.json") -Encoding utf8
    $script:Results.concurrent_blocked = $true
    Save-UpdaterLog "concurrent" $scenario
    Stop-RestartedApp $scenario.Install

    $scenario = New-Scenario "active-reopen" $fromRoot
    $reopenResume = Join-Path $scenario.Root "active-reopen.resume"
    $first = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate `
        -CurrentVersion $fromManifest.version -TargetVersion $syntheticManifest.version `
        -ReleaseDir $syntheticRelease -PauseAt "after-lock" -ResumeFile $reopenResume -KeepPaused
    $activePath = Join-Path $scenario.Local "Sky-Auto-Player\update-state\active-update.json"
    $activeDeadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        if (Test-Path -LiteralPath $activePath -PathType Leaf) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $activeDeadline)
    if (-not (Test-Path -LiteralPath $activePath -PathType Leaf)) { throw "active state was not created" }
    $reopenStdout = Join-Path $first.Run "reopen.stdout.txt"
    $reopenStderr = Join-Path $first.Run "reopen.stderr.txt"
    $reopened = Start-Process -FilePath (Join-Path $scenario.Install "Sky-Auto-Player.exe") `
        -WorkingDirectory $scenario.Install -RedirectStandardOutput $reopenStdout `
        -RedirectStandardError $reopenStderr -PassThru
    if (-not $reopened.WaitForExit(3000)) {
        Stop-ProcessIfRunning $reopened
        Stop-ProcessIfRunning $first.Process
        throw "reopened app stayed alive while updater was active"
    }
    $reopenOutput = if (Test-Path -LiteralPath $reopenStdout) {
        Get-Content -LiteralPath $reopenStdout -Raw
    } else { "" }
    if ($reopened.ExitCode -ne 0 -or
        $reopenOutput -notmatch "Sky Auto Player is currently updating to v" -or
        $reopenOutput -notmatch "The updater window will restart the app automatically") {
        Stop-ProcessIfRunning $first.Process
        throw "reopened app did not exit cleanly through the active-update startup guard"
    }
    $first.Process.Refresh()
    $activeAfterReopen = Get-Content -LiteralPath $activePath -Raw | ConvertFrom-Json
    if ($first.Process.HasExited -or $activeAfterReopen.updater_pid -ne $first.Process.Id) {
        Stop-ProcessIfRunning $first.Process
        throw "original updater did not retain active ownership after reopen guard"
    }
    $reopenCompleted = Resume-UpdaterAndReadResult $scenario $first $reopenResume
    if ($reopenCompleted.Result.status -ne "success" -or $reopenCompleted.ExitCode -ne 0) {
        throw "active-reopen updater did not complete after the guarded reopen"
    }
    Write-EvidenceJson "active-reopen-result.json" ([ordered]@{
        startup_guard_exit_code = $reopened.ExitCode
        startup_guard_output = $reopenOutput.Trim()
        active_state_created = $true
        active_state_owned_by_paused_updater = $true
        updater_remained_alive = $true
        resumed_result = $reopenCompleted.Result
    })
    $script:Results.active_reopen_guard = $true
    Save-UpdaterLog "active-reopen" $scenario
    Stop-RestartedApp $scenario.Install

    $scenario = New-Scenario "precommit-failure" $fromRoot
    $run = Invoke-UpdaterExpectTerminal $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $syntheticRelease `
        -FailAt "apply:before-replace:Sky-Auto-Player-Updater.exe" `
        -ExpectedStatus "rolled_back" -ExpectedErrorCode "ROLLED_BACK"
    if ($run.Result.status -ne "rolled_back" -or
        (Test-Path (Join-Path $scenario.Install ".sky-update-transaction"))) {
        throw "precommit failure did not produce a clean rolled-back result"
    }
    Assert-NoInstallMutation $scenario
    Assert-PreservedState $scenario
    Write-EvidenceJson "precommit-failure-result.json" $run.Result
    $script:Results.precommit_rollback = $true
    Save-UpdaterLog "precommit-failure" $scenario

    $scenario = New-Scenario "cleanup-access-denied" $fromRoot
    $oldPrimaryPath = Join-Path $scenario.Install "Sky-Auto-Player.exe"
    $oldPrimaryVersionInfo = (Get-Item -LiteralPath $oldPrimaryPath).VersionInfo
    $oldPrimaryHash = (Get-FileHash -LiteralPath $oldPrimaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $installedPrimaryPath = $oldPrimaryPath
    $parent = Start-ParentFixture $scenario.Install
    $parentPid = if ($parent) { [uint32]$parent.Id } else { [uint32]1 }
    $cleanupResume = Join-Path $scenario.Root "cleanup-resume.signal"
    $accessRun = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate `
        -ParentPid $parentPid -CurrentVersion $fromManifest.version -TargetVersion $syntheticManifest.version `
        -ReleaseDir $syntheticRelease -PauseAt "after-replace:apply:Sky-Auto-Player.exe" `
        -ResumeFile $cleanupResume `
        -Restart -KeepPaused -RequireProgressWindow
    # The production updater waits for the real parent to exit before it can
    # create the emergency backup. Stop it immediately after ready/progress;
    # waiting for .bak before this point would deadlock the acceptance path.
    Stop-ProcessIfRunning $parent
    $backup = Wait-ForPrimaryEmergencyBackup $scenario `
        -ExpectedOriginalFilename "Sky-Auto-Player.exe" `
        -ExpectedFileVersion $oldPrimaryVersionInfo.FileVersion `
        -ExpectedProductVersion $oldPrimaryVersionInfo.ProductVersion `
        -ExpectedSha256 $oldPrimaryHash
    if (-not $backup) {
        Stop-ProcessIfRunning $accessRun.Process
        throw "cleanup AccessDenied fixture did not create a forensic Sky-Auto-Player.exe backup"
    }
    if ($backup.OriginalFilename -ne "Sky-Auto-Player.exe" -or
        $backup.Sha256 -ne $backup.SourceSha256) {
        Stop-ProcessIfRunning $accessRun.Process
        throw "forensic backup metadata or source hash did not match Sky-Auto-Player.exe"
    }
    $backupPath = $backup.Path
    $deleteBlocker = $null
    try {
        # FileShare.Read deliberately omits FILE_SHARE_DELETE. The updater must
        # surface cleanup_pending while the verified install and restart still
        # succeed, then retry this exact .bak path after the handle is released.
        $deleteBlocker = [IO.File]::Open(
            $backupPath,
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            [IO.FileShare]::Read
        )
        if (-not (Test-Path -LiteralPath $backupPath -PathType Leaf)) {
            throw "locked forensic backup disappeared before cleanup began"
        }
        $accessRunResult = Resume-UpdaterAndReadResult $scenario $accessRun $cleanupResume
        $accessResult = $accessRunResult.Result
        $accessResultPath = $accessRunResult.ResultPath
        $matchingWarning = @($accessResult.warnings) | Where-Object {
            $_.path -eq $backupPath -and $_.os_error -in @(5, 32)
        } | Select-Object -First 1
        $targetPrimaryHash = ($syntheticManifest.files | Where-Object { $_.path -eq "Sky-Auto-Player.exe" }).sha256
        $installedPrimaryHash = (Get-FileHash -LiteralPath $installedPrimaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($accessResult.status -ne "success" -or -not $accessResult.cleanup_pending -or
            $null -eq $matchingWarning -or $installedPrimaryHash -ne $targetPrimaryHash -or
            -not (Assert-RestartObserved $scenario.Install) -or -not (Test-Path -LiteralPath $backupPath -PathType Leaf)) {
            throw "cleanup AccessDenied did not preserve success, exact warning provenance, target bytes, restart, and locked backup"
        }
        Write-EvidenceJson "cleanup-access-denied-handle.json" ([ordered]@{
            locked_backup = $backupPath
            original_filename = $backup.OriginalFilename
            file_version = $backup.FileVersion
            product_version = $backup.ProductVersion
            source_sha256 = $backup.SourceSha256
            backup_sha256 = $backup.Sha256
            warning_path = $matchingWarning.path
            warning_os_error = $matchingWarning.os_error
            cleanup_pending = [bool]$accessResult.cleanup_pending
            status = $accessResult.status
            backup_present_while_handle_held = $true
            restart_verified = $true
        })
        $script:Results.cleanup_access_denied = $true
        Save-ScenarioEvidence "cleanup-access-denied" $scenario $accessRunResult $syntheticManifest | Out-Null
        Save-UpdaterLog "cleanup-access-denied" $scenario
    } finally {
        if ($deleteBlocker) { $deleteBlocker.Dispose() }
        Stop-RestartedApp $scenario.Install
    }
    $cleanupRetry = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate `
        -CurrentVersion $fromManifest.version -TargetVersion $syntheticTargetVersion `
        -CleanupOnly
    if (-not $cleanupRetry.Process.WaitForExit(15000) -or $cleanupRetry.Process.ExitCode -ne 0 -or
        (Test-Path -LiteralPath $backupPath -PathType Leaf)) {
        Stop-ProcessIfRunning $cleanupRetry.Process
        throw "released forensic backup was not removed by the later best-effort cleanup cycle"
    }
    $primaryAfterRetry = (Get-FileHash -LiteralPath $installedPrimaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($primaryAfterRetry -ne $targetPrimaryHash) { throw "later cleanup cycle changed managed primary bytes" }
    Write-EvidenceJson "cleanup-access-denied-result.json" ([ordered]@{
        status = "PASS"
        warning = $matchingWarning
        backup_removed_after_handle_release = $true
        primary_sha256_after_retry = $primaryAfterRetry
    })

    $scenario = New-Scenario "restart-failure" $fromRoot
    $run = Invoke-UpdaterExpectTerminal $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $syntheticRelease `
        -ExpectedStatus "failure" -ExpectedErrorCode "RESTART_FAILED" `
        -Restart -FailRestart
    $restartManifest = Get-ManifestObject $scenario.Install
    if ($run.Result.status -ne "failure" -or $run.Result.error_code -ne "RESTART_FAILED" -or
        (Test-Path (Join-Path $scenario.Install ".sky-update-transaction")) -or
        $restartManifest.version -ne $syntheticManifest.version) {
        throw "restart failure did not preserve the committed install and failure result"
    }
    Assert-PreservedState $scenario
    Write-EvidenceJson "restart-failure-result.json" $run.Result
    $script:Results.restart_failure_result = $true
    Save-UpdaterLog "restart-failure" $scenario

    $scenario = New-Scenario "rollback-fault" $fromRoot
    $prepared = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate `
        -CurrentVersion $fromManifest.version -TargetVersion $syntheticManifest.version -ReleaseDir $syntheticRelease `
        -PauseAt "after-replace:apply:Sky-Auto-Player-Updater.exe" -KeepPaused
    $preparedJournal = Wait-ForPreparedJournal $scenario.Install
    Stop-ProcessIfRunning $prepared.Process
    if (-not $preparedJournal) { throw "rollback fault fixture did not leave Prepared journal" }
    $run = Invoke-UpdaterExpectTerminal $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $syntheticRelease `
        -FailAt "rollback:after-restore:Sky-Auto-Player-Updater.exe" `
        -ExpectedStatus "failure" -ExpectedErrorCode "ROLLBACK_ATOMIC_REPLACE_FAILED"
    if ($run.Result.error_code -ne "ROLLBACK_ATOMIC_REPLACE_FAILED" -or -not (Test-Path (Join-Path $scenario.Install ".sky-update-transaction"))) { throw "rollback fault safety check failed" }
    if (-not (Test-Path (Join-Path $scenario.Install "Sky-Auto-Player-Updater.exe"))) { throw "rollback fault removed updater" }
    $rollbackUpdaterHash = (Get-FileHash (Join-Path $scenario.Install "Sky-Auto-Player-Updater.exe") -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($rollbackUpdaterHash -ne $scenario.BeforeHashes["Sky-Auto-Player-Updater.exe"]) { throw "rollback fault did not preserve old updater hash" }
    Write-EvidenceJson "rollback-fault-result.json" $run.Result
    $script:Results.rollback_preserved_updater = $true
    Save-UpdaterLog "rollback-fault" $scenario
    $recovered = Invoke-Updater $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $syntheticRelease
    if ($recovered.Result.status -ne "success" -or (Test-Path (Join-Path $scenario.Install ".sky-update-transaction"))) { throw "rollback recovery failed" }
    Save-ScenarioEvidence "rollback-fault-recovery" $scenario $recovered $syntheticManifest | Out-Null

    $scenario = New-Scenario "crash-recovery" $fromRoot
    $first = Start-UpdaterProcess -Install $scenario.Install -Candidate $e2eCandidate -CurrentVersion $fromManifest.version `
        -TargetVersion $syntheticManifest.version -ReleaseDir $syntheticRelease `
        -PauseAt "after-replace:apply:Sky-Auto-Player-Updater.exe" -KeepPaused
    $crashJournal = Wait-ForPreparedJournal $scenario.Install
    Stop-ProcessIfRunning $first.Process
    if (-not $crashJournal) { throw "crash fixture did not leave Prepared journal" }
    $run = Invoke-Updater $scenario $e2eCandidate $fromManifest.version $syntheticManifest.version $syntheticRelease
    if ($run.Result.status -ne "success" -or (Test-Path (Join-Path $scenario.Install ".sky-update-transaction"))) { throw "crash recovery failed" }
    Assert-ManagedManifestFiles $scenario.Install $syntheticManifest | Out-Null
    Assert-PreservedState $scenario
    Write-EvidenceJson "crash-recovery-result.json" $run.Result
    $script:Results.crash_recovery = $true
    Save-UpdaterLog "crash-recovery" $scenario

    $script:Results.overall = "PASS"
    $script:Results.restart_verified = $true
    $script:AllPassed = $true
}
catch {
    $script:Results.overall = "FAILED"
    $script:Results.failure = $_.Exception.Message
    Write-Warning $_
}
finally {
    $env:LOCALAPPDATA = $script:PreviousLocalAppData
    New-Item -ItemType Directory -Path $script:EvidenceRoot -Force | Out-Null
    if (-not $script:HarnessElevated -and $null -ne $script:DefenderRunStart) {
        try {
            $script:DefenderAfter = Invoke-ElevatedDefenderSnapshot "after" $script:DefenderRunStart
            Assert-DefenderEnabled $script:DefenderAfter "after acceptance"
            if ($null -eq $script:DefenderBefore) {
                throw "Defender baseline was not captured"
            }
            $script:DefenderExclusionsUnchanged = Test-DefenderExclusionsUnchanged `
                $script:DefenderBefore $script:DefenderAfter
            if (-not $script:DefenderExclusionsUnchanged) {
                throw "Defender exclusions changed during acceptance"
            }
            $script:DefenderThreatCount = if ($script:DefenderAfter) {
                [int]$script:DefenderAfter.threat_detection_count
            } else {
                $null
            }
            Write-EvidenceJson "defender-after.json" $script:DefenderAfter
        }
        catch {
            $script:Results.overall = "FAILED"
            $script:Results.failure = $_.Exception.Message
            $script:AllPassed = $false
            Write-Warning $_
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $script:EvidenceRoot "updater.log"))) {
        New-Item -ItemType File -Path (Join-Path $script:EvidenceRoot "updater.log") -Force | Out-Null
    }
    if (-not $script:Results.Contains("overall")) { $script:Results.overall = "FAILED" }
    Write-EvidenceJson "environment.json" ([ordered]@{
        os = [Environment]::OSVersion.VersionString
        powershell = $PSVersionTable.PSVersion.ToString()
        computer = $env:COMPUTERNAME
        user = $env:USERNAME
        timestamp_utc = $script:Timestamp
        defender_exclusions_changed = if ($script:DefenderBefore -and $script:DefenderAfter) { -not $script:DefenderExclusionsUnchanged } else { $null }
        defender_antivirus_enabled_before = if ($script:DefenderBefore) { [bool]$script:DefenderBefore.antivirus_enabled } else { $false }
        defender_antivirus_enabled_after = if ($script:DefenderAfter) { [bool]$script:DefenderAfter.antivirus_enabled } else { $false }
        defender_realtime_enabled_before = if ($script:DefenderBefore) { [bool]$script:DefenderBefore.realtime_protection_enabled } else { $false }
        defender_realtime_enabled_after = if ($script:DefenderAfter) { [bool]$script:DefenderAfter.realtime_protection_enabled } else { $false }
        defender_exclusions_before = if ($script:DefenderBefore) { @($script:DefenderBefore.exclusions) } else { @() }
        defender_exclusions_after = if ($script:DefenderAfter) { @($script:DefenderAfter.exclusions) } else { @() }
        defender_exclusions_unchanged = [bool]$script:DefenderExclusionsUnchanged
        defender_detections_since_start = $script:DefenderThreatCount
        harness_elevated = [bool]$script:HarnessElevated
        defender_snapshot_elevated = [bool]$script:DefenderSnapshotElevated
        sandbox = $script:SandboxRoot
    })
    Write-EvidenceJson "summary.json" ([ordered]@{
        overall = $script:Results.overall
        canonical_v345_to_v346 = if ($script:Results.canonical_v345_to_v346) { $script:Results.canonical_v345_to_v346.status } else { "FAIL" }
        ready_visible = [bool]$script:Results.ready_visible
        happy_github = if ($script:Results.happy_github) { $script:Results.happy_github.status } else { "NOT_RUN" }
        one_click_launcher = [bool]$script:Results.one_click_launcher
        integrity_failure_ui = [bool]$script:Results.integrity_failure_ui
        locked_primary_safe = [bool]$script:Results.locked_primary_safe
        concurrent_blocked = [bool]$script:Results.concurrent_blocked
        active_reopen_guard = [bool]$script:Results.active_reopen_guard
        precommit_rollback = [bool]$script:Results.precommit_rollback
        rollback_preserved_updater = [bool]$script:Results.rollback_preserved_updater
        cleanup_access_denied = [bool]$script:Results.cleanup_access_denied
        restart_failure_result = [bool]$script:Results.restart_failure_result
        restart_verified = [bool]$script:Results.restart_verified
        crash_recovery = [bool]$script:Results.crash_recovery
        defender_antivirus_enabled_before = if ($script:DefenderBefore) { [bool]$script:DefenderBefore.antivirus_enabled } else { $false }
        defender_antivirus_enabled_after = if ($script:DefenderAfter) { [bool]$script:DefenderAfter.antivirus_enabled } else { $false }
        defender_realtime_enabled_before = if ($script:DefenderBefore) { [bool]$script:DefenderBefore.realtime_protection_enabled } else { $false }
        defender_realtime_enabled_after = if ($script:DefenderAfter) { [bool]$script:DefenderAfter.realtime_protection_enabled } else { $false }
        defender_exclusions_unchanged = [bool]$script:DefenderExclusionsUnchanged
        harness_elevated = [bool]$script:HarnessElevated
        defender_snapshot_elevated = [bool]$script:DefenderSnapshotElevated
    })
    if ($script:AllPassed -and -not $KeepEvidence -and (Test-Path $script:SandboxRoot)) {
        Remove-Item -LiteralPath $script:SandboxRoot -Recurse -Force
    }
}

if (-not $script:AllPassed) {
    exit 1
}
Write-Host "Windows updater E2E PASS. Evidence: $script:EvidenceRoot"
