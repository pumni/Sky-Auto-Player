param(
    [Parameter(Mandatory)]
    [ValidateSet('timing-margin-sweep', 'release-gap-stress')]
    [string]$Scenario,
    [Parameter(Mandatory)]
    [ValidateScript({ $_ -ge 0 -and $_ -le 3000 -and $_ % 100 -eq 0 })]
    [int]$TimingMarginUs,
    [Parameter(Mandatory)]
    [ValidateScript({ $_ -ge 0 -and $_ -le 5000 -and $_ % 100 -eq 0 })]
    [int]$LateDownToleranceUs
)

$ErrorActionPreference = 'Stop'
$root = (Get-Location).Path
$sinkScript = Join-Path $root 'scripts\native_acceptance_sink.ps1'
$pwshPath = (Get-Command pwsh.exe).Source
$runId = 'native-case-' + (Get-Date -Format 'yyyyMMddTHHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$runDir = Join-Path $root (Join-Path '.benchmarks\physical-native-cases' $runId)
New-Item -ItemType Directory -Force -Path $runDir | Out-Null
$readyPath = Join-Path $runDir 'sink-ready.json'
$eventsPath = Join-Path $runDir 'sink-events.jsonl'
$stdoutPath = Join-Path $runDir 'harness-output.log'
$reportPath = Join-Path $runDir 'rt-native-acceptance.jsonl'
$runConfigPath = Join-Path $runDir 'run-config.json'
$invocationPath = Join-Path $runDir 'invocation.json'
$windowPath = Join-Path $runDir 'sink-event-window.json'
$harness = Join-Path $root 'rust\target\release\rt-native-acceptance.exe'
$sinkProcess = $null
$nativeExit = $null
$report = $null
$windowIntegrity = $false
$resultCode = 2
$head = $null

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
    if (-not [string]::Equals($owner.Path, $pwshPath, [System.StringComparison]::OrdinalIgnoreCase) -or $owner.SessionId -ne $SessionId -or [long]$owner.MainWindowHandle -ne [long]$Record.hwnd -or $owner.MainWindowTitle -ne $Record.title) {
        throw 'live sink HWND/process identity mismatch'
    }
    $records = @(Get-Content -LiteralPath $eventsPath | ForEach-Object { $_ | ConvertFrom-Json })
    if ($records.Count -ne 1 -or $records[0].kind -ne 'stream_start' -or $records[0].sequence -ne 0 -or $records[0].run_id -ne $runId -or $records[0].event_log_id -ne $Record.event_log_id) {
        throw 'fresh sink event log is not bound to its ready record'
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

try {
    if (-not [Environment]::UserInteractive) {
        throw 'Windows is not interactive; refusing physical acceptance'
    }
    if (-not (Test-Path -LiteralPath $harness)) {
        throw 'feature-gated release acceptance harness is missing; build it before the run'
    }
    $head = (& rtk git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source revision' }
    $trackedChanges = (& rtk git status --porcelain --untracked-files=no) -join ''
    if ($LASTEXITCODE -ne 0) { throw 'could not verify source-tree cleanliness' }
    $sourceTreeClean = [string]::IsNullOrWhiteSpace($trackedChanges)
    $sessionId = (Get-Process -Id $PID).SessionId
    $sinkArgv = @('-NoProfile', '-File', $sinkScript, '-Mode', 'ReceiveOnly', '-RunId', $runId, '-ReadyFile', $readyPath, '-EventLog', $eventsPath)
    $sinkProcess = Start-Process -FilePath $pwshPath -ArgumentList $sinkArgv -PassThru -WindowStyle Normal -RedirectStandardOutput (Join-Path $runDir 'sink.stdout.log') -RedirectStandardError (Join-Path $runDir 'sink.stderr.log')
    $sink = Wait-ReadyRecord $readyPath $sinkProcess
    Assert-SinkIdentity $sink $sessionId

    $harnessHash = Get-Sha256 $harness
    $runnerHash = Get-Sha256 $PSCommandPath
    $runConfig = [ordered]@{
        run_id = $runId
        source_revision = $head
        source_tree_clean = $sourceTreeClean
        scenario = $Scenario
        timing_margin_us = $TimingMarginUs
        late_down_tolerance_us = $LateDownToleranceUs
        expected_hold_us = 16667 + $TimingMarginUs
        expected_release_gap_us = 16667 + $TimingMarginUs
        harness_path = $harness
        harness_sha256 = $harnessHash
        runner_sha256 = $runnerHash
        sink_ready_path = $readyPath
        sink_event_path = $eventsPath
        sink_pid = [int]$sink.pid
        sink_hwnd = [long]$sink.hwnd
        sink_event_log_id = $sink.event_log_id
    }
    $runConfig | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $runConfigPath -Encoding UTF8
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $runDir 'runner.ps1')

    $startedUtc = [DateTimeOffset]::UtcNow.ToString('O')
    $harnessArgs = @(
        'run', '--allow-real-input', '--run-id', $runId,
        '--sink-ready', $readyPath, '--sink-events', $eventsPath,
        '--target-hwnd', ([long]$sink.hwnd).ToString([System.Globalization.CultureInfo]::InvariantCulture),
        '--scenario', $Scenario, '--evidence', $reportPath,
        '--timing-margin-us', [string]$TimingMarginUs,
        '--down-late-grace-us', [string]$LateDownToleranceUs
    )
    $captured = @(& $harness @harnessArgs 2>&1)
    $nativeExit = $LASTEXITCODE
    $endedUtc = [DateTimeOffset]::UtcNow.ToString('O')
    $capturedText = ($captured | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
    Set-Content -LiteralPath $stdoutPath -Value $capturedText -Encoding UTF8

    $reportLines = @(Get-Content -LiteralPath $reportPath)
    if ($reportLines.Count -ne 1) { throw 'native harness did not emit exactly one report' }
    $report = $reportLines[0] | ConvertFrom-Json
    if ($report.run_id -ne $runId -or $report.scenario -ne $Scenario -or $report.timing_margin_us -ne $TimingMarginUs -or $report.down_late_grace_us -ne $LateDownToleranceUs) {
        throw 'native report identity or selected settings do not match this invocation'
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
    $window = [ordered]@{
        event_log_id = $sink.event_log_id
        sink_pid = [int]$sink.pid
        sink_hwnd = [long]$sink.hwnd
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
        source_tree_clean = $sourceTreeClean
        scenario = $Scenario
        timing_margin_us = $TimingMarginUs
        late_down_tolerance_us = $LateDownToleranceUs
        started_utc = $startedUtc
        ended_utc = $endedUtc
        exit_code = $nativeExit
        status = $report.status
        reason = $report.reason
        event_log_id = $sink.event_log_id
        cursor_sequence_before_arm = $cursor
        cursor_offset_before_arm = [long]$details.sink_cursor_offset_before_arm
        first_authorized_sequence = $first
        last_authorized_sequence = $last
        expected_event_count = [long]$details.expected_sink_event_count
        observed_event_count = $observedWindow.Count
        report_window_match = $windowIntegrity
        report_path = $reportPath
    }
    $invocation | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invocationPath -Encoding UTF8
    if ($nativeExit -ne 0 -or $report.status -ne 'PASS' -or -not $windowIntegrity) {
        Write-Output ('QUALIFICATION ' + $report.status + ': ' + $report.reason)
        Write-Output ('EVIDENCE ' + $runDir)
        $resultCode = 1
    } else {
        Write-Output ('PASS ' + $Scenario + ' margin=' + $TimingMarginUs + ' tolerance=' + $LateDownToleranceUs + '; sink window ' + $first + '..' + $last + ' (' + $observedWindow.Count + ' events)')
        Write-Output ('EVIDENCE ' + $runDir)
        $resultCode = 0
    }
} catch {
    $failure = [ordered]@{
        run_id = $runId
        scenario = $Scenario
        timing_margin_us = $TimingMarginUs
        late_down_tolerance_us = $LateDownToleranceUs
        source_revision = if ($head) { $head } else { $null }
        error = $_.Exception.Message
    }
    $failure | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $runDir 'runner-failure.json') -Encoding UTF8
    Write-Output ('RUNNER FAILURE: ' + $_.Exception.Message)
    Write-Output ('EVIDENCE ' + $runDir)
    $resultCode = 2
} finally {
    if ($sinkProcess) {
        $live = Get-Process -Id $sinkProcess.Id -ErrorAction SilentlyContinue
        if ($live) {
            [void]$live.CloseMainWindow()
            if (-not $live.WaitForExit(3000)) {
                Stop-Process -Id $sinkProcess.Id -Force -ErrorAction SilentlyContinue
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
