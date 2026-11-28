param(
    [ValidateSet('DispatchCore', 'Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2', 'PlaybackCoreLifecycleV3', 'ChordWidthPilotV1')]
    [string]$QualificationProfile = 'DispatchCore',
    [Parameter(Mandatory)]
[ValidateSet('canonical-single', 'canonical-chord', 'canonical-max-chord', 'hold', 'long-single-sequence', 'dense-alternating', 'chord-sweep', 'near-minimum-retrigger', 'rapid-retrigger', 'short-up-gap-stress', 'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary', 'modifier-held-after-owned', 'cleanup-full-release', 'focus-loss', 'target-hwnd-change', 'owner-mismatch', 'owner-query-failure', 'owner-process-termination', 'pause-resume', 'pause-resume-active-cleanup', 'suspend-resume', 'stop-cleanup', 'skip-cleanup', 'supervisor-lease-expiry', 'lease-age-watchdog-delayed', 'noncanonical-profile', 'timing-margin-sweep', 'long-idle', 'hol-interleaved', 'chord-width-pilot-v1')]
    [string]$Scenario,
    [ValidateSet('canonical-single', 'safety-smoke', 'dense-alternating', 'mixed-up-down', 'canonical-max-chord', 'near-minimum-retrigger', 'long-idle', 'boundary-60fps-margin-0', 'boundary-120fps-margin-500', 'chord-width-pilot-v1')]
    [string]$Workload,
    [ValidateSet(0, 1, 5, 15)]
    [int]$PilotChordWidth = 0,
    [Parameter(Mandatory)]
    [ValidateScript({ $_ -ge 0 -and $_ -le 3000 -and $_ % 100 -eq 0 })]
    [int]$TimingMarginUs,
    [switch]$TimingObserverForQualification,
    [switch]$CpuContention,
    [switch]$HistoricalSourceAllowed,
    [switch]$QpcReadCountingEvidenceOnly,
    [string]$HarnessPath,
    [string]$HarnessManifestPath,
    [string]$OutputRoot,
    [switch]$OwnerApprovedReceiveOnlyRun,
    [switch]$PreflightOnly,
    [ValidateRange(15, 15)]
    [int]$SampleSeconds,
    [ValidateSet(60, 120)]
    [int]$GameFps = 60,
    [int]$HolStallUs = 0,
    [int]$HolCycles = 0,
    [ValidateSet('control-a', 'candidate-b-up-diagnostic')]
    [string]$AdmissionVariant = 'control-a',
    [string]$ApprovedPreflightPlanPath,
    [string]$ExpectedApprovedSourceRevision,
    [string]$ExpectedApprovedSourceTree,
    [string]$ExpectedApprovedManifestSha256,
    [string]$ExpectedApprovedBinarySha256,
    [string]$ExpectedApprovedPreflightPlanSha256
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if ($AdmissionVariant -eq 'candidate-b-up-diagnostic') {
    throw 'candidate-b-up-diagnostic has been retired following LAB4 evaluation: Candidate B-Up was rejected/unqualified for shipping'
}
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Import-Module (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_profiles.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_report.psm1') -Force
$isDispatchCleanupM3 = $QualificationProfile -eq 'DispatchCleanupM3'
$isSessionRedesignProfile = $QualificationProfile -eq 'DispatchSessionRedesignM3'
$isPlaybackCoreLifecycle = $QualificationProfile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2', 'PlaybackCoreLifecycleV3')
$isChordWidthPilotV1 = $QualificationProfile -eq 'ChordWidthPilotV1'
if ($PreflightOnly -and -not ($isPlaybackCoreLifecycle -or $isChordWidthPilotV1)) {
    throw '-PreflightOnly is restricted to PlaybackCoreLifecycle and ChordWidthPilotV1 profiles'
}
if ($PreflightOnly -and $OwnerApprovedReceiveOnlyRun) {
    throw '-PreflightOnly cannot be combined with -OwnerApprovedReceiveOnlyRun'
}
if ($isChordWidthPilotV1) {
    if ($Scenario -ne 'chord-width-pilot-v1' -or $Workload -ne 'chord-width-pilot-v1' -or
        $PilotChordWidth -notin @(1, 5, 15) -or $GameFps -ne 60 -or $TimingMarginUs -ne 500 -or
        $SampleSeconds -ne 15 -or
        -not $TimingObserverForQualification -or $AdmissionVariant -ne 'control-a' -or
        $CpuContention -or $HolStallUs -ne 0 -or $HolCycles -ne 0) {
        throw 'ChordWidthPilotV1 requires scenario/workload chord-width-pilot-v1, PilotChordWidth in {1,5,15}, 60 FPS, 500 us margin (H=17167), 15s sample, TimingObserverForQualification, control-a, quiet-only, and no HOL overrides'
    }
} elseif ($PilotChordWidth -ne 0 -or $Scenario -eq 'chord-width-pilot-v1' -or $Workload -eq 'chord-width-pilot-v1' -or
    -not [string]::IsNullOrWhiteSpace($ApprovedPreflightPlanPath) -or
    -not [string]::IsNullOrWhiteSpace($ExpectedApprovedSourceRevision) -or
    -not [string]::IsNullOrWhiteSpace($ExpectedApprovedSourceTree) -or
    -not [string]::IsNullOrWhiteSpace($ExpectedApprovedManifestSha256) -or
    -not [string]::IsNullOrWhiteSpace($ExpectedApprovedBinarySha256) -or
    -not [string]::IsNullOrWhiteSpace($ExpectedApprovedPreflightPlanSha256)) {
    throw 'chord-width-pilot-v1, -PilotChordWidth, and pilot entry-gate approval parameters are valid only with -QualificationProfile ChordWidthPilotV1'
}
$usesSessionQualificationContracts = $isDispatchCleanupM3 -or $isSessionRedesignProfile -or $isPlaybackCoreLifecycle -or $isChordWidthPilotV1
$dispatchCleanupM3 = if ($isDispatchCleanupM3) { Get-DispatchCleanupM3Profile } else { $null }
$sessionRedesignProfile = if ($isSessionRedesignProfile) { Get-DispatchSessionRedesignM3Profile } else { $null }
$playbackCoreProfile = if ($isPlaybackCoreLifecycle) { Get-PlaybackCoreLifecycleProfileForName -Name $QualificationProfile } else { $null }
$expectedHarnessManifestSchemaVersion = if ($isPlaybackCoreLifecycle) {
    $playbackCoreProfile.HarnessManifestSchemaVersion
} elseif ($isSessionRedesignProfile) {
    $sessionRedesignProfile.HarnessManifestSchemaVersion
} else {
    1
}
$sessionTimingFloors = if ($isDispatchCleanupM3) {
    Get-DispatchCleanupM3TimingFloors -Scenario $Scenario -GameFps $GameFps -TimingMarginUs $TimingMarginUs
} elseif ($isSessionRedesignProfile) {
    Assert-DispatchSessionRedesignM3SessionIdentity -Scenario $Scenario -Workload $Workload `
        -TimingMarginUs $TimingMarginUs -GameFps $GameFps -SampleSeconds $SampleSeconds
} elseif ($isPlaybackCoreLifecycle) {
    Assert-PlaybackCoreLifecycleSessionIdentity -Scenario $Scenario -Workload $Workload `
        -TimingMarginUs $TimingMarginUs -GameFps $GameFps -SampleSeconds $SampleSeconds `
        -ProfileVersion $playbackCoreProfile.Version
} elseif ($isChordWidthPilotV1) {
    @{ expected_hold_us = 17167; configured_H_us = 17167 }
} else { $null }
if ($usesSessionQualificationContracts) {
    if (-not $HarnessPath -or -not $HarnessManifestPath -or -not $OutputRoot) {
        throw ($QualificationProfile + ' requires explicit immutable binary, manifest, and external evidence paths')
    }
    if ($QpcReadCountingEvidenceOnly) {
        throw ($QualificationProfile + ' does not permit qpc-read-counting outside a separately approved evidence protocol')
    }
    if (-not (Test-DispatchM3PathOutside -Path $OutputRoot -RepositoryRoot $root)) {
        throw ($QualificationProfile + ' native evidence output root must be outside the repository checkout')
    }
}
$sinkScript = Join-Path $root 'scripts\native_acceptance_sink.ps1'
$qualifiedProfile = $QualificationProfile -in @('Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2', 'PlaybackCoreLifecycleV3', 'ChordWidthPilotV1')
$receiveOnlyObserverVersion = if ($qualifiedProfile) {
    'scheduler-receiveonly-queue-v3-compat3'
} else {
    $null
}
$pwshPath = (Get-Command pwsh.exe).Source
$pwshExecutablePath = (Get-Process -Id $PID).Path
$runId = 'native-case-' + (Get-Date -Format 'yyyyMMddTHHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$evidenceRoot = if ($OutputRoot) {
    [System.IO.Path]::GetFullPath($OutputRoot)
} else {
    Join-Path $root '.benchmarks\physical-native-cases'
}
$runDir = Join-Path $evidenceRoot $runId
$readyPath = Join-Path $runDir 'sink-ready.json'
$eventsPath = Join-Path $runDir 'sink-events.jsonl'
$sinkObserverSummaryPath = Join-Path $runDir 'sink-observer-summary.json'
$stdoutPath = Join-Path $runDir 'harness-output.log'
$reportPath = Join-Path $runDir 'rt-native-acceptance.jsonl'
$runConfigPath = Join-Path $runDir 'run-config.json'
$invocationPath = Join-Path $runDir 'invocation.json'
$windowPath = Join-Path $runDir 'sink-event-window.json'
$probeReadyPath = Join-Path $runDir 'focus-probe-ready.json'
$probeEventsPath = Join-Path $runDir 'focus-probe-events.jsonl'
$focusRestoreRequestPath = Join-Path $runDir 'focus-restore.request'
$harness = if ($HarnessPath) {
    [System.IO.Path]::GetFullPath($HarnessPath)
} else {
    Join-Path $root 'target\release\rt-native-acceptance.exe'
}
$harnessManifest = $null
$harnessManifestHash = $null
$sinkProcess = $null
$probeProcess = $null
$contentionProcesses = @()
$nativeExit = $null
$report = $null
$windowIntegrity = $false
$resultCode = 2
$head = $null
$pilotEntryGateVerified = $false
$pilotRemoteTip = $null
$pilotActualLockSha = $null
$pilotCargoVer = $null
$pilotSinkSha = $null
$pilotAnalyzerSha = $null
$pilotCaseRunnerSha = $null
$pilotPrepareSha = $null
$profileBaseRevision = if ($isDispatchCleanupM3) { $dispatchCleanupM3.BaselineRevision } elseif ($isSessionRedesignProfile) { $sessionRedesignProfile.BaselineRevision } elseif ($QualificationProfile -eq 'FrameSnapshot') { '8a800ff07a5dc362a5ee3aaa59204ff40b6ae327' } else { 'b9ed666e3ddddc4952831626f1654e1cc9071a10' }
$profileBaseTree = if ($isDispatchCleanupM3) { $dispatchCleanupM3.BaselineTree } elseif ($isSessionRedesignProfile) { $sessionRedesignProfile.BaselineTree } elseif ($QualificationProfile -eq 'FrameSnapshot') { '48eec424c155e98fa1a81ad018bc832b6e6b964f' } else { 'f554c41b23a3969d02ed29842d4178f6143c9d86' }
$profileBranch = if ($isDispatchCleanupM3) { $dispatchCleanupM3.CandidateLineageBranch } elseif ($isSessionRedesignProfile) { $sessionRedesignProfile.CandidateLineageBranch } elseif ($QualificationProfile -eq 'FrameSnapshot') { 'refactor/frame-snapshot-dispatch' } else { 'refactor/scheduler-single-owner-20261112' }

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class NativeAcceptanceForegroundPermission {
    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool AllowSetForegroundWindow(uint processId);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
}
"@

function Wait-ReadyRecord {
    param([string]$Path, [System.Diagnostics.Process]$Process)
    $deadline = (Get-Date).AddSeconds(20)
    while (-not (Test-Path -LiteralPath $Path) -and (Get-Date) -lt $deadline -and -not $Process.HasExited) {
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path -LiteralPath $Path)) {
        throw ('receive-only sink ready record missing for process ' + $Process.Id)
    }
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Assert-SinkIdentity {
    param($Record, [int]$SessionId, [string]$ExpectedObserverVersion)
    if ($Record.schema_version -ne 3 -or $Record.event_schema_version -ne 3 -or $Record.run_id -ne $runId -or $Record.role -ne 'ReceiveOnly') {
        throw 'receive-only sink ready record schema, role, or run ID mismatch'
    }
    $gameTargetProperty = $Record.PSObject.Properties['game_target']
    if ($Record.sink_kind -ne 'SkyAutoPlayer.NativeAcceptanceSink' -or
        $Record.target_class -ne 'project-owned-non-game-receive-only' -or
        $Record.target_name -ne 'Sky Auto Player — Native Acceptance Sink' -or
        $null -eq $gameTargetProperty -or
        $gameTargetProperty.Value -isnot [bool] -or
        $gameTargetProperty.Value -ne $false -or
        $Record.title -ne 'Sky Auto Player — Native Acceptance Sink' -or
        $Record.process -ne 'native_acceptance_sink.ps1' -or
        $Record.input_policy -ne 'receives_only; benchmark must use SendInput') {
        throw 'ready record does not identify the project-owned receive-only sink'
    }
    if ($Record.pid -le 0 -or $Record.hwnd -le 0 -or $Record.event_log_id.Length -eq 0 -or $Record.process_start_time_filetime -le 0) {
        throw 'sink ready record omitted process, HWND, event-log, or start-time identity'
    }
    if ($ExpectedObserverVersion) {
        if ($ExpectedObserverVersion -ne 'scheduler-receiveonly-queue-v3-compat3' -or
            $Record.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
            $Record.observer_version -ne $ExpectedObserverVersion -or
            $Record.observer_queue_capacity -ne 32768 -or
            $Record.observer_qpc_frequency_hz -le 0) {
            throw 'Scheduler ReceiveOnly V3 Compat3 observer mode, version, or queue provenance mismatch'
        }
    } elseif ($Record.PSObject.Properties['observer_version'] -or $Record.PSObject.Properties['observer_mode']) {
        throw 'DispatchCore must retain the legacy ReceiveOnly observer protocol'
    }
    $owner = Get-Process -Id ([int]$Record.pid)
    $deadline = (Get-Date).AddSeconds(3)
    while ((Get-Date) -lt $deadline -and ($owner.MainWindowHandle -ne [long]$Record.hwnd -or $owner.MainWindowTitle -ne $Record.title)) {
        Start-Sleep -Milliseconds 50
        $owner.Refresh()
    }
    $ownerStartTimeFiletime = [long]$owner.StartTime.ToUniversalTime().ToFileTimeUtc()
    $sameExecutable = [string]::Equals($owner.Path, $pwshExecutablePath, [System.StringComparison]::OrdinalIgnoreCase)
    if (-not $sameExecutable -or $owner.SessionId -ne $SessionId -or $ownerStartTimeFiletime -ne [long]$Record.process_start_time_filetime -or [long]$owner.MainWindowHandle -ne [long]$Record.hwnd -or $owner.MainWindowTitle -ne $Record.title) {
        throw ('live sink HWND/process identity mismatch (path=' + $owner.Path + '; expected_path=' + $pwshExecutablePath + '; session=' + $owner.SessionId + '; expected_session=' + $SessionId + '; start=' + $ownerStartTimeFiletime + '; expected_start=' + $Record.process_start_time_filetime + '; hwnd=' + $owner.MainWindowHandle + '; expected_hwnd=' + $Record.hwnd + '; title=' + $owner.MainWindowTitle + '; expected_title=' + $Record.title + ')')
    }
    $records = @(Get-Content -LiteralPath $eventsPath | ForEach-Object { $_ | ConvertFrom-Json })
    if ($records.Count -lt 1 -or $records[0].kind -ne 'stream_start' -or $records[0].sequence -ne 0) {
        throw 'fresh sink event log is missing its stream header'
    }
    for ($index = 0; $index -lt $records.Count; $index++) {
        $event = $records[$index]
        $expectedEventSchemaVersion = 3
        if ($event.schema_version -ne $expectedEventSchemaVersion -or $event.run_id -ne $runId -or $event.role -ne 'ReceiveOnly' -or $event.event_log_id -ne $Record.event_log_id -or $event.sequence -ne $index -or ($index -gt 0 -and $event.kind -eq 'stream_start')) {
            throw ('pre-arm sink stream identity or sequence mismatch at record ' + $index)
        }
        if ($ExpectedObserverVersion -and
            ($event.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
                $event.observer_version -ne $ExpectedObserverVersion)) {
            throw ('pre-arm Scheduler observer mode or version mismatch at record ' + $index)
        }
        if (-not $ExpectedObserverVersion -and
            ($event.PSObject.Properties['observer_mode'] -or $event.PSObject.Properties['observer_version'])) {
            throw ('pre-arm DispatchCore stream must retain the legacy observer protocol at record ' + $index)
        }
    }
}

function Assert-DispatchCleanupM3SessionIdentity {
    param(
        [Parameter(Mandatory)] [string]$Scenario,
        [Parameter(Mandatory)] [string]$Workload,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [int]$GameFps,
        [int]$SampleSeconds
    )
    $performanceWorkloads = @('dense-alternating', 'mixed-up-down', 'canonical-max-chord', 'near-minimum-retrigger', 'long-idle')
    $safetyScenarios = @(
        'canonical-single', 'canonical-max-chord', 'near-minimum-retrigger', 'short-up-gap-stress',
        'mixed-up-down', 'ambiguous-packet', 'preflight-user-held',
        'modifier-held-final-boundary', 'modifier-held-after-owned', 'focus-loss', 'target-hwnd-change',
        'owner-mismatch', 'owner-query-failure', 'owner-process-termination', 'pause-resume',
        'suspend-resume', 'stop-cleanup', 'skip-cleanup',
        'supervisor-lease-expiry', 'lease-age-watchdog-delayed', 'noncanonical-profile'
    )
    if ($Workload -in $performanceWorkloads) {
        if ($Scenario -ne $Workload -or $TimingMarginUs -ne 0 -or $GameFps -ne 60 -or $SampleSeconds -ne 15) {
            throw 'DispatchCleanupM3 performance sessions require 60 FPS, 0 us performance margin, and 15-second samples'
        }
        return $true
    }
    if ($Workload -eq 'boundary-60fps-margin-0' -or $Workload -eq 'boundary-120fps-margin-500') {
        $validBoundary = $Scenario -eq 'timing-margin-sweep' -and
            (($Workload -eq 'boundary-60fps-margin-0' -and $GameFps -eq 60 -and $TimingMarginUs -eq 0) -or
                ($Workload -eq 'boundary-120fps-margin-500' -and $GameFps -eq 120 -and $TimingMarginUs -eq 500))
        if (-not $validBoundary -or $SampleSeconds -gt 0) {
            throw 'DispatchCleanupM3 boundary sessions must be 60 FPS/0 us or 120 FPS/500 us without a timed performance sample'
        }
        return $true
    }
    if ($Workload -in @('canonical-single', 'safety-smoke') -and $Scenario -in $safetyScenarios) {
        if ($TimingMarginUs -ne 500 -or $GameFps -ne 60 -or $SampleSeconds -gt 0) {
            throw 'DispatchCleanupM3 safety sessions require 60 FPS, 500 us safety margin, and no performance sample'
        }
        return $true
    }
    throw ('DispatchCleanupM3 does not authorize workload/scenario ' + $Workload + '/' + $Scenario)
}

function Assert-SchedulerObserverEvidence {
    param($Ready, $Summary, [object[]]$Records, [string]$ExpectedObserverVersion)
    if ($ExpectedObserverVersion -ne 'scheduler-receiveonly-queue-v3-compat3' -or
        $Ready.schema_version -ne 3 -or $Ready.event_schema_version -ne 3 -or
        $Ready.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
        $Ready.observer_version -ne $ExpectedObserverVersion -or
        $Summary.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
        $Summary.observer_version -ne $ExpectedObserverVersion) {
        throw 'ReceiveOnly observer identity is not the authorized SchedulerQueuedV3Compat3 protocol'
    }
    if ($Summary.schema_version -ne 1 -or $Summary.run_id -ne $runId -or
        $Summary.role -ne 'ReceiveOnly' -or $Summary.event_log_id -ne $Ready.event_log_id -or
        [long]$Summary.target_hwnd -ne [long]$Ready.hwnd -or
        [int]$Summary.target_pid -ne [int]$Ready.pid -or
        [long]$Summary.qpc_frequency_hz -ne [long]$Ready.observer_qpc_frequency_hz -or
        [int]$Summary.queue_capacity -ne 32768 -or
        -not [bool]$Summary.stream_header_flushed -or
        -not [bool]$Summary.events_flushed -or
        [long]$Summary.observer_startup_failures -ne 0 -or
        -not [bool]$Summary.consumer_stopped -or -not [bool]$Summary.queue_drained) {
        throw 'Scheduler observer summary identity, version, or drain evidence is invalid'
    }
    if ([bool]$Summary.queue_overflow -or [long]$Summary.queue_dropped -ne 0 -or
        [long]$Summary.writer_failures -ne 0 -or
        [long]$Summary.observer_internal_failures -ne 0 -or
        [long]$Summary.handler_latency_sample_dropped -ne 0 -or
        [long]$Summary.events_seen -ne [long]$Summary.events_enqueued -or
        [long]$Summary.events_enqueued -ne [long]$Summary.events_written -or
        [long]$Summary.queue_high_water -gt [long]$Summary.queue_capacity -or
        [long]$Summary.queue_high_water -lt 0) {
        throw 'Scheduler observer overflow, loss, or incomplete counters fail closed'
    }
    $events = @($Records | Where-Object { $_.kind -ne 'stream_start' })
    if ([long]$Summary.events_seen -ne $events.Count -or
        [long]$Summary.handler_latency_count -ne $events.Count -or
        [long]$Summary.handler_latency_sample_count -ne $events.Count) {
        throw 'Scheduler observer event and handler-latency counts do not reconcile'
    }
    if ($Records.Count -lt 1 -or $Records[0].kind -ne 'stream_start') {
        throw 'Scheduler observer stream header is missing'
    }
    $previousQpc = -1L
    foreach ($index in 0..($Records.Count - 1)) {
        $event = $Records[$index]
        if ($event.schema_version -ne 3 -or $event.run_id -ne $Ready.run_id -or
            $event.role -ne 'ReceiveOnly' -or $event.event_log_id -ne $Ready.event_log_id -or
            $event.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
            $event.observer_version -ne $ExpectedObserverVersion -or
            [long]$event.sequence -ne $index) {
            throw ('Scheduler observer identity or sequence mismatch at record ' + $index)
        }
        if ($index -eq 0) {
            if ($event.kind -ne 'stream_start' -or
                $null -eq $event.PSObject.Properties['observed_qpc_ticks'] -or
                [long]$event.observed_qpc_ticks -le 0 -or
                [long]$event.target_hwnd -ne [long]$Ready.hwnd -or
                [int]$event.target_pid -ne [int]$Ready.pid -or
                $null -eq $event.PSObject.Properties['foreground_hwnd'] -or
                $null -eq $event.PSObject.Properties['foreground_pid']) {
                throw 'Scheduler observer header is missing schema-3-compatible provenance'
            }
            $previousQpc = [long]$event.observed_qpc_ticks
            continue
        }
        if ($event.kind -notin @('key_press', 'key_release') -or
            $null -eq $event.PSObject.Properties['scan_code'] -or
            [int]$event.scan_code -le 0 -or [int]$event.scan_code -gt 0xff -or
            $null -eq $event.PSObject.Properties['extended'] -or
            [long]$event.target_hwnd -ne [long]$Ready.hwnd -or
            [int]$event.target_pid -ne [int]$Ready.pid -or
            [long]$event.foreground_hwnd -ne [long]$Ready.hwnd -or
            [int]$event.foreground_pid -ne [int]$Ready.pid) {
            throw ('Scheduler event arrived at a wrong target or foreground at record ' + $index)
        }
        if ($null -eq $event.PSObject.Properties['virtual_key'] -or
            $null -eq $event.PSObject.Properties['message'] -or
            $null -eq $event.PSObject.Properties['observed_qpc_ticks'] -or
            [long]$event.observed_qpc_ticks -le 0 -or
            [int]$event.virtual_key -le 0 -or [int]$event.virtual_key -gt 0xffff) {
            throw ('Scheduler key metadata is missing or invalid at record ' + $index)
        }
        $expectedKind = switch ([int]$event.message) {
            { $_ -in @(0x0100, 0x0104) } { 'key_press'; break }
            { $_ -in @(0x0101, 0x0105) } { 'key_release'; break }
            default { $null }
        }
        if (-not $expectedKind -or $event.kind -ne $expectedKind) {
            throw ('Scheduler message, VK, and direction are inconsistent at record ' + $index)
        }
        $qpc = [long]$event.observed_qpc_ticks
        if ($qpc -lt $previousQpc) { throw ('Scheduler observer event order moved backward at record ' + $index) }
        $previousQpc = $qpc
    }
    if ($events.Count -gt 0 -and
        ($null -eq $Summary.handler_latency_p50_us -or
            $null -eq $Summary.handler_latency_p95_us -or
            $null -eq $Summary.handler_latency_p99_us -or
            $null -eq $Summary.handler_latency_max_us -or
            [long]$Summary.handler_latency_p50_us -gt [long]$Summary.handler_latency_p95_us -or
            [long]$Summary.handler_latency_p95_us -gt [long]$Summary.handler_latency_p99_us -or
            [long]$Summary.handler_latency_p99_us -gt [long]$Summary.handler_latency_max_us)) {
        throw 'Scheduler observer handler-latency distribution is missing or unordered'
    }
}

function Get-Sha256 {
    param([string]$Path)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.IO.File]::ReadAllBytes($Path)
        [System.BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Assert-PathOutsideRepository {
    param([string]$Path, [string]$Label)
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $rootPrefix = [System.IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
    if ([string]::Equals($fullPath.TrimEnd('\'), [System.IO.Path]::GetFullPath($root).TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase) -or
        $fullPath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw ($Label + ' must be outside the source checkout')
    }
}

function Assert-NoReparsePointInPilotPath {
    param([Parameter(Mandatory)] [string]$Path, [Parameter(Mandatory)] [string]$Label)
    $current = [System.IO.Path]::GetFullPath($Path)
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw ($Label + ' contains a reparse point / symlink at ' + $current)
            }
        }
        $parent = Split-Path -Path $current -Parent
        if ([string]::IsNullOrWhiteSpace($parent) -or $parent -eq $current) { break }
        $current = $parent
    }
}

function Assert-ChordWidthPilotV1ExecutionEntryGate {
    # Fail-closed pilot execution entry gate. Hash agreement is integrity binding
    # to the reviewed source/build/preflight artifacts, not proof of an
    # independently trusted authority.
    if ([string]::IsNullOrWhiteSpace($ApprovedPreflightPlanPath) -or
        [string]::IsNullOrWhiteSpace($ExpectedApprovedSourceRevision) -or
        [string]::IsNullOrWhiteSpace($ExpectedApprovedSourceTree) -or
        [string]::IsNullOrWhiteSpace($ExpectedApprovedManifestSha256) -or
        [string]::IsNullOrWhiteSpace($ExpectedApprovedBinarySha256) -or
        [string]::IsNullOrWhiteSpace($ExpectedApprovedPreflightPlanSha256)) {
        throw 'ChordWidthPilotV1 execution entry gate requires -ApprovedPreflightPlanPath and all five independently supplied -ExpectedApproved* hashes (SourceRevision, SourceTree, ManifestSha256, BinarySha256, PreflightPlanSha256)'
    }
    if ($ExpectedApprovedSourceRevision -notmatch '^[0-9a-f]{40}$' -or
        $ExpectedApprovedSourceTree -notmatch '^[0-9a-f]{40}$' -or
        $ExpectedApprovedManifestSha256 -notmatch '^[0-9a-f]{64}$' -or
        $ExpectedApprovedBinarySha256 -notmatch '^[0-9a-f]{64}$' -or
        $ExpectedApprovedPreflightPlanSha256 -notmatch '^[0-9a-f]{64}$') {
        throw 'ChordWidthPilotV1 execution entry gate expected hashes must be exact lowercase hex (40-hex SHA-1 / 64-hex SHA-256)'
    }
    $planFull = [System.IO.Path]::GetFullPath($ApprovedPreflightPlanPath)
    Assert-PathOutsideRepository $planFull 'approved preflight plan'
    Assert-NoReparsePointInPilotPath -Path $planFull -Label 'approved preflight plan'
    if (-not (Test-Path -LiteralPath $planFull -PathType Leaf)) {
        throw ('ChordWidthPilotV1 approved preflight plan missing at ' + $planFull)
    }
    $actualPlanSha = Get-Sha256 $planFull
    if ($actualPlanSha -ne $ExpectedApprovedPreflightPlanSha256) {
        throw ('ChordWidthPilotV1 approved preflight plan SHA-256 mismatch (' + $actualPlanSha + ' != ' + $ExpectedApprovedPreflightPlanSha256 + ')')
    }
    if ($head -ne $ExpectedApprovedSourceRevision -or
        $sourceTree -ne $ExpectedApprovedSourceTree -or
        $harnessManifestHash -ne $ExpectedApprovedManifestSha256 -or
        $harnessManifest.binary_sha256 -ne $ExpectedApprovedBinarySha256) {
        throw 'ChordWidthPilotV1 current source, tree, manifest, or binary identity does not match independently supplied approved values'
    }
    if ($harnessManifest.build_provenance -ne 'fresh_pinned_dist_build') {
        throw ('ChordWidthPilotV1 execution entry gate rejects non-fresh manifest build_provenance=' + $harnessManifest.build_provenance)
    }
    $plan = Get-Content -LiteralPath $planFull -Raw | ConvertFrom-Json
    if ($plan.schema_version -ne 1 -or
        $plan.qualification_profile -ne 'ChordWidthPilotV1' -or
        $plan.pilot_identity -ne 'ChordWidthPilotV1' -or
        $plan.mode -ne 'PreflightOnly' -or
        $plan.native_build_status -ne 'BUILT_NONPHYSICAL_DIST' -or
        $plan.native_manifest_status -ne 'VERIFIED_FRESH_PINNED_BUILD' -or
        $plan.build_provenance -ne 'fresh_pinned_dist_build' -or
        -not [bool]$plan.binary_provenance_verified_by_fresh_build -or
        $plan.cargo_build_exit_code -ne 0 -or
        [string]::IsNullOrWhiteSpace([string]$plan.cargo_build_command) -or
        $plan.source_revision -ne $head -or
        $plan.source_tree -ne $sourceTree -or
        $plan.branch -ne 'refactor/completion-anchored-pulse-engine' -or
        $plan.remote_tip_revision -ne $pilotRemoteTip -or
        $plan.frozen_baseline_revision -ne 'e7293877a6ce58a387cb8edfe0a88399a85d2b15' -or
        $plan.cargo_lock_sha256 -ne $pilotActualLockSha -or
        $plan.rustc_version -ne $rustcVersion.Trim() -or
        $plan.cargo_version -ne $pilotCargoVer -or
        $plan.sink_script_sha256 -ne $pilotSinkSha -or
        $plan.analyzer_script_sha256 -ne $pilotAnalyzerSha -or
        $plan.case_runner_script_sha256 -ne $pilotCaseRunnerSha -or
        $plan.preflight_script_sha256 -ne $pilotPrepareSha -or
        $plan.manifest_sha256 -ne $harnessManifestHash -or
        $plan.binary_sha256 -ne $harnessManifest.binary_sha256 -or
        [bool]$plan.receiver_started -or
        [bool]$plan.harness_started -or
        -not [bool]$plan.no_native_invocation -or
        $plan.sendinput_calls -ne 0) {
        throw 'ChordWidthPilotV1 approved preflight plan provenance, fresh-build status, source/remote/lock/toolchain/script/manifest/binary hashes, or nonphysical markers failed validation'
    }
    if ($null -eq $plan.receiver_contract -or
        $plan.receiver_contract.role -ne 'ReceiveOnly' -or
        [bool]$plan.receiver_contract.game_target -or
        $plan.receiver_contract.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
        $plan.receiver_contract.observer_version -ne 'scheduler-receiveonly-queue-v3-compat3' -or
        $plan.receiver_contract.event_schema_version -ne 3 -or
        $plan.receiver_contract.observer_queue_capacity -ne 32768) {
        throw 'ChordWidthPilotV1 approved preflight plan receiver contract mismatch'
    }
    $expectedWidthOrder = @(15, 1, 5)
    $plannedSessions = @($plan.planned_sessions)
    $preflightResults = @($plan.case_runner_preflight_results)
    if ($plan.planned_smoke_session_count -ne 3 -or $plannedSessions.Count -ne 3 -or $preflightResults.Count -ne 3) {
        throw 'ChordWidthPilotV1 approved preflight plan must contain exactly 3 planned sessions and 3 passing case-runner preflight results'
    }
    for ($i = 0; $i -lt 3; $i++) {
        $s = $plannedSessions[$i]
        $r = $preflightResults[$i]
        $wantW = $expectedWidthOrder[$i]
        if ([int]$s.pilot_chord_width -ne $wantW -or
            $s.qualification_profile -ne 'ChordWidthPilotV1' -or
            $s.scenario -ne 'chord-width-pilot-v1' -or
            $s.workload_definition_identity -ne 'chord-width-pilot-v1' -or
            $s.admission_variant -ne 'control-a' -or
            $s.calibration_placement -ne 'baseline' -or
            $s.load_condition -ne 'Quiet' -or
            $s.game_fps -ne 60 -or
            $s.timing_margin_us -ne 500 -or
            $s.configured_hold_us -ne 17167 -or
            $s.cadence_us -ne 100000 -or
            $s.onset_count -ne 150 -or
            $s.sample_seconds -ne 15 -or
            $s.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
            $s.event_schema_version -ne 3 -or
            -not [bool]$s.timing_observer_for_qualification) {
            throw ('ChordWidthPilotV1 approved preflight plan session ' + ($i + 1) + ' geometry or width order (expected 15->1->5) is invalid')
        }
        if ([int]$r.pilot_chord_width -ne $wantW -or
            $r.status -ne 'CASE_PREFLIGHT_ONLY PASS' -or
            [bool]$r.output_created -or
            [bool]$r.receiver_started -or
            [bool]$r.harness_started) {
            throw ('ChordWidthPilotV1 approved preflight plan case-runner preflight result ' + ($i + 1) + ' is missing or invalid')
        }
    }
    $actualPlanSha
}

try {
    if ($isPlaybackCoreLifecycle -and -not $OwnerApprovedReceiveOnlyRun -and -not $PreflightOnly) {
        throw 'PlaybackCoreLifecycle native case requires -OwnerApprovedReceiveOnlyRun after explicit owner approval of this Windows machine and project-owned receiver'
    }
    if ($isChordWidthPilotV1 -and -not $OwnerApprovedReceiveOnlyRun -and -not $PreflightOnly) {
        throw 'ChordWidthPilotV1 native case requires -OwnerApprovedReceiveOnlyRun after explicit owner approval of this Windows machine and project-owned receiver'
    }
    if (-not $PreflightOnly -and -not [Environment]::UserInteractive) {
        throw 'Windows is not interactive; refusing physical acceptance'
    }
    if ([bool]$HarnessPath -ne [bool]$HarnessManifestPath) {
        throw 'immutable harness runs require both -HarnessPath and -HarnessManifestPath'
    }
    if ($QualificationProfile -eq 'FrameSnapshot' -and -not $HarnessManifestPath) {
        throw 'FrameSnapshot acceptance requires its immutable role-specific manifest and binary path'
    }
    if ($SampleSeconds -and $Scenario -notin @('canonical-max-chord', 'dense-alternating', 'near-minimum-retrigger', 'mixed-up-down', 'long-idle', 'hol-interleaved', 'chord-width-pilot-v1')) {
        throw '-SampleSeconds is valid only for canonical-max-chord, dense-alternating, near-minimum-retrigger, mixed-up-down, long-idle, hol-interleaved, and chord-width-pilot-v1'
    }
    if ($QpcReadCountingEvidenceOnly -and (-not $HarnessManifestPath -or $Scenario -ne 'canonical-single' -or $SampleSeconds -or -not $TimingObserverForQualification)) {
        throw 'QPC counting evidence requires an immutable counted harness, one canonical-single ReceiveOnly case, no timed sample, and the qualification observer'
    }
    if ($HarnessPath) {
        Assert-PathOutsideRepository $harness 'immutable harness binary'
        Assert-PathOutsideRepository $HarnessManifestPath 'immutable harness manifest'
    }
    if ($OutputRoot) { Assert-PathOutsideRepository ([System.IO.Path]::GetFullPath($OutputRoot)) 'native evidence output root' }
    if (-not (Test-Path -LiteralPath $harness -PathType Leaf)) {
        throw 'feature-gated release acceptance harness is missing; build it before the run'
    }
    $head = (& git -C $root rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source revision' }
    $sourceTree = (& git -C $root rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source tree identity' }
    $treeChanges = (& git -C $root status --porcelain --untracked-files=all) -join ''
    if ($LASTEXITCODE -ne 0) { throw 'could not verify source-tree cleanliness' }
    $sourceTreeClean = [string]::IsNullOrWhiteSpace($treeChanges)
    $c1TimedVerified = $false
    if ($HarnessManifestPath) {
        if (-not $sourceTreeClean) { throw 'immutable harness acceptance requires a clean source checkout' }
        $harnessManifest = (Get-Content -LiteralPath $HarnessManifestPath -Raw) | ConvertFrom-Json
        if ($isDispatchCleanupM3) {
            $expectedRole = [string]$harnessManifest.source_role
            if ($expectedRole -notin @('Baseline', 'Candidate')) { throw 'DispatchCleanupM3 manifest source role is invalid' }
            Assert-DispatchCleanupM3ManifestIdentity -Manifest $harnessManifest -ExpectedRole $expectedRole -RepositoryRoot $root | Out-Null
            $runnerCheckoutIdentity = Get-DispatchCleanupM3RunnerIdentity -RunnerRoot $root
            if ($runnerCheckoutIdentity.revision -ne $head -or $runnerCheckoutIdentity.tree -ne $sourceTree) {
                throw 'DispatchCleanupM3 runner identity changed during native case preflight'
            }
            Assert-DispatchCleanupM3RunnerProvenance -Manifest $harnessManifest -RunnerIdentity $runnerCheckoutIdentity -RunnerRoot $root | Out-Null
            $manifestSourceRoot = [System.IO.Path]::GetFullPath([string]$harnessManifest.source_root)
            if (-not (Test-DispatchM3PathOutside -Path $evidenceRoot -RepositoryRoot $manifestSourceRoot)) {
                throw 'DispatchCleanupM3 native evidence output must be outside the immutable source checkout'
            }
            $manifestSourceCheckoutIdentity = Assert-DispatchCleanupM3SourceCheckout -SourceRoot $manifestSourceRoot -SourceRole $expectedRole -RunnerRoot $root
            if ($manifestSourceCheckoutIdentity.revision -ne $harnessManifest.source_revision -or
                $manifestSourceCheckoutIdentity.tree -ne $harnessManifest.source_tree_sha -or
                $manifestSourceCheckoutIdentity.checkout.git_dir -ne $harnessManifest.source_checkout_identity.git_dir -or
                $manifestSourceCheckoutIdentity.checkout.git_common_dir -ne $harnessManifest.source_checkout_identity.git_common_dir) {
                throw 'DispatchCleanupM3 manifest source identity differs from the exact detached worktree'
            }
            Assert-ImmutableHarnessManifestIntegrity -Manifest $harnessManifest -ManifestPath $HarnessManifestPath | Out-Null
        } elseif ($isSessionRedesignProfile) {
            $expectedRole = [string]$harnessManifest.source_role
            if ($expectedRole -notin @('Baseline', 'Candidate')) { throw 'DispatchSessionRedesignM3 manifest source role is invalid' }
            Assert-DispatchSessionRedesignM3ManifestIdentity -Manifest $harnessManifest -ExpectedRole $expectedRole -RepositoryRoot $root | Out-Null
            $runnerCheckoutIdentity = Get-DispatchSessionRedesignM3RunnerIdentity -RunnerRoot $root
            if ($runnerCheckoutIdentity.revision -ne $head -or $runnerCheckoutIdentity.tree -ne $sourceTree) {
                throw 'DispatchSessionRedesignM3 runner identity changed during native case preflight'
            }
            Assert-DispatchSessionRedesignM3RunnerProvenance -Manifest $harnessManifest -RunnerIdentity $runnerCheckoutIdentity -RunnerRoot $root | Out-Null
            $manifestSourceRoot = [System.IO.Path]::GetFullPath([string]$harnessManifest.source_root)
            if (-not (Test-DispatchM3PathOutside -Path $evidenceRoot -RepositoryRoot $manifestSourceRoot)) {
                throw 'DispatchSessionRedesignM3 native evidence output must be outside the immutable source checkout'
            }
            $manifestSourceCheckoutIdentity = Assert-DispatchSessionRedesignM3SourceCheckout -SourceRoot $manifestSourceRoot -SourceRole $expectedRole -RunnerRoot $root
            if ($manifestSourceCheckoutIdentity.revision -ne $harnessManifest.source_revision -or
                $manifestSourceCheckoutIdentity.tree -ne $harnessManifest.source_tree_sha -or
                $manifestSourceCheckoutIdentity.checkout.git_dir -ne $harnessManifest.source_checkout_identity.git_dir -or
                $manifestSourceCheckoutIdentity.checkout.git_common_dir -ne $harnessManifest.source_checkout_identity.git_common_dir) {
                throw 'DispatchSessionRedesignM3 manifest source identity differs from the exact detached worktree'
            }
            Assert-ImmutableHarnessManifestIntegrity -Manifest $harnessManifest -ManifestPath $HarnessManifestPath | Out-Null
        } elseif ($isPlaybackCoreLifecycle) {
            $expectedRole = [string]$harnessManifest.source_role
            if ($expectedRole -notin @('Baseline', 'Candidate')) { throw 'PlaybackCoreLifecycle manifest source role is invalid' }
            Assert-PlaybackCoreLifecycleManifestIdentity -Manifest $harnessManifest -ExpectedRole $expectedRole -RepositoryRoot $root | Out-Null
            $runnerCheckoutIdentity = Get-PlaybackCoreLifecycleRunnerIdentity -RunnerRoot $root -ProfileVersion $playbackCoreProfile.Version
            if ($runnerCheckoutIdentity.revision -ne $head -or $runnerCheckoutIdentity.tree -ne $sourceTree) {
                throw 'PlaybackCoreLifecycle runner identity changed during native case preflight'
            }
            Assert-PlaybackCoreLifecycleRunnerProvenance -Manifest $harnessManifest -RunnerIdentity $runnerCheckoutIdentity -RunnerRoot $root | Out-Null
            $manifestSourceRoot = [System.IO.Path]::GetFullPath([string]$harnessManifest.source_root)
            if (-not (Test-DispatchM3PathOutside -Path $evidenceRoot -RepositoryRoot $manifestSourceRoot)) {
                throw 'PlaybackCoreLifecycle native evidence output must be outside the immutable source checkout'
            }
            $manifestSourceIdentity = Assert-PlaybackCoreLifecycleSourceCheckout -SourceRoot $manifestSourceRoot -SourceRole $expectedRole -RunnerRoot $root -ProfileVersion $playbackCoreProfile.Version
            if ($manifestSourceIdentity.revision -ne $harnessManifest.source_revision -or
                $manifestSourceIdentity.tree -ne $harnessManifest.source_tree_sha -or
                $manifestSourceIdentity.checkout.git_dir -ne [string]$harnessManifest.source_checkout_identity.git_dir -or
                $manifestSourceIdentity.checkout.git_common_dir -ne [string]$harnessManifest.source_checkout_identity.git_common_dir) {
                throw 'PlaybackCoreLifecycle manifest source identity differs from its exact source checkout'
            }
            Assert-ImmutableHarnessManifestIntegrity -Manifest $harnessManifest -ManifestPath $HarnessManifestPath | Out-Null
        } elseif ($isChordWidthPilotV1) {
            $manifestSourceRoot = [System.IO.Path]::GetFullPath([string]$harnessManifest.source_root)
            if ($harnessManifest.qualification_profile -ne 'ChordWidthPilotV1' -or
                $harnessManifest.source_role -ne 'Candidate' -or
                -not [bool]$harnessManifest.source_tree_clean -or
                $harnessManifest.base_source_revision -ne 'e7293877a6ce58a387cb8edfe0a88399a85d2b15' -or
                $harnessManifest.base_source_tree_sha -ne '1e2aa8c1a6b1d48a46cd419d89536a1051510890' -or
                $harnessManifest.receiveonly_observer_mode -ne 'SchedulerQueuedV3Compat3' -or
                $harnessManifest.receiveonly_observer_version -ne 'scheduler-receiveonly-queue-v3-compat3' -or
                $harnessManifest.receiveonly_observer_event_schema_version -ne 3 -or
                -not (Test-Path -LiteralPath (Join-Path $manifestSourceRoot 'rust\Cargo.toml') -PathType Leaf)) {
                throw 'ChordWidthPilotV1 manifest role, pinned base, observer protocol, source cleanliness, or source root is invalid'
            }
            $manifestSourceRevision = (& git -C $manifestSourceRoot rev-parse HEAD).Trim()
            if ($LASTEXITCODE -ne 0) { throw 'ChordWidthPilotV1 manifest source root is not a Git checkout' }
            $manifestSourceTree = (& git -C $manifestSourceRoot rev-parse 'HEAD^{tree}').Trim()
            if ($LASTEXITCODE -ne 0) { throw 'ChordWidthPilotV1 manifest source tree could not be resolved' }
            $manifestSourceBranch = ((& git -C $manifestSourceRoot branch --show-current) -join '').Trim()
            if ($LASTEXITCODE -ne 0) { throw 'ChordWidthPilotV1 manifest source branch could not be resolved' }
            $manifestSourceChanges = (& git -C $manifestSourceRoot status --porcelain --untracked-files=all) -join ''
            if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace($manifestSourceChanges) -or
                -not [string]::Equals($manifestSourceRoot, $root, [System.StringComparison]::OrdinalIgnoreCase) -or
                $manifestSourceRevision -ne $head -or $manifestSourceRevision -ne $harnessManifest.source_revision -or
                $manifestSourceTree -ne $sourceTree -or $manifestSourceTree -ne $harnessManifest.source_tree_sha -or
                $manifestSourceBranch -ne 'refactor/completion-anchored-pulse-engine') {
                throw 'ChordWidthPilotV1 candidate manifest must match the current clean refactor/completion-anchored-pulse-engine checkout'
            }
            & git -C $manifestSourceRoot merge-base --is-ancestor 'e7293877a6ce58a387cb8edfe0a88399a85d2b15' HEAD
            if ($LASTEXITCODE -ne 0) { throw 'ChordWidthPilotV1 candidate does not descend from frozen baseline e7293877a6ce58a387cb8edfe0a88399a85d2b15' }
            $pilotRemoteRef = 'refs/heads/refactor/completion-anchored-pulse-engine'
            $pilotRemoteLines = @(& git -C $manifestSourceRoot ls-remote --heads origin $pilotRemoteRef 2>&1 | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            if ($LASTEXITCODE -ne 0 -or $pilotRemoteLines.Count -ne 1) {
                throw 'ChordWidthPilotV1 could not verify unique origin/refactor/completion-anchored-pulse-engine remote tip'
            }
            $pilotRemoteTip = ($pilotRemoteLines[0].Trim() -split '\s+')[0]
            if ($pilotRemoteTip -ne $head -or $harnessManifest.remote_tip_revision -ne $pilotRemoteTip) {
                throw 'ChordWidthPilotV1 local HEAD or manifest remote_tip_revision does not match pushed origin tip'
            }
            $pilotLockPath = Join-Path $manifestSourceRoot 'rust\Cargo.lock'
            if (-not (Test-Path -LiteralPath $pilotLockPath -PathType Leaf)) { throw 'ChordWidthPilotV1 rust/Cargo.lock is missing' }
            $pilotLockBytes = [System.IO.File]::ReadAllBytes($pilotLockPath)
            if ($pilotLockBytes -contains [byte]13) { throw 'ChordWidthPilotV1 rust/Cargo.lock contains CR bytes' }
            $pilotActualLockSha = Get-Sha256 $pilotLockPath
            if ($pilotActualLockSha -ne '9b9ba6c03131d96029421bfa9f6f69f9ce331e003e32e095eac54a714e73021e' -or
                $harnessManifest.cargo_lock_sha256 -ne $pilotActualLockSha) {
                throw 'ChordWidthPilotV1 rust/Cargo.lock LF SHA-256 does not match frozen expected lock or manifest'
            }
            $pilotCargoVer = ((& cargo --version) -join ' ').Trim()
            if ($LASTEXITCODE -ne 0 -or $pilotCargoVer -notmatch '^cargo 1\.99\.1(?:\s|$)' -or $harnessManifest.cargo_version -ne $pilotCargoVer) {
                throw 'ChordWidthPilotV1 manifest cargo_version mismatch'
            }
            $pilotSinkSha = Get-Sha256 (Join-Path $manifestSourceRoot 'scripts\native_acceptance_sink.ps1')
            $pilotAnalyzerSha = Get-Sha256 (Join-Path $manifestSourceRoot 'scripts\analyze_chord_reception.mjs')
            $pilotCaseRunnerSha = Get-Sha256 (Join-Path $manifestSourceRoot 'scripts\run_native_acceptance_case.ps1')
            $pilotPrepareSha = Get-Sha256 (Join-Path $manifestSourceRoot 'scripts\prepare_chord_width_pilot_v1.ps1')
            if ($harnessManifest.sink_script_sha256 -ne $pilotSinkSha -or
                $harnessManifest.analyzer_script_sha256 -ne $pilotAnalyzerSha -or
                $harnessManifest.case_runner_script_sha256 -ne $pilotCaseRunnerSha -or
                $harnessManifest.preflight_script_sha256 -ne $pilotPrepareSha) {
                throw 'ChordWidthPilotV1 manifest script SHA-256 provenance mismatch'
            }
            $peHeader = New-Object byte[] 2
            $fs = [System.IO.File]::OpenRead($harness)
            try { $readCount = $fs.Read($peHeader, 0, 2) } finally { $fs.Dispose() }
            if ($readCount -lt 2 -or $peHeader[0] -ne 0x4D -or $peHeader[1] -ne 0x5A) {
                throw 'ChordWidthPilotV1 harness binary does not have a valid Windows PE MZ header'
            }
        } elseif ($qualifiedProfile) {
            $manifestSourceRoot = [System.IO.Path]::GetFullPath([string]$harnessManifest.source_root)
            if ($QualificationProfile -eq 'FrameSnapshot') {
                $expectedRole = [string]$harnessManifest.source_role
                if ($expectedRole -notin @('Baseline', 'Candidate')) { throw 'FrameSnapshot manifest source role is invalid' }
                Assert-FrameSnapshotManifestIdentity -Manifest $harnessManifest -ExpectedRole $expectedRole
            }
            if ($harnessManifest.qualification_profile -ne $QualificationProfile -or
                $harnessManifest.source_role -notin @('Baseline', 'Candidate') -or
                -not [bool]$harnessManifest.source_tree_clean -or
                $harnessManifest.base_source_revision -ne $profileBaseRevision -or
                $harnessManifest.base_source_tree_sha -ne $profileBaseTree -or
                -not (Test-Path -LiteralPath (Join-Path $manifestSourceRoot 'rust\Cargo.toml') -PathType Leaf)) {
                if ($QualificationProfile -eq 'Scheduler') {
                    throw 'Scheduler manifest role, pinned base, source cleanliness, or source root is invalid'
                }
                throw 'FrameSnapshot manifest role, pinned base, source cleanliness, or source root is invalid'
            }
            $manifestSourceRevision = (& git -C $manifestSourceRoot rev-parse HEAD).Trim()
            if ($LASTEXITCODE -ne 0) { throw ($QualificationProfile + ' manifest source root is not a Git checkout') }
            $manifestSourceTree = (& git -C $manifestSourceRoot rev-parse 'HEAD^{tree}').Trim()
            if ($LASTEXITCODE -ne 0) { throw ($QualificationProfile + ' manifest source tree could not be resolved') }
            $manifestSourceBranch = ((& git -C $manifestSourceRoot branch --show-current) -join '').Trim()
            if ($LASTEXITCODE -ne 0) { throw ($QualificationProfile + ' manifest source branch could not be resolved') }
            $manifestSourceChanges = (& git -C $manifestSourceRoot status --porcelain --untracked-files=all) -join ''
            if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace($manifestSourceChanges) -or
                $manifestSourceRevision -ne $harnessManifest.source_revision -or
                $manifestSourceTree -ne $harnessManifest.source_tree_sha) {
                throw ($QualificationProfile + ' manifest SHA/tree does not match a clean source checkout')
            }
            if ($harnessManifest.source_role -eq 'Candidate') {
                if (-not [string]::Equals($manifestSourceRoot, $root, [System.StringComparison]::OrdinalIgnoreCase) -or $manifestSourceRevision -ne $head -or
                    $manifestSourceTree -ne $sourceTree -or $manifestSourceBranch -ne $profileBranch) {
                    throw ($QualificationProfile + ' candidate manifest must match the current clean candidate branch')
                }
                & git -C $manifestSourceRoot merge-base --is-ancestor $profileBaseRevision HEAD
                if ($LASTEXITCODE -ne 0) { throw ($QualificationProfile + ' candidate does not descend from the pinned base') }
            } elseif ($manifestSourceRevision -ne $profileBaseRevision -or
                $manifestSourceTree -ne $profileBaseTree -or
                -not [string]::IsNullOrWhiteSpace($manifestSourceBranch)) {
                throw ($QualificationProfile + ' baseline must be a detached clean checkout at the pinned base SHA/tree')
            }
            if ($QualificationProfile -in @('Scheduler', 'FrameSnapshot')) {
                Assert-ImmutableHarnessManifestIntegrity -Manifest $harnessManifest -ManifestPath $HarnessManifestPath | Out-Null
            }
        } else {
            $c1TimedVerified = Assert-C1TimedControlProvenance -Manifest $harnessManifest -RepositoryRoot $root -Role 'case-runner' -AllowHistoricalControl:$HistoricalSourceAllowed
        }
        $harnessManifestHash = Get-Sha256 $HarnessManifestPath
        $manifestFeatures = @($harnessManifest.cargo_features)
        $expectedManifestFeatures = if ($QpcReadCountingEvidenceOnly) { @('real-input-acceptance', 'qpc-read-counting') } else { @('real-input-acceptance') }
        $manifestBinaryPath = [System.IO.Path]::GetFullPath([string]$harnessManifest.binary_path)
        $expectedObserverMode = if ($TimingObserverForQualification) { 'qualification-on' } else { 'off' }
        $rustcVersion = (& rustc -Vv) -join [Environment]::NewLine
        if ($LASTEXITCODE -ne 0) { throw 'could not identify active rustc version' }
        $sourceRevisionMatches = $harnessManifest.source_revision -eq $head -and $harnessManifest.source_tree_sha -eq $sourceTree
        if ($harnessManifest.guard_mode -eq 'adaptive') {
            throw 'adaptive guard mode is retired (#646); only frozen manifests are accepted for qualification'
        }
        if ($harnessManifest.schema_version -ne $expectedHarnessManifestSchemaVersion) {
            throw ($QualificationProfile + ' case runner requires harness manifest schema ' + $expectedHarnessManifestSchemaVersion + ', got ' + $harnessManifest.schema_version)
        }
        if ($harnessManifest.qualification_profile -ne $QualificationProfile) {
            throw ($QualificationProfile + ' case runner cannot consume a ' + $harnessManifest.qualification_profile + ' harness manifest')
        }
        $expectedSourceBranch = if ($isChordWidthPilotV1) { 'refactor/completion-anchored-pulse-engine' } elseif (($qualifiedProfile -and $harnessManifest.source_role -eq 'Baseline') -or ($usesSessionQualificationContracts -and -not $isPlaybackCoreLifecycle)) { '' } elseif ($QualificationProfile -eq 'FrameSnapshot') { 'refactor/frame-snapshot-dispatch' } elseif ($QualificationProfile -eq 'Scheduler') { 'refactor/scheduler-single-owner-20261112' } elseif ($isPlaybackCoreLifecycle) { $playbackCoreProfile.CandidateLineageBranch } else { 'perf/dispatch-core-20261109' }
        $expectedWorkloadVersion = if ($isChordWidthPilotV1) { 'chord-width-pilot-v1' } elseif ($isDispatchCleanupM3) { $dispatchCleanupM3.WorkloadVersion } elseif ($isSessionRedesignProfile) { $sessionRedesignProfile.WorkloadVersion } elseif ($isPlaybackCoreLifecycle) { $playbackCoreProfile.WorkloadVersion } elseif ($QualificationProfile -eq 'FrameSnapshot') { 'frame-snapshot-v1' } elseif ($QualificationProfile -eq 'Scheduler') { 'scheduler-single-owner-v1' } else { 'dispatch-core-646-a1' }
        $sourceIdentityAllowed = if ($isDispatchCleanupM3) {
            $true
        } elseif ($qualifiedProfile) {
            $harnessManifest.source_role -eq 'Baseline' -or $sourceRevisionMatches
        } else {
            $HistoricalSourceAllowed -or $sourceRevisionMatches
        }
        if (-not $sourceIdentityAllowed -or
            $harnessManifest.source_revision -notmatch '^[0-9a-f]{40}$' -or
            $harnessManifest.source_tree_sha -notmatch '^[0-9a-f]{40}$' -or
            $harnessManifest.source_branch -ne $expectedSourceBranch -or
            ([bool]$harnessManifest.source_tree_clean -ne (-not $c1TimedVerified)) -or
            -not [bool]$harnessManifest.clean_build -or
            $harnessManifest.cargo_profile -ne 'dist' -or
            (($manifestFeatures -join ',') -ne ($expectedManifestFeatures -join ',')) -or
            ([bool]$harnessManifest.qpc_read_counting -ne [bool]$QpcReadCountingEvidenceOnly) -or
            $harnessManifest.observer_mode -ne $expectedObserverMode -or
            $harnessManifest.guard_mode -ne 'frozen' -or
            $harnessManifest.workload_version -ne $expectedWorkloadVersion -or
            $harnessManifest.target -ne 'x86_64-pc-windows-msvc' -or
            $harnessManifest.rustc_version -ne $rustcVersion.Trim() -or
            -not [string]::Equals($manifestBinaryPath, $harness, [System.StringComparison]::OrdinalIgnoreCase) -or
            $harnessManifest.binary_name -ne 'rt-native-acceptance.exe' -or
            $harnessManifest.binary_sha256 -ne (Get-Sha256 $harness)) {
            throw 'immutable harness manifest source, profile, features, path, name, or SHA-256 mismatch'
        }
        if ($QualificationProfile -eq 'FrameSnapshot') {
            $expectedRole = [string]$harnessManifest.source_role
            Assert-FrameSnapshotManifestIdentity -Manifest $harnessManifest -ExpectedRole $expectedRole
        }
        if ($isDispatchCleanupM3) {
            if ([string]::IsNullOrWhiteSpace($Workload)) { throw 'DispatchCleanupM3 native cases require an explicit workload identity' }
            Assert-DispatchCleanupM3SessionIdentity -Scenario $Scenario -Workload $Workload -TimingMarginUs $TimingMarginUs -GameFps $GameFps -SampleSeconds $SampleSeconds | Out-Null
        } elseif ($isSessionRedesignProfile) {
            if ([string]::IsNullOrWhiteSpace($Workload)) { throw 'DispatchSessionRedesignM3 native cases require an explicit workload identity' }
            Assert-DispatchSessionRedesignM3SessionIdentity -Scenario $Scenario -Workload $Workload `
                -TimingMarginUs $TimingMarginUs -GameFps $GameFps -SampleSeconds $SampleSeconds | Out-Null
        } elseif ($isPlaybackCoreLifecycle) {
            if ([string]::IsNullOrWhiteSpace($Workload)) { throw 'PlaybackCoreLifecycle native cases require an explicit workload identity' }
            Assert-PlaybackCoreLifecycleSessionIdentity -Scenario $Scenario -Workload $Workload `
                -TimingMarginUs $TimingMarginUs -GameFps $GameFps -SampleSeconds $SampleSeconds `
                -ProfileVersion $playbackCoreProfile.Version | Out-Null
        }
    }
    if ($PreflightOnly) {
        if (Test-Path -LiteralPath $evidenceRoot) {
            throw '-PreflightOnly requires a nonexistent output root so it cannot create or alter case evidence'
        }
        $pilotGateStatus = ''
        if ($isChordWidthPilotV1 -and (
            -not [string]::IsNullOrWhiteSpace($ApprovedPreflightPlanPath) -or
            -not [string]::IsNullOrWhiteSpace($ExpectedApprovedSourceRevision) -or
            -not [string]::IsNullOrWhiteSpace($ExpectedApprovedSourceTree) -or
            -not [string]::IsNullOrWhiteSpace($ExpectedApprovedManifestSha256) -or
            -not [string]::IsNullOrWhiteSpace($ExpectedApprovedBinarySha256) -or
            -not [string]::IsNullOrWhiteSpace($ExpectedApprovedPreflightPlanSha256))) {
            $verifiedPlanSha = Assert-ChordWidthPilotV1ExecutionEntryGate
            $pilotGateStatus = ' entry_gate_binding=VERIFIED plan_sha256=' + $verifiedPlanSha
        }
        $pilotExtra = if ($isChordWidthPilotV1) { ' pilot_chord_width=' + $PilotChordWidth + ' observer_mode=SchedulerQueuedV3Compat3 event_schema=3 manifest_hash_verified_only=true manifest_build_provenance=' + [string]$harnessManifest.build_provenance + $pilotGateStatus } else { '' }
        Write-Output ('CASE_PREFLIGHT_ONLY PASS profile=' + $QualificationProfile + ' role=' + $harnessManifest.source_role +
            ' schema=' + $harnessManifest.schema_version + ' scenario=' + $Scenario + ' workload=' + $Workload + $pilotExtra +
            ' manifest_sha256=' + $harnessManifestHash + ' binary_sha256=' + $harnessManifest.binary_sha256 +
            ' receiver_started=false harness_started=false output_created=false')
        $resultCode = 0
        return
    }
    if ($isChordWidthPilotV1) {
        $verifiedPlanSha = Assert-ChordWidthPilotV1ExecutionEntryGate
        $pilotEntryGateVerified = $true
    }
    New-Item -ItemType Directory -Force -Path $runDir | Out-Null
    $sessionId = (Get-Process -Id $PID).SessionId
    $sinkArgv = @('-NoProfile', '-File', $sinkScript, '-Mode', 'ReceiveOnly', '-RunId', $runId, '-ReadyFile', $readyPath, '-EventLog', $eventsPath, '-ActivateRequestFile', $focusRestoreRequestPath)
    if ($receiveOnlyObserverVersion) {
        $sinkArgv += @('-ObserverMode', 'SchedulerQueuedV3Compat3', '-ObserverSummaryFile', $sinkObserverSummaryPath)
    }
    $sinkProcess = Start-Process -FilePath $pwshPath -ArgumentList $sinkArgv -PassThru -WindowStyle Normal -RedirectStandardOutput (Join-Path $runDir 'sink.stdout.log') -RedirectStandardError (Join-Path $runDir 'sink.stderr.log')
    $sink = Wait-ReadyRecord $readyPath $sinkProcess
    Assert-SinkIdentity $sink $sessionId $receiveOnlyObserverVersion
    $foregroundPermissionGranted = [NativeAcceptanceForegroundPermission]::AllowSetForegroundWindow([uint32]$sink.pid)

    if ($Scenario -eq 'focus-loss') {
        $probeArgv = @('-NoProfile', '-File', $sinkScript, '-Mode', 'InertFocusProbe', '-RunId', $runId, '-ReadyFile', $probeReadyPath, '-EventLog', $probeEventsPath)
        $probeProcess = Start-Process -FilePath $pwshPath -ArgumentList $probeArgv -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $runDir 'focus-probe.stdout.log') -RedirectStandardError (Join-Path $runDir 'focus-probe.stderr.log')
        $probe = Wait-ReadyRecord $probeReadyPath $probeProcess
        Write-Output 'The visible inert probe does not activate on show. Focus the exact ReceiveOnly sink; the focus-loss control starts in 15 seconds.'
        Start-Sleep -Seconds 15
    }

    if ($CpuContention) {
        $loadCommand = 'while ($true) { [void]([Math]::Sqrt(12345.6789)) }'
        for ($index = 0; $index -lt 2; $index++) {
            $loadProcess = Start-Process -FilePath $pwshPath -ArgumentList @('-NoProfile', '-Command', $loadCommand) -PassThru -WindowStyle Hidden
            $loadProcess.PriorityClass = 'BelowNormal'
            $contentionProcesses += $loadProcess
        }
    }

    $harnessHash = Get-Sha256 $harness
    $runnerHash = Get-Sha256 $PSCommandPath
    $operatingSystem = Get-CimInstance Win32_OperatingSystem
    $processor = Get-CimInstance Win32_Processor | Select-Object -First 1
    $rustVersion = (& rustc --version).Trim()
    $runConfig = [ordered]@{
        run_id = $runId
        owner_approved_receive_only_run = [bool]$OwnerApprovedReceiveOnlyRun
        source_revision = $head
        source_tree_sha = $sourceTree
        source_tree_clean = $sourceTreeClean
        runner_source_revision = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'runner_source_revision' } else { $null }
        runner_source_tree_sha = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'runner_source_tree_sha' } else { $null }
        runner_source_branch = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'runner_source_branch' } else { $null }
        candidate_lineage_branch = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'candidate_lineage_branch' } else { $null }
        candidate_lineage_tip_revision = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'candidate_lineage_tip_revision' } else { $null }
        runner_script_provenance = if ($usesSessionQualificationContracts -and $harnessManifest) { @($harnessManifest.runner_script_provenance) } else { $null }
        scenario = $Scenario
        cpu_contention = [bool]$CpuContention
        cpu_contention_processes = $contentionProcesses.Count
        cpu_contention_method = if ($CpuContention) { 'two hidden pwsh busy workers at BelowNormal priority' } else { $null }
        timing_margin_us = $TimingMarginUs
        qualification_profile = $QualificationProfile
        performance_timing_margin_us = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'performance_timing_margin_us' } else { $null }
        safety_timing_margin_us = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'safety_timing_margin_us' } else { $null }
        game_fps = $GameFps
        sample_seconds = if ($SampleSeconds) { $SampleSeconds } else { $null }
        hol_stall_us = $HolStallUs
        hol_cycles = $HolCycles
        expected_hold_us = if ($usesSessionQualificationContracts) { [long]$sessionTimingFloors.expected_hold_us } else { [long][Math]::Ceiling(1000000.0 / $GameFps) * 2 + $TimingMarginUs }
        profile = 'production'
        cargo_profile = if ($harnessManifest) { $harnessManifest.cargo_profile } else { 'legacy-checkout-release' }
        cargo_features = if ($harnessManifest) { @($harnessManifest.cargo_features) } else { @() }
        cargo_lock_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'cargo_lock_sha256' } else { $null }
        harness_source_revision = if ($harnessManifest) { $harnessManifest.source_revision } else { $null }
        harness_source_tree_sha = if ($harnessManifest) { $harnessManifest.source_tree_sha } else { $null }
        harness_control_kind = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'control_kind' } else { $null }
        harness_control_label = if ($c1TimedVerified) { 'C1-TIMED' } elseif ($QualificationProfile -eq 'FrameSnapshot') { if ($harnessManifest.source_role -eq 'Baseline') { 'FrameSnapshot-Baseline' } else { 'FrameSnapshot-Candidate' } } elseif ($QualificationProfile -eq 'DispatchCleanupM3') { if ($harnessManifest.source_role -eq 'Baseline') { 'DispatchCleanupM3-Baseline' } else { 'DispatchCleanupM3-Candidate' } } elseif ($QualificationProfile -eq 'DispatchSessionRedesignM3') { if ($harnessManifest.source_role -eq 'Baseline') { 'DispatchSessionRedesignM3-Baseline' } else { 'DispatchSessionRedesignM3-Candidate' } } elseif ($isPlaybackCoreLifecycle) { if ($harnessManifest.source_role -eq 'Baseline') { $playbackCoreProfile.Name + '-Baseline' } else { $playbackCoreProfile.Name + '-Candidate' } } elseif ($QualificationProfile -eq 'Scheduler') { if ($harnessManifest.source_role -eq 'Baseline') { 'Scheduler-Baseline' } else { 'Scheduler-Candidate' } } elseif ($HistoricalSourceAllowed) { 'C0' } else { 'A' }
        harness_base_source_revision = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_revision' } else { $null }
        harness_base_source_tree_sha = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_tree_sha' } else { $null }
        harness_base_source_archive_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_archive_sha256' } else { $null }
        harness_base_source_file_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_file_sha256' } else { $null }
        harness_overlay_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'overlay_sha256' } else { $null }
        harness_derived_source_file_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'derived_source_file_sha256' } else { $null }
        harness_source_changes_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'source_changes_sha256' } else { $null }
        harness_derived_source_inventory_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'derived_source_inventory_sha256' } else { $null }
        qpc_read_counting = [bool]($harnessManifest -and $harnessManifest.qpc_read_counting)
        observer_mode = if ($TimingObserverForQualification) { 'qualification-on' } else { 'off' }
        guard_mode = if ($harnessManifest) { $harnessManifest.guard_mode } else { 'unknown' }
        workload_version = if ($harnessManifest) { $harnessManifest.workload_version } else { 'legacy' }
        telemetry_mode = 'ring'
        telemetry_capacity = 1024
        timing_observer_for_qualification = [bool]$TimingObserverForQualification
        waitable_timer_enabled = $true
        event_wait_enabled = $true
        priority_mode = 'auto'
        harness_path = [System.IO.Path]::GetFileName($harness)
        harness_sha256 = $harnessHash
        harness_manifest_path = if ($HarnessManifestPath) { '<immutable-manifest>' } else { $null }
        harness_manifest_sha256 = $harnessManifestHash
        runner_sha256 = $runnerHash
        rust_version = $rustVersion
        windows_caption = $operatingSystem.Caption
        windows_version = $operatingSystem.Version
        windows_build = $operatingSystem.BuildNumber
        processor_name = $processor.Name
        processor_cores = $processor.NumberOfCores
        processor_logical_processors = $processor.NumberOfLogicalProcessors
        power_scheme = (& powercfg /getactivescheme | Out-String).Trim()
        sink_ready_path = 'sink-ready.json'
        sink_event_path = 'sink-events.jsonl'
        sink_observer_summary_path = if ($receiveOnlyObserverVersion) { 'sink-observer-summary.json' } else { $null }
        receiveonly_observer_mode = if ($receiveOnlyObserverVersion) { 'SchedulerQueuedV3Compat3' } else { 'LegacyInline' }
        receiveonly_observer_version = $receiveOnlyObserverVersion
        sink_pid = [int]$sink.pid
        sink_hwnd = [long]$sink.hwnd
        sink_target_class = [string]$sink.target_class
        sink_target_name = [string]$sink.target_name
        sink_game_target = [bool]$sink.game_target
        sink_event_log_id = $sink.event_log_id
        sink_process_start_time_filetime = [long]$sink.process_start_time_filetime
        sink_process_image_path = $pwshExecutablePath
        foreground_permission_granted_to_sink = [bool]$foregroundPermissionGranted
        admission_variant = $AdmissionVariant
    }
    if ($usesSessionQualificationContracts) {
        $runConfig['workload'] = $Workload
        $runConfig['workload_version'] = $harnessManifest.workload_version
        $runConfig['session_seed'] = $null
        $runConfig['seed_mode'] = 'none; deterministic workload definitions do not accept a PRNG seed'
        $runConfig['configured_H_us'] = [long]$sessionTimingFloors.configured_H_us
    }
    $runConfig['machine_name'] = [Environment]::MachineName
    $runConfig['os_boot_time_utc'] = $operatingSystem.LastBootUpTime.ToUniversalTime().ToString('O')
    $runConfig | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $runConfigPath -Encoding UTF8
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $runDir 'runner.ps1')

    $startedUtc = [DateTimeOffset]::UtcNow.ToString('O')
    $harnessArgs = @(
        'run', '--allow-real-input', '--run-id', $runId,
        '--sink-ready', $readyPath, '--sink-events', $eventsPath,
        '--target-hwnd', ([long]$sink.hwnd).ToString([System.Globalization.CultureInfo]::InvariantCulture),
        '--scenario', $Scenario, '--evidence', $reportPath,
        '--timing-margin-us', [string]$TimingMarginUs,
        '--game-fps', [string]$GameFps
    )
    if ($SampleSeconds) { $harnessArgs += @('--sample-seconds', [string]$SampleSeconds) }
    if ($isChordWidthPilotV1) { $harnessArgs += @('--pilot-chord-width', [string]$PilotChordWidth) }
    if ($TimingObserverForQualification) { $harnessArgs += '--timing-observer-for-qualification' }
    if ($HolStallUs -gt 0) {
        $harnessArgs += @('--hol-stall-us', [string]$HolStallUs)
        if ($HolCycles -gt 0) {
            $harnessArgs += @('--hol-cycles', [string]$HolCycles)
        }
    }
    if ($harnessManifest -and $harnessManifest.guard_mode) {
        $harnessArgs += @('--guard-mode', [string]$harnessManifest.guard_mode)
    }
    if ($AdmissionVariant -ne 'control-a') {
        $harnessArgs += @('--admission-variant', $AdmissionVariant)
    }
    if ($Scenario -eq 'focus-loss') {
        $harnessArgs += @('--focus-probe-ready', $probeReadyPath, '--focus-probe-events', $probeEventsPath, '--focus-probe-hwnd', ([long]$probe.hwnd).ToString([System.Globalization.CultureInfo]::InvariantCulture), '--focus-restore-request', $focusRestoreRequestPath)
    }
    if ($harnessManifest -and (Get-Sha256 $HarnessManifestPath) -ne $harnessManifestHash) {
        throw 'immutable harness manifest changed after identity validation'
    }
    if ($isChordWidthPilotV1) {
        if ((Get-Sha256 $harness) -ne $ExpectedApprovedBinarySha256) {
            throw 'ChordWidthPilotV1 approved harness binary changed before child launch'
        }
        if ((Get-Sha256 $ApprovedPreflightPlanPath) -ne $ExpectedApprovedPreflightPlanSha256) {
            throw 'ChordWidthPilotV1 approved preflight plan changed before child launch'
        }
        $preLaunchHead = (& git -C $root rev-parse HEAD).Trim()
        $preLaunchTree = (& git -C $root rev-parse 'HEAD^{tree}').Trim()
        $preLaunchDirty = (& git -C $root status --porcelain --untracked-files=all) -join ''
        if ($LASTEXITCODE -ne 0 -or $preLaunchHead -ne $ExpectedApprovedSourceRevision -or
            $preLaunchTree -ne $ExpectedApprovedSourceTree -or -not [string]::IsNullOrWhiteSpace($preLaunchDirty)) {
            throw 'ChordWidthPilotV1 checkout identity or cleanliness changed before child launch'
        }
    }

    # Signal the sink process to bring itself to the foreground via the existing ActivateRequestFile mechanism
    if ($focusRestoreRequestPath) {
        New-Item -ItemType File -Path $focusRestoreRequestPath -Force | Out-Null
    }

    # Bounded foreground handshake: ensure the exact ReceiveOnly sink window is in the foreground.
    # Background processes on Windows 11 cannot unconditionally steal foreground focus.
    # Verify GetForegroundWindow matches the sink HWND. If not, prompt the operator to click it.
    $targetHwnd = [IntPtr][long]$sink.hwnd
    $foregroundDeadline = (Get-Date).AddSeconds(15)
    $foregroundConfirmed = $false
    while ((Get-Date) -lt $foregroundDeadline) {
        $fg = [NativeAcceptanceForegroundPermission]::GetForegroundWindow()
        if ($fg -eq $targetHwnd) {
            $foregroundConfirmed = $true
            break
        }
        Write-Host "Waiting for ReceiveOnly Sink window (HWND: $targetHwnd) to acquire foreground. Please click it if needed..."
        Start-Sleep -Milliseconds 500
    }
    if (-not $foregroundConfirmed) {
        $lastFg = [NativeAcceptanceForegroundPermission]::GetForegroundWindow()
        throw "Failed to bring ReceiveOnly sink (HWND: $targetHwnd) to foreground within 15 seconds (Current foreground HWND: $lastFg). Failing closed to ensure 0 physical sends."
    }

    $captured = @(& $harness @harnessArgs 2>&1)
    $nativeExit = $LASTEXITCODE
    if ($harnessManifest -and (Get-Sha256 $harness) -ne $harnessManifest.binary_sha256) {
        throw 'immutable harness binary changed during acceptance run'
    }
    if ($harnessManifest -and (Get-Sha256 $HarnessManifestPath) -ne $harnessManifestHash) {
        throw 'immutable harness manifest changed during acceptance run'
    }
    $endedUtc = [DateTimeOffset]::UtcNow.ToString('O')
    $capturedText = ($captured | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
    Set-Content -LiteralPath $stdoutPath -Value $capturedText -Encoding UTF8

    if ($receiveOnlyObserverVersion -and $Scenario -ne 'owner-process-termination') {
        $sinkProcess.Refresh()
        if (-not $sinkProcess.HasExited) {
            [void]$sinkProcess.CloseMainWindow()
            if (-not $sinkProcess.WaitForExit(30000)) {
                throw 'Scheduler ReceiveOnly observer did not stop and flush within 30 seconds after native report completion'
            }
        }
        $sinkProcess.Refresh()
        if ($sinkProcess.HasExited -and $sinkProcess.ExitCode -ne 0) {
            throw ('Scheduler ReceiveOnly observer exited fail-closed with code ' + $sinkProcess.ExitCode)
        }
        if (-not (Test-Path -LiteralPath $sinkObserverSummaryPath -PathType Leaf)) {
            throw 'Scheduler ReceiveOnly observer summary was not written after consumer shutdown'
        }
    }

    $reportLines = @(Get-Content -LiteralPath $reportPath)
    if ($reportLines.Count -ne 1) { throw 'native harness did not emit exactly one report' }
    $report = $reportLines[0] | ConvertFrom-Json
    $expectedSampleSeconds = if ($SampleSeconds) { $SampleSeconds } else { $null }
    if ($report.run_id -ne $runId -or $report.scenario -ne $Scenario -or $report.timing_margin_us -ne $TimingMarginUs -or $report.sample_seconds -ne $expectedSampleSeconds -or $report.game_fps -ne $GameFps) {
        throw 'native report identity does not match this invocation'
    }
    if ($report.admission_variant -ne $AdmissionVariant) {
        throw "native report admission_variant ($($report.admission_variant)) does not match invocation ($AdmissionVariant)"
    }
    if ($report.status -eq 'PASS' -and ([bool]$report.details.timing_observer_for_qualification -ne [bool]$TimingObserverForQualification)) {
        throw 'passing native report settings do not match this invocation'
    }

    $ownerProcessTerminationVerified = $null
    if ($Scenario -eq 'owner-process-termination') {
        $terminationFixture = $report.details.owner_process_termination_fixture
        [void]$sinkProcess.WaitForExit(5000)
        $sinkProcess.Refresh()
        $ownerProcessTerminationVerified = $sinkProcess.HasExited -and
            $sinkProcess.ExitCode -eq 0x646 -and
            [bool]$terminationFixture.os_process_terminated -and
            [int]$terminationFixture.owner_pid -eq [int]$sink.pid -and
            [long]$terminationFixture.owner_hwnd -eq [long]$sink.hwnd -and
            [long]$terminationFixture.owner_process_start_time_filetime -eq [long]$sink.process_start_time_filetime -and
            [string]::Equals([string]$terminationFixture.owner_process_image_basename, [System.IO.Path]::GetFileName($pwshExecutablePath), [System.StringComparison]::OrdinalIgnoreCase) -and
            [long]$terminationFixture.exit_code -eq 0x646
    }

    $sinkRecords = @(Get-Content -LiteralPath $eventsPath | ForEach-Object { $_ | ConvertFrom-Json })
    $observerSummary = $null
    # This scenario deliberately terminates the ReceiveOnly process before the
    # first Down admission, so it cannot flush its post-shutdown summary.
    if ($receiveOnlyObserverVersion -and $Scenario -ne 'owner-process-termination') {
        $observerSummary = Get-Content -LiteralPath $sinkObserverSummaryPath -Raw | ConvertFrom-Json
        Assert-SchedulerObserverEvidence -Ready $sink -Summary $observerSummary -Records $sinkRecords -ExpectedObserverVersion $receiveOnlyObserverVersion
    }
    for ($index = 0; $index -lt $sinkRecords.Count; $index++) {
        $record = $sinkRecords[$index]
        if ($record.run_id -ne $runId -or $record.event_log_id -ne $sink.event_log_id -or $record.sequence -ne $index) {
            throw ('sink stream identity or sequence failed at record ' + $index)
        }
    }
    $details = $report.details
    $cursor = [long]$details.sink_cursor_sequence_before_arm
    $observedWindow = @($sinkRecords | Where-Object { [long]$_.sequence -gt $cursor })
    $first = if ($observedWindow.Count -gt 0) { [long]$observedWindow[0].sequence } else { $null }
    $last = if ($observedWindow.Count -gt 0) { [long]$observedWindow[-1].sequence } else { $null }
    $windowIntegrity = $details.sink_event_log_id -eq $sink.event_log_id -and
        $details.expected_sink_event_count -eq $details.observed_sink_event_count -and
        $details.observed_sink_event_count -eq $observedWindow.Count -and
        $details.first_authorized_sequence -eq $first -and
        $details.last_authorized_sequence -eq $last -and
        $details.sink_event_count -eq $observedWindow.Count
    if ($Scenario -eq 'owner-process-termination') {
        $windowIntegrity = $windowIntegrity -and [bool]$ownerProcessTerminationVerified
    }
    $window = [ordered]@{
        event_log_id = $sink.event_log_id
        sink_pid = [int]$sink.pid
        sink_hwnd = [long]$sink.hwnd
        sink_process_start_time_filetime = [long]$sink.process_start_time_filetime
        sink_process_image_path = $pwshExecutablePath
        owner_process_termination_verified = $ownerProcessTerminationVerified
        cursor_sequence_before_arm = $cursor
        cursor_offset_before_arm = [long]$details.sink_cursor_offset_before_arm
        first_authorized_sequence = $first
        last_authorized_sequence = $last
        expected_event_count = [long]$details.expected_sink_event_count
        observed_event_count = $observedWindow.Count
        harness_observed_event_count = [long]$details.observed_sink_event_count
        sequence_contiguous = $true
        report_window_match = $windowIntegrity
        receiveonly_observer_mode = if ($receiveOnlyObserverVersion) { 'SchedulerQueuedV3Compat3' } else { 'LegacyInline' }
        receiveonly_observer_version = $receiveOnlyObserverVersion
        observer_events_seen = if ($observerSummary) { [long]$observerSummary.events_seen } else { $null }
        observer_events_enqueued = if ($observerSummary) { [long]$observerSummary.events_enqueued } else { $null }
        observer_events_written = if ($observerSummary) { [long]$observerSummary.events_written } else { $null }
        observer_queue_capacity = if ($observerSummary) { [long]$observerSummary.queue_capacity } else { $null }
        observer_queue_high_water = if ($observerSummary) { [long]$observerSummary.queue_high_water } else { $null }
        observer_queue_dropped = if ($observerSummary) { [long]$observerSummary.queue_dropped } else { $null }
        observer_queue_overflow = if ($observerSummary) { [bool]$observerSummary.queue_overflow } else { $null }
        observer_handler_latency_count = if ($observerSummary) { [long]$observerSummary.handler_latency_count } else { $null }
        observer_handler_latency_sample_count = if ($observerSummary) { [long]$observerSummary.handler_latency_sample_count } else { $null }
        observer_handler_latency_sample_dropped = if ($observerSummary) { [long]$observerSummary.handler_latency_sample_dropped } else { $null }
        observer_handler_latency_p50_us = if ($observerSummary) { $observerSummary.handler_latency_p50_us } else { $null }
        observer_handler_latency_p95_us = if ($observerSummary) { $observerSummary.handler_latency_p95_us } else { $null }
        observer_handler_latency_p99_us = if ($observerSummary) { $observerSummary.handler_latency_p99_us } else { $null }
        observer_handler_latency_max_us = if ($observerSummary) { $observerSummary.handler_latency_max_us } else { $null }
        observer_writer_failures = if ($observerSummary) { [long]$observerSummary.writer_failures } else { $null }
        observer_internal_failures = if ($observerSummary) { [long]$observerSummary.observer_internal_failures } else { $null }
        raw_events = $observedWindow
    }
    $window | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $windowPath -Encoding UTF8
    $invocation = [ordered]@{
        run_id = $runId
        qualification_profile = $QualificationProfile
        workload = if ($usesSessionQualificationContracts) { $Workload } else { $null }
        source_revision = $head
        source_tree_sha = $sourceTree
        source_tree_clean = $sourceTreeClean
        runner_source_revision = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'runner_source_revision' } else { $null }
        runner_source_tree_sha = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'runner_source_tree_sha' } else { $null }
        runner_source_branch = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'runner_source_branch' } else { $null }
        candidate_lineage_branch = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'candidate_lineage_branch' } else { $null }
        candidate_lineage_tip_revision = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'candidate_lineage_tip_revision' } else { $null }
        runner_script_provenance = if ($usesSessionQualificationContracts -and $harnessManifest) { @($harnessManifest.runner_script_provenance) } else { $null }
        workload_version = if ($usesSessionQualificationContracts -and $harnessManifest) { $harnessManifest.workload_version } else { $null }
        session_seed = $null
        seed_mode = if ($usesSessionQualificationContracts) { 'none; deterministic workload definitions do not accept a PRNG seed' } else { $null }
        configured_H_us = if ($usesSessionQualificationContracts) { [long]$sessionTimingFloors.configured_H_us } else { $null }
        harness_source_revision = if ($harnessManifest) { $harnessManifest.source_revision } else { $null }
        harness_source_tree_sha = if ($harnessManifest) { $harnessManifest.source_tree_sha } else { $null }
        harness_source_role = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'source_role' } else { $null }
        harness_binary_path = if ($harnessManifest) { $harnessManifest.binary_path } else { $null }
        harness_binary_sha256 = if ($harnessManifest) { $harnessManifest.binary_sha256 } else { $null }
        performance_timing_margin_us = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'performance_timing_margin_us' } else { $null }
        safety_timing_margin_us = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'safety_timing_margin_us' } else { $null }
        harness_control_kind = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'control_kind' } else { $null }
        harness_control_label = if ($c1TimedVerified) { 'C1-TIMED' } elseif ($QualificationProfile -eq 'FrameSnapshot') { if ($harnessManifest.source_role -eq 'Baseline') { 'FrameSnapshot-Baseline' } else { 'FrameSnapshot-Candidate' } } elseif ($QualificationProfile -eq 'DispatchCleanupM3') { if ($harnessManifest.source_role -eq 'Baseline') { 'DispatchCleanupM3-Baseline' } else { 'DispatchCleanupM3-Candidate' } } elseif ($QualificationProfile -eq 'DispatchSessionRedesignM3') { if ($harnessManifest.source_role -eq 'Baseline') { 'DispatchSessionRedesignM3-Baseline' } else { 'DispatchSessionRedesignM3-Candidate' } } elseif ($QualificationProfile -eq 'Scheduler') { if ($harnessManifest.source_role -eq 'Baseline') { 'Scheduler-Baseline' } else { 'Scheduler-Candidate' } } elseif ($HistoricalSourceAllowed) { 'C0' } else { 'A' }
        harness_base_source_revision = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_revision' } else { $null }
        harness_base_source_tree_sha = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_tree_sha' } else { $null }
        harness_base_source_archive_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_archive_sha256' } else { $null }
        harness_base_source_file_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_file_sha256' } else { $null }
        harness_overlay_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'overlay_sha256' } else { $null }
        harness_derived_source_file_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'derived_source_file_sha256' } else { $null }
        harness_source_changes_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'source_changes_sha256' } else { $null }
        harness_derived_source_inventory_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'derived_source_inventory_sha256' } else { $null }
        scenario = $Scenario
        cpu_contention = [bool]$CpuContention
        cpu_contention_processes = $contentionProcesses.Count
        cpu_contention_method = if ($CpuContention) { 'two hidden pwsh busy workers at BelowNormal priority' } else { $null }
        timing_margin_us = $TimingMarginUs
        game_fps = $GameFps
        sample_seconds = if ($SampleSeconds) { $SampleSeconds } else { $null }
        hol_stall_us = $HolStallUs
        hol_cycles = $HolCycles
        started_utc = $startedUtc
        ended_utc = $endedUtc
        exit_code = $nativeExit
        status = $report.status
        reason = $report.reason
        event_log_id = $sink.event_log_id
        owner_approved_receive_only_run = [bool]$OwnerApprovedReceiveOnlyRun
        target_identity = [ordered]@{
            role = [string]$sink.role
            sink_kind = [string]$sink.sink_kind
            target_class = [string]$sink.target_class
            target_name = [string]$sink.target_name
            game_target = [bool]$sink.game_target
            title = [string]$sink.title
            process = [string]$sink.process
            input_policy = [string]$sink.input_policy
            pid = [int]$sink.pid
            hwnd = [long]$sink.hwnd
            process_start_time_filetime = [long]$sink.process_start_time_filetime
            event_log_id = [string]$sink.event_log_id
        }
        machine_name = [Environment]::MachineName
        os_boot_time_utc = $operatingSystem.LastBootUpTime.ToUniversalTime().ToString('O')
        foreground_permission_granted_to_sink = [bool]$foregroundPermissionGranted
        cursor_sequence_before_arm = $cursor
        cursor_offset_before_arm = [long]$details.sink_cursor_offset_before_arm
        first_authorized_sequence = $first
        last_authorized_sequence = $last
        expected_event_count = [long]$details.expected_sink_event_count
        observed_event_count = $observedWindow.Count
        report_window_match = $windowIntegrity
        receiveonly_observer_mode = if ($receiveOnlyObserverVersion) { 'SchedulerQueuedV3Compat3' } else { 'LegacyInline' }
        receiveonly_observer_version = $receiveOnlyObserverVersion
        observer_events_seen = if ($observerSummary) { [long]$observerSummary.events_seen } else { $null }
        observer_events_enqueued = if ($observerSummary) { [long]$observerSummary.events_enqueued } else { $null }
        observer_events_written = if ($observerSummary) { [long]$observerSummary.events_written } else { $null }
        observer_queue_capacity = if ($observerSummary) { [long]$observerSummary.queue_capacity } else { $null }
        observer_queue_high_water = if ($observerSummary) { [long]$observerSummary.queue_high_water } else { $null }
        observer_queue_dropped = if ($observerSummary) { [long]$observerSummary.queue_dropped } else { $null }
        observer_queue_overflow = if ($observerSummary) { [bool]$observerSummary.queue_overflow } else { $null }
        observer_handler_latency_count = if ($observerSummary) { [long]$observerSummary.handler_latency_count } else { $null }
        observer_handler_latency_sample_count = if ($observerSummary) { [long]$observerSummary.handler_latency_sample_count } else { $null }
        observer_handler_latency_sample_dropped = if ($observerSummary) { [long]$observerSummary.handler_latency_sample_dropped } else { $null }
        observer_handler_latency_p50_us = if ($observerSummary) { $observerSummary.handler_latency_p50_us } else { $null }
        observer_handler_latency_p95_us = if ($observerSummary) { $observerSummary.handler_latency_p95_us } else { $null }
        observer_handler_latency_p99_us = if ($observerSummary) { $observerSummary.handler_latency_p99_us } else { $null }
        observer_handler_latency_max_us = if ($observerSummary) { $observerSummary.handler_latency_max_us } else { $null }
        observer_writer_failures = if ($observerSummary) { [long]$observerSummary.writer_failures } else { $null }
        observer_internal_failures = if ($observerSummary) { [long]$observerSummary.observer_internal_failures } else { $null }
        owner_process_termination_verified = $ownerProcessTerminationVerified
        report_path = 'rt-native-acceptance.jsonl'
    }
    $invocation | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invocationPath -Encoding UTF8
    if ($nativeExit -ne 0 -or $report.status -ne 'PASS' -or -not $windowIntegrity) {
        Write-Output ('QUALIFICATION ' + $report.status + ': ' + $report.reason)
        Write-Output ('EVIDENCE ' + $runDir)
        $resultCode = 1
    } else {
        Write-Output ('PASS ' + $Scenario + ' margin=' + $TimingMarginUs + '; sink window ' + $first + '..' + $last + ' (' + $observedWindow.Count + ' events)')
        Write-Output ('EVIDENCE ' + $runDir)
        $resultCode = 0
    }
} catch {
    if ($PreflightOnly) {
        Write-Output ('CASE_PREFLIGHT_ONLY REJECTED profile=' + $QualificationProfile + ' scenario=' + $Scenario +
            ' workload=' + $Workload + ' error=' + $_.Exception.Message)
    } elseif ($isChordWidthPilotV1 -and -not $pilotEntryGateVerified) {
        Write-Output ('PILOT_ENTRY_GATE REJECTED profile=' + $QualificationProfile + ' scenario=' + $Scenario +
            ' workload=' + $Workload + ' output_created=false receiver_started=false harness_started=false error=' + $_.Exception.Message)
    } else {
        New-Item -ItemType Directory -Force -Path $runDir | Out-Null
        $failure = [ordered]@{
            run_id = $runId
            scenario = $Scenario
            timing_margin_us = $TimingMarginUs
            game_fps = $GameFps
            sample_seconds = if ($SampleSeconds) { $SampleSeconds } else { $null }
            hol_stall_us = $HolStallUs
            hol_cycles = $HolCycles
            source_revision = if ($head) { $head } else { $null }
            error = $_.Exception.Message
        }
        $failure | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $runDir 'runner-failure.json') -Encoding UTF8
        Write-Output ('RUNNER FAILURE: ' + $_.Exception.Message)
        Write-Output ('EVIDENCE ' + $runDir)
    }
    $resultCode = 2
} finally {
    foreach ($process in $contentionProcesses) {
        if ($process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
    if ($probeProcess) {
        $liveProbe = Get-Process -Id $probeProcess.Id -ErrorAction SilentlyContinue
        if ($liveProbe) {
            [void]$liveProbe.CloseMainWindow()
            if (-not $liveProbe.WaitForExit(3000)) {
                Stop-Process -Id $probeProcess.Id -Force -ErrorAction SilentlyContinue
            }
        }
    }
    if ($sinkProcess) {
        $sinkProcess.Refresh()
        if (-not $sinkProcess.HasExited) {
            [void]$sinkProcess.CloseMainWindow()
            if (-not $sinkProcess.WaitForExit(3000)) {
                Stop-Process -InputObject $sinkProcess -Force -ErrorAction SilentlyContinue
            }
        }
    }
}

if ($PreflightOnly -or ($isChordWidthPilotV1 -and -not $pilotEntryGateVerified)) { exit $resultCode }

$checksumLines = @(Get-ChildItem -LiteralPath $runDir -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name | ForEach-Object {
    $hash = Get-Sha256 $_.FullName
    $hash + '  ' + $_.Name
})
Set-Content -LiteralPath (Join-Path $runDir 'SHA256SUMS.txt') -Value $checksumLines -Encoding UTF8
exit $resultCode
