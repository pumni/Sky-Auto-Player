$ErrorActionPreference = 'Stop'
$root = (Get-Location).Path
$runId = 'timing-margin-' + (Get-Date -Format 'yyyyMMddTHHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8)
$runBase = Join-Path $root '.benchmarks\physical-timing-margin'
$runDir = Join-Path $runBase $runId
New-Item -ItemType Directory -Force -Path $runDir | Out-Null
$scriptPath = Join-Path $root 'scripts\native_acceptance_sink.ps1'
$pwshPath = (Get-Command pwsh.exe).Source
$expectedImage = [System.IO.Path]::GetFileName($pwshPath)
$probeReady = Join-Path $runDir 'focus-probe-ready.json'
$probeEvents = Join-Path $runDir 'focus-probe-events.jsonl'
$sinkReady = Join-Path $runDir 'sink-ready.json'
$sinkEvents = Join-Path $runDir 'sink-events.jsonl'
$probeOut = Join-Path $runDir 'focus-probe.stdout.log'
$probeErr = Join-Path $runDir 'focus-probe.stderr.log'
$sinkOut = Join-Path $runDir 'sink.stdout.log'
$sinkErr = Join-Path $runDir 'sink.stderr.log'
$evidence = Join-Path $runDir 'rt-native-acceptance.jsonl'
$invocationLog = Join-Path $runDir 'invocations.jsonl'
$harnessOutput = Join-Path $runDir 'harness-output.log'
$configFile = Join-Path $runDir 'run-config.json'
$probeProcess = $null
$sinkProcess = $null
$haltReason = $null
$completed = 0
$resultCode = 0

function Wait-ReadyRecord {
    param([string]$Path, [System.Diagnostics.Process]$Process)
    $deadline = (Get-Date).AddSeconds(20)
    while (-not (Test-Path -LiteralPath $Path) -and (Get-Date) -lt $deadline -and -not $Process.HasExited) {
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path -LiteralPath $Path)) {
        throw ('ready evidence missing for process id ' + $Process.Id)
    }
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Assert-SinkIdentity {
    param($Record, [string]$Role, [string]$Kind, [string]$Title, [string]$EventPath, [string]$OwnerImage, [int]$SessionId)
    if ($Record.schema_version -ne 3 -or $Record.event_schema_version -ne 3 -or $Record.run_id -ne $runId) {
        throw ($Role + ' ready record schema/run ID mismatch')
    }
    if ($Record.role -ne $Role -or $Record.sink_kind -ne $Kind -or $Record.title -ne $Title -or $Record.process -ne 'native_acceptance_sink.ps1' -or $Record.input_policy -ne 'receives_only; benchmark must use SendInput') {
        throw ($Role + ' ready record did not match the project-owned receive-only protocol')
    }
    if ($Record.pid -le 0 -or $Record.hwnd -le 0 -or $Record.event_log_id.Length -eq 0 -or $Record.process_start_time_filetime -le 0) {
        throw ($Role + ' ready record omitted identity fields')
    }
    $owner = Get-Process -Id ([int]$Record.pid)
    if (-not [string]::Equals($owner.Path, $OwnerImage, [System.StringComparison]::OrdinalIgnoreCase) -or $owner.SessionId -ne $SessionId -or [long]$owner.MainWindowHandle -ne [long]$Record.hwnd -or $owner.MainWindowTitle -ne $Title) {
        throw ($Role + ' live HWND/process identity mismatch')
    }
    $headerLines = @(Get-Content -LiteralPath $EventPath)
    if ($headerLines.Count -ne 1) {
        throw ($Role + ' event log was not fresh at preflight')
    }
    $header = $headerLines[0] | ConvertFrom-Json
    if ($header.schema_version -ne 3 -or $header.run_id -ne $runId -or $header.role -ne $Role -or $header.event_log_id -ne $Record.event_log_id -or $header.kind -ne 'stream_start' -or $header.sequence -ne 0) {
        throw ($Role + ' event stream binding mismatch')
    }
}

try {
    if (-not [Environment]::UserInteractive) {
        throw 'Windows is not interactive; refusing physical acceptance'
    }
    $parentSession = (Get-Process -Id $PID).SessionId
    $probeArgv = @('-NoProfile','-File',$scriptPath,'-Mode','InertFocusProbe','-RunId',$runId,'-ReadyFile',$probeReady,'-EventLog',$probeEvents)
    $probeProcess = Start-Process -FilePath $pwshPath -ArgumentList $probeArgv -PassThru -WindowStyle Normal -RedirectStandardOutput $probeOut -RedirectStandardError $probeErr
    $probe = Wait-ReadyRecord $probeReady $probeProcess
    Assert-SinkIdentity $probe 'InertFocusProbe' 'SkyAutoPlayer.NativeAcceptanceFocusProbe' 'Sky Auto Player — Native Acceptance Focus Probe' $probeEvents $pwshPath $parentSession

    $sinkArgv = @('-NoProfile','-File',$scriptPath,'-Mode','ReceiveOnly','-RunId',$runId,'-ReadyFile',$sinkReady,'-EventLog',$sinkEvents)
    $sinkProcess = Start-Process -FilePath $pwshPath -ArgumentList $sinkArgv -PassThru -WindowStyle Normal -RedirectStandardOutput $sinkOut -RedirectStandardError $sinkErr
    $sink = Wait-ReadyRecord $sinkReady $sinkProcess
    Assert-SinkIdentity $sink 'ReceiveOnly' 'SkyAutoPlayer.NativeAcceptanceSink' 'Sky Auto Player — Native Acceptance Sink' $sinkEvents $pwshPath $parentSession

    $harness = Join-Path $root 'rust\target\release\rt-native-acceptance.exe'
    if (-not (Test-Path -LiteralPath $harness)) {
        throw 'feature-gated release acceptance harness is missing'
    }
    $defaultScenarios = @('canonical-single','canonical-chord','canonical-max-chord','hold','rapid-retrigger','mixed-up-down','target-hwnd-change','pause-resume','stop-cleanup','skip-cleanup','cleanup-full-release','w4-noncanonical')
    $cases = @()
    foreach ($scenarioName in $defaultScenarios) {
        $cases += [pscustomobject]@{ scenario = $scenarioName; margin = 800 }
    }
    foreach ($marginValue in @(0,500,800,1000,3000)) {
        $cases += [pscustomobject]@{ scenario = 'timing-margin-sweep'; margin = $marginValue }
    }
    $cases += [pscustomobject]@{ scenario = 'focus-loss'; margin = 800 }
    $config = [ordered]@{
        run_id = $runId
        run_dir = $runDir
        harness = $harness
        sink = [ordered]@{ ready = $sinkReady; events = $sinkEvents; hwnd = [long]$sink.hwnd; pid = [int]$sink.pid; process_image = $expectedImage; event_log_id = $sink.event_log_id }
        focus_probe = [ordered]@{ ready = $probeReady; events = $probeEvents; hwnd = [long]$probe.hwnd; pid = [int]$probe.pid; process_image = $expectedImage; event_log_id = $probe.event_log_id }
        default_margin_us = 800
        fixed_down_late_grace_us = 500
        hold_frames = 1.0
        scenarios = $cases
    }
    $config | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $configFile -Encoding UTF8
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $runDir 'runner.ps1')
    $targetHwnd = ([long]$sink.hwnd).ToString([System.Globalization.CultureInfo]::InvariantCulture)
    $index = 0

    foreach ($case in $cases) {
        $index++
        $scenarioName = [string]$case.scenario
        $marginValue = [int]$case.margin
        $runArgs = @('run','--allow-real-input','--run-id',$runId,'--sink-ready',$sinkReady,'--sink-events',$sinkEvents,'--target-hwnd',$targetHwnd,'--scenario',$scenarioName,'--evidence',$evidence,'--timing-margin-us',([string]$marginValue),'--down-late-grace-us','500')
        if ($scenarioName -eq 'focus-loss') {
            $probeHwnd = ([long]$probe.hwnd).ToString([System.Globalization.CultureInfo]::InvariantCulture)
            $runArgs += @('--focus-probe-ready',$probeReady,'--focus-probe-events',$probeEvents,'--focus-probe-hwnd',$probeHwnd)
        }
        Write-Output ('starting {0}/{1}: {2}, margin={3} us, grace=500 us' -f $index,$cases.Count,$scenarioName,$marginValue)
        $startedUtc = [DateTime]::UtcNow.ToString('O')
        $captured = @(& $harness @runArgs 2>&1)
        $nativeExit = $LASTEXITCODE
        $capturedText = ($captured | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
        if ($capturedText.Length -gt 0) {
            Add-Content -LiteralPath $harnessOutput -Value $capturedText -Encoding UTF8
        }
        $reportLines = @(Get-Content -LiteralPath $evidence)
        if ($reportLines.Count -ne $index) {
            throw ('evidence report count mismatch after ' + $scenarioName + ' margin=' + $marginValue)
        }
        $report = $reportLines[$index - 1] | ConvertFrom-Json
        $ledger = [ordered]@{ started_utc = $startedUtc; scenario = $scenarioName; timing_margin_us = $marginValue; down_late_grace_us = 500; exit_code = $nativeExit; status = $report.status; reason = $report.reason; run_id = $report.run_id; stdout = $capturedText }
        $ledger | ConvertTo-Json -Compress -Depth 8 | Add-Content -LiteralPath $invocationLog -Encoding UTF8
        if ($report.run_id -ne $runId -or $report.scenario -ne $scenarioName -or $report.timing_margin_us -ne $marginValue -or $report.down_late_grace_us -ne 500) {
            throw ('report identity/config mismatch after ' + $scenarioName + ' margin=' + $marginValue)
        }
        if ($scenarioName -eq 'timing-margin-sweep') {
            $expectedHold = 16667 + $marginValue
            $expectedGap = 16667 + $marginValue
            $targets = @($report.details.authored_packet_targets)
            if ($report.min_hold_us -ne $expectedHold -or $report.min_release_gap_us -ne $expectedGap -or $targets.Count -ne 4) {
                throw ('timing-margin report floor/packet count mismatch at margin=' + $marginValue)
            }
            if (($targets[1].scheduled_us - $targets[0].scheduled_us) -ne $report.min_hold_us -or ($targets[2].scheduled_us - $targets[1].scheduled_us) -ne $report.min_release_gap_us -or ($targets[3].scheduled_us - $targets[2].scheduled_us) -ne $report.min_hold_us) {
                throw ('authored packet targets contained hidden timing at margin=' + $marginValue)
            }
            if ($targets[0].down_mask -eq 0 -or $targets[1].up_mask -eq 0 -or $targets[2].down_mask -eq 0 -or $targets[3].up_mask -eq 0) {
                throw ('timing-margin packet directions were not Down/Up/Down/Up at margin=' + $marginValue)
            }
        } elseif ($report.min_hold_us -ne 17467 -or $report.min_release_gap_us -ne 17467) {
            throw ('default 1.0-frame/800-us policy mismatch after ' + $scenarioName)
        }
        if ($nativeExit -ne 0 -or $report.status -ne 'PASS') {
            $haltReason = 'fail-fast after ' + $scenarioName + ' margin=' + $marginValue + ' exit=' + $nativeExit + ' verdict=' + [string]$report.status + ': ' + [string]$report.reason
            Write-Output $haltReason
            $resultCode = 1
            break
        }
        $completed++
        Write-Output ('completed {0}/{1}: PASS, hold={2} us, release-gap={3} us, down-cutoff/grace={4} us' -f $index,$cases.Count,$report.min_hold_us,$report.min_release_gap_us,$report.down_late_grace_us)
    }

    $reports = @((Get-Content -LiteralPath $evidence) | ForEach-Object { $_ | ConvertFrom-Json })
    $probeLines = @(Get-Content -LiteralPath $probeEvents)
    $probeEventCount = [Math]::Max(0,$probeLines.Count - 1)
    $summary = [ordered]@{
        run_id = $runId
        run_dir = $runDir
        evidence = $evidence
        sink_events = $sinkEvents
        focus_probe_events = $probeEvents
        requested_cases = $cases.Count
        completed_passes = $completed
        report_count = $reports.Count
        pass_count = @($reports | Where-Object status -eq 'PASS').Count
        focus_probe_keyboard_event_count = $probeEventCount
        sink_pid = [int]$sink.pid
        sink_hwnd = [long]$sink.hwnd
        focus_probe_pid = [int]$probe.pid
        focus_probe_hwnd = [long]$probe.hwnd
        halt_reason = $haltReason
    }
    $summary | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $runDir 'summary.json') -Encoding UTF8
    Write-Output ('SUMMARY ' + ($summary | ConvertTo-Json -Compress))
} finally {
    $closeIds = @()
    if ($sinkProcess) { $closeIds += [int]$sinkProcess.Id }
    if ($probeProcess) { $closeIds += [int]$probeProcess.Id }
    foreach ($processId in $closeIds) {
        $live = Get-Process -Id $processId -ErrorAction SilentlyContinue
        if ($live) {
            [void]$live.CloseMainWindow()
            if (-not $live.WaitForExit(3000)) {
                Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue
            }
        }
    }
}

exit $resultCode
