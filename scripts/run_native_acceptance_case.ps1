param(
    [Parameter(Mandatory)]
[ValidateSet('canonical-single', 'canonical-chord', 'canonical-max-chord', 'hold', 'long-single-sequence', 'dense-alternating', 'chord-sweep', 'near-minimum-retrigger', 'rapid-retrigger', 'release-gap-stress', 'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary', 'modifier-held-after-owned', 'cleanup-full-release', 'focus-loss', 'target-hwnd-change', 'owner-mismatch', 'owner-query-failure', 'owner-process-termination', 'pause-resume', 'pause-resume-release-floor', 'suspend-resume', 'stop-cleanup', 'skip-cleanup', 'supervisor-lease-expiry', 'lease-age-watchdog-delayed', 'noncanonical-profile', 'timing-margin-sweep', 'long-idle', 'hol-interleaved')]
    [string]$Scenario,
    [Parameter(Mandatory)]
    [ValidateScript({ $_ -ge 0 -and $_ -le 3000 -and $_ % 100 -eq 0 })]
    [int]$TimingMarginUs,
    [ValidateSet('Baseline', 'Candidate')]
    [string]$CalibrationPlacement = 'Baseline',
    [switch]$TimingObserverForQualification,
    [switch]$CpuContention,
    [switch]$HistoricalSourceAllowed,
    [switch]$QpcReadCountingEvidenceOnly,
    [string]$HarnessPath,
    [string]$HarnessManifestPath,
    [string]$OutputRoot,
    [ValidateRange(15, 15)]
    [int]$SampleSeconds,
    [ValidateSet(60, 120)]
    [int]$GameFps = 60,
    [int]$HolStallUs = 0,
    [int]$HolCycles = 0
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Import-Module (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Force
$sinkScript = Join-Path $root 'scripts\native_acceptance_sink.ps1'
$pwshPath = (Get-Command pwsh.exe).Source
$pwshExecutablePath = (Get-Process -Id $PID).Path
$runId = 'native-case-' + (Get-Date -Format 'yyyyMMddTHHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$evidenceRoot = if ($OutputRoot) {
    [System.IO.Path]::GetFullPath($OutputRoot)
} else {
    Join-Path $root '.benchmarks\physical-native-cases'
}
$runDir = Join-Path $evidenceRoot $runId
New-Item -ItemType Directory -Force -Path $runDir | Out-Null
$readyPath = Join-Path $runDir 'sink-ready.json'
$eventsPath = Join-Path $runDir 'sink-events.jsonl'
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
    Join-Path $root 'rust\target\release\rt-native-acceptance.exe'
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

Add-Type -TypeDefinition @"
using System.Runtime.InteropServices;
public static class NativeAcceptanceForegroundPermission {
    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool AllowSetForegroundWindow(uint processId);
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
    param($Record, [int]$SessionId)
    if ($Record.schema_version -ne 3 -or $Record.event_schema_version -ne 3 -or $Record.run_id -ne $runId -or $Record.role -ne 'ReceiveOnly') {
        throw 'receive-only sink ready record schema, role, or run ID mismatch'
    }
    if ($Record.sink_kind -ne 'SkyAutoPlayer.NativeAcceptanceSink' -or $Record.title -ne 'Sky Auto Player — Native Acceptance Sink' -or $Record.process -ne 'native_acceptance_sink.ps1' -or $Record.input_policy -ne 'receives_only; benchmark must use SendInput') {
        throw 'ready record does not identify the project-owned receive-only sink'
    }
    if ($Record.pid -le 0 -or $Record.hwnd -le 0 -or $Record.event_log_id.Length -eq 0 -or $Record.process_start_time_filetime -le 0) {
        throw 'sink ready record omitted process, HWND, event-log, or start-time identity'
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
        if ($event.schema_version -ne 3 -or $event.run_id -ne $runId -or $event.role -ne 'ReceiveOnly' -or $event.event_log_id -ne $Record.event_log_id -or $event.sequence -ne $index -or ($index -gt 0 -and $event.kind -eq 'stream_start')) {
            throw ('pre-arm sink stream identity or sequence mismatch at record ' + $index)
        }
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

try {
    if (-not [Environment]::UserInteractive) {
        throw 'Windows is not interactive; refusing physical acceptance'
    }
    if ([bool]$HarnessPath -ne [bool]$HarnessManifestPath) {
        throw 'immutable harness runs require both -HarnessPath and -HarnessManifestPath'
    }
    if ($SampleSeconds -and $Scenario -notin @('canonical-max-chord', 'dense-alternating', 'near-minimum-retrigger', 'mixed-up-down', 'long-idle', 'hol-interleaved')) {
        throw '-SampleSeconds is valid only for canonical-max-chord, dense-alternating, near-minimum-retrigger, mixed-up-down, long-idle, and hol-interleaved'
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
        $c1TimedVerified = Assert-C1TimedControlProvenance -Manifest $harnessManifest -RepositoryRoot $root -Role 'case-runner' -AllowHistoricalControl:$HistoricalSourceAllowed
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
        if ($harnessManifest.schema_version -ne 1 -or
            (-not $HistoricalSourceAllowed -and -not $sourceRevisionMatches) -or
            $harnessManifest.source_revision -notmatch '^[0-9a-f]{40}$' -or
            $harnessManifest.source_tree_sha -notmatch '^[0-9a-f]{40}$' -or
            $harnessManifest.source_branch -ne 'perf/dispatch-core-20261109' -or
            ([bool]$harnessManifest.source_tree_clean -ne (-not $c1TimedVerified)) -or
            -not [bool]$harnessManifest.clean_build -or
            $harnessManifest.cargo_profile -ne 'dist' -or
            (($manifestFeatures -join ',') -ne ($expectedManifestFeatures -join ',')) -or
            ([bool]$harnessManifest.qpc_read_counting -ne [bool]$QpcReadCountingEvidenceOnly) -or
            $harnessManifest.observer_mode -ne $expectedObserverMode -or
            $harnessManifest.guard_mode -ne 'frozen' -or
            $harnessManifest.workload_version -ne 'dispatch-core-646-a1' -or
            $harnessManifest.target -ne 'x86_64-pc-windows-msvc' -or
            $harnessManifest.rustc_version -ne $rustcVersion.Trim() -or
            -not [string]::Equals($manifestBinaryPath, $harness, [System.StringComparison]::OrdinalIgnoreCase) -or
            $harnessManifest.binary_name -ne 'rt-native-acceptance.exe' -or
            $harnessManifest.binary_sha256 -ne (Get-Sha256 $harness)) {
            throw 'immutable harness manifest source, profile, features, path, name, or SHA-256 mismatch'
        }
    }
    $sessionId = (Get-Process -Id $PID).SessionId
    $sinkArgv = @('-NoProfile', '-File', $sinkScript, '-Mode', 'ReceiveOnly', '-RunId', $runId, '-ReadyFile', $readyPath, '-EventLog', $eventsPath, '-ActivateRequestFile', $focusRestoreRequestPath)
    $sinkProcess = Start-Process -FilePath $pwshPath -ArgumentList $sinkArgv -PassThru -WindowStyle Normal -RedirectStandardOutput (Join-Path $runDir 'sink.stdout.log') -RedirectStandardError (Join-Path $runDir 'sink.stderr.log')
    $sink = Wait-ReadyRecord $readyPath $sinkProcess
    Assert-SinkIdentity $sink $sessionId
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
        source_revision = $head
        source_tree_sha = $sourceTree
        source_tree_clean = $sourceTreeClean
        scenario = $Scenario
        calibration_placement = $CalibrationPlacement.ToLowerInvariant()
        cpu_contention = [bool]$CpuContention
        cpu_contention_processes = $contentionProcesses.Count
        cpu_contention_method = if ($CpuContention) { 'two hidden pwsh busy workers at BelowNormal priority' } else { $null }
        timing_margin_us = $TimingMarginUs
        game_fps = $GameFps
        sample_seconds = if ($SampleSeconds) { $SampleSeconds } else { $null }
        hol_stall_us = $HolStallUs
        hol_cycles = $HolCycles
        expected_hold_us = [long][Math]::Ceiling(1000000.0 / $GameFps) * 2 + $TimingMarginUs
        expected_release_gap_us = [long][Math]::Ceiling(1000000.0 / $GameFps) + $TimingMarginUs
        profile = 'production'
        cargo_profile = if ($harnessManifest) { $harnessManifest.cargo_profile } else { 'legacy-checkout-release' }
        cargo_features = if ($harnessManifest) { @($harnessManifest.cargo_features) } else { @() }
        harness_source_revision = if ($harnessManifest) { $harnessManifest.source_revision } else { $null }
        harness_source_tree_sha = if ($harnessManifest) { $harnessManifest.source_tree_sha } else { $null }
        harness_control_kind = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'control_kind' } else { $null }
        harness_control_label = if ($c1TimedVerified) { 'C1-TIMED' } elseif ($HistoricalSourceAllowed) { 'C0' } else { 'A' }
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
        sink_pid = [int]$sink.pid
        sink_hwnd = [long]$sink.hwnd
        sink_event_log_id = $sink.event_log_id
        sink_process_start_time_filetime = [long]$sink.process_start_time_filetime
        sink_process_image_path = $pwshExecutablePath
        foreground_permission_granted_to_sink = [bool]$foregroundPermissionGranted
    }
    $runConfig | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $runConfigPath -Encoding UTF8
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $runDir 'runner.ps1')

    $startedUtc = [DateTimeOffset]::UtcNow.ToString('O')
    $harnessArgs = @(
        'run', '--allow-real-input', '--run-id', $runId,
        '--sink-ready', $readyPath, '--sink-events', $eventsPath,
        '--target-hwnd', ([long]$sink.hwnd).ToString([System.Globalization.CultureInfo]::InvariantCulture),
        '--scenario', $Scenario, '--evidence', $reportPath,
        '--calibration-placement', $CalibrationPlacement.ToLowerInvariant(),
        '--timing-margin-us', [string]$TimingMarginUs,
        '--game-fps', [string]$GameFps
    )
    if ($SampleSeconds) { $harnessArgs += @('--sample-seconds', [string]$SampleSeconds) }
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
    if ($Scenario -eq 'focus-loss') {
        $harnessArgs += @('--focus-probe-ready', $probeReadyPath, '--focus-probe-events', $probeEventsPath, '--focus-probe-hwnd', ([long]$probe.hwnd).ToString([System.Globalization.CultureInfo]::InvariantCulture), '--focus-restore-request', $focusRestoreRequestPath)
    }
    $captured = @(& $harness @harnessArgs 2>&1)
    $nativeExit = $LASTEXITCODE
    if ($harnessManifest -and (Get-Sha256 $harness) -ne $harnessManifest.binary_sha256) {
        throw 'immutable harness binary changed during acceptance run'
    }
    $endedUtc = [DateTimeOffset]::UtcNow.ToString('O')
    $capturedText = ($captured | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
    Set-Content -LiteralPath $stdoutPath -Value $capturedText -Encoding UTF8

    $reportLines = @(Get-Content -LiteralPath $reportPath)
    if ($reportLines.Count -ne 1) { throw 'native harness did not emit exactly one report' }
    $report = $reportLines[0] | ConvertFrom-Json
    $expectedSampleSeconds = if ($SampleSeconds) { $SampleSeconds } else { $null }
    if ($report.run_id -ne $runId -or $report.scenario -ne $Scenario -or $report.timing_margin_us -ne $TimingMarginUs -or $report.sample_seconds -ne $expectedSampleSeconds -or $report.game_fps -ne $GameFps) {
        throw 'native report identity does not match this invocation'
    }
    if ($report.status -eq 'PASS' -and ($report.details.calibration_placement -ne $CalibrationPlacement.ToLowerInvariant() -or [bool]$report.details.timing_observer_for_qualification -ne [bool]$TimingObserverForQualification)) {
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
        raw_events = $observedWindow
    }
    $window | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $windowPath -Encoding UTF8
    $invocation = [ordered]@{
        run_id = $runId
        source_revision = $head
        source_tree_sha = $sourceTree
        source_tree_clean = $sourceTreeClean
        harness_source_revision = if ($harnessManifest) { $harnessManifest.source_revision } else { $null }
        harness_source_tree_sha = if ($harnessManifest) { $harnessManifest.source_tree_sha } else { $null }
        harness_control_kind = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'control_kind' } else { $null }
        harness_control_label = if ($c1TimedVerified) { 'C1-TIMED' } elseif ($HistoricalSourceAllowed) { 'C0' } else { 'A' }
        harness_base_source_revision = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_revision' } else { $null }
        harness_base_source_tree_sha = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_tree_sha' } else { $null }
        harness_base_source_archive_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_archive_sha256' } else { $null }
        harness_base_source_file_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'base_source_file_sha256' } else { $null }
        harness_overlay_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'overlay_sha256' } else { $null }
        harness_derived_source_file_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'derived_source_file_sha256' } else { $null }
        harness_source_changes_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'source_changes_sha256' } else { $null }
        harness_derived_source_inventory_sha256 = if ($harnessManifest) { Get-ManifestProperty $harnessManifest 'derived_source_inventory_sha256' } else { $null }
        scenario = $Scenario
        calibration_placement = $CalibrationPlacement.ToLowerInvariant()
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
        foreground_permission_granted_to_sink = [bool]$foregroundPermissionGranted
        cursor_sequence_before_arm = $cursor
        cursor_offset_before_arm = [long]$details.sink_cursor_offset_before_arm
        first_authorized_sequence = $first
        last_authorized_sequence = $last
        expected_event_count = [long]$details.expected_sink_event_count
        observed_event_count = $observedWindow.Count
        report_window_match = $windowIntegrity
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

$checksumLines = @(Get-ChildItem -LiteralPath $runDir -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name | ForEach-Object {
    $hash = Get-Sha256 $_.FullName
    $hash + '  ' + $_.Name
})
Set-Content -LiteralPath (Join-Path $runDir 'SHA256SUMS.txt') -Value $checksumLines -Encoding UTF8
exit $resultCode
