$ErrorActionPreference = 'Stop'

$runnerPath = Join-Path $PSScriptRoot 'qualify_dispatch_core.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $runnerPath,
    [ref]$tokens,
    [ref]$parseErrors
)
if ($parseErrors.Count -ne 0) {
    throw ('qualify_dispatch_core.ps1 parser errors: ' + (($parseErrors | ForEach-Object Message) -join '; '))
}

foreach ($scriptName in @('build_dispatch_core_harness.ps1', 'run_native_acceptance_case.ps1', 'native_acceptance_sink.ps1', 'dispatch_harness_manifest.psm1', 'dispatch_qualification_profiles.psm1', 'dispatch_qualification_report.psm1', 'dispatch_qualification_orchestration.psm1', 'lib/playback_core_lifecycle_collector.psm1')) {
    $path = Join-Path $PSScriptRoot $scriptName
    $scriptTokens = $null
    $scriptErrors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $path,
        [ref]$scriptTokens,
        [ref]$scriptErrors
    )
    if ($scriptErrors.Count -ne 0) {
        throw ($scriptName + ' parser errors: ' + (($scriptErrors | ForEach-Object Message) -join '; '))
    }
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixtureRoot = Join-Path $env:TEMP ('native-v3-compat3-contract-' + [guid]::NewGuid().ToString('N'))
$fixtureTestName = 'native_reader_accepts_powershell_v3_compat3_fixture_and_rolling_window'
$previousFixtureRoot = $env:SKY_NATIVE_V3_COMPAT3_FIXTURE_ROOT
try {
    $observerSelfTest = @(& (Get-Command pwsh.exe).Source -NoProfile -File (Join-Path $PSScriptRoot 'native_acceptance_sink.ps1') -SelfTest -SelfTestFixtureRoot $fixtureRoot 2>&1)
    if ($LASTEXITCODE -ne 0 -or ($observerSelfTest -join "`n") -notmatch 'Scheduler observer self-test: PASS') {
        throw ('native_acceptance_sink.ps1 observer self-test failed: ' + ($observerSelfTest -join ' '))
    }
    $fixtureReady = Get-Content -LiteralPath (Join-Path $fixtureRoot 'ready.json') -Raw | ConvertFrom-Json
    $fixtureRecords = @(Get-Content -LiteralPath (Join-Path $fixtureRoot 'events.jsonl') | ForEach-Object { $_ | ConvertFrom-Json })
    if ($fixtureReady.schema_version -ne 3 -or $fixtureReady.event_schema_version -ne 3 -or
        $fixtureReady.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
        $fixtureReady.observer_version -ne 'scheduler-receiveonly-queue-v3-compat3' -or
        $fixtureRecords.Count -ne 5 -or
        (@($fixtureRecords | Where-Object { $_.schema_version -ne 3 -or $_.observer_mode -ne 'SchedulerQueuedV3Compat3' -or $_.observer_version -ne 'scheduler-receiveonly-queue-v3-compat3' }).Count -ne 0) -or
        (@($fixtureRecords | Select-Object -Skip 1 | Where-Object { $_.virtual_key -le 0 -or $_.scan_code -le 0 -or $_.message -notin @(0x0100, 0x0101, 0x0104, 0x0105) }).Count -ne 0)) {
        throw 'native_acceptance_sink.ps1 Compat3 serializer fixture lost schema, provenance, VK, scan, or WM metadata'
    }
    $env:SKY_NATIVE_V3_COMPAT3_FIXTURE_ROOT = $fixtureRoot
    & cargo test --locked --manifest-path (Join-Path $repositoryRoot 'rust\Cargo.toml') -p sky_player --bin rt-native-acceptance --features real-input-acceptance $fixtureTestName
    $crossLanguageExit = $LASTEXITCODE
    if ($crossLanguageExit -ne 0) { throw "Rust V3 Compat3 cross-language fixture test failed with exit $crossLanguageExit" }
} finally {
    if ($null -eq $previousFixtureRoot) {
        Remove-Item Env:SKY_NATIVE_V3_COMPAT3_FIXTURE_ROOT -ErrorAction SilentlyContinue
    } else {
        $env:SKY_NATIVE_V3_COMPAT3_FIXTURE_ROOT = $previousFixtureRoot
    }
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}

$requiredFunctions = @(
    'Get-OptionalProperty',
    'Get-Percentiles',
    'Get-MetricPath',
    'Get-ComparisonDescription',
    'Get-PairedMetrics',
    'Get-DeadlineCrossingToPreCallSample',
    'Get-TimingMetrics',
    'Invoke-NativeCaseFailStopSequence',
    'Assert-SchedulerManifestIdentity',
    'Assert-SchedulerObserverPairMode',
    'Assert-QualificationTimingMargin'
)
foreach ($name in $requiredFunctions) {
    $functionAst = $ast.Find({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true)
    if ($null -eq $functionAst) { throw "missing runner function: $name" }
    Invoke-Expression $functionAst.Extent.Text
}
$caseRunnerPath = Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1'
$caseRunnerTokens = $null
$caseRunnerErrors = $null
$caseRunnerAst = [System.Management.Automation.Language.Parser]::ParseFile(
    $caseRunnerPath,
    [ref]$caseRunnerTokens,
    [ref]$caseRunnerErrors
)
if ($caseRunnerErrors.Count -ne 0) { throw 'run_native_acceptance_case.ps1 parser errors' }
$observerFunctionAst = $caseRunnerAst.Find({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-SchedulerObserverEvidence'
}, $true)
if ($null -eq $observerFunctionAst) { throw 'missing case-runner contract function: Assert-SchedulerObserverEvidence' }
Invoke-Expression $observerFunctionAst.Extent.Text
$sinkIdentityFunctionAst = $caseRunnerAst.Find({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-SinkIdentity'
}, $true)
if ($null -eq $sinkIdentityFunctionAst) { throw 'missing case-runner contract function: Assert-SinkIdentity' }
Invoke-Expression $sinkIdentityFunctionAst.Extent.Text
$m3SessionFunctionAst = $caseRunnerAst.Find({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-DispatchCleanupM3SessionIdentity'
}, $true)
if ($null -eq $m3SessionFunctionAst) { throw 'missing case-runner contract function: Assert-DispatchCleanupM3SessionIdentity' }
Invoke-Expression $m3SessionFunctionAst.Extent.Text
$runId = 'observer-contract-run'
$observerMode = 'SchedulerQueuedV3Compat3'
$observerVersion = 'scheduler-receiveonly-queue-v3-compat3'
$observerReady = [pscustomobject]@{
    schema_version = 3; event_schema_version = 3; run_id = $runId; role = 'ReceiveOnly'
    event_log_id = 'observer-contract-log'; hwnd = 42; pid = 17
    observer_mode = $observerMode
    observer_version = $observerVersion; observer_queue_capacity = 32768
    observer_qpc_frequency_hz = 10000000
}
$observerHeader = [pscustomobject]@{
    schema_version = 3; run_id = $runId; role = 'ReceiveOnly'; event_log_id = $observerReady.event_log_id
    observer_mode = $observerMode
    observer_version = $observerVersion; sequence = 0; kind = 'stream_start'; observed_qpc_ticks = 100
    scan_code = 0; extended = $false; virtual_key = 0; message = 0; observed_utc = '2026-01-01T00:00:00Z'
    target_hwnd = 42; target_pid = 17; foreground_hwnd = 35; foreground_pid = 12
}
$observerDown = [pscustomobject]@{
    schema_version = 3; run_id = $runId; role = 'ReceiveOnly'; event_log_id = $observerReady.event_log_id
    observer_mode = $observerMode
    observer_version = $observerVersion; sequence = 1; kind = 'key_press'; observed_qpc_ticks = 101
    scan_code = 0x1E; extended = $false; virtual_key = 0x41; message = 0x0100; observed_utc = '2026-01-01T00:00:00.0000001Z'
    target_hwnd = 42; target_pid = 17; foreground_hwnd = 42; foreground_pid = 17
}
$observerUp = [pscustomobject]@{
    schema_version = 3; run_id = $runId; role = 'ReceiveOnly'; event_log_id = $observerReady.event_log_id
    observer_mode = $observerMode
    observer_version = $observerVersion; sequence = 2; kind = 'key_release'; observed_qpc_ticks = 102
    scan_code = 0x1E; extended = $false; virtual_key = 0x41; message = 0x0101; observed_utc = '2026-01-01T00:00:00.0000002Z'
    target_hwnd = 42; target_pid = 17; foreground_hwnd = 42; foreground_pid = 17
}
$observerSysDown = [pscustomobject]@{
    schema_version = 3; run_id = $runId; role = 'ReceiveOnly'; event_log_id = $observerReady.event_log_id
    observer_mode = $observerMode
    observer_version = $observerVersion; sequence = 3; kind = 'key_press'; observed_qpc_ticks = 103
    scan_code = 0x38; extended = $false; virtual_key = 0x12; message = 0x0104; observed_utc = '2026-01-01T00:00:00.0000003Z'
    target_hwnd = 42; target_pid = 17; foreground_hwnd = 42; foreground_pid = 17
}
$observerSysUp = [pscustomobject]@{
    schema_version = 3; run_id = $runId; role = 'ReceiveOnly'; event_log_id = $observerReady.event_log_id
    observer_mode = $observerMode
    observer_version = $observerVersion; sequence = 4; kind = 'key_release'; observed_qpc_ticks = 104
    scan_code = 0x38; extended = $false; virtual_key = 0x12; message = 0x0105; observed_utc = '2026-01-01T00:00:00.0000004Z'
    target_hwnd = 42; target_pid = 17; foreground_hwnd = 42; foreground_pid = 17
}
$observerRecords = @($observerHeader, $observerDown, $observerUp, $observerSysDown, $observerSysUp)
$observerSummary = [pscustomobject]@{
    schema_version = 1; observer_mode = $observerMode; observer_version = $observerVersion; run_id = $runId; role = 'ReceiveOnly'
    event_log_id = $observerReady.event_log_id; target_hwnd = 42; target_pid = 17
    qpc_frequency_hz = 10000000; queue_capacity = 32768; events_seen = 4; events_enqueued = 4
    events_written = 4; queue_dropped = 0; queue_high_water = 4; queue_overflow = $false
    handler_latency_count = 4; handler_latency_sample_count = 4; handler_latency_sample_dropped = 0
    handler_latency_p50_us = 2; handler_latency_p95_us = 3; handler_latency_p99_us = 4
    handler_latency_max_us = 5; writer_failures = 0; observer_internal_failures = 0
    observer_startup_failures = 0; stream_header_flushed = $true; events_flushed = $true
    consumer_stopped = $true; queue_drained = $true
}
Assert-SchedulerObserverEvidence -Ready $observerReady -Summary $observerSummary -Records $observerRecords -ExpectedObserverVersion $observerVersion

$identityTestRoot = Join-Path $env:TEMP ('observer-identity-contract-' + [guid]::NewGuid().ToString('N'))
$eventsPath = Join-Path $identityTestRoot 'events.jsonl'
$pwshExecutablePath = 'C:\contract\pwsh.exe'
$identityProcessStart = [long][DateTime]::UtcNow.ToFileTimeUtc()
$script:mockSinkOwner = [pscustomobject]@{
    Path = $pwshExecutablePath; SessionId = 1; StartTime = [DateTime]::FromFileTimeUtc($identityProcessStart)
    MainWindowHandle = [long]42; MainWindowTitle = 'Sky Auto Player — Native Acceptance Sink'
}
$priorGetProcessFunction = Get-Item Function:\Get-Process -ErrorAction SilentlyContinue
function Get-Process {
    param([int]$Id)
    return $script:mockSinkOwner
}
try {
    New-Item -ItemType Directory -Path $identityTestRoot | Out-Null
    $observerHeader | ConvertTo-Json -Compress | Set-Content -LiteralPath $eventsPath -Encoding UTF8
    $sinkReady = [pscustomobject]@{
        schema_version = 3; event_schema_version = 3; run_id = $runId; role = 'ReceiveOnly'
        sink_kind = 'SkyAutoPlayer.NativeAcceptanceSink'; title = 'Sky Auto Player — Native Acceptance Sink'
        target_class = 'project-owned-non-game-receive-only'; target_name = 'Sky Auto Player — Native Acceptance Sink'; game_target = $false
        process = 'native_acceptance_sink.ps1'; input_policy = 'receives_only; benchmark must use SendInput'
        pid = 17; hwnd = 42; event_log_id = $observerReady.event_log_id
        process_start_time_filetime = $identityProcessStart
        observer_mode = $observerMode; observer_version = $observerVersion
        observer_queue_capacity = 32768; observer_qpc_frequency_hz = 10000000
    }
    Assert-SinkIdentity -Record $sinkReady -SessionId 1 -ExpectedObserverVersion $observerVersion
    $wrongTargetClass = $sinkReady | Select-Object *
    $wrongTargetClass.target_class = 'game'
    $wrongTargetClassRejected = $false
    try { Assert-SinkIdentity -Record $wrongTargetClass -SessionId 1 -ExpectedObserverVersion $observerVersion } catch { $wrongTargetClassRejected = $true }
    if (-not $wrongTargetClassRejected) { throw 'ReceiveOnly sink accepted game target class metadata' }
    $gameTargetMarked = $sinkReady | Select-Object *
    $gameTargetMarked.game_target = $true
    $gameTargetRejected = $false
    try { Assert-SinkIdentity -Record $gameTargetMarked -SessionId 1 -ExpectedObserverVersion $observerVersion } catch { $gameTargetRejected = $true }
    if (-not $gameTargetRejected) { throw 'ReceiveOnly sink accepted game_target=true metadata' }
    $missingGameTarget = $sinkReady | Select-Object *
    [void]$missingGameTarget.PSObject.Properties.Remove('game_target')
    $missingGameTargetRejected = $false
    try { Assert-SinkIdentity -Record $missingGameTarget -SessionId 1 -ExpectedObserverVersion $observerVersion } catch { $missingGameTargetRejected = $true }
    if (-not $missingGameTargetRejected) { throw 'ReceiveOnly sink accepted missing game_target identity' }

    $wrongIdentityMode = $observerHeader | Select-Object *
    $wrongIdentityMode.observer_mode = 'SchedulerQueuedV2'
    $wrongIdentityMode | ConvertTo-Json -Compress | Set-Content -LiteralPath $eventsPath -Encoding UTF8
    $identityModeRejected = $false
    try { Assert-SinkIdentity -Record $sinkReady -SessionId 1 -ExpectedObserverVersion $observerVersion } catch { $identityModeRejected = $true }
    if (-not $identityModeRejected) { throw 'pre-arm sink accepted a mismatched observer mode' }

    $wrongIdentitySchema = $observerHeader | Select-Object *
    $wrongIdentitySchema.schema_version = 4
    $wrongIdentitySchema | ConvertTo-Json -Compress | Set-Content -LiteralPath $eventsPath -Encoding UTF8
    $identitySchemaRejected = $false
    try { Assert-SinkIdentity -Record $sinkReady -SessionId 1 -ExpectedObserverVersion $observerVersion } catch { $identitySchemaRejected = $true }
    if (-not $identitySchemaRejected) { throw 'pre-arm sink accepted an incompatible event schema' }
} finally {
    if ($null -eq $priorGetProcessFunction) {
        Remove-Item Function:\Get-Process -ErrorAction SilentlyContinue
    } else {
        Set-Item Function:\Get-Process -Value $priorGetProcessFunction.ScriptBlock
    }
    if (Test-Path -LiteralPath $identityTestRoot) { Remove-Item -LiteralPath $identityTestRoot -Recurse -Force }
    Remove-Variable mockSinkOwner -Scope Script -ErrorAction SilentlyContinue
}
$assertObserverRejected = {
    param($Ready, $Summary, [object[]]$Records, [string]$ExpectedVersion, [string]$CaseName)
    $rejected = $false
    try { Assert-SchedulerObserverEvidence -Ready $Ready -Summary $Summary -Records $Records -ExpectedObserverVersion $ExpectedVersion } catch { $rejected = $true }
    if (-not $rejected) { throw ('Scheduler observer accepted invalid ' + $CaseName + ' evidence') }
}
$gapRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$gapRecords[4].sequence = 5
& $assertObserverRejected $observerReady $observerSummary $gapRecords $observerVersion 'sequence gap'
$duplicateRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$duplicateRecords[4].sequence = 3
& $assertObserverRejected $observerReady $observerSummary $duplicateRecords $observerVersion 'duplicate sequence'
$reversedRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$reversedRecords[4].observed_qpc_ticks = 99
& $assertObserverRejected $observerReady $observerSummary $reversedRecords $observerVersion 'backward timestamp'
$wrongMessageOrderRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongMessageOrderRecords[1].message = 0x0101
& $assertObserverRejected $observerReady $observerSummary $wrongMessageOrderRecords $observerVersion 'message and direction order'
$wrongVirtualKeyRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongVirtualKeyRecords[1].virtual_key = 0
& $assertObserverRejected $observerReady $observerSummary $wrongVirtualKeyRecords $observerVersion 'zero virtual key'
$wrongScanRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongScanRecords[1].scan_code = 0
& $assertObserverRejected $observerReady $observerSummary $wrongScanRecords $observerVersion 'zero scan code'
$missingVirtualKeyRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$missingVirtualKeyRecords[1].PSObject.Properties.Remove('virtual_key')
& $assertObserverRejected $observerReady $observerSummary $missingVirtualKeyRecords $observerVersion 'missing virtual key metadata'
$missingMessageRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$missingMessageRecords[1].PSObject.Properties.Remove('message')
& $assertObserverRejected $observerReady $observerSummary $missingMessageRecords $observerVersion 'missing message metadata'
$missingObserverRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$missingObserverRecords[1].PSObject.Properties.Remove('observer_version')
& $assertObserverRejected $observerReady $observerSummary $missingObserverRecords $observerVersion 'missing per-event observer version'
$wrongModeRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongModeRecords[1].observer_mode = 'SchedulerQueuedV2'
& $assertObserverRejected $observerReady $observerSummary $wrongModeRecords $observerVersion 'mixed observer mode'
$wrongEventRunRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongEventRunRecords[1].run_id = 'other-run'
& $assertObserverRejected $observerReady $observerSummary $wrongEventRunRecords $observerVersion 'wrong event run ID'
$wrongEventLogRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongEventLogRecords[1].event_log_id = 'other-stream'
& $assertObserverRejected $observerReady $observerSummary $wrongEventLogRecords $observerVersion 'wrong event log ID'
$overflowSummary = $observerSummary | Select-Object *
$overflowSummary.queue_overflow = $true; $overflowSummary.queue_dropped = 1
& $assertObserverRejected $observerReady $overflowSummary $observerRecords $observerVersion 'overflow and dropped-event counters'
$writerFailureSummary = $observerSummary | Select-Object *
$writerFailureSummary.writer_failures = 1
& $assertObserverRejected $observerReady $writerFailureSummary $observerRecords $observerVersion 'writer failure'
$incompleteDrainSummary = $observerSummary | Select-Object *
$incompleteDrainSummary.events_flushed = $false; $incompleteDrainSummary.queue_drained = $false
& $assertObserverRejected $observerReady $incompleteDrainSummary $observerRecords $observerVersion 'incomplete writer drain'
$wrongTargetRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongTargetRecords[1].target_hwnd = 43
& $assertObserverRejected $observerReady $observerSummary $wrongTargetRecords $observerVersion 'wrong target HWND'
$wrongForegroundRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongForegroundRecords[1].foreground_pid = 18
& $assertObserverRejected $observerReady $observerSummary $wrongForegroundRecords $observerVersion 'wrong foreground PID'
$wrongEventSchemaRecords = @($observerRecords | ForEach-Object { $_ | Select-Object * })
$wrongEventSchemaRecords[1].schema_version = 4
& $assertObserverRejected $observerReady $observerSummary $wrongEventSchemaRecords $observerVersion 'event schema version'
$wrongReadySchema = $observerReady | Select-Object *
$wrongReadySchema.event_schema_version = 4
& $assertObserverRejected $wrongReadySchema $observerSummary $observerRecords $observerVersion 'ready event schema version'
$wrongReadyEnvelope = $observerReady | Select-Object *
$wrongReadyEnvelope.schema_version = 4
& $assertObserverRejected $wrongReadyEnvelope $observerSummary $observerRecords $observerVersion 'ready envelope schema 4'
$legacyReady = $observerReady | Select-Object *
$legacyReady.PSObject.Properties.Remove('observer_mode')
$legacyReady.PSObject.Properties.Remove('observer_version')
& $assertObserverRejected $legacyReady $observerSummary $observerRecords $observerVersion 'LegacyInline schema-3 masquerade'
$historicalV2Ready = $observerReady | Select-Object *
$historicalV2Ready.event_schema_version = 4
$historicalV2Ready.observer_mode = 'SchedulerQueuedV2'
$historicalV2Ready.observer_version = 'scheduler-receiveonly-queue-v2'
& $assertObserverRejected $historicalV2Ready $observerSummary $observerRecords $observerVersion 'historical v2 schema-4 ready'
$wrongVersionSummary = $observerSummary | Select-Object *
$wrongVersionSummary.observer_version = 'scheduler-receiveonly-queue-v2'
& $assertObserverRejected $observerReady $wrongVersionSummary $observerRecords $observerVersion 'historical v2 observer version'
Import-Module (Join-Path $PSScriptRoot 'dispatch_pair_screen.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Force

$runnerSource = Get-Content -LiteralPath $runnerPath -Raw
if ([regex]::Matches($runnerSource, 'Invoke-NativeCaseFailStopSequence').Count -lt 3) {
    throw 'Smoke and Final must both use the tested fail-stop sequence helper'
}
$finalSafetyBlocks = [regex]::Matches($runnerSource, '(?s)\$finalSafetyCases\s*=\s*@\((.*?)\n\s*\)\s*\n\s*\$sequenceResult', [System.Text.RegularExpressions.RegexOptions]::Singleline)
$finalSafetyBlock = if ($finalSafetyBlocks.Count -gt 0) { $finalSafetyBlocks[$finalSafetyBlocks.Count - 1] } else { [System.Text.RegularExpressions.Match]::Empty }
if (-not $finalSafetyBlock.Success -or
    $finalSafetyBlock.Groups[1].Value -notmatch "foreach \(\`$load in @\('Quiet', 'Contention'\)\)" -or
    $finalSafetyBlock.Groups[1].Value -notmatch "load = \`$load" -or
    $finalSafetyBlock.Groups[1].Value -notmatch "boundary-60fps-margin-0" -or
    $finalSafetyBlock.Groups[1].Value -notmatch "boundary-120fps-margin-500" -or
    $runnerSource -notmatch '-Load \$case\.load') {
    throw 'Final safety case construction, loads, boundaries, or case-specific load dispatch changed'
}
$finalScenarioNames = @(
    'canonical-single', 'canonical-max-chord', 'near-minimum-retrigger', 'release-gap-stress',
    'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary',
    'modifier-held-after-owned', 'focus-loss', 'target-hwnd-change', 'owner-mismatch',
    'owner-query-failure', 'owner-process-termination', 'pause-resume', 'pause-resume-release-floor',
    'suspend-resume', 'stop-cleanup', 'skip-cleanup', 'supervisor-lease-expiry',
    'lease-age-watchdog-delayed', 'noncanonical-profile'
)
$finalModel = @(
    foreach ($load in @('Quiet', 'Contention')) {
        foreach ($scenario in $finalScenarioNames) { [pscustomobject]@{ scenario = $scenario; load = $load } }
    }
    [pscustomobject]@{ scenario = 'timing-margin-sweep'; workload = 'boundary-60fps-margin-0'; load = 'Quiet'; margin = 0; fps = 60 }
    [pscustomobject]@{ scenario = 'timing-margin-sweep'; workload = 'boundary-120fps-margin-500'; load = 'Quiet'; margin = 500; fps = 120 }
)
$safetyModel = @($finalModel | Where-Object { $_.scenario -ne 'timing-margin-sweep' })
if ($finalModel.Count -ne 46 -or @($safetyModel | Where-Object load -eq 'Quiet').Count -ne 22 -or
    @($safetyModel | Where-Object load -eq 'Contention').Count -ne 22 -or
    @($safetyModel | Group-Object load | Where-Object { $_.Count -ne 22 -or @($_.Group.scenario | Select-Object -Unique).Count -ne 22 }).Count -ne 0 -or
    $finalModel[44].workload -ne 'boundary-60fps-margin-0' -or $finalModel[44].margin -ne 0 -or $finalModel[44].fps -ne 60 -or
    $finalModel[45].workload -ne 'boundary-120fps-margin-500' -or $finalModel[45].margin -ne 500 -or $finalModel[45].fps -ne 120) {
    throw 'Final model must contain 22 unique scenarios per load and the two fixed boundaries'
}
$scenarioBlock = [regex]::Match($runnerSource, '(?s)\$scenarios\s*=\s*@\((.*?)\n\s*\)')
$runnerScenarioNames = if ($scenarioBlock.Success) {
    @([regex]::Matches($scenarioBlock.Groups[1].Value, "'([^']+)'") | ForEach-Object { $_.Groups[1].Value })
} else { @() }
if ($runnerScenarioNames.Count -ne $finalScenarioNames.Count -or
    (@(Compare-Object -ReferenceObject $finalScenarioNames -DifferenceObject $runnerScenarioNames -SyncWindow 0)).Count -ne 0) {
    throw 'Final safety scenarios differ from the approved 22-scenario inventory'
}
$acceptanceSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1') -Raw
foreach ($requiredSchedulerLabel in @('Scheduler-Baseline', 'Scheduler-Candidate')) {
    if (-not $acceptanceSource.Contains($requiredSchedulerLabel)) { throw "Missing Scheduler metadata label: $requiredSchedulerLabel" }
}
if (-not $acceptanceSource.Contains("elseif (`$HistoricalSourceAllowed) { 'C0' } else { 'A' }")) {
    throw 'DispatchCore or historical harness label fallback changed'
}

$schedulerBaseRevision = 'b9ed666e3ddddc4952831626f1654e1cc9071a10'
$schedulerBaseTree = 'f554c41b23a3969d02ed29842d4178f6143c9d86'
$validSchedulerBaseline = [pscustomobject]@{
    qualification_profile = 'Scheduler'
    source_role = 'Baseline'
    source_tree_clean = $true
    base_source_revision = $schedulerBaseRevision
    base_source_tree_sha = $schedulerBaseTree
}
Assert-SchedulerManifestIdentity -Manifest $validSchedulerBaseline -Role 'control' -ExpectedBaseRevision $schedulerBaseRevision -ExpectedBaseTree $schedulerBaseTree
$validSchedulerCandidate = [pscustomobject]@{
    qualification_profile = 'Scheduler'
    source_role = 'Candidate'
    source_tree_clean = $true
    base_source_revision = $schedulerBaseRevision
    base_source_tree_sha = $schedulerBaseTree
}
Assert-SchedulerManifestIdentity -Manifest $validSchedulerCandidate -Role 'candidate' -ExpectedBaseRevision $schedulerBaseRevision -ExpectedBaseTree $schedulerBaseTree
$rejectedWrongRole = $false
try {
    Assert-SchedulerManifestIdentity -Manifest $validSchedulerBaseline -Role 'candidate' -ExpectedBaseRevision $schedulerBaseRevision -ExpectedBaseTree $schedulerBaseTree
} catch { $rejectedWrongRole = $true }
if (-not $rejectedWrongRole) { throw 'Scheduler accepted a baseline manifest as the candidate role' }
$wrongSchedulerBase = [pscustomobject]@{
    qualification_profile = 'Scheduler'
    source_role = 'Baseline'
    source_tree_clean = $true
    base_source_revision = '0000000000000000000000000000000000000000'
    base_source_tree_sha = $schedulerBaseTree
}
$rejectedWrongBase = $false
try {
    Assert-SchedulerManifestIdentity -Manifest $wrongSchedulerBase -Role 'control' -ExpectedBaseRevision $schedulerBaseRevision -ExpectedBaseTree $schedulerBaseTree
} catch { $rejectedWrongBase = $true }
if (-not $rejectedWrongBase) { throw 'Scheduler accepted a control manifest with an unpinned base revision' }
$caseRunnerSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1') -Raw
if (-not $caseRunnerSource.Contains("Assert-C1TimedControlProvenance -Manifest `$harnessManifest") -or
    -not $runnerSource.Contains("Assert-C1TimedControlProvenance -Manifest `$manifest")) {
    throw 'Scheduler profile changes removed the established #646 provenance validation path'
}
if (-not $caseRunnerSource.Contains("if (`$receiveOnlyObserverVersion -and `$Scenario -ne 'owner-process-termination')")) {
    throw 'owner-process-termination must not require a ReceiveOnly observer shutdown summary after terminating that observer process'
}

$frameBaseRevision = '8a800ff07a5dc362a5ee3aaa59204ff40b6ae327'
$frameBaseTree = '48eec424c155e98fa1a81ad018bc832b6e6b964f'
$validFrameCandidate = [pscustomobject]@{
    qualification_profile = 'FrameSnapshot'; source_role = 'Candidate'; source_branch = 'refactor/frame-snapshot-dispatch'
    source_tree_clean = $true; clean_build = $true; source_revision = ('a' * 40); source_tree_sha = ('b' * 40)
    base_source_revision = $frameBaseRevision; base_source_tree_sha = $frameBaseTree; workload_version = 'frame-snapshot-v1'
    receiveonly_observer_mode = 'SchedulerQueuedV3Compat3'; receiveonly_observer_version = 'scheduler-receiveonly-queue-v3-compat3'
    performance_timing_margin_us = 0; safety_timing_margin_us = 500; target = 'x86_64-pc-windows-msvc'; binary_sha256 = ('c' * 64)
}
$validFrameBaseline = $validFrameCandidate | Select-Object *
$validFrameBaseline.source_role = 'Baseline'; $validFrameBaseline.source_branch = ''
$validFrameBaseline.source_revision = $frameBaseRevision; $validFrameBaseline.source_tree_sha = $frameBaseTree
Assert-FrameSnapshotManifestIdentity -Manifest $validFrameBaseline -ExpectedRole Baseline
Assert-FrameSnapshotManifestIdentity -Manifest $validFrameCandidate -ExpectedRole Candidate

$frameInvalidCases = @(
    @{ name = 'branch'; mutate = { param($m) $m.source_branch = 'wrong-branch' } },
    @{ name = 'base'; mutate = { param($m) $m.base_source_revision = ('0' * 40) } },
    @{ name = 'source hash'; mutate = { param($m) $m.source_tree_sha = 'not-a-tree-hash' } },
    @{ name = 'role'; mutate = { param($m) $m.source_role = 'Baseline' } },
    @{ name = 'dirty source'; mutate = { param($m) $m.source_tree_clean = $false } }
)
foreach ($case in $frameInvalidCases) {
    $invalidFrame = $validFrameCandidate | Select-Object *
    & $case.mutate $invalidFrame
    $rejected = $false
    try { Assert-FrameSnapshotManifestIdentity -Manifest $invalidFrame -ExpectedRole Candidate } catch { $rejected = $true }
    if (-not $rejected) { throw ('FrameSnapshot accepted invalid manifest ' + $case.name) }
}
$wrongFramePinnedHash = $validFrameBaseline | Select-Object *
$wrongFramePinnedHash.source_tree_sha = ('0' * 40)
$wrongFrameHashRejected = $false
try { Assert-FrameSnapshotManifestIdentity -Manifest $wrongFramePinnedHash -ExpectedRole Baseline } catch { $wrongFrameHashRejected = $true }
if (-not $wrongFrameHashRejected) { throw 'FrameSnapshot accepted a baseline source tree hash other than the pinned tree' }

$frameBuilderSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1') -Raw
$m3ProfileSet = "ValidateSet\('DispatchCore', 'Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle'\)"
if ($frameBuilderSource -notmatch $m3ProfileSet -or
    $acceptanceSource -notmatch $m3ProfileSet) {
    throw 'FrameSnapshot profile is missing from an immutable build or case-runner boundary'
}
$qualifierSource = Get-Content -LiteralPath $runnerPath -Raw
if ($qualifierSource -notmatch $m3ProfileSet) {
    throw 'FrameSnapshot or DispatchCleanupM3 profile is missing from qualification runner'
}
Assert-QualificationTimingMargin -RequestedStage Canary -Profile FrameSnapshot -MarginUs 500 -PairCount 1 -SampleDurationSeconds 0 | Out-Null
Assert-QualificationTimingMargin -RequestedStage Smoke -Profile FrameSnapshot -MarginUs 500 -PairCount 1 -SampleDurationSeconds 0 | Out-Null
Assert-QualificationTimingMargin -RequestedStage Compare -Profile FrameSnapshot -MarginUs 0 -PairCount 6 -SampleDurationSeconds 15 | Out-Null
Assert-QualificationTimingMargin -RequestedStage Final -Profile FrameSnapshot -MarginUs 0 -PairCount 6 -SampleDurationSeconds 15 | Out-Null
Assert-QualificationTimingMargin -RequestedStage Compare -Profile Scheduler -MarginUs 500 -PairCount 6 -SampleDurationSeconds 15 | Out-Null
$wrongFrameMarginRejected = $false
try { Assert-QualificationTimingMargin -RequestedStage Compare -Profile FrameSnapshot -MarginUs 500 -PairCount 6 -SampleDurationSeconds 15 } catch { $wrongFrameMarginRejected = $true }
if (-not $wrongFrameMarginRejected) { throw 'FrameSnapshot Compare accepted a nonzero performance margin' }
$wrongFrameCanaryRejected = $false
try { Assert-QualificationTimingMargin -RequestedStage Canary -Profile FrameSnapshot -MarginUs 0 -PairCount 1 -SampleDurationSeconds 0 } catch { $wrongFrameCanaryRejected = $true }
if (-not $wrongFrameCanaryRejected) { throw 'FrameSnapshot safety canary accepted a non-safety margin' }
Assert-QualificationTimingMargin -RequestedStage Canary -Profile DispatchCleanupM3 -MarginUs 500 -PairCount 1 -SampleDurationSeconds 0 | Out-Null
Assert-QualificationTimingMargin -RequestedStage Smoke -Profile DispatchCleanupM3 -MarginUs 500 -PairCount 1 -SampleDurationSeconds 0 | Out-Null
Assert-QualificationTimingMargin -RequestedStage Compare -Profile DispatchCleanupM3 -MarginUs 0 -PairCount 6 -SampleDurationSeconds 15 | Out-Null
Assert-QualificationTimingMargin -RequestedStage Final -Profile DispatchCleanupM3 -MarginUs 0 -PairCount 6 -SampleDurationSeconds 15 | Out-Null
$wrongM3PerformanceMarginRejected = $false
try { Assert-QualificationTimingMargin -RequestedStage Compare -Profile DispatchCleanupM3 -MarginUs 500 -PairCount 6 -SampleDurationSeconds 15 } catch { $wrongM3PerformanceMarginRejected = $true }
if (-not $wrongM3PerformanceMarginRejected) { throw 'DispatchCleanupM3 Compare accepted a nonzero performance margin' }
$wrongM3SafetyMarginRejected = $false
try { Assert-QualificationTimingMargin -RequestedStage Canary -Profile DispatchCleanupM3 -MarginUs 0 -PairCount 1 -SampleDurationSeconds 0 } catch { $wrongM3SafetyMarginRejected = $true }
if (-not $wrongM3SafetyMarginRejected) { throw 'DispatchCleanupM3 Canary accepted a non-safety margin' }
if ($qualifierSource -notmatch "ValidateSet\('Canary', 'Smoke', 'Compare', 'Final'\)" -or
    -not $qualifierSource.Contains("if (`$Stage -eq 'Canary' -and `$QualificationProfile -notin @('FrameSnapshot', 'DispatchCleanupM3'))")) {
    throw 'FrameSnapshot or DispatchCleanupM3 canary stage is missing its profile boundary'
}

Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_profiles.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_report.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_orchestration.psm1') -Force
if (-not (Get-Command Get-DispatchCleanupM3Profile -ErrorAction SilentlyContinue)) {
    throw 'DispatchCleanupM3 profile helper must remain available after manifest module initialization'
}
$manifestImportPattern = 'Import-Module (Join-Path $PSScriptRoot ''dispatch_harness_manifest.psm1'') -Force'
$profileImportPattern = 'Import-Module (Join-Path $PSScriptRoot ''dispatch_qualification_profiles.psm1'') -Force'
foreach ($scriptPath in @(
        (Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1'),
        (Join-Path $PSScriptRoot 'qualify_dispatch_core.ps1'),
        (Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1')
    )) {
    $manifestImportLine = (Select-String -LiteralPath $scriptPath -SimpleMatch -Pattern $manifestImportPattern | Select-Object -First 1).LineNumber
    $profileImportLine = (Select-String -LiteralPath $scriptPath -SimpleMatch -Pattern $profileImportPattern | Select-Object -First 1).LineNumber
    if (-not $manifestImportLine -or -not $profileImportLine -or $profileImportLine -le $manifestImportLine) {
        throw ('DispatchCleanupM3 caller imports its public profile helper before initializing the manifest module: ' + $scriptPath)
    }
}
$m3Profile = Get-DispatchCleanupM3Profile
$m3RunnerGit = Get-DispatchM3GitCheckoutIdentity $repositoryRoot
$m3RemoteRows = @(& git -C $repositoryRoot ls-remote origin ('refs/heads/' + $m3Profile.CandidateLineageBranch))
if ($LASTEXITCODE -ne 0 -or $m3RemoteRows.Count -ne 1) { throw 'could not resolve the current M3 runner branch tip for identity contracts' }
$m3LineageTip = ([string]$m3RemoteRows[0] -split '\s+')[0]
& git -C $repositoryRoot merge-base --is-ancestor $m3Profile.CandidateRevision $m3LineageTip
if ($LASTEXITCODE -ne 0) { throw 'accepted M2 candidate is not an ancestor of the observed runner branch tip' }
# The DispatchCleanupM3 parser fixtures model their historical runner identity.
# The checked-out branch now hosts DispatchSessionRedesignM3, so its live branch
# name must not be substituted into a legacy pinned artifact fixture.
$m3RunnerTree = (& git -C $repositoryRoot rev-parse ($m3LineageTip + '^{tree}')).Trim()
if ($LASTEXITCODE -ne 0) { throw 'could not resolve the historical DispatchCleanupM3 runner tree for artifact fixtures' }
$m3RunnerGit.revision = $m3LineageTip
$m3RunnerGit.tree = $m3RunnerTree
$m3RunnerGit.branch = $m3Profile.CandidateLineageBranch
$m3RunnerGit.checkout.revision = $m3LineageTip
$m3RunnerGit.checkout.tree = $m3RunnerTree
$m3RunnerGit.checkout.branch = $m3Profile.CandidateLineageBranch
$m3RunnerCheckout = [pscustomobject]@{
    root = $repositoryRoot; top_level = $m3RunnerGit.checkout.top_level; git_dir = $m3RunnerGit.checkout.git_dir
    git_common_dir = $m3RunnerGit.checkout.git_common_dir; revision = $m3RunnerGit.revision
    tree = $m3RunnerGit.tree; branch = $m3RunnerGit.branch; clean = $true
}
$m3RunnerIdentityFixture = [pscustomobject]@{
    root = $repositoryRoot; revision = $m3RunnerGit.revision; tree = $m3RunnerGit.tree; branch = $m3RunnerGit.branch
    clean = $true; candidate_lineage_branch = $m3Profile.CandidateLineageBranch
    candidate_lineage_tip_revision = $m3LineageTip; candidate_lineage_verified = $true; checkout = $m3RunnerCheckout
}
$m3ContractRoot = Join-Path $env:TEMP ('dispatch-cleanup-m3-contract-' + [guid]::NewGuid().ToString('N'))
$m3CandidateSourceRoot = Join-Path $m3ContractRoot 'candidate-worktree'
$m3BaselineSourceRoot = Join-Path $m3ContractRoot 'baseline-worktree'
$m3OutputRoot = Join-Path $m3ContractRoot 'candidate-output'
$m3RunnerPaths = @($m3Profile.RunnerProvenancePaths)
$m3RunnerScriptProvenance = @(
    foreach ($relativePath in $m3RunnerPaths) {
        $actualPath = Join-Path $repositoryRoot ($relativePath.Replace('/', '\'))
        [pscustomobject]@{
            source_path = $relativePath
            evidence_path = 'runner-provenance/' + $relativePath.Replace('/', '\')
            sha256 = (Get-FileHash -LiteralPath $actualPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
)
$m3CandidateSourceCheckout = [pscustomobject]@{
    root = $m3CandidateSourceRoot; top_level = $m3CandidateSourceRoot
    git_dir = (Join-Path $m3ContractRoot 'git-worktrees\candidate'); git_common_dir = $m3RunnerGit.checkout.git_common_dir
    revision = $m3Profile.CandidateRevision; tree = $m3Profile.CandidateTree; branch = ''; clean = $true
}
$validM3Candidate = [pscustomobject]@{
    schema_version = 1; qualification_profile = $m3Profile.Name; workload_version = $m3Profile.WorkloadVersion
    source_role = 'Candidate'; source_revision = $m3Profile.CandidateRevision; source_tree_sha = $m3Profile.CandidateTree
    source_branch = ''; source_tree_clean = $true; clean_build = $true
    base_source_revision = $m3Profile.BaselineRevision; base_source_tree_sha = $m3Profile.BaselineTree
    baseline_source_revision = $m3Profile.BaselineRevision; baseline_source_tree_sha = $m3Profile.BaselineTree
    candidate_source_revision = $m3Profile.CandidateRevision; candidate_source_tree_sha = $m3Profile.CandidateTree
    source_root = $m3CandidateSourceRoot; source_checkout_identity = $m3CandidateSourceCheckout
    cargo_profile = 'dist'; cargo_features = @('real-input-acceptance'); qpc_read_counting = $false
    observer_mode = 'qualification-on'; receiveonly_observer_mode = $m3Profile.ObserverMode
    receiveonly_observer_version = $m3Profile.ObserverVersion
    receiveonly_observer_schema_version = 3; receiveonly_observer_event_schema_version = 3
    guard_mode = 'frozen'; performance_timing_margin_us = 0; safety_timing_margin_us = 500
    target = $m3Profile.Target; target_identity = $m3Profile.Target
    rustc_version = "rustc 1.99.1 (contract)`nrelease: 1.99.1`nhost: x86_64-pc-windows-msvc"
    cargo_version = 'cargo 1.99.1 (contract)'; cargo_clean_exit_code = 0; cargo_build_exit_code = 0
    binary_name = 'rt-native-acceptance.exe'; binary_sha256 = ('c' * 64)
    builder_script_sha256 = [string](@($m3RunnerScriptProvenance | Where-Object source_path -eq 'scripts/build_dispatch_core_harness.ps1')[0].sha256)
    runner_source_revision = $m3RunnerGit.revision; runner_source_tree_sha = $m3RunnerGit.tree
    runner_source_branch = $m3RunnerGit.branch; runner_source_root = $repositoryRoot; runner_source_clean = $true
    runner_source_checkout_identity = $m3RunnerCheckout
    candidate_lineage_branch = $m3Profile.CandidateLineageBranch
    candidate_lineage_tip_revision = $m3LineageTip; candidate_lineage_verified = $true
    runner_script_provenance = $m3RunnerScriptProvenance
    output_root = $m3OutputRoot; cargo_target_dir = (Join-Path $m3OutputRoot 'cargo-target')
}
Assert-DispatchCleanupM3ManifestIdentity -Manifest $validM3Candidate -ExpectedRole Candidate -RepositoryRoot $repositoryRoot | Out-Null
$crlfM3Candidate = $validM3Candidate | ConvertTo-Json -Depth 12 | ConvertFrom-Json
$crlfM3Candidate.rustc_version = $crlfM3Candidate.rustc_version.Replace("`n", "`r`n")
Assert-DispatchCleanupM3ManifestIdentity -Manifest $crlfM3Candidate -ExpectedRole Candidate -RepositoryRoot $repositoryRoot | Out-Null
Assert-DispatchCleanupM3RunnerProvenance -Manifest $validM3Candidate -RunnerIdentity $m3RunnerIdentityFixture -RunnerRoot $repositoryRoot | Out-Null
$validM3Baseline = $validM3Candidate | ConvertTo-Json -Depth 12 | ConvertFrom-Json
$validM3Baseline.source_role = 'Baseline'
$validM3Baseline.source_revision = $m3Profile.BaselineRevision
$validM3Baseline.source_tree_sha = $m3Profile.BaselineTree
$validM3Baseline.source_root = $m3BaselineSourceRoot
$validM3Baseline.source_checkout_identity.root = $m3BaselineSourceRoot
$validM3Baseline.source_checkout_identity.top_level = $m3BaselineSourceRoot
$validM3Baseline.source_checkout_identity.git_dir = Join-Path $m3ContractRoot 'git-worktrees\baseline'
$validM3Baseline.source_checkout_identity.revision = $m3Profile.BaselineRevision
$validM3Baseline.source_checkout_identity.tree = $m3Profile.BaselineTree
Assert-DispatchCleanupM3ManifestIdentity -Manifest $validM3Baseline -ExpectedRole Baseline -RepositoryRoot $repositoryRoot | Out-Null

$m3InvalidCases = @(
    @{ name = 'candidate SHA'; mutate = { param($m) $m.source_revision = ('0' * 40) } },
    @{ name = 'candidate tree'; mutate = { param($m) $m.source_tree_sha = ('0' * 40) } },
    @{ name = 'runner HEAD substituted for accepted M2 candidate'; mutate = { param($m) $m.source_revision = $m.runner_source_revision; $m.source_tree_sha = $m.runner_source_tree_sha } },
    @{ name = 'unverified candidate lineage'; mutate = { param($m) $m.candidate_lineage_verified = $false } },
    @{ name = 'wrong lineage branch'; mutate = { param($m) $m.candidate_lineage_branch = 'refactor/frame-snapshot-dispatch' } },
    @{ name = 'wrong lineage tip'; mutate = { param($m) $m.candidate_lineage_tip_revision = ('0' * 40) } },
    @{ name = 'dirty source'; mutate = { param($m) $m.source_tree_clean = $false; $m.source_checkout_identity.clean = $false } },
    @{ name = 'wrong role'; mutate = { param($m) $m.source_role = 'Baseline' } },
    @{ name = 'role swap'; expected_role = 'Baseline'; mutate = { param($m) } },
    @{ name = 'wrong profile'; mutate = { param($m) $m.qualification_profile = 'FrameSnapshot' } },
    @{ name = 'wrong workload version'; mutate = { param($m) $m.workload_version = 'frame-snapshot-v1' } },
    @{ name = 'wrong Rust target'; mutate = { param($m) $m.target = 'x86_64-unknown-linux-gnu' } },
    @{ name = 'wrong target identity'; mutate = { param($m) $m.target_identity = 'x86_64-unknown-linux-gnu' } },
    @{ name = 'wrong build profile'; mutate = { param($m) $m.cargo_profile = 'release' } },
    @{ name = 'wrong feature'; mutate = { param($m) $m.cargo_features = @('real-input-acceptance', 'qpc-read-counting') } },
    @{ name = 'wrong Rust version'; mutate = { param($m) $m.rustc_version = $m.rustc_version.Replace('release: 1.99.1', 'release: 1.99.0') } },
    @{ name = 'wrong Cargo version'; mutate = { param($m) $m.cargo_version = 'cargo 1.98.0' } },
    @{ name = 'wrong observer mode'; mutate = { param($m) $m.receiveonly_observer_mode = 'SchedulerQueuedV2' } },
    @{ name = 'wrong observer version'; mutate = { param($m) $m.receiveonly_observer_version = 'scheduler-receiveonly-queue-v2' } },
    @{ name = 'wrong observer schema'; mutate = { param($m) $m.receiveonly_observer_schema_version = 2 } },
    @{ name = 'wrong observer event schema'; mutate = { param($m) $m.receiveonly_observer_event_schema_version = 4 } },
    @{ name = 'wrong performance margin'; mutate = { param($m) $m.performance_timing_margin_us = 500 } },
    @{ name = 'wrong safety margin'; mutate = { param($m) $m.safety_timing_margin_us = 0 } },
    @{ name = 'wrong runner identity'; mutate = { param($m) $m.runner_source_tree_sha = ('0' * 40) } },
    @{ name = 'evidence root inside repository'; mutate = { param($m) $m.output_root = Join-Path $repositoryRoot '.benchmarks\m3-contract' } }
)
$m3NegativeContractCount = 0
foreach ($case in $m3InvalidCases) {
    $invalidM3 = $validM3Candidate | ConvertTo-Json -Depth 12 | ConvertFrom-Json
    & $case.mutate $invalidM3
    $expectedRole = if ($case.expected_role) { $case.expected_role } else { 'Candidate' }
    $rejected = $false
    try { Assert-DispatchCleanupM3ManifestIdentity -Manifest $invalidM3 -ExpectedRole $expectedRole -RepositoryRoot $repositoryRoot | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw ('DispatchCleanupM3 accepted invalid ' + $case.name) }
    $m3NegativeContractCount++
}
Assert-DispatchCleanupM3SessionIdentity -Scenario 'canonical-max-chord' -Workload 'canonical-max-chord' -TimingMarginUs 0 -GameFps 60 -SampleSeconds 15 | Out-Null
Assert-DispatchCleanupM3SessionIdentity -Scenario 'canonical-max-chord' -Workload 'safety-smoke' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0 | Out-Null
Assert-DispatchCleanupM3SessionIdentity -Scenario 'canonical-single' -Workload 'canonical-single' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0 | Out-Null
Assert-DispatchCleanupM3SessionIdentity -Scenario 'timing-margin-sweep' -Workload 'boundary-60fps-margin-0' -TimingMarginUs 0 -GameFps 60 -SampleSeconds 0 | Out-Null
Assert-DispatchCleanupM3SessionIdentity -Scenario 'timing-margin-sweep' -Workload 'boundary-120fps-margin-500' -TimingMarginUs 500 -GameFps 120 -SampleSeconds 0 | Out-Null
$m3BadSessionCases = @(
    @{ name = 'performance margin'; scenario = 'canonical-max-chord'; workload = 'canonical-max-chord'; margin = 500; fps = 60; sample = 15 },
    @{ name = 'safety margin'; scenario = 'canonical-single'; workload = 'canonical-single'; margin = 0; fps = 60; sample = 0 },
    @{ name = 'performance FPS'; scenario = 'dense-alternating'; workload = 'dense-alternating'; margin = 0; fps = 120; sample = 15 },
    @{ name = 'safety timed sample'; scenario = 'focus-loss'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 15 },
    @{ name = 'workload scenario mismatch'; scenario = 'dense-alternating'; workload = 'mixed-up-down'; margin = 0; fps = 60; sample = 15 },
    @{ name = 'wrong boundary margin'; scenario = 'timing-margin-sweep'; workload = 'boundary-60fps-margin-0'; margin = 500; fps = 60; sample = 0 },
    @{ name = 'unsupported workload'; scenario = 'hol-interleaved'; workload = 'unqualified'; margin = 500; fps = 60; sample = 0 }
)
foreach ($case in $m3BadSessionCases) {
    $rejected = $false
    try { Assert-DispatchCleanupM3SessionIdentity -Scenario $case.scenario -Workload $case.workload -TimingMarginUs $case.margin -GameFps $case.fps -SampleSeconds $case.sample | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw ('DispatchCleanupM3 accepted invalid session ' + $case.name) }
    $m3NegativeContractCount++
}
$wrongM3BaselineSha = $validM3Baseline | ConvertTo-Json -Depth 12 | ConvertFrom-Json
$wrongM3BaselineSha.source_revision = ('0' * 40)
$rejectedWrongM3BaselineSha = $false
try { Assert-DispatchCleanupM3ManifestIdentity -Manifest $wrongM3BaselineSha -ExpectedRole Baseline -RepositoryRoot $repositoryRoot | Out-Null } catch { $rejectedWrongM3BaselineSha = $true }
if (-not $rejectedWrongM3BaselineSha) { throw 'DispatchCleanupM3 accepted a wrong baseline SHA' }
$m3NegativeContractCount++
$wrongM3BaselineTree = $validM3Baseline | ConvertTo-Json -Depth 12 | ConvertFrom-Json
$wrongM3BaselineTree.source_tree_sha = ('0' * 40)
$rejectedWrongM3BaselineTree = $false
try { Assert-DispatchCleanupM3ManifestIdentity -Manifest $wrongM3BaselineTree -ExpectedRole Baseline -RepositoryRoot $repositoryRoot | Out-Null } catch { $rejectedWrongM3BaselineTree = $true }
if (-not $rejectedWrongM3BaselineTree) { throw 'DispatchCleanupM3 accepted a wrong baseline tree' }
$m3NegativeContractCount++
$dirtyM3Baseline = $validM3Baseline | ConvertTo-Json -Depth 12 | ConvertFrom-Json
$dirtyM3Baseline.source_tree_clean = $false
$dirtyM3Baseline.source_checkout_identity.clean = $false
$dirtyM3BaselineRejected = $false
try { Assert-DispatchCleanupM3ManifestIdentity -Manifest $dirtyM3Baseline -ExpectedRole Baseline -RepositoryRoot $repositoryRoot | Out-Null } catch { $dirtyM3BaselineRejected = $true }
if (-not $dirtyM3BaselineRejected) { throw 'DispatchCleanupM3 accepted a dirty baseline checkout' }
$m3NegativeContractCount++
$wrongM3RunnerIdentity = $validM3Candidate | ConvertTo-Json -Depth 12 | ConvertFrom-Json
$wrongM3RunnerIdentity.runner_source_revision = ('d' * 40)
$wrongM3RunnerIdentity.runner_source_checkout_identity.revision = ('d' * 40)
$wrongM3RunnerIdentity.candidate_lineage_tip_revision = ('d' * 40)
$wrongM3RunnerRejected = $false
try { Assert-DispatchCleanupM3RunnerProvenance -Manifest $wrongM3RunnerIdentity -RunnerIdentity $m3RunnerIdentityFixture -RunnerRoot $repositoryRoot | Out-Null } catch { $wrongM3RunnerRejected = $true }
if (-not $wrongM3RunnerRejected) { throw 'DispatchCleanupM3 accepted a manifest tied to another runner commit' }
$m3NegativeContractCount++
$wrongM3RunnerScriptHash = $validM3Candidate | ConvertTo-Json -Depth 12 | ConvertFrom-Json
$wrongM3RunnerScriptHash.runner_script_provenance[0].sha256 = ('0' * 64)
$wrongM3RunnerScriptRejected = $false
try { Assert-DispatchCleanupM3RunnerProvenance -Manifest $wrongM3RunnerScriptHash -RunnerIdentity $m3RunnerIdentityFixture -RunnerRoot $repositoryRoot | Out-Null } catch { $wrongM3RunnerScriptRejected = $true }
if (-not $wrongM3RunnerScriptRejected) { throw 'DispatchCleanupM3 accepted a tampered runner script hash' }
$m3NegativeContractCount++
$m3BaselineRejectedByOldProfile = $false
try { Assert-FrameSnapshotManifestIdentity -Manifest $validM3Baseline -ExpectedRole Baseline } catch { $m3BaselineRejectedByOldProfile = $true }
if (-not $m3BaselineRejectedByOldProfile) { throw 'old FrameSnapshot validator accepted the M3 baseline identity' }
$m3NegativeContractCount++
$m3CandidateRejectedByOldProfile = $false
try { Assert-FrameSnapshotManifestIdentity -Manifest $validM3Candidate -ExpectedRole Candidate } catch { $m3CandidateRejectedByOldProfile = $true }
if (-not $m3CandidateRejectedByOldProfile) { throw 'old FrameSnapshot validator accepted the M3 candidate identity' }
$m3NegativeContractCount++
if ((Get-QualificationReportRowLabel -Profile DispatchCleanupM3 -Role candidate -Manifest $validM3Candidate) -ne 'DispatchCleanupM3-Candidate' -or
    (Get-QualificationReportRowLabel -Profile DispatchCleanupM3 -Role control -Manifest $validM3Baseline) -ne 'DispatchCleanupM3-Baseline') {
    throw 'DispatchCleanupM3 report projection confused candidate and baseline roles'
}

$integrityRoot = Join-Path $env:TEMP ('dispatch-manifest-integrity-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $integrityRoot | Out-Null
try {
    $integrityBinary = Join-Path $integrityRoot 'rt-native-acceptance.exe'
    $integrityManifestPath = Join-Path $integrityRoot 'manifest.json'
    $integrityBuilder = Join-Path $integrityRoot 'build_dispatch_core_harness.ps1'
    $integrityBuildLog = Join-Path $integrityRoot 'build.log'
    $integrityCleanLog = Join-Path $integrityRoot 'clean.log'
    [System.IO.File]::WriteAllBytes($integrityBinary, [byte[]](1, 2, 3, 4))
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1') -Destination $integrityBuilder
    [System.IO.File]::WriteAllText($integrityBuildLog, 'build fixture')
    [System.IO.File]::WriteAllText($integrityCleanLog, 'clean fixture')
    $integrityManifest = [pscustomobject]@{
        binary_name = 'rt-native-acceptance.exe'; binary_path = $integrityBinary
        binary_sha256 = (Get-FileHash -LiteralPath $integrityBinary -Algorithm SHA256).Hash.ToLowerInvariant()
        builder_script_sha256 = (Get-FileHash -LiteralPath $integrityBuilder -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $integrityManifest | ConvertTo-Json | Set-Content -LiteralPath $integrityManifestPath -Encoding UTF8
    $integrityRows = @(
        ((Get-FileHash -LiteralPath $integrityManifestPath -Algorithm SHA256).Hash.ToLowerInvariant() + '  manifest.json'),
        ($integrityManifest.binary_sha256 + '  rt-native-acceptance.exe'),
        ($integrityManifest.builder_script_sha256 + '  build_dispatch_core_harness.ps1'),
        ((Get-FileHash -LiteralPath $integrityBuildLog -Algorithm SHA256).Hash.ToLowerInvariant() + '  build.log'),
        ((Get-FileHash -LiteralPath $integrityCleanLog -Algorithm SHA256).Hash.ToLowerInvariant() + '  clean.log')
    )
    Set-Content -LiteralPath (Join-Path $integrityRoot 'SHA256SUMS.txt') -Value $integrityRows -Encoding UTF8
    Assert-ImmutableHarnessManifestIntegrity -Manifest $integrityManifest -ManifestPath $integrityManifestPath | Out-Null
    [System.IO.File]::WriteAllBytes($integrityBinary, [byte[]](4, 3, 2, 1))
    $integrityRejected = $false
    try { Assert-ImmutableHarnessManifestIntegrity -Manifest $integrityManifest -ManifestPath $integrityManifestPath | Out-Null } catch { $integrityRejected = $true }
    if (-not $integrityRejected) { throw 'immutable harness manifest integrity accepted a changed binary' }
} finally {
    Remove-Item -LiteralPath $integrityRoot -Recurse -Force
}

$m3IntegrityRoot = Join-Path $env:TEMP ('dispatch-cleanup-m3-manifest-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $m3IntegrityRoot | Out-Null
try {
    $m3IntegrityBinary = Join-Path $m3IntegrityRoot 'rt-native-acceptance.exe'
    $m3IntegrityManifestPath = Join-Path $m3IntegrityRoot 'manifest.json'
    [System.IO.File]::WriteAllBytes($m3IntegrityBinary, [byte[]](1, 2, 3, 4))
    $m3IntegrityManifest = $validM3Candidate | ConvertTo-Json -Depth 12 | ConvertFrom-Json
    $m3IntegrityManifest.output_root = $m3IntegrityRoot
    $m3IntegrityManifest.cargo_target_dir = Join-Path $m3IntegrityRoot 'cargo-target'
    $m3IntegrityManifest | Add-Member -NotePropertyName binary_path -NotePropertyValue $m3IntegrityBinary -Force
    $m3IntegrityManifest.binary_sha256 = (Get-FileHash -LiteralPath $m3IntegrityBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    $m3IntegrityManifest | Add-Member -NotePropertyName builder_script_sha256 -NotePropertyValue ((Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1') -Algorithm SHA256).Hash.ToLowerInvariant()) -Force
    $m3IntegrityManifest | Add-Member -NotePropertyName manifest_path -NotePropertyValue $m3IntegrityManifestPath -Force
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1') -Destination (Join-Path $m3IntegrityRoot 'build_dispatch_core_harness.ps1')
    [System.IO.File]::WriteAllText((Join-Path $m3IntegrityRoot 'build.log'), 'm3 build fixture')
    [System.IO.File]::WriteAllText((Join-Path $m3IntegrityRoot 'clean.log'), 'm3 clean fixture')
    foreach ($entry in $m3IntegrityManifest.runner_script_provenance) {
        $sourceFile = Join-Path $repositoryRoot ($entry.source_path.Replace('/', '\'))
        $copyPath = Join-Path $m3IntegrityRoot ($entry.evidence_path.Replace('/', '\'))
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $copyPath) | Out-Null
        Copy-Item -LiteralPath $sourceFile -Destination $copyPath
    }
    $m3IntegrityManifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $m3IntegrityManifestPath -Encoding UTF8
    $m3ChecksumLines = [System.Collections.Generic.List[string]]::new()
    $m3ChecksumLines.Add(((Get-FileHash -LiteralPath $m3IntegrityManifestPath -Algorithm SHA256).Hash.ToLowerInvariant() + '  manifest.json'))
    $m3ChecksumLines.Add(($m3IntegrityManifest.binary_sha256 + '  rt-native-acceptance.exe'))
    $m3ChecksumLines.Add(($m3IntegrityManifest.builder_script_sha256 + '  build_dispatch_core_harness.ps1'))
    $m3ChecksumLines.Add(((Get-FileHash -LiteralPath (Join-Path $m3IntegrityRoot 'build.log') -Algorithm SHA256).Hash.ToLowerInvariant() + '  build.log'))
    $m3ChecksumLines.Add(((Get-FileHash -LiteralPath (Join-Path $m3IntegrityRoot 'clean.log') -Algorithm SHA256).Hash.ToLowerInvariant() + '  clean.log'))
    foreach ($entry in $m3IntegrityManifest.runner_script_provenance) {
        $copyPath = Join-Path $m3IntegrityRoot ($entry.evidence_path.Replace('/', '\'))
        $m3ChecksumLines.Add(([string]$entry.sha256 + '  ' + [string]$entry.evidence_path.Replace('\', '/')))
    }
    $m3ChecksumPath = Join-Path $m3IntegrityRoot 'SHA256SUMS.txt'
    Set-Content -LiteralPath $m3ChecksumPath -Value $m3ChecksumLines.ToArray() -Encoding UTF8
    Assert-ImmutableHarnessManifestIntegrity -Manifest $m3IntegrityManifest -ManifestPath $m3IntegrityManifestPath | Out-Null

    $originalBinaryHash = $m3IntegrityManifest.binary_sha256
    $m3IntegrityManifest.binary_sha256 = ('d' * 64)
    $m3BinaryHashTamperRejected = $false
    try { Assert-ImmutableHarnessManifestIntegrity -Manifest $m3IntegrityManifest -ManifestPath $m3IntegrityManifestPath | Out-Null } catch { $m3BinaryHashTamperRejected = $true }
    if (-not $m3BinaryHashTamperRejected) { throw 'DispatchCleanupM3 immutable manifest integrity accepted a tampered binary hash' }
    $m3NegativeContractCount++
    $m3IntegrityManifest.binary_sha256 = $originalBinaryHash

    $originalBinary = [System.IO.File]::ReadAllBytes($m3IntegrityBinary)
    [System.IO.File]::WriteAllBytes($m3IntegrityBinary, [byte[]](4, 3, 2, 1))
    $m3BinaryTamperRejected = $false
    try { Assert-ImmutableHarnessManifestIntegrity -Manifest $m3IntegrityManifest -ManifestPath $m3IntegrityManifestPath | Out-Null } catch { $m3BinaryTamperRejected = $true }
    if (-not $m3BinaryTamperRejected) { throw 'DispatchCleanupM3 immutable manifest integrity accepted a tampered binary' }
    $m3NegativeContractCount++
    [System.IO.File]::WriteAllBytes($m3IntegrityBinary, $originalBinary)

    $originalManifestBytes = [System.IO.File]::ReadAllBytes($m3IntegrityManifestPath)
    $manifestText = [System.Text.Encoding]::UTF8.GetString($originalManifestBytes)
    [System.IO.File]::WriteAllText($m3IntegrityManifestPath, $manifestText + ' ')
    $m3ManifestTamperRejected = $false
    try { Assert-ImmutableHarnessManifestIntegrity -Manifest $m3IntegrityManifest -ManifestPath $m3IntegrityManifestPath | Out-Null } catch { $m3ManifestTamperRejected = $true }
    if (-not $m3ManifestTamperRejected) { throw 'DispatchCleanupM3 immutable manifest integrity accepted a tampered manifest hash' }
    $m3NegativeContractCount++
    [System.IO.File]::WriteAllBytes($m3IntegrityManifestPath, $originalManifestBytes)

    $originalChecksumBytes = [System.IO.File]::ReadAllBytes($m3ChecksumPath)
    $tamperedChecksumLines = @($m3ChecksumLines.ToArray())
    $tamperedChecksumLines[0] = (('0' * 64) + '  manifest.json')
    Set-Content -LiteralPath $m3ChecksumPath -Value $tamperedChecksumLines -Encoding UTF8
    $m3ChecksumTamperRejected = $false
    try { Assert-ImmutableHarnessManifestIntegrity -Manifest $m3IntegrityManifest -ManifestPath $m3IntegrityManifestPath | Out-Null } catch { $m3ChecksumTamperRejected = $true }
    if (-not $m3ChecksumTamperRejected) { throw 'DispatchCleanupM3 immutable manifest integrity accepted a tampered checksum file' }
    $m3NegativeContractCount++
    [System.IO.File]::WriteAllBytes($m3ChecksumPath, $originalChecksumBytes)

    $firstProvenance = $m3IntegrityManifest.runner_script_provenance[0]
    $firstProvenancePath = Join-Path $m3IntegrityRoot ($firstProvenance.evidence_path.Replace('/', '\'))
    $originalProvenanceBytes = [System.IO.File]::ReadAllBytes($firstProvenancePath)
    [System.IO.File]::WriteAllText($firstProvenancePath, 'tampered runner script')
    $m3RunnerScriptTamperRejected = $false
    try { Assert-ImmutableHarnessManifestIntegrity -Manifest $m3IntegrityManifest -ManifestPath $m3IntegrityManifestPath | Out-Null } catch { $m3RunnerScriptTamperRejected = $true }
    if (-not $m3RunnerScriptTamperRejected) { throw 'DispatchCleanupM3 immutable manifest integrity accepted a tampered runner source copy' }
    $m3NegativeContractCount++
    [System.IO.File]::WriteAllBytes($firstProvenancePath, $originalProvenanceBytes)
} finally {
    $safeTempRoot = [System.IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    $safeM3IntegrityRoot = [System.IO.Path]::GetFullPath($m3IntegrityRoot)
    if (-not $safeM3IntegrityRoot.StartsWith($safeTempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'refusing to clean M3 manifest fixtures outside the temporary directory'
    }
    Remove-Item -LiteralPath $m3IntegrityRoot -Recurse -Force
}

$smokeRuns = [System.Collections.Generic.List[string]]::new()
$smoke = Invoke-NativeCaseFailStopSequence -Cases @('PASS', 'FAIL', 'PASS') -InvokeCase {
    param($case)
    $smokeRuns.Add($case)
    [pscustomobject]@{
        case_exit_code = if ($case -eq 'PASS') { 0 } else { 1 }
        status = $case
    }
}
if (-not $smoke.stopped_at_first_native_failure -or $smokeRuns.Count -ne 2 -or
    $smokeRuns[0] -ne 'PASS' -or $smokeRuns[1] -ne 'FAIL' -or
    $smoke.attempted_rows.Count -ne 2) {
    throw 'PASS -> FAIL -> PASS did not stop after the second completed native case'
}

$finalRuns = [System.Collections.Generic.List[string]]::new()
$finalCases = @(
    [pscustomobject]@{ name = 'safety-1'; status = 'PASS' }
    [pscustomobject]@{ name = 'boundary-60fps-margin-0'; status = 'FAIL' }
    [pscustomobject]@{ name = 'boundary-120fps-margin-500'; status = 'PASS' }
)
$final = Invoke-NativeCaseFailStopSequence -Cases $finalCases -InvokeCase {
    param($case)
    $finalRuns.Add($case.name)
    [pscustomobject]@{
        case_exit_code = if ($case.status -eq 'PASS') { 0 } else { 1 }
        status = $case.status
    }
}
if (-not $final.stopped_at_first_native_failure -or $finalRuns.Count -ne 2 -or
    $finalRuns[1] -ne 'boundary-60fps-margin-0' -or
    $final.attempted_rows.Count -ne 2) {
    throw 'Final boundary failure did not block every later safety/boundary case'
}

foreach ($failedLoad in @('Quiet', 'Contention')) {
    $loadRuns = [System.Collections.Generic.List[string]]::new()
    $loadCases = @(
        [pscustomobject]@{ load = $failedLoad; scenario = 'first' }
        [pscustomobject]@{ load = $failedLoad; scenario = 'middle' }
        [pscustomobject]@{ load = $failedLoad; scenario = 'last' }
        [pscustomobject]@{ load = 'Quiet'; scenario = 'boundary' }
    )
    $loadResult = Invoke-NativeCaseFailStopSequence -Cases $loadCases -InvokeCase {
        param($case)
        $loadRuns.Add($case.load + ':' + $case.scenario)
        [pscustomobject]@{ case_exit_code = if ($case.scenario -eq 'middle') { 1 } else { 0 }; status = if ($case.scenario -eq 'middle') { 'FAIL' } else { 'PASS' } }
    }
    if (-not $loadResult.stopped_at_first_native_failure -or $loadRuns.Count -ne 2 -or
        $loadRuns[1] -ne ($failedLoad + ':middle') -or $loadRuns.Contains('Quiet:boundary')) {
        throw "Final $failedLoad failure did not stop later safety and boundary cases"
    }
}

$timingRecord = [ordered]@{
    kind = 0; down_mask = 1; up_mask = 0; flags = 1; send_status = 0
    requested_count = 1; sent_count = 1; send_attempts = 1
    authored_offset_ticks = 100; source_action_index = 0; event_index = 0
    compiled_packet_index_available = $false; compiled_packet_index = 0
    authored_lateness_available = $false; authored_lateness_ticks = 0
    floor_displacement_available = $false; physical_floor_displacement_ticks = 0
    residual_lateness_available = $false; residual_lateness_ticks = 0
    sendinput_envelope_available = $false; sendinput_envelope_ticks = 0
    waiter_return_qpc_available = $false; waiter_return_qpc_ticks = 0
    waiter_deadline_crossing_qpc_available = $true
    waiter_deadline_crossing_qpc_ticks = 100
    waiter_deadline_crossing_qpc_provenance = 'measured'
    pre_call_qpc_available = $true; pre_call_qpc_ticks = 150
    pre_call_qpc_provenance = 'measured'
    timer_arm_qpc_available = $false; timer_arm_target_qpc_available = $false
    timer_arm_generation_available = $false; timer_arm_qpc_provenance = 'unavailable'
    timer_arm_target_qpc_provenance = 'unavailable'; timer_return_qpc_available = $false
    timer_return_generation_available = $false; physical_eligibility_qpc_available = $false
    sendinput_completion_qpc_available = $false; sendinput_completion_qpc_ticks = 0
}
$unmeasuredCrossing = [ordered]@{}
foreach ($entry in $timingRecord.GetEnumerator()) { $unmeasuredCrossing[$entry.Key] = $entry.Value }
$unmeasuredCrossing.waiter_deadline_crossing_qpc_provenance = 'configured_target'
$telemetry = [pscustomobject]@{
    qpc_frequency_hz = 1000000
    records = @([pscustomobject]$timingRecord, [pscustomobject]$unmeasuredCrossing)
    accepted = 2; dropped = 0; observer_queue_dropped = 0; truncated = $false
    up_slot_records = @(); up_slot_records_dropped = 0
}
$report = [pscustomobject]@{
    details = [pscustomobject]@{
        native_timing_telemetry = $telemetry
        worker_cpu_time_us = $null; process_cpu_time_us = $null; spin_time_us = $null
        spin_time_available = $false; spin_time_complete = $false; spin_duty_cycle_ppm = $null
        spin_observation_expected_samples = 0; spin_observation_dropped_samples = 0
        spin_observation_missing_samples = 0
    }
}
$metrics = Get-TimingMetrics $report
if ($metrics.metrics.waiter_to_pre_call_us.sample_count -ne 0 -or
    $metrics.counts.missing_waiter_to_pre_call -ne 2) {
    throw 'waiter-to-pre-call became available without an actual waiter-return QPC'
}
if ($metrics.metrics.deadline_crossing_to_pre_call_us.sample_count -ne 1 -or
    $metrics.metrics.deadline_crossing_to_pre_call_us.p50 -ne 50 -or
    $metrics.counts.missing_deadline_crossing_to_pre_call -ne 1) {
    throw 'measured deadline-crossing-to-pre-call diagnostic lost provenance or used an invalid endpoint'
}

$runRows = @()
$QualificationProfile = 'Scheduler'
$p95Deltas = @(10, 20, 30, 40, 50, 60)
$p99Deltas = @(2, 4, 6, 8, 10, 12)
for ($index = 0; $index -lt 6; $index++) {
    $controlMetric = [pscustomobject]@{
        p50 = 40; p95 = 100; p99 = 200; max = 300; sample_count = 20
    }
    $candidateMetric = [pscustomobject]@{
        p50 = 45; p95 = 100 + $p95Deltas[$index]
        p99 = 200 + $p99Deltas[$index]; max = 310; sample_count = 20
    }
    $runRows += [pscustomobject]@{
        cell_id = 'Dispatch/Quiet'; pair_id = ('Dispatch/Quiet/{0:D2}' -f ($index + 1))
        pair_order = if ($index % 2 -eq 0) { 'AB' } else { 'BA' }
        role = 'control'; status = 'PASS'; receiveonly_observer_mode = 'SchedulerQueuedV3Compat3'
        receiveonly_observer_version = 'scheduler-receiveonly-queue-v3-compat3'
        timing_metrics = [pscustomobject]@{
            metrics = [pscustomobject]@{ authored_lateness_per_chord_us = $controlMetric }
        }
    }
    $runRows += [pscustomobject]@{
        cell_id = 'Dispatch/Quiet'; pair_id = ('Dispatch/Quiet/{0:D2}' -f ($index + 1))
        pair_order = if ($index % 2 -eq 0) { 'AB' } else { 'BA' }
        role = 'candidate'; status = 'PASS'; receiveonly_observer_mode = 'SchedulerQueuedV3Compat3'
        receiveonly_observer_version = 'scheduler-receiveonly-queue-v3-compat3'
        timing_metrics = [pscustomobject]@{
            metrics = [pscustomobject]@{ authored_lateness_per_chord_us = $candidateMetric }
        }
    }
}
$pairedMetrics = Get-PairedMetrics
$compatiblePair = [pscustomobject]@{ receiveonly_observer_mode = 'SchedulerQueuedV3Compat3'; receiveonly_observer_version = 'scheduler-receiveonly-queue-v3-compat3' }
Assert-SchedulerObserverPairMode -Control $compatiblePair -Candidate $compatiblePair
$pairRejected = $false
try {
    Assert-SchedulerObserverPairMode -Control $compatiblePair -Candidate ([pscustomobject]@{ receiveonly_observer_mode = 'LegacyInline'; receiveonly_observer_version = $null })
} catch { $pairRejected = $true }
if (-not $pairRejected) { throw 'Scheduler accepted baseline/candidate sessions with mismatched ReceiveOnly observer modes' }
$pairVersionRejected = $false
try {
    Assert-SchedulerObserverPairMode -Control $compatiblePair -Candidate ([pscustomobject]@{ receiveonly_observer_mode = 'SchedulerQueuedV2'; receiveonly_observer_version = 'scheduler-receiveonly-queue-v2' })
} catch { $pairVersionRejected = $true }
if (-not $pairVersionRejected) { throw 'Scheduler accepted historical v2/schema4 observer evidence paired with Compat3' }
$pairedCell = @($pairedMetrics.cell_summary | Where-Object metric -eq 'metrics.authored_lateness_per_chord_us') | Select-Object -First 1
if ($null -eq $pairedCell -or $pairedCell.attempted_pairs -ne 6 -or
    $pairedCell.p95_delta_sample_count -ne 6 -or $pairedCell.p95_delta_p50_us -ne 30 -or
    $pairedCell.p95_delta_p95_us -ne 60 -or $pairedCell.p95_delta_p99_us -ne 60 -or
    $pairedCell.p95_delta_max_us -ne 60 -or $pairedCell.p99_delta_p50_us -ne 6 -or
    $pairedCell.p99_delta_p95_us -ne 12 -or $pairedCell.p99_delta_max_us -ne 12 -or
    $pairedCell.PSObject.Properties['p95_delta_us'] -or
    $pairedCell.p95_delta_p50_us -is [System.Collections.IDictionary]) {
    throw 'paired cell tail deltas are not flat numeric summaries'
}

$comparisonControl = [pscustomobject]@{ guard_mode = 'frozen' }
$comparisonCandidate = [pscustomobject]@{ guard_mode = 'adaptive' }
if ((Get-ComparisonDescription -Profile 'Scheduler' -Control $comparisonControl -Candidate $comparisonCandidate) -ne
    'C0 Scheduler baseline vs Scheduler-Candidate') {
    throw 'Scheduler provenance still describes the pinned comparison as C1-TIMED'
}
if ((Get-ComparisonDescription -Profile 'DispatchCore' -Control ([pscustomobject]@{}) -Candidate ([pscustomobject]@{})) -ne
    'C1-TIMED-R2 vs Candidate-R2; C0 not timed-qualified' -or
    (Get-ComparisonDescription -Profile 'DispatchCore' -Control $comparisonControl -Candidate $comparisonCandidate) -ne 'Frozen vs Adaptive') {
    throw 'DispatchCore comparison provenance changed outside the Scheduler profile'
}

$pairedCsv = Join-Path $env:TEMP ('paired-cells-contract-' + [guid]::NewGuid().ToString('N') + '.csv')
try {
    @($pairedCell) | Export-Csv -LiteralPath $pairedCsv -NoTypeInformation -Encoding UTF8
    $csvCell = Import-Csv -LiteralPath $pairedCsv | Select-Object -First 1
    if ([double]$csvCell.p95_delta_p50_us -ne 30 -or
        [double]$csvCell.p95_delta_p95_us -ne 60 -or
        [double]$csvCell.p99_delta_p50_us -ne 6 -or
        $csvCell.p95_delta_p50_us -match 'OrderedDictionary|System.Collections') {
        throw 'paired-cells CSV did not preserve scalar numeric delta percentiles'
    }
} finally {
    Remove-Item -LiteralPath $pairedCsv -Force -ErrorAction SilentlyContinue
}

Set-StrictMode -Version Latest
Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_report.psm1') -Force
if (-not $qualifierSource.Contains('New-QualificationReportProjection') -or
    -not $qualifierSource.Contains('Get-QualificationReportRowLabel')) {
    throw 'qualification runner is not using the tested report projection and role-label functions'
}

# Historical Scheduler/DispatchCore manifests predate the separate margin
# fields. They must still reach report serialization under strict mode.
$legacyCandidate = [pscustomobject]@{
    qualification_profile = 'Scheduler'; source_role = 'Candidate';
    workload_version = 'scheduler-single-owner-v1'; observer_mode = 'SchedulerQueuedV3Compat3';
    guard_mode = 'frozen';
}
$legacyControl = [pscustomobject]@{
    qualification_profile = 'Scheduler'; source_role = 'Baseline';
    workload_version = 'scheduler-single-owner-v1'; observer_mode = 'SchedulerQueuedV3Compat3';
    guard_mode = 'frozen';
}
$legacyProjection = New-QualificationReportProjection -Profile Scheduler -Candidate $legacyCandidate `
    -Control $legacyControl -Stage Compare -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500
$legacySummaryJson = [ordered]@{
    candidate_label = $legacyProjection.CandidateLabel
    control_label = $legacyProjection.ControlLabel
    parameters = $legacyProjection.SummaryParameters
} | ConvertTo-Json -Depth 8
$legacyManifestJson = [ordered]@{
    candidate_label = $legacyProjection.CandidateLabel
    control_label = $legacyProjection.ControlLabel
    parameters = $legacyProjection.ManifestParameters
} | ConvertTo-Json -Depth 8
$legacySummary = $legacySummaryJson | ConvertFrom-Json
$legacyResultManifest = $legacyManifestJson | ConvertFrom-Json
if ($null -ne $legacySummary.parameters.performance_timing_margin_us -or
    $null -ne $legacySummary.parameters.safety_timing_margin_us -or
    $null -ne $legacyResultManifest.parameters.performance_timing_margin_us -or
    $null -ne $legacyResultManifest.parameters.safety_timing_margin_us) {
    throw 'legacy report projection fabricated missing timing margins instead of preserving null'
}

$historicalDispatchCoreManifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'fixtures\dispatch-core-c0-b52106b4.manifest.json') -Raw | ConvertFrom-Json
if ($historicalDispatchCoreManifest.PSObject.Properties['qualification_profile'] -or
    $historicalDispatchCoreManifest.PSObject.Properties['performance_timing_margin_us'] -or
    $historicalDispatchCoreManifest.PSObject.Properties['safety_timing_margin_us']) {
    throw 'pinned historical DispatchCore fixture unexpectedly contains the new optional qualification fields'
}
$historicalProjection = New-QualificationReportProjection -Profile DispatchCore `
    -Candidate $historicalDispatchCoreManifest -Control $historicalDispatchCoreManifest `
    -Stage Compare -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500
$historicalSummaryJson = [ordered]@{
    candidate = [ordered]@{
        label = $historicalProjection.CandidateLabel
        qualification_profile = $historicalProjection.CandidateQualificationProfile
        performance_timing_margin_us = $historicalProjection.CandidateMargins.performance_timing_margin_us
        safety_timing_margin_us = $historicalProjection.CandidateMargins.safety_timing_margin_us
    }
    control = [ordered]@{
        control_label = $historicalProjection.ControlLabel
        qualification_profile = $historicalProjection.ControlQualificationProfile
        performance_timing_margin_us = $historicalProjection.ControlMargins.performance_timing_margin_us
        safety_timing_margin_us = $historicalProjection.ControlMargins.safety_timing_margin_us
    }
    parameters = $historicalProjection.SummaryParameters
} | ConvertTo-Json -Depth 8
$historicalResultManifestJson = [ordered]@{
    candidate_manifest = [ordered]@{
        label = $historicalProjection.CandidateLabel
        qualification_profile = $historicalProjection.CandidateQualificationProfile
    }
    control_manifest = [ordered]@{
        control_label = $historicalProjection.ControlLabel
        qualification_profile = $historicalProjection.ControlQualificationProfile
    }
    parameters = $historicalProjection.ManifestParameters
} | ConvertTo-Json -Depth 8
$historicalSummary = $historicalSummaryJson | ConvertFrom-Json
$historicalResultManifest = $historicalResultManifestJson | ConvertFrom-Json
if ($historicalSummary.candidate.label -ne 'Candidate-R2' -or
    $historicalSummary.control.control_label -ne 'C0' -or
    $null -ne $historicalSummary.candidate.qualification_profile -or
    $null -ne $historicalSummary.control.qualification_profile -or
    $null -ne $historicalSummary.candidate.performance_timing_margin_us -or
    $null -ne $historicalSummary.candidate.safety_timing_margin_us -or
    $null -ne $historicalSummary.control.performance_timing_margin_us -or
    $null -ne $historicalSummary.control.safety_timing_margin_us -or
    $historicalResultManifest.candidate_manifest.label -ne 'Candidate-R2' -or
    $historicalResultManifest.control_manifest.control_label -ne 'C0') {
    throw 'historical DispatchCore report projection failed strict-mode provenance serialization'
}
if (-not $qualifierSource.Contains('report_projection_sha256') -or
    -not $qualifierSource.Contains('dispatch_qualification_report.psm1')) {
    throw 'qualification evidence package does not hash and preserve the report projection module'
}

# Exercise the same role/margin projection consumed by sessions.csv,
# summary.json, summary.md, and the run result manifest for a FrameSnapshot
# baseline/candidate pair.
$frameProjection = New-QualificationReportProjection -Profile FrameSnapshot -Candidate $validFrameCandidate `
    -Control $validFrameBaseline -Stage Final -Pairs 6 -SampleSeconds 15 -TimingMarginUs 0
$frameRows = @(
    [pscustomobject]@{ role = 'control'; control_label = $frameProjection.CsvRoleLabels.control },
    [pscustomobject]@{ role = 'candidate'; control_label = $frameProjection.CsvRoleLabels.candidate }
)
$reportRoot = Join-Path $env:TEMP ('qualification-report-projection-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $reportRoot | Out-Null
try {
    $frameRows | Export-Csv -LiteralPath (Join-Path $reportRoot 'sessions.csv') -NoTypeInformation -Encoding UTF8
    [ordered]@{
        candidate = [ordered]@{
            label = $frameProjection.CandidateLabel
            performance_timing_margin_us = $frameProjection.CandidateMargins.performance_timing_margin_us
            safety_timing_margin_us = $frameProjection.CandidateMargins.safety_timing_margin_us
        }
        control = [ordered]@{
            control_label = $frameProjection.ControlLabel
            performance_timing_margin_us = $frameProjection.ControlMargins.performance_timing_margin_us
            safety_timing_margin_us = $frameProjection.ControlMargins.safety_timing_margin_us
        }
        parameters = $frameProjection.SummaryParameters
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $reportRoot 'summary.json') -Encoding UTF8
    @(
        '- Candidate: ' + $frameProjection.MarkdownCandidatePrefix.TrimEnd()
        '- Control: ' + $frameProjection.MarkdownControlLabel
    ) |
        Set-Content -LiteralPath (Join-Path $reportRoot 'summary.md') -Encoding UTF8
    [ordered]@{
        control_manifest = [ordered]@{
            control_label = $frameProjection.ControlLabel
            performance_timing_margin_us = $frameProjection.ControlMargins.performance_timing_margin_us
            safety_timing_margin_us = $frameProjection.ControlMargins.safety_timing_margin_us
        }
        candidate_manifest = [ordered]@{
            label = $frameProjection.CandidateLabel
            performance_timing_margin_us = $frameProjection.CandidateMargins.performance_timing_margin_us
            safety_timing_margin_us = $frameProjection.CandidateMargins.safety_timing_margin_us
        }
        parameters = $frameProjection.ManifestParameters
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $reportRoot 'manifest.json') -Encoding UTF8

    $csvReport = @(Import-Csv -LiteralPath (Join-Path $reportRoot 'sessions.csv'))
    $summaryReport = Get-Content -LiteralPath (Join-Path $reportRoot 'summary.json') -Raw | ConvertFrom-Json
    $markdownReport = Get-Content -LiteralPath (Join-Path $reportRoot 'summary.md') -Raw
    $manifestReport = Get-Content -LiteralPath (Join-Path $reportRoot 'manifest.json') -Raw | ConvertFrom-Json
    if ($csvReport[0].control_label -ne 'FrameSnapshot-Baseline' -or
        $csvReport[1].control_label -ne 'FrameSnapshot-Candidate' -or
        $summaryReport.candidate.label -ne 'FrameSnapshot-Candidate' -or
        $summaryReport.control.control_label -ne 'FrameSnapshot-Baseline' -or
        $manifestReport.candidate_manifest.label -ne 'FrameSnapshot-Candidate' -or
        $manifestReport.control_manifest.control_label -ne 'FrameSnapshot-Baseline' -or
        -not $markdownReport.Contains('- Candidate: FrameSnapshot-Candidate') -or
        -not $markdownReport.Contains('- Control: FrameSnapshot-Baseline')) {
        throw 'FrameSnapshot role labels differ between CSV, summary JSON, Markdown, and result manifest'
    }
    foreach ($report in @($summaryReport, $manifestReport)) {
        if ($report.parameters.performance_timing_margin_us -ne 0 -or
            $report.parameters.safety_timing_margin_us -ne 500) {
            throw 'FrameSnapshot report parameters lost the performance/safety margin distinction'
        }
    }
    if ($summaryReport.control.performance_timing_margin_us -ne 0 -or
        $summaryReport.control.safety_timing_margin_us -ne 500 -or
        $manifestReport.control_manifest.performance_timing_margin_us -ne 0 -or
        $manifestReport.control_manifest.safety_timing_margin_us -ne 500) {
        throw 'FrameSnapshot baseline report margin provenance is incorrect'
    }
} finally {
    Remove-Item -LiteralPath $reportRoot -Recurse -Force
}

# M3 timing floors mirror acceptance/scenarios.rs: the ordinary acceptance
# hold uses one configured frame; HolInterleaved alone holds for two frames.
$acceptanceLimitsSource = Get-Content -LiteralPath (Join-Path $repositoryRoot 'rust\crates\sky_player\src\bin\acceptance\acceptance_limits.rs') -Raw
$acceptanceScenariosSource = Get-Content -LiteralPath (Join-Path $repositoryRoot 'rust\crates\sky_player\src\bin\acceptance\scenarios.rs') -Raw
if ($acceptanceLimitsSource -notmatch 'ACCEPTANCE_HOLD_FRAMES:\s*u64\s*=\s*1\s*;' -or
    $acceptanceScenariosSource -notmatch 'Scenario::HolInterleaved\s*\{\s*2\s*\}\s*else\s*\{\s*ACCEPTANCE_HOLD_FRAMES') {
    throw 'native acceptance hold-frame source contract changed; M3 timing expectations need review'
}
$m3Floors60Margin0 = Get-DispatchCleanupM3TimingFloors -Scenario 'canonical-single' -GameFps 60 -TimingMarginUs 0
$m3Floors60Margin500 = Get-DispatchCleanupM3TimingFloors -Scenario 'canonical-single' -GameFps 60 -TimingMarginUs 500
$m3Floors120Margin500 = Get-DispatchCleanupM3TimingFloors -Scenario 'canonical-single' -GameFps 120 -TimingMarginUs 500
$m3HolFloors60Margin500 = Get-DispatchCleanupM3TimingFloors -Scenario 'hol-interleaved' -GameFps 60 -TimingMarginUs 500
if ($m3Floors60Margin0.configured_H_us -ne 16667 -or $m3Floors60Margin0.configured_G_us -ne 16667 -or
    $m3Floors60Margin500.configured_H_us -ne 17167 -or $m3Floors60Margin500.configured_G_us -ne 17167 -or
    $m3Floors120Margin500.configured_H_us -ne 8834 -or $m3Floors120Margin500.configured_G_us -ne 8834 -or
    $m3HolFloors60Margin500.configured_H_us -ne 33834 -or $m3HolFloors60Margin500.configured_G_us -ne 17167 -or
    $m3HolFloors60Margin500.hold_frames -ne 2) {
    throw 'DispatchCleanupM3 frame, hold, or release-gap arithmetic does not match native scenario configuration'
}

$m3ReportCandidate = [pscustomobject]@{
    qualification_profile = 'DispatchCleanupM3'; source_role = 'Candidate'
    workload_version = 'dispatch-runtime-cleanup-m3-v1'; observer_mode = 'SchedulerQueuedV3Compat3'
    guard_mode = 'frozen'; performance_timing_margin_us = 0; safety_timing_margin_us = 500
}
$m3ReportBaseline = [pscustomobject]@{
    qualification_profile = 'DispatchCleanupM3'; source_role = 'Baseline'
    workload_version = 'dispatch-runtime-cleanup-m3-v1'; observer_mode = 'SchedulerQueuedV3Compat3'
    guard_mode = 'frozen'; performance_timing_margin_us = 0; safety_timing_margin_us = 500
}
$m3SafetyScenarioNames = @(
    'canonical-single', 'canonical-max-chord', 'near-minimum-retrigger', 'release-gap-stress',
    'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary',
    'modifier-held-after-owned', 'focus-loss', 'target-hwnd-change', 'owner-mismatch',
    'owner-query-failure', 'owner-process-termination', 'pause-resume', 'pause-resume-release-floor',
    'suspend-resume', 'stop-cleanup', 'skip-cleanup', 'supervisor-lease-expiry',
    'lease-age-watchdog-delayed', 'noncanonical-profile'
)
$m3CompareWorkloads = @('dense-alternating', 'mixed-up-down', 'canonical-max-chord')
$m3FinalWorkloads = @('dense-alternating', 'mixed-up-down', 'canonical-max-chord', 'near-minimum-retrigger', 'long-idle')
$m3CanaryProjection = New-QualificationReportProjection -Profile DispatchCleanupM3 -Candidate $m3ReportCandidate `
    -Stage Canary -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 -SafetyScenarios $m3SafetyScenarioNames
$m3SmokeProjection = New-QualificationReportProjection -Profile DispatchCleanupM3 -Candidate $m3ReportCandidate `
    -Stage Smoke -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 -SafetyScenarios $m3SafetyScenarioNames
$m3CompareProjection = New-QualificationReportProjection -Profile DispatchCleanupM3 -Candidate $m3ReportCandidate `
    -Control $m3ReportBaseline -Stage Compare -Pairs 6 -SampleSeconds 15 -TimingMarginUs 0 `
    -SafetyScenarios $m3SafetyScenarioNames -PerformanceWorkloads $m3CompareWorkloads
$m3FinalProjection = New-QualificationReportProjection -Profile DispatchCleanupM3 -Candidate $m3ReportCandidate `
    -Control $m3ReportBaseline -Stage Final -Pairs 6 -SampleSeconds 15 -TimingMarginUs 0 `
    -SafetyScenarios $m3SafetyScenarioNames -PerformanceWorkloads $m3FinalWorkloads

if ($m3CanaryProjection.StageSummary.planned_session_count -ne 1 -or
    $m3CanaryProjection.StageSummary.stage_kind -ne 'candidate-only safety Canary' -or
    $m3CanaryProjection.StageSummary.candidate_session_count -ne 1 -or
    $m3CanaryProjection.StageSummary.control_session_count -ne 0 -or
    $m3CanaryProjection.StageSummary.configured_H_us -ne 17167 -or
    $m3CanaryProjection.StageSummary.configured_G_us -ne 17167 -or
    $m3CanaryProjection.SummaryParameters.pairs -ne $null -or
    $m3CanaryProjection.SummaryParameters.sample_seconds -ne $null -or
    $m3CanaryProjection.ManifestParameters.pairs -ne $null -or
    $m3CanaryProjection.ManifestParameters.sample_seconds -ne $null -or
    $m3CanaryProjection.ManifestParameters.pair_order -ne $null -or
    $m3CanaryProjection.SummaryParameters.performance_timing_margin_us -ne 0 -or
    $m3CanaryProjection.SummaryParameters.safety_timing_margin_us -ne 500 -or
    -not $m3CanaryProjection.MarkdownStageLines[0].Contains('Canary') -or
    -not (($m3CanaryProjection.MarkdownStageLines -join "`n").Contains('candidate-only safety stage')) -or
    -not (($m3CanaryProjection.MarkdownStageLines -join "`n").Contains('no timed performance samples')) -or
    (($m3CanaryProjection.MarkdownStageLines -join "`n") -match 'AB/BA|six pairs|15 seconds per performance session')) {
    throw 'DispatchCleanupM3 Canary projection implies a comparison/sample or reports incorrect H/G/session identity'
}
if ($m3SmokeProjection.StageSummary.planned_session_count -ne 22 -or
    $m3SmokeProjection.StageSummary.stage_kind -ne 'safety-only; candidate-only' -or
    $m3SmokeProjection.StageSummary.candidate_session_count -ne 22 -or
    $m3SmokeProjection.StageSummary.control_session_count -ne 0 -or
    $m3SmokeProjection.StageSummary.workload_summary -ne '22 candidate-only safety scenarios' -or
    -not $m3SmokeProjection.MarkdownStageLines[0].Contains('safety-only') -or
    $m3SmokeProjection.StageSummary.timed_performance_samples -or
    $m3SmokeProjection.StageSummary.configured_H_us -ne 17167 -or
    $m3SmokeProjection.StageSummary.configured_G_us -ne 17167) {
    throw 'DispatchCleanupM3 Smoke projection does not describe its candidate-only safety cases'
}
if ($m3CompareProjection.StageSummary.planned_session_count -ne 72 -or
    $m3CompareProjection.StageSummary.stage_kind -ne 'paired performance' -or
    $m3CompareProjection.StageSummary.candidate_session_count -ne 36 -or
    $m3CompareProjection.StageSummary.control_session_count -ne 36 -or
    $m3CompareProjection.StageSummary.game_fps -ne 60 -or
    $m3CompareProjection.StageSummary.timing_margin_us -ne 0 -or
    $m3CompareProjection.StageSummary.configured_H_us -ne 16667 -or
    $m3CompareProjection.StageSummary.configured_G_us -ne 16667 -or
    $m3CompareProjection.StageSummary.pairs -ne 6 -or
    -not $m3CompareProjection.StageSummary.pair_order.Contains('AB/BA') -or
    -not $m3CompareProjection.ManifestParameters.pair_order.Contains('AB/BA') -or
    $m3CompareProjection.SummaryParameters.performance_timing_margin_us -ne 0 -or
    $m3CompareProjection.SummaryParameters.safety_timing_margin_us -ne 500) {
    throw 'DispatchCleanupM3 Compare projection lost its paired performance protocol or per-session timing settings'
}
if ($m3FinalProjection.StageSummary.planned_session_count -ne 166 -or
    $m3FinalProjection.StageSummary.stage_kind -ne 'paired performance plus candidate-only safety and boundaries' -or
    $m3FinalProjection.StageSummary.candidate_session_count -ne 106 -or
    $m3FinalProjection.StageSummary.control_session_count -ne 60 -or
    $null -ne $m3FinalProjection.StageSummary.game_fps -or
    $null -ne $m3FinalProjection.StageSummary.timing_margin_us -or
    $null -ne $m3FinalProjection.StageSummary.configured_H_us -or
    $null -ne $m3FinalProjection.StageSummary.configured_G_us -or
    $m3FinalProjection.StageSummary.configured_H_G_mode -ne 'varies_by_session' -or
    -not $m3FinalProjection.StageSummary.configured_H_G_summary.Contains('varies by session') -or
    -not $m3FinalProjection.ManifestParameters.configured_H_G_summary.Contains('varies by session') -or
    -not $m3FinalProjection.StageSummary.timing_margin_summary.Contains('0 us performance; 500 us safety') -or
    $m3FinalProjection.StageSummary.timed_performance_sample_seconds -ne 15) {
    throw 'DispatchCleanupM3 Final projection fabricated aggregate timing values for a mixed stage'
}

$m3RunConfig = [pscustomobject]@{
    qualification_profile = 'DispatchCleanupM3'; scenario = 'canonical-single'; workload = 'canonical-single'
    game_fps = 60; timing_margin_us = 500; expected_hold_us = 17167; expected_release_gap_us = 17167
    configured_H_us = 17167; configured_G_us = 17167
}
$verifiedCanaryFloors = Assert-DispatchCleanupM3RunConfigTiming -ExpectedScenario 'canonical-single' `
    -RunConfig $m3RunConfig -ExpectedWorkload 'canonical-single' -ExpectedGameFps 60 -ExpectedTimingMarginUs 500
$m3CanaryRow = [pscustomobject]@{
    stage = $m3CanaryProjection.StageSummary.stage
    qualification_profile = $m3CanaryProjection.StageSummary.qualification_profile
    planned_session_count = $m3CanaryProjection.StageSummary.planned_session_count
    attempted_session_count = 1; role = 'candidate'; scenario = $m3RunConfig.scenario
    game_fps = $m3RunConfig.game_fps; timing_margin_us = $m3RunConfig.timing_margin_us
    configured_H_us = $m3RunConfig.configured_H_us; configured_G_us = $m3RunConfig.configured_G_us
}
$m3ReportRoot = Join-Path $env:TEMP ('m3-report-contract-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $m3ReportRoot | Out-Null
try {
    $m3CanaryRow | Export-Csv -LiteralPath (Join-Path $m3ReportRoot 'sessions.csv') -NoTypeInformation -Encoding UTF8
    [ordered]@{
        stage = $m3CanaryProjection.StageSummary.stage
        qualification_profile = $m3CanaryProjection.StageSummary.qualification_profile
        session_plan = $m3CanaryProjection.StageSummary
        parameters = $m3CanaryProjection.SummaryParameters
    } | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $m3ReportRoot 'summary.json') -Encoding UTF8
    $csvCanary = Import-Csv -LiteralPath (Join-Path $m3ReportRoot 'sessions.csv') | Select-Object -First 1
    $jsonCanary = Get-Content -LiteralPath (Join-Path $m3ReportRoot 'summary.json') -Raw | ConvertFrom-Json
    if ([long]$m3RunConfig.expected_hold_us -ne [long]$jsonCanary.session_plan.configured_H_us -or
        [long]$m3RunConfig.expected_release_gap_us -ne [long]$jsonCanary.session_plan.configured_G_us -or
        [long]$csvCanary.configured_H_us -ne [long]$jsonCanary.session_plan.configured_H_us -or
        [long]$csvCanary.configured_G_us -ne [long]$jsonCanary.session_plan.configured_G_us -or
        $csvCanary.stage -ne $jsonCanary.stage -or
        $csvCanary.qualification_profile -ne $jsonCanary.qualification_profile -or
        $csvCanary.planned_session_count -ne $jsonCanary.session_plan.planned_session_count -or
        $csvCanary.role -ne 'candidate' -or $csvCanary.game_fps -ne $jsonCanary.session_plan.game_fps -or
        $csvCanary.timing_margin_us -ne $jsonCanary.session_plan.timing_margin_us -or
        $verifiedCanaryFloors.configured_H_us -ne 17167 -or $verifiedCanaryFloors.configured_G_us -ne 17167) {
        throw 'DispatchCleanupM3 Canary run-config, CSV, and JSON timing/stage identity disagree'
    }
} finally {
    Remove-Item -LiteralPath $m3ReportRoot -Recurse -Force
}
$badM3HRunConfig = $m3RunConfig | ConvertTo-Json -Depth 4 | ConvertFrom-Json
$badM3HRunConfig.configured_H_us = 33834
$badM3HRunConfig.expected_hold_us = 33834
$badM3HRejected = $false
try { Assert-DispatchCleanupM3RunConfigTiming -ExpectedScenario canonical-single -RunConfig $badM3HRunConfig `
    -ExpectedWorkload canonical-single -ExpectedGameFps 60 -ExpectedTimingMarginUs 500 | Out-Null } catch { $badM3HRejected = $true }
if (-not $badM3HRejected) { throw 'DispatchCleanupM3 accepted the old two-frame H on canonical-single' }
$m3NegativeContractCount++
$wrongScenarioRunConfig = $m3RunConfig | ConvertTo-Json -Depth 4 | ConvertFrom-Json
$wrongScenarioRejected = $false
try { Assert-DispatchCleanupM3RunConfigTiming -ExpectedScenario hol-interleaved -RunConfig $wrongScenarioRunConfig `
    -ExpectedWorkload canonical-single -ExpectedGameFps 60 -ExpectedTimingMarginUs 500 | Out-Null } catch { $wrongScenarioRejected = $true }
if (-not $wrongScenarioRejected) { throw 'DispatchCleanupM3 accepted incorrect scenario attribution for H/G reporting' }
$m3NegativeContractCount++
$badM3GRunConfig = $m3RunConfig | ConvertTo-Json -Depth 4 | ConvertFrom-Json
$badM3GRunConfig.configured_G_us = $badM3GRunConfig.configured_G_us + 1
$badM3GRejected = $false
try { Assert-DispatchCleanupM3RunConfigTiming -ExpectedScenario canonical-single -RunConfig $badM3GRunConfig `
    -ExpectedWorkload canonical-single -ExpectedGameFps 60 -ExpectedTimingMarginUs 500 | Out-Null } catch { $badM3GRejected = $true }
if (-not $badM3GRejected) { throw 'DispatchCleanupM3 accepted a release gap different from frame interval plus margin' }
$m3NegativeContractCount++

$sessionRedesignProfile = Get-DispatchSessionRedesignM3Profile
if ($sessionRedesignProfile.BaselineRevision -ne 'd0f65e7c412041a6d502f81f469872198db7adc5' -or
    $sessionRedesignProfile.BaselineTree -ne '19aae9e41bc667a2f991001ed105b6ffdebb8bc6' -or
    $sessionRedesignProfile.CandidateLineageBranch -ne 'refactor/dispatch-session-redesign' -or
    $sessionRedesignProfile.HarnessManifestSchemaVersion -ne 2 -or
    @($sessionRedesignProfile.RunnerProvenancePaths).Count -ne 11 -or
    $sessionRedesignProfile.Target -ne 'x86_64-pc-windows-msvc' -or
    $sessionRedesignProfile.RustVersion -ne '1.99.1' -or $sessionRedesignProfile.CargoVersion -ne '1.99.1' -or
    $sessionRedesignProfile.CargoProfile -ne 'dist' -or
    @($sessionRedesignProfile.RequiredFeatures).Count -ne 1 -or $sessionRedesignProfile.RequiredFeatures[0] -ne 'real-input-acceptance' -or
    $sessionRedesignProfile.PerformanceTimingMarginUs -ne 500 -or $sessionRedesignProfile.SafetyTimingMarginUs -ne 500 -or
    $sessionRedesignProfile.ObserverMode -ne 'SchedulerQueuedV3Compat3' -or
    @($sessionRedesignProfile.SafetyCases).Count -ne 7 -or @($sessionRedesignProfile.SafetyScenarios).Count -ne 7) {
    throw 'DispatchSessionRedesignM3 profile differs from the pinned baseline, Windows toolchain, immutable build, or seven-case safety plan'
}
if ($sessionRedesignProfile.CanaryCase.Scenario -ne 'canonical-single' -or
    $sessionRedesignProfile.CanaryCase.Workload -ne 'canonical-single' -or
    $sessionRedesignProfile.CanaryCase.Load -ne 'Quiet' -or
    $sessionRedesignProfile.PerformanceCase.Scenario -ne 'mixed-up-down' -or
    $sessionRedesignProfile.PerformanceCase.Workload -ne 'mixed-up-down' -or
    (@($sessionRedesignProfile.PerformanceCase.LoadCells) -join '|') -ne 'Quiet|Contention' -or
    $sessionRedesignProfile.PerformanceCase.GameFps -ne 60 -or
    $sessionRedesignProfile.PerformanceCase.TimingMarginUs -ne 500 -or
    $sessionRedesignProfile.PerformanceCase.PairCount -ne 6 -or
    $sessionRedesignProfile.PerformanceCase.SampleSeconds -ne 15) {
    throw 'DispatchSessionRedesignM3 canary or paired workload mapping differs from the approved run proposal'
}
$expectedSafetyIds = @('focus-loss', 'stop-cleanup', 'pause-resume', 'owner-process-termination', 'target-hwnd-change', 'near-minimum-retrigger@60', 'near-minimum-retrigger@120')
if ((@($sessionRedesignProfile.SafetyScenarios) -join '|') -ne ($expectedSafetyIds -join '|')) {
    throw 'DispatchSessionRedesignM3 safety case order or IDs differ from the proposed seven sessions'
}
foreach ($safetyCase in $sessionRedesignProfile.SafetyCases) {
    $expectedWorkload = if ($safetyCase.Scenario -eq 'near-minimum-retrigger') { 'near-minimum-retrigger' } else { 'safety-smoke' }
    $expectedFps = if ($safetyCase.Id -eq 'near-minimum-retrigger@120') { 120 } else { 60 }
    if ($safetyCase.Workload -ne $expectedWorkload -or $safetyCase.Load -ne 'Quiet' -or
        $safetyCase.GameFps -ne $expectedFps -or $safetyCase.TimingMarginUs -ne 500 -or
        $safetyCase.SampleSeconds -ne 0) {
        throw ('DispatchSessionRedesignM3 safety case identity is invalid: ' + $safetyCase.Id)
    }
}
$floors60 = Get-DispatchSessionRedesignM3TimingFloors -Scenario canonical-single -GameFps 60 -TimingMarginUs 500
$floors120 = Get-DispatchSessionRedesignM3TimingFloors -Scenario near-minimum-retrigger -GameFps 120 -TimingMarginUs 500
if ($floors60.frame_us -ne 16667 -or $floors60.configured_H_us -ne 17167 -or $floors60.configured_G_us -ne 17167 -or
    $floors120.frame_us -ne 8334 -or $floors120.configured_H_us -ne 8834 -or $floors120.configured_G_us -ne 8834) {
    throw 'DispatchSessionRedesignM3 H/G floors differ from ceiling frame interval plus the fixed 500 us margin'
}
Assert-QualificationTimingMargin -RequestedStage Final -Profile DispatchSessionRedesignM3 -MarginUs 500 -PairCount 6 -SampleDurationSeconds 15 | Out-Null
$sessionRedesignValidCases = @(
    @{ scenario = 'canonical-single'; workload = 'canonical-single'; margin = 500; fps = 60; sample = 0 },
    @{ scenario = 'focus-loss'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 0 },
    @{ scenario = 'stop-cleanup'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 0 },
    @{ scenario = 'pause-resume'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 0 },
    @{ scenario = 'owner-process-termination'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 0 },
    @{ scenario = 'target-hwnd-change'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 0 },
    @{ scenario = 'near-minimum-retrigger'; workload = 'near-minimum-retrigger'; margin = 500; fps = 60; sample = 0 },
    @{ scenario = 'near-minimum-retrigger'; workload = 'near-minimum-retrigger'; margin = 500; fps = 120; sample = 0 },
    @{ scenario = 'mixed-up-down'; workload = 'mixed-up-down'; margin = 500; fps = 60; sample = 15 }
)
foreach ($case in $sessionRedesignValidCases) {
    Assert-DispatchSessionRedesignM3SessionIdentity -Scenario $case.scenario -Workload $case.workload `
        -TimingMarginUs $case.margin -GameFps $case.fps -SampleSeconds $case.sample | Out-Null
}
$sessionRedesignInvalidCases = @(
    @{ name = 'performance zero margin'; scenario = 'mixed-up-down'; workload = 'mixed-up-down'; margin = 0; fps = 60; sample = 15 },
    @{ name = 'performance wrong workload'; scenario = 'mixed-up-down'; workload = 'dense-alternating'; margin = 500; fps = 60; sample = 15 },
    @{ name = 'performance wrong fps'; scenario = 'mixed-up-down'; workload = 'mixed-up-down'; margin = 500; fps = 120; sample = 15 },
    @{ name = 'performance wrong duration'; scenario = 'mixed-up-down'; workload = 'mixed-up-down'; margin = 500; fps = 60; sample = 0 },
    @{ name = 'safety wrong margin'; scenario = 'focus-loss'; workload = 'safety-smoke'; margin = 0; fps = 60; sample = 0 },
    @{ name = 'safety timed sample'; scenario = 'stop-cleanup'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 15 },
    @{ name = 'near minimum wrong workload'; scenario = 'near-minimum-retrigger'; workload = 'safety-smoke'; margin = 500; fps = 120; sample = 0 },
    @{ name = 'unsupported case'; scenario = 'suspend-resume'; workload = 'safety-smoke'; margin = 500; fps = 60; sample = 0 },
    @{ name = 'canary wrong fps'; scenario = 'canonical-single'; workload = 'canonical-single'; margin = 500; fps = 120; sample = 0 }
)
foreach ($case in $sessionRedesignInvalidCases) {
    $rejected = $false
    try { Assert-DispatchSessionRedesignM3SessionIdentity -Scenario $case.scenario -Workload $case.workload -TimingMarginUs $case.margin -GameFps $case.fps -SampleSeconds $case.sample | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw ('DispatchSessionRedesignM3 accepted invalid session ' + $case.name) }
}
$sessionRedesignProjection = New-QualificationReportProjection -Profile DispatchSessionRedesignM3 `
    -Candidate ([pscustomobject]@{ qualification_profile = 'DispatchSessionRedesignM3'; workload_version = $sessionRedesignProfile.WorkloadVersion; performance_timing_margin_us = 500; safety_timing_margin_us = 500; observer_mode = 'qualification-on'; guard_mode = 'frozen'; receiveonly_observer_mode = 'SchedulerQueuedV3Compat3'; receiveonly_observer_version = $sessionRedesignProfile.ObserverVersion }) `
    -Control ([pscustomobject]@{ qualification_profile = 'DispatchSessionRedesignM3'; performance_timing_margin_us = 500; safety_timing_margin_us = 500 }) `
    -Stage Final -Pairs 6 -SampleSeconds 15 -TimingMarginUs 500 `
    -SafetyScenarios $sessionRedesignProfile.SafetyScenarios -PerformanceWorkloads @('mixed-up-down')
if ($sessionRedesignProjection.StageSummary.planned_session_count -ne 32 -or
    $sessionRedesignProjection.StageSummary.candidate_session_count -ne 20 -or
    $sessionRedesignProjection.StageSummary.control_session_count -ne 12 -or
    $sessionRedesignProjection.StageSummary.safety_timing_margin_us -ne 500 -or
    $sessionRedesignProjection.StageSummary.performance_timing_margin_us -ne 500 -or
    $sessionRedesignProjection.StageSummary.game_fps_summary -ne 'varies by session (60, 120 FPS)' -or
    $sessionRedesignProjection.StageSummary.configured_H_G_summary -ne '60 FPS H=G=17,167 us; 120 FPS H=G=8,834 us' -or
    -not $sessionRedesignProjection.StageSummary.timed_performance_samples -or
    $sessionRedesignProjection.StageSummary.timed_performance_sample_seconds -ne 15) {
    throw 'DispatchSessionRedesignM3 report projection lost its session count, timing, workload, FPS, or sample provenance'
}
if ($sessionRedesignProjection.CandidateLabel -ne 'DispatchSessionRedesignM3-Candidate' -or
    $sessionRedesignProjection.ControlLabel -ne 'DispatchSessionRedesignM3-Baseline' -or
    $sessionRedesignProjection.ManifestParameters.planned_session_count -ne 32 -or
    $sessionRedesignProjection.ManifestParameters.observer_mode -ne 'qualification-on' -or
    $sessionRedesignProjection.ManifestParameters.guard_mode -ne 'frozen') {
    throw 'DispatchSessionRedesignM3 report labels, manifest count, observer, or waiter provenance changed'
}

Set-StrictMode -Version Latest
$collectionFixtures = @(
    [pscustomobject]@{ name = 'zero'; value = $null; expected = 0 },
    [pscustomobject]@{ name = 'one'; value = 'mixed-up-down'; expected = 1 },
    [pscustomobject]@{ name = 'many'; value = @('Quiet', 'Contention', 'Safety'); expected = 3 }
)
foreach ($fixture in $collectionFixtures) {
    $collection = ConvertTo-DispatchQualificationCollection -Value $fixture.value
    if ($collection.Items -isnot [object[]] -or $collection.Length -ne $fixture.expected -or
        $collection.Items.Length -ne $fixture.expected) {
        throw ('StrictMode collection normalization failed for cardinality ' + $fixture.name)
    }
}

$offlineOrder = [System.Collections.Generic.List[string]]::new()
$offlineCalls = [System.Collections.Generic.List[object]]::new()
$offlineCandidateIdentity = [pscustomobject]@{ role = 'Candidate'; validated = $true }
$offlineBaselineIdentity = [pscustomobject]@{ role = 'Baseline'; validated = $true }
$offlineFinal = Invoke-DispatchSessionRedesignM3FinalControlPath -Profile $sessionRedesignProfile `
    -Pairs 6 -TimingMarginUs 500 -SampleSeconds 15 `
    -OnPlan {
        param($plan)
        $offlineOrder.Add('plan')
    } -InvokeCase {
        param($case)
        $offlineOrder.Add('case')
        $identity = if ($case.role -eq 'control') { $offlineBaselineIdentity } else { $offlineCandidateIdentity }
        if (-not $identity.validated) { throw 'offline Final attempted a case with an unvalidated synthetic identity' }
        $offlineCalls.Add([pscustomobject]@{ case = $case; identity_role = $identity.role })
        [pscustomobject]@{ case_exit_code = 0; status = 'PASS'; identity_role = $identity.role }
    }
$offlinePlan = $offlineFinal.Plan
$offlinePerformanceCases = [object[]]@($offlinePlan.Cases | Where-Object phase -eq 'performance')
if ($offlinePlan.Cases -isnot [object[]] -or $offlinePlan.PlannedSessionCount -ne 32 -or
    $offlinePlan.CandidateSessionCount -ne 20 -or $offlinePlan.ControlSessionCount -ne 12 -or
    $offlineFinal.Execution.AttemptedSessionCount -ne 32 -or
    $offlineFinal.Execution.StoppedAtFirstNonPass -or $offlineCalls.Count -ne 32 -or
    $offlineOrder.Count -ne 33 -or $offlineOrder[0] -ne 'plan' -or $offlineOrder[1] -ne 'case' -or
    $offlinePlan.Cases[0].phase -ne 'canary' -or
    $offlinePlan.Cases[0].scenario -ne 'canonical-single' -or
    $offlinePlan.Cases[0].workload -ne 'canonical-single' -or
    $offlinePlan.Cases[0].role -ne 'candidate' -or
    (@($offlinePlan.Cases | Select-Object -Skip 1 -First 7 | ForEach-Object { $_.id }) -join '|') -ne ($expectedSafetyIds -join '|') -or
    $offlinePerformanceCases.Length -ne 24) {
    throw 'offline top-level M3 Final control path lost its 32-case plan, candidate/baseline counts, canary, safety order, or invocation order'
}
for ($caseIndex = 0; $caseIndex -lt $offlinePlan.Cases.Length; $caseIndex++) {
    if ($offlineCalls[$caseIndex].case.id -ne $offlinePlan.Cases[$caseIndex].id) {
        throw ('offline M3 Final invoked a case out of plan order at index ' + $caseIndex)
    }
}
if (@($offlineCalls | Where-Object identity_role -eq 'Candidate').Length -ne 20 -or
    @($offlineCalls | Where-Object identity_role -eq 'Baseline').Length -ne 12) {
    throw 'offline M3 Final did not route exactly 20 candidate and 12 baseline cases to their validated identities'
}
foreach ($load in @('Quiet', 'Contention')) {
    [object[]]$cellCases = @($offlinePerformanceCases | Where-Object load -eq $load)
    if ($cellCases.Length -ne 12) { throw ('offline M3 Final plan has an invalid ' + $load + ' cell size') }
    for ($pair = 1; $pair -le 6; $pair++) {
        $pairId = 'mixed-up-down/' + $load + '/' + ('{0:D2}' -f $pair)
        [object[]]$pairCases = @($cellCases | Where-Object pair_id -eq $pairId)
        $expectedOrder = if ($pair % 2 -eq 1) { 'AB' } else { 'BA' }
        $expectedRoles = if ($expectedOrder -eq 'AB') { 'control,candidate' } else { 'candidate,control' }
        if ($pairCases.Length -ne 2 -or
            (@($pairCases | ForEach-Object { $_.pair_order } | Select-Object -Unique) -join '') -ne $expectedOrder -or
            (@($pairCases | ForEach-Object { $_.role }) -join ',') -ne $expectedRoles -or
            (@($pairCases | ForEach-Object { $_.historical }) -join ',') -ne $(if ($expectedOrder -eq 'AB') { 'True,False' } else { 'False,True' })) {
            throw ('offline M3 Final pair mapping/order is invalid for ' + $pairId)
        }
    }
}

$firstFailureCalls = [System.Collections.Generic.List[object]]::new()
$firstFailure = Invoke-DispatchSessionRedesignM3FinalControlPath -Profile $sessionRedesignProfile `
    -Pairs 6 -TimingMarginUs 500 -SampleSeconds 15 -InvokeCase {
        param($case)
        $firstFailureCalls.Add($case)
        [pscustomobject]@{ case_exit_code = 1; status = 'FAIL' }
    }
if (-not $firstFailure.Execution.StoppedAtFirstNonPass -or
    $firstFailure.Execution.AttemptedSessionCount -ne 1 -or
    $firstFailureCalls.Count -ne 1 -or
    $firstFailureCalls[0].phase -ne 'canary' -or
    $firstFailureCalls[0].scenario -ne 'canonical-single') {
    throw 'offline M3 Final first-case failure did not stop before the second case'
}

$zeroAttemptCalls = [System.Collections.Generic.List[object]]::new()
$zeroAttemptError = $null
try {
    [void](Invoke-DispatchSessionRedesignM3FinalControlPath -Profile $sessionRedesignProfile `
        -Pairs 5 -TimingMarginUs 500 -SampleSeconds 15 -InvokeCase {
            param($case)
            $zeroAttemptCalls.Add($case)
            [pscustomobject]@{ case_exit_code = 0; status = 'PASS' }
        })
} catch { $zeroAttemptError = $_ }
if ($null -eq $zeroAttemptError -or $zeroAttemptCalls.Count -ne 0) {
    throw 'offline M3 Final pre-case planning failure did not preserve zero attempts'
}
$offlineFailureRoot = Join-Path $env:TEMP ('m3-runner-failure-contract-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $offlineFailureRoot | Out-Null
try {
    $zeroFailureArtifacts = New-QualificationRunnerFailureArtifacts -ErrorRecord $zeroAttemptError `
        -RunId 'offline-zero-attempts' -Stage 'Final' -SourceRevision 'd76d4fb7cdcc49fcd0f0a7d80c1f691578c86fed' `
        -SourceTreeSha '17a8ea0d3a22ee4bf2771988dc92fa9ddabfe926' -AttemptedSessionCount $zeroAttemptCalls.Count `
        -EvidenceRoot $offlineFailureRoot -FallbackScriptPath $runnerPath
    Write-QualificationRunnerFailureArtifacts -Artifacts $zeroFailureArtifacts -EvidenceRoot $offlineFailureRoot `
        -CommandRows ([System.Collections.Generic.List[string]]::new())
    $zeroSummary = Get-Content -LiteralPath (Join-Path $offlineFailureRoot 'summary.json') -Raw | ConvertFrom-Json
    $zeroFailures = Get-Content -LiteralPath (Join-Path $offlineFailureRoot 'failures.json') -Raw | ConvertFrom-Json
    $zeroErrorDetail = Get-Content -LiteralPath (Join-Path $offlineFailureRoot 'runner-error.json') -Raw | ConvertFrom-Json
    $zeroMarkdown = Get-Content -LiteralPath (Join-Path $offlineFailureRoot 'summary.md') -Raw
    $zeroSummaryKeys = @($zeroSummary.PSObject.Properties.Name) -join '|'
    if ($zeroSummary.run_count -ne 0 -or $zeroSummary.disposition -ne 'runner failed before completing protocol' -or
        @($zeroSummary.failures).Length -ne 1 -or @($zeroFailures).Length -ne 1 -or
        $zeroFailures[0].status -ne 'RUNNER_FAILURE' -or
        $zeroSummaryKeys -ne 'schema_version|run_id|stage|source_revision|source_tree_sha|run_count|failures|disposition' -or
        [string]::IsNullOrWhiteSpace([string]$zeroErrorDetail.script_path) -or
        [int]$zeroErrorDetail.line_number -le 0 -or
        [string]::IsNullOrWhiteSpace([string]$zeroErrorDetail.script_stack_trace) -or
        [string]::IsNullOrWhiteSpace([string]$zeroErrorDetail.exception_to_string) -or
        $zeroMarkdown -notmatch 'NotRun') {
        throw 'offline pre-case runner failure did not write accurate zero-attempt summary and exception evidence'
    }
} finally {
    $safeTempRoot = [System.IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    $safeFailureRoot = [System.IO.Path]::GetFullPath($offlineFailureRoot)
    if (-not $safeFailureRoot.StartsWith($safeTempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'refusing to clean offline runner failure fixtures outside the temporary directory'
    }
    Remove-Item -LiteralPath $offlineFailureRoot -Recurse -Force
}

$sessionRedesignRunner = Get-Content -LiteralPath $runnerPath -Raw
$sessionRedesignOrchestration = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'dispatch_qualification_orchestration.psm1') -Raw
$sessionRedesignBuilder = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1') -Raw
$sessionRedesignManifestValidator = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Raw
if ($sessionRedesignRunner -notmatch 'if \(\$isSessionRedesignProfile -and \$Stage -ne ''Final''\)' -or
    -not $sessionRedesignRunner.Contains('Invoke-DispatchSessionRedesignM3FinalControlPath') -or
    -not $sessionRedesignRunner.Contains('[string[]]$performanceWorkloads') -or
    $sessionRedesignRunner -match '\$performanceWorkloads\.Count' -or
    -not $sessionRedesignRunner.Contains('New-QualificationRunnerFailureArtifacts') -or
    -not $sessionRedesignBuilder.Contains('schema_version = if ($isSessionRedesignProfile) { $sessionRedesignProfile.HarnessManifestSchemaVersion } elseif ($isPlaybackCoreLifecycle) { $playbackCoreProfile.HarnessManifestSchemaVersion } else { 1 }') -or
    $sessionRedesignManifestValidator -notmatch "schema_version.*profile\.HarnessManifestSchemaVersion" -or
    -not $sessionRedesignOrchestration.Contains('New-DispatchSessionRedesignM3FinalPlan') -or
    -not $sessionRedesignOrchestration.Contains('Invoke-DispatchSessionRedesignM3FinalPlan') -or
    -not (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'dispatch_qualification_profiles.psm1') -Raw).Contains('scripts/dispatch_qualification_orchestration.psm1')) {
    throw 'DispatchSessionRedesignM3 Final path is missing the typed-plan dispatcher, fail-stop runner, or provenance inventory'
}

$playbackCoreProfileSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'dispatch_qualification_profiles.psm1') -Raw
$playbackCoreBuilderSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1') -Raw
$playbackCoreManifestSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Raw
$playbackCoreRunnerSource = Get-Content -LiteralPath $runnerPath -Raw
$playbackCoreCollectorSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'lib/playback_core_lifecycle_collector.psm1') -Raw
$playbackCoreCaseRunnerSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1') -Raw
if (-not $playbackCoreProfileSource.Contains("Name = 'PlaybackCoreLifecycle'") -or
    -not $playbackCoreProfileSource.Contains("BaselineRevision = '2544daa13eb23fdf03bd922e65b96fcffaba421c'") -or
    -not $playbackCoreProfileSource.Contains("CandidateLineageBranch = 'refactor/playback-core-lifecycle'") -or
    -not $playbackCoreProfileSource.Contains("'near-minimum-retrigger'") -or
    -not $playbackCoreBuilderSource.Contains('Assert-PlaybackCoreLifecycleSourceCheckout') -or
    -not $playbackCoreBuilderSource.Contains("['qualification_plan'] = [ordered]@{") -or
    -not $playbackCoreBuilderSource.Contains("cargo_lock_sha256") -or
    -not $playbackCoreManifestSource.Contains('Assert-PlaybackCoreLifecycleManifestIdentity') -or
    -not $playbackCoreManifestSource.Contains('Assert-PlaybackCoreLifecycleRunnerProvenance') -or
    -not $playbackCoreRunnerSource.Contains('[switch]$PlanOnly') -or
    -not $playbackCoreRunnerSource.Contains('native_attempts=0 target_capture=NOT_RUN') -or
    -not $playbackCoreRunnerSource.Contains('[switch]$OwnerApprovedReceiveOnlyRun') -or
    -not $playbackCoreRunnerSource.Contains('repository owner explicitly approves this Windows machine, target receiver, and physical input scope') -or
    -not $playbackCoreProfileSource.Contains("target_class = 'project-owned-non-game-receive-only'") -or
    -not $playbackCoreRunnerSource.Contains('New-PlaybackCoreLifecycleMeasurementReport') -or
    -not $playbackCoreRunnerSource.Contains("'lifecycle-metrics.json'") -or
    -not $playbackCoreProfileSource.Contains("'scripts/lib/playback_core_lifecycle_collector.psm1'") -or
    -not $playbackCoreCollectorSource.Contains('project-owned-non-game-receive-only') -or
    -not $playbackCoreCollectorSource.Contains('manifest_sha256') -or
    -not $playbackCoreCollectorSource.Contains('game_target') -or
    -not $playbackCoreCaseRunnerSource.Contains('requires -OwnerApprovedReceiveOnlyRun')) {
    throw 'PlaybackCoreLifecycle exact-source manifests, workload matrix, immutable provenance, or no-send dry-run boundary is incomplete'
}

Import-Module (Join-Path $PSScriptRoot 'lib/playback_core_lifecycle_collector.psm1') -Force
$pclValidSafetyIdentity = Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'stop-cleanup' -Workload 'safety-smoke' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0
$pclValidPerformanceIdentity = Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'mixed-up-down' -Workload 'mixed-up-down' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 15
if ($pclValidSafetyIdentity.configured_H_us -ne 17167 -or $pclValidSafetyIdentity.configured_G_us -ne 17167 -or
    $pclValidPerformanceIdentity.workload -ne 'mixed-up-down') {
    throw 'PlaybackCoreLifecycle case identity did not preserve its 60 FPS/500 us workload configuration'
}
$pclInvalidCaseRejected = $false
try { [void](Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'timing-margin-sweep' -Workload 'boundary-60fps-margin-0' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0) } catch { $pclInvalidCaseRejected = $true }
if (-not $pclInvalidCaseRejected) { throw 'PlaybackCoreLifecycle accepted a case outside its exact profile matrix' }
$pclRevision = '1111111111111111111111111111111111111111'
$pclTree = '2222222222222222222222222222222222222222'
$pclManifestHash = 'a' * 64
$pclBinaryHash = 'b' * 64
$pclMachine = 'qualification-contract-machine'
$pclBootUtc = '2026-11-19T00:00:00.0000000Z'
$pclTargetIdentity = [pscustomobject]@{
    role = 'ReceiveOnly'; sink_kind = 'SkyAutoPlayer.NativeAcceptanceSink'
    target_class = 'project-owned-non-game-receive-only'; target_name = 'Sky Auto Player — Native Acceptance Sink'
    game_target = $false; title = 'Sky Auto Player — Native Acceptance Sink'; process = 'native_acceptance_sink.ps1'
    input_policy = 'receives_only; benchmark must use SendInput'; pid = 17; hwnd = 42
    process_start_time_filetime = 134100000000000000; event_log_id = 'pcl-contract-log'
}
$pclLifecycleMeasurement = [pscustomobject]@{
    schema_version = 1; qpc_frequency_hz = 10000000; session_arm_qpc_ticks = 100
    stop_requested_qpc_ticks = 150; worker_joined_qpc_ticks = 180; verified_release_qpc_ticks = 190
    verified_release_requires_join_clean_journal_and_validated_receiveonly_up = $true; raw_up_call_count = 2
}
$pclRows = @(
    [pscustomobject]@{ sequence_index = 0; run_id = 'pcl-candidate-stop'; role = 'candidate'; source_revision = $pclRevision; source_tree_sha = $pclTree; manifest_sha256 = $pclManifestHash; harness_sha256 = $pclBinaryHash; scenario = 'stop-cleanup'; status = 'PASS'; case_exit_code = 0; lifecycle_measurement = $pclLifecycleMeasurement; target_identity = $pclTargetIdentity; machine_name = $pclMachine; os_boot_time_utc = $pclBootUtc; case_launch_qpc_ticks = 10; case_launch_qpc_frequency_hz = 10000000 },
    [pscustomobject]@{ sequence_index = 1; run_id = 'pcl-baseline-next'; role = 'control'; source_revision = '3333333333333333333333333333333333333333'; source_tree_sha = '4444444444444444444444444444444444444444'; manifest_sha256 = 'c' * 64; harness_sha256 = 'd' * 64; scenario = 'mixed-up-down'; status = 'PASS'; case_exit_code = 0; lifecycle_measurement = $null; target_identity = $pclTargetIdentity; machine_name = $pclMachine; os_boot_time_utc = $pclBootUtc; case_launch_qpc_ticks = 220; case_launch_qpc_frequency_hz = 10000000 },
    [pscustomobject]@{ sequence_index = 2; run_id = 'pcl-candidate-next'; role = 'candidate'; source_revision = $pclRevision; source_tree_sha = $pclTree; manifest_sha256 = $pclManifestHash; harness_sha256 = $pclBinaryHash; scenario = 'mixed-up-down'; status = 'PASS'; case_exit_code = 0; lifecycle_measurement = ([pscustomobject]@{ schema_version = 1; qpc_frequency_hz = 10000000; session_arm_qpc_ticks = 300; worker_joined_qpc_ticks = 500 }); target_identity = $pclTargetIdentity; machine_name = $pclMachine; os_boot_time_utc = $pclBootUtc; case_launch_qpc_ticks = 300; case_launch_qpc_frequency_hz = 10000000 }
)
$pclReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclReport.status -ne 'COMPLETE' -or
    $pclReport.stop_to_verified_release_us.status -ne 'COMPLETE' -or
    $pclReport.stop_to_verified_release_us.statistics.p50 -ne 4 -or
    $pclReport.worker_joined_to_next_start_us.statistics.p50 -ne 4 -or
    $pclReport.raw_up_call_count.statistics.p50 -ne 2 -or
    $pclReport.source_identity.manifest_sha256 -ne $pclManifestHash) {
    throw 'PlaybackCoreLifecycle fake measurements did not produce provenance-bearing verified release, next-start, and raw Up statistics'
}
$pclWrongTargetRows = @($pclRows | ForEach-Object { $_ | Select-Object * })
$pclWrongTargetRows[1].target_identity = $pclTargetIdentity | Select-Object *
$pclWrongTargetRows[1].target_identity.game_target = $true
$pclWrongTargetRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongTargetRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongTargetRejected = $true }
if (-not $pclWrongTargetRejected) { throw 'PlaybackCoreLifecycle collector accepted game_target=true' }
$pclWrongClassRows = @($pclRows | ForEach-Object { $_ | Select-Object * })
$pclWrongClassRows[0].target_identity = $pclTargetIdentity | Select-Object *
$pclWrongClassRows[0].target_identity.target_class = 'Sky'
$pclWrongClassRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongClassRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongClassRejected = $true }
if (-not $pclWrongClassRejected) { throw 'PlaybackCoreLifecycle collector accepted a Sky target class' }
$pclMissingGameTargetRows = @($pclRows | ForEach-Object { $_ | Select-Object * })
$pclMissingGameTargetRows[0].target_identity = $pclTargetIdentity | Select-Object *
[void]$pclMissingGameTargetRows[0].target_identity.PSObject.Properties.Remove('game_target')
$pclMissingGameTargetRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclMissingGameTargetRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclMissingGameTargetRejected = $true }
if (-not $pclMissingGameTargetRejected) { throw 'PlaybackCoreLifecycle collector accepted missing game_target identity' }
$pclWrongHashRows = @($pclRows | ForEach-Object { $_ | Select-Object * })
$pclWrongHashRows[0].manifest_sha256 = 'e' * 64
$pclWrongHashRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongHashRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongHashRejected = $true }
if (-not $pclWrongHashRejected) { throw 'PlaybackCoreLifecycle collector accepted a candidate manifest hash mismatch' }
$pclIncompleteRows = @($pclRows | ForEach-Object { $_ | Select-Object * })
$pclIncompleteRows[0].lifecycle_measurement = $pclLifecycleMeasurement | Select-Object *
$pclIncompleteRows[0].lifecycle_measurement.verified_release_requires_join_clean_journal_and_validated_receiveonly_up = $false
$pclIncompleteReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclIncompleteRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclIncompleteReport.status -ne 'INCOMPLETE' -or $pclIncompleteReport.stop_to_verified_release_us.status -ne 'INCOMPLETE') {
    throw 'PlaybackCoreLifecycle collector did not fail closed when verified-release proof was absent'
}

Write-Output ('PASS: DispatchCleanupM3 contract positives plus ' + $m3NegativeContractCount + ' negative cases; DispatchSessionRedesignM3 schema-v2 provenance inventory, mapping, timing, count, and fail-stop contracts pass; existing Scheduler, FrameSnapshot, DispatchCore, sink, and fail-stop contracts remain passing')
Write-Output 'PASS: PlaybackCoreLifecycle exact baseline/candidate provenance, owner-gated ReceiveOnly workflow, fake lifecycle collector metrics, fail-closed target identity, and PlanOnly dry-run contracts pass'
Write-Output 'PASS: PowerShell parser, paired CSV/provenance, Smoke/Final fail-stop, and timestamp provenance contracts'
