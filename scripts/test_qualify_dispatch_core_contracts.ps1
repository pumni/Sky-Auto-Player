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
    'Get-TelemetryDownChordIdentity',
    'Get-TimingMetrics',
    'Get-PlaybackCoreLifecycleTimedRowDisposition',
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
$identityFixture25 = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'fixtures/telemetry_identity_schema_25.json') -Raw | ConvertFrom-Json
$identityFixture26 = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'fixtures/telemetry_identity_schema_26.json') -Raw | ConvertFrom-Json
$identityFixture27 = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'fixtures/telemetry_identity_schema_27.json') -Raw | ConvertFrom-Json
$identityFixture28 = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'fixtures/telemetry_identity_schema_28.json') -Raw | ConvertFrom-Json
$identityFixture28Missing = $identityFixture28 | ConvertTo-Json -Depth 8 | ConvertFrom-Json
$identityFixture28Missing.records[0].authored_down_chord_index_available = $false
$identityFixture29 = $identityFixture28 | ConvertTo-Json -Depth 8 | ConvertFrom-Json
$identityFixture29.schema_version = 29
$identityMetrics25 = Get-TimingMetrics ([pscustomobject]@{ details = [pscustomobject]@{ native_timing_telemetry = $identityFixture25 } })
$identityMetrics26 = Get-TimingMetrics ([pscustomobject]@{ details = [pscustomobject]@{ native_timing_telemetry = $identityFixture26 } })
$identityMetrics27 = Get-TimingMetrics ([pscustomobject]@{ details = [pscustomobject]@{ native_timing_telemetry = $identityFixture27 } })
$identityMetrics28 = Get-TimingMetrics ([pscustomobject]@{ details = [pscustomobject]@{ native_timing_telemetry = $identityFixture28 } })
$identityMetrics28Missing = Get-TimingMetrics ([pscustomobject]@{ details = [pscustomobject]@{ native_timing_telemetry = $identityFixture28Missing } })
$identityMetrics29 = Get-TimingMetrics ([pscustomobject]@{ details = [pscustomobject]@{ native_timing_telemetry = $identityFixture29 } })
$identityRecord25 = @($identityMetrics25.event_chord_identities)[0]
$identityRecord26 = @($identityMetrics26.event_chord_identities)[0]
$identityRecord27 = @($identityMetrics27.event_chord_identities)[0]
$identityRecord28 = @($identityMetrics28.event_chord_identities)[0]
$identityRecord28Missing = @($identityMetrics28Missing.event_chord_identities)[0]
$identityRecord29 = @($identityMetrics29.event_chord_identities)[0]
if ($identityRecord25.down_chord_index -ne 17 -or
    $identityRecord25.down_chord_identity_kind -ne 'compiled_packet_index_v25' -or
    $identityRecord25.down_chord_identity_schema_version -ne 25 -or
    -not $identityRecord25.down_chord_identity_available -or
    $identityRecord26.down_chord_index -ne 23 -or
    $identityRecord26.down_chord_identity_kind -ne 'authored_down_chord_index_v26' -or
    $identityRecord26.down_chord_identity_schema_version -ne 26 -or
    -not $identityRecord26.down_chord_identity_available -or
    $identityRecord27.down_chord_index -ne 31 -or
    $identityRecord27.down_chord_identity_kind -ne 'authored_down_chord_index_v27' -or
    $identityRecord27.down_chord_identity_schema_version -ne 27 -or
    -not $identityRecord27.down_chord_identity_available -or
    $identityRecord28.down_chord_index -ne 37 -or
    $identityRecord28.down_chord_identity_kind -ne 'authored_down_chord_index_v28' -or
    $identityRecord28.down_chord_identity_schema_version -ne 28 -or
    -not $identityRecord28.down_chord_identity_available -or
    $identityMetrics25.counts.missing_down_chord_identity -ne 0 -or
    $identityMetrics26.counts.missing_down_chord_identity -ne 0 -or
    $identityMetrics27.counts.missing_down_chord_identity -ne 0 -or
    $identityMetrics28.counts.missing_down_chord_identity -ne 0 -or
    $identityRecord28Missing.down_chord_identity_available -or
    $null -ne $identityRecord28Missing.down_chord_index -or
    $identityRecord28Missing.down_chord_identity_kind -ne 'authored_down_chord_index_v28' -or
    $identityMetrics28Missing.counts.missing_down_chord_identity -ne 1 -or
    $identityRecord29.down_chord_identity_available -or
    $identityRecord29.down_chord_identity_kind -ne 'unsupported_schema_v29' -or
    $identityRecord29.down_chord_identity_schema_version -ne 29 -or
    $identityMetrics29.counts.missing_down_chord_identity -ne 1) {
    throw 'qualification reader did not preserve v25/v26/v27/v28 Down chord identity or fail closed for missing and future identities'
}
$identityDisposition27 = Get-PlaybackCoreLifecycleTimedRowDisposition -Profile 'PlaybackCoreLifecycleV2' `
    -SampleSeconds 15 -Status 'PASS' -Reason 'native report passed' -Metrics $identityMetrics27
$identityDisposition28V2 = Get-PlaybackCoreLifecycleTimedRowDisposition -Profile 'PlaybackCoreLifecycleV2' `
    -SampleSeconds 15 -Status 'PASS' -Reason 'native report passed' -Metrics $identityMetrics28
$identityDisposition28V3 = Get-PlaybackCoreLifecycleTimedRowDisposition -Profile 'PlaybackCoreLifecycleV3' `
    -SampleSeconds 15 -Status 'PASS' -Reason 'native report passed' -Metrics $identityMetrics28
$identityDisposition28MissingV2 = Get-PlaybackCoreLifecycleTimedRowDisposition -Profile 'PlaybackCoreLifecycleV2' `
    -SampleSeconds 15 -Status 'PASS' -Reason 'native report passed' -Metrics $identityMetrics28Missing
$identityDisposition28MissingV3 = Get-PlaybackCoreLifecycleTimedRowDisposition -Profile 'PlaybackCoreLifecycleV3' `
    -SampleSeconds 15 -Status 'PASS' -Reason 'native report passed' -Metrics $identityMetrics28Missing
$identityDisposition29V2 = Get-PlaybackCoreLifecycleTimedRowDisposition -Profile 'PlaybackCoreLifecycleV2' `
    -SampleSeconds 15 -Status 'PASS' -Reason 'native report passed' -Metrics $identityMetrics29
$identityDisposition29V3 = Get-PlaybackCoreLifecycleTimedRowDisposition -Profile 'PlaybackCoreLifecycleV3' `
    -SampleSeconds 15 -Status 'PASS' -Reason 'native report passed' -Metrics $identityMetrics29
$identityMetrics27Json = ConvertTo-Json -InputObject $identityMetrics27 -Depth 12 -Compress
if ($identityDisposition27.status -ne 'PASS' -or
    $identityDisposition27.attribution_status -ne 'COMPLETE' -or
    $identityDisposition27.attributed_down_chord_count -ne 1 -or
    $identityDisposition28V2.status -ne 'PASS' -or
    $identityDisposition28V2.attribution_status -ne 'COMPLETE' -or
    $identityDisposition28V2.attributed_down_chord_count -ne 1 -or
    $identityDisposition28V3.status -ne 'PASS' -or
    $identityDisposition28V3.attribution_status -ne 'COMPLETE' -or
    $identityDisposition28V3.attributed_down_chord_count -ne 1 -or
    $identityDisposition28MissingV2.status -ne 'INCOMPLETE' -or
    $identityDisposition28MissingV2.attribution_status -ne 'INCOMPLETE' -or
    $identityDisposition28MissingV3.status -ne 'INCOMPLETE' -or
    $identityDisposition28MissingV3.attribution_status -ne 'INCOMPLETE' -or
    $identityDisposition29V2.status -ne 'INCOMPLETE' -or
    $identityDisposition29V2.attribution_status -ne 'INCOMPLETE' -or
    $identityDisposition29V3.status -ne 'INCOMPLETE' -or
    $identityDisposition29V3.attribution_status -ne 'INCOMPLETE' -or
    $identityDisposition28MissingV3.reason -notmatch 'attribution-dependent timed results are ineligible' -or
    $identityMetrics27Json -match 'resume_cleanup_delay') {
    throw ('PlaybackCoreLifecycle V2/V3 timed rows did not accept complete schema-28 identity, reject missing/future identity, or preserve the retired delay diagnostic: v27=' +
        (ConvertTo-Json -InputObject $identityDisposition27 -Compress) + '; v28-v2=' +
        (ConvertTo-Json -InputObject $identityDisposition28V2 -Compress) + '; v28-v3=' +
        (ConvertTo-Json -InputObject $identityDisposition28V3 -Compress) + '; v28-missing-v3=' +
        (ConvertTo-Json -InputObject $identityDisposition28MissingV3 -Compress) + '; v29-v3=' +
        (ConvertTo-Json -InputObject $identityDisposition29V3 -Compress) + '; retired_delay=' +
        [string]($identityMetrics27Json -match 'resume_cleanup_delay'))
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
    'canonical-single', 'canonical-max-chord', 'near-minimum-retrigger', 'short-up-gap-stress',
    'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary',
    'modifier-held-after-owned', 'focus-loss', 'target-hwnd-change', 'owner-mismatch',
    'owner-query-failure', 'owner-process-termination', 'pause-resume',
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
if ($finalModel.Count -ne 44 -or @($safetyModel | Where-Object load -eq 'Quiet').Count -ne 21 -or
    @($safetyModel | Where-Object load -eq 'Contention').Count -ne 21 -or
    @($safetyModel | Group-Object load | Where-Object { $_.Count -ne 21 -or @($_.Group.scenario | Select-Object -Unique).Count -ne 21 }).Count -ne 0 -or
    $finalModel[42].workload -ne 'boundary-60fps-margin-0' -or $finalModel[42].margin -ne 0 -or $finalModel[42].fps -ne 60 -or
    $finalModel[43].workload -ne 'boundary-120fps-margin-500' -or $finalModel[43].margin -ne 500 -or $finalModel[43].fps -ne 120) {
    throw 'Final model must contain 21 unique scenarios per load and the two fixed boundaries'
}
$scenarioBlock = [regex]::Match($runnerSource, '(?s)\$scenarios\s*=\s*@\((.*?)\n\s*\)')
$runnerScenarioNames = if ($scenarioBlock.Success) {
    @([regex]::Matches($scenarioBlock.Groups[1].Value, "'([^']+)'") | ForEach-Object { $_.Groups[1].Value })
} else { @() }
if ($runnerScenarioNames.Count -ne $finalScenarioNames.Count -or
    (@(Compare-Object -ReferenceObject $finalScenarioNames -DifferenceObject $runnerScenarioNames -SyncWindow 0)).Count -ne 0) {
    throw 'Final safety scenarios differ from the current 21-scenario inventory'
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
$m3ProfileSet = "ValidateSet\('DispatchCore', 'Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2', 'PlaybackCoreLifecycleV3'\)"
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
$reportSchemaSelectionFixtures = @(
    @{ Profile = 'DispatchCleanupM3'; Expected = 2 },
    @{ Profile = 'DispatchSessionRedesignM3'; Expected = 2 },
    @{ Profile = 'PlaybackCoreLifecycle'; Expected = 2 },
    @{ Profile = 'PlaybackCoreLifecycleV2'; Expected = 2 },
    @{ Profile = 'PlaybackCoreLifecycleV3'; Expected = 2 },
    @{ Profile = 'DispatchCore'; Expected = 1 },
    @{ Profile = 'Scheduler'; Expected = 1 }
)
$successReportSchemaFixture = [ordered]@{}
foreach ($fixture in $reportSchemaSelectionFixtures) {
    $selectedSchemaVersion = Get-QualificationReportSchemaVersion -Profile $fixture.Profile
    $successReportSchemaFixture[$fixture.Profile] = $selectedSchemaVersion
    if ($selectedSchemaVersion -ne $fixture.Expected) {
        throw ('success report schema selection changed for profile ' + $fixture.Profile)
    }
}
if ($successReportSchemaFixture.DispatchCleanupM3 -ne 2 -or
    $successReportSchemaFixture.DispatchSessionRedesignM3 -ne 2 -or
    $successReportSchemaFixture.PlaybackCoreLifecycle -ne 2 -or
    $successReportSchemaFixture.PlaybackCoreLifecycleV2 -ne 2 -or
    $successReportSchemaFixture.PlaybackCoreLifecycleV3 -ne 2 -or
    -not $qualifierSource.Contains('$qualificationReportSchemaVersion = Get-QualificationReportSchemaVersion -Profile $QualificationProfile') -or
    -not $qualifierSource.Contains('schema_version = $qualificationReportSchemaVersion')) {
    throw 'shared current report projection schema v2 boundary changed or leaked into unrelated profiles'
}
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
    authored_down_chord_index_available = $false; authored_down_chord_index = 0
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
if ($m3Floors60Margin0.configured_H_us -ne 16667 -or
    $m3Floors60Margin500.configured_H_us -ne 17167 -or
    $m3Floors120Margin500.configured_H_us -ne 8834 -or
    $m3HolFloors60Margin500.configured_H_us -ne 33834 -or
    $m3HolFloors60Margin500.hold_frames -ne 2) {
    throw 'DispatchCleanupM3 frame and hold arithmetic does not match native scenario configuration'
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
    'canonical-single', 'canonical-max-chord', 'near-minimum-retrigger', 'short-up-gap-stress',
    'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary',
    'modifier-held-after-owned', 'focus-loss', 'target-hwnd-change', 'owner-mismatch',
    'owner-query-failure', 'owner-process-termination', 'pause-resume',
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
    $m3CanaryProjection.StageSummary.configured_H_summary -ne 'H=17167 us' -or
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
    throw 'DispatchCleanupM3 Canary projection implies a comparison/sample or reports incorrect hold/session identity'
}
if ($m3SmokeProjection.StageSummary.planned_session_count -ne 21 -or
    $m3SmokeProjection.StageSummary.stage_kind -ne 'safety-only; candidate-only' -or
    $m3SmokeProjection.StageSummary.candidate_session_count -ne 21 -or
    $m3SmokeProjection.StageSummary.control_session_count -ne 0 -or
    $m3SmokeProjection.StageSummary.workload_summary -ne '21 candidate-only safety scenarios' -or
    -not $m3SmokeProjection.MarkdownStageLines[0].Contains('safety-only') -or
    $m3SmokeProjection.StageSummary.timed_performance_samples -or
    $m3SmokeProjection.StageSummary.configured_H_us -ne 17167 -or
    $m3SmokeProjection.StageSummary.configured_H_summary -ne 'H=17167 us') {
    throw 'DispatchCleanupM3 Smoke projection does not describe its candidate-only safety cases'
}
if ($m3CompareProjection.StageSummary.planned_session_count -ne 72 -or
    $m3CompareProjection.StageSummary.stage_kind -ne 'paired performance' -or
    $m3CompareProjection.StageSummary.candidate_session_count -ne 36 -or
    $m3CompareProjection.StageSummary.control_session_count -ne 36 -or
    $m3CompareProjection.StageSummary.game_fps -ne 60 -or
    $m3CompareProjection.StageSummary.timing_margin_us -ne 0 -or
    $m3CompareProjection.StageSummary.configured_H_us -ne 16667 -or
    $m3CompareProjection.StageSummary.configured_H_summary -ne 'H=16667 us' -or
    $m3CompareProjection.StageSummary.pairs -ne 6 -or
    -not $m3CompareProjection.StageSummary.pair_order.Contains('AB/BA') -or
    -not $m3CompareProjection.ManifestParameters.pair_order.Contains('AB/BA') -or
    $m3CompareProjection.SummaryParameters.performance_timing_margin_us -ne 0 -or
    $m3CompareProjection.SummaryParameters.safety_timing_margin_us -ne 500) {
    throw 'DispatchCleanupM3 Compare projection lost its paired performance protocol or per-session timing settings'
}
if ($m3FinalProjection.StageSummary.planned_session_count -ne 164 -or
    $m3FinalProjection.StageSummary.stage_kind -ne 'paired performance plus candidate-only safety and boundaries' -or
    $m3FinalProjection.StageSummary.candidate_session_count -ne 104 -or
    $m3FinalProjection.StageSummary.control_session_count -ne 60 -or
    $null -ne $m3FinalProjection.StageSummary.game_fps -or
    $null -ne $m3FinalProjection.StageSummary.timing_margin_us -or
    $null -ne $m3FinalProjection.StageSummary.configured_H_us -or
    $m3FinalProjection.StageSummary.configured_H_mode -ne 'varies_by_session' -or
    -not $m3FinalProjection.StageSummary.configured_H_summary.Contains('H varies by session') -or
    -not $m3FinalProjection.ManifestParameters.configured_H_summary.Contains('H varies by session') -or
    -not $m3FinalProjection.StageSummary.timing_margin_summary.Contains('0 us performance; 500 us safety') -or
    $m3FinalProjection.StageSummary.timed_performance_sample_seconds -ne 15) {
    throw 'DispatchCleanupM3 Final projection fabricated aggregate timing values for a mixed stage'
}

$m3RunConfig = [pscustomobject]@{
    qualification_profile = 'DispatchCleanupM3'; scenario = 'canonical-single'; workload = 'canonical-single'
    game_fps = 60; timing_margin_us = 500; expected_hold_us = 17167
    configured_H_us = 17167
}
$verifiedCanaryFloors = Assert-DispatchCleanupM3RunConfigTiming -ExpectedScenario 'canonical-single' `
    -RunConfig $m3RunConfig -ExpectedWorkload 'canonical-single' -ExpectedGameFps 60 -ExpectedTimingMarginUs 500
$m3CanaryRow = [pscustomobject]@{
    stage = $m3CanaryProjection.StageSummary.stage
    qualification_profile = $m3CanaryProjection.StageSummary.qualification_profile
    planned_session_count = $m3CanaryProjection.StageSummary.planned_session_count
    attempted_session_count = 1; role = 'candidate'; scenario = $m3RunConfig.scenario
    game_fps = $m3RunConfig.game_fps; timing_margin_us = $m3RunConfig.timing_margin_us
    configured_H_us = $m3RunConfig.configured_H_us
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
        [long]$csvCanary.configured_H_us -ne [long]$jsonCanary.session_plan.configured_H_us -or
        $csvCanary.stage -ne $jsonCanary.stage -or
        $csvCanary.qualification_profile -ne $jsonCanary.qualification_profile -or
        $csvCanary.planned_session_count -ne $jsonCanary.session_plan.planned_session_count -or
        $csvCanary.role -ne 'candidate' -or $csvCanary.game_fps -ne $jsonCanary.session_plan.game_fps -or
        $csvCanary.timing_margin_us -ne $jsonCanary.session_plan.timing_margin_us -or
        $verifiedCanaryFloors.configured_H_us -ne 17167 -or
        $jsonCanary.session_plan.PSObject.Properties.Name -contains 'resume_cleanup_delay_us' -or
        $csvCanary.PSObject.Properties.Name -contains 'resume_cleanup_delay_us') {
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
if (-not $wrongScenarioRejected) { throw 'DispatchCleanupM3 accepted incorrect scenario attribution for hold/resume-delay reporting' }
$m3NegativeContractCount++
$badM3RetiredFieldRunConfig = $m3RunConfig | ConvertTo-Json -Depth 4 | ConvertFrom-Json
$badM3RetiredFieldRunConfig | Add-Member -NotePropertyName resume_cleanup_delay_us -NotePropertyValue 17167
$badM3RetiredFieldRejected = $false
try { Assert-DispatchCleanupM3RunConfigTiming -ExpectedScenario canonical-single -RunConfig $badM3RetiredFieldRunConfig `
    -ExpectedWorkload canonical-single -ExpectedGameFps 60 -ExpectedTimingMarginUs 500 | Out-Null } catch { $badM3RetiredFieldRejected = $true }
if (-not $badM3RetiredFieldRejected) { throw 'DispatchCleanupM3 accepted a retired resume cleanup timing field' }
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
if ($floors60.frame_us -ne 16667 -or $floors60.configured_H_us -ne 17167 -or
    $floors120.frame_us -ne 8334 -or $floors120.configured_H_us -ne 8834) {
    throw 'DispatchSessionRedesignM3 hold values differ from ceiling frame interval plus the fixed 500 us margin'
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
    $sessionRedesignProjection.StageSummary.configured_H_summary -ne '60 FPS H=17,167 us; 120 FPS H=8,834 us' -or
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
    $zeroFailureArtifactsV2 = New-QualificationRunnerFailureArtifacts -ErrorRecord $zeroAttemptError `
        -RunId 'offline-zero-attempts-v2' -Stage 'Final' -SourceRevision 'd76d4fb7cdcc49fcd0f0a7d80c1f691578c86fed' `
        -SourceTreeSha '17a8ea0d3a22ee4bf2771988dc92fa9ddabfe926' -AttemptedSessionCount 0 `
        -EvidenceRoot $offlineFailureRoot -FallbackScriptPath $runnerPath -SummarySchemaVersion 2
    $cleanupFailureSchemaVersion = Get-QualificationReportSchemaVersion -Profile 'DispatchCleanupM3'
    $sessionRedesignFailureSchemaVersion = Get-QualificationReportSchemaVersion -Profile 'DispatchSessionRedesignM3'
    $dispatchCoreFailureSchemaVersion = Get-QualificationReportSchemaVersion -Profile 'DispatchCore'
    $zeroCleanupFailureArtifacts = New-QualificationRunnerFailureArtifacts -ErrorRecord $zeroAttemptError `
        -RunId 'offline-zero-attempts-cleanup-m3' -Stage 'Final' -SourceRevision 'd76d4fb7cdcc49fcd0f0a7d80c1f691578c86fed' `
        -SourceTreeSha '17a8ea0d3a22ee4bf2771988dc92fa9ddabfe926' -AttemptedSessionCount 0 `
        -EvidenceRoot $offlineFailureRoot -FallbackScriptPath $runnerPath -SummarySchemaVersion $cleanupFailureSchemaVersion
    $zeroSessionRedesignFailureArtifacts = New-QualificationRunnerFailureArtifacts -ErrorRecord $zeroAttemptError `
        -RunId 'offline-zero-attempts-session-redesign-m3' -Stage 'Final' -SourceRevision 'd76d4fb7cdcc49fcd0f0a7d80c1f691578c86fed' `
        -SourceTreeSha '17a8ea0d3a22ee4bf2771988dc92fa9ddabfe926' -AttemptedSessionCount 0 `
        -EvidenceRoot $offlineFailureRoot -FallbackScriptPath $runnerPath -SummarySchemaVersion $sessionRedesignFailureSchemaVersion
    $zeroDispatchCoreFailureArtifacts = New-QualificationRunnerFailureArtifacts -ErrorRecord $zeroAttemptError `
        -RunId 'offline-zero-attempts-dispatch-core' -Stage 'Final' -SourceRevision 'd76d4fb7cdcc49fcd0f0a7d80c1f691578c86fed' `
        -SourceTreeSha '17a8ea0d3a22ee4bf2771988dc92fa9ddabfe926' -AttemptedSessionCount 0 `
        -EvidenceRoot $offlineFailureRoot -FallbackScriptPath $runnerPath -SummarySchemaVersion $dispatchCoreFailureSchemaVersion
    if ($zeroFailureArtifacts.Summary.schema_version -ne 1 -or $zeroFailureArtifactsV2.Summary.schema_version -ne 2 -or
        $cleanupFailureSchemaVersion -ne 2 -or $sessionRedesignFailureSchemaVersion -ne 2 -or
        $zeroCleanupFailureArtifacts.Summary.schema_version -ne $cleanupFailureSchemaVersion -or
        $zeroSessionRedesignFailureArtifacts.Summary.schema_version -ne $sessionRedesignFailureSchemaVersion -or
        $dispatchCoreFailureSchemaVersion -ne 1 -or
        $zeroDispatchCoreFailureArtifacts.Summary.schema_version -ne $dispatchCoreFailureSchemaVersion) {
        throw 'runner failure summaries did not preserve the selected current report schema version'
    }
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
$playbackCoreNativeSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\rust\crates\sky_player\src\bin\rt_native_acceptance.rs') -Raw
$playbackCoreRepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$v3PreflightIndex = $playbackCoreRunnerSource.IndexOf('Assert-PlaybackCoreLifecyclePairLineage -RepositoryRoot $root', [System.StringComparison]::Ordinal)
$v3FinalStageGateIndex = $playbackCoreRunnerSource.IndexOf("if (`$Stage -in @('Compare', 'Final') -and -not `$ControlManifest)", [System.StringComparison]::Ordinal)
$v3SharedMarginIndex = if ($v3FinalStageGateIndex -ge 0) { $playbackCoreRunnerSource.IndexOf('Assert-QualificationTimingMargin', $v3FinalStageGateIndex, [System.StringComparison]::Ordinal) } else { -1 }
$v3PlanOnlyBranchIndex = $playbackCoreRunnerSource.IndexOf('if ($PlanOnly) {', $v3SharedMarginIndex, [System.StringComparison]::Ordinal)
$v3PlanOnlyReturnIndex = if ($v3PlanOnlyBranchIndex -ge 0) { $playbackCoreRunnerSource.IndexOf('return', $v3PlanOnlyBranchIndex, [System.StringComparison]::Ordinal) } else { -1 }
$v3NativePathIndex = if ($v3PlanOnlyReturnIndex -ge 0) { $playbackCoreRunnerSource.IndexOf('Invoke-NativeCase -Scenario', $v3PlanOnlyReturnIndex, [System.StringComparison]::Ordinal) } else { -1 }
if ($v3PreflightIndex -lt 0 -or $v3SharedMarginIndex -lt $v3FinalStageGateIndex -or
    $v3PlanOnlyBranchIndex -lt $v3SharedMarginIndex -or $v3PlanOnlyReturnIndex -lt $v3PlanOnlyBranchIndex -or
    $v3PreflightIndex -gt $v3PlanOnlyBranchIndex -or $v3NativePathIndex -lt $v3PlanOnlyReturnIndex) {
    throw 'PlaybackCoreLifecycle PlanOnly and native execution must share final parameter, manifest, and pair-lineage preflight before either no-send return or native case entry'
}
$pclV3ZeroMarginRejected = $false
try { Assert-QualificationTimingMargin -RequestedStage Final -Profile PlaybackCoreLifecycleV3 -MarginUs 0 -PairCount 6 -SampleDurationSeconds 15 | Out-Null } catch { $pclV3ZeroMarginRejected = $true }
if (-not $pclV3ZeroMarginRejected -or
    -not (Assert-QualificationTimingMargin -RequestedStage Final -Profile PlaybackCoreLifecycleV3 -MarginUs 500 -PairCount 6 -SampleDurationSeconds 15)) {
    throw 'PlaybackCoreLifecycleV3 PlanOnly timing margin gate must reject 0 us and accept 500 us'
}
$playbackCoreBaselineNativeSource = (& git -C $playbackCoreRepoRoot show '2544daa13eb23fdf03bd922e65b96fcffaba421c:rust/crates/sky_player/src/bin/rt_native_acceptance.rs') -join [Environment]::NewLine
if ($LASTEXITCODE -ne 0) { throw 'could not read the frozen PlaybackCoreLifecycle baseline native acceptance source' }
$playbackCoreWorkerBootSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\rust\crates\sky_player\src\engine\worker\boot.rs') -Raw
$playbackCoreWorkerFinalizeSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\rust\crates\sky_player\src\engine\worker\cleanup\finalize.rs') -Raw
if (-not $playbackCoreProfileSource.Contains("Name = switch (`$Version)") -or
    -not $playbackCoreProfileSource.Contains("BaselineRevision = switch (`$Version)") -or
    -not $playbackCoreProfileSource.Contains("BaselineTree = switch (`$Version)") -or
    -not $playbackCoreProfileSource.Contains("'PlaybackCoreLifecycleV3'") -or
    -not $playbackCoreProfileSource.Contains("LineageAnchorRevision = if (`$Version -eq 3)") -or
    -not $playbackCoreProfileSource.Contains('Assert-PlaybackCoreLifecycleV3BaselineSuccessor') -or
    -not $playbackCoreProfileSource.Contains('Assert-PlaybackCoreLifecycleV3LockfileBytes') -or
    -not $playbackCoreProfileSource.Contains("CandidateLineageBranch = if (`$Version -eq 1) { 'refactor/playback-core-lifecycle' } else { 'refactor/playback-remove-release-gap' }") -or
    -not $playbackCoreProfileSource.Contains("'near-minimum-retrigger'") -or
    -not $playbackCoreBuilderSource.Contains('Assert-PlaybackCoreLifecycleSourceCheckout') -or
    -not $playbackCoreBuilderSource.Contains("['qualification_plan'] = [ordered]@{") -or
    -not $playbackCoreBuilderSource.Contains("cargo_lock_sha256") -or
    -not $playbackCoreManifestSource.Contains('Assert-PlaybackCoreLifecycleManifestIdentity') -or
    -not $playbackCoreManifestSource.Contains('Assert-PlaybackCoreLifecycleRunnerProvenance') -or
    -not $playbackCoreManifestSource.Contains("Get-ManifestProperty `$targetIdentity 'target_class'") -or
    -not $playbackCoreManifestSource.Contains("Get-ManifestProperty `$targetIdentity 'target_name'") -or
    $playbackCoreManifestSource.Contains("Get-ManifestProperty `$targetIdentity 'application'") -or
    -not $playbackCoreManifestSource.Contains("`$targetGameTarget -isnot [bool]") -or
    -not $playbackCoreRunnerSource.Contains('[switch]$PlanOnly') -or
    -not $playbackCoreRunnerSource.Contains('schema_version = $qualificationReportSchemaVersion') -or
    -not $playbackCoreRunnerSource.Contains('native_attempts=0 target_capture=NOT_RUN') -or
    -not $playbackCoreRunnerSource.Contains('[switch]$OwnerApprovedReceiveOnlyRun') -or
    -not $playbackCoreRunnerSource.Contains('repository owner explicitly approves this Windows machine, target receiver, and physical input scope') -or
    -not $playbackCoreProfileSource.Contains("target_class = 'project-owned-non-game-receive-only'") -or
    -not $playbackCoreRunnerSource.Contains('New-PlaybackCoreLifecycleMeasurementReport') -or
    -not $playbackCoreRunnerSource.Contains('PlaybackCoreLifecycleV2') -or
    -not $playbackCoreRunnerSource.Contains('PlaybackCoreLifecycleV3') -or
    -not $playbackCoreRunnerSource.Contains('Assert-PlaybackCoreLifecyclePairedWorkloadParity') -or
    -not $playbackCoreRunnerSource.Contains("'lifecycle-metrics.json'") -or
    -not $playbackCoreProfileSource.Contains("'scripts/lib/playback_core_lifecycle_collector.psm1'") -or
    -not $playbackCoreCollectorSource.Contains('project-owned-non-game-receive-only') -or
    -not $playbackCoreCollectorSource.Contains('manifest_sha256') -or
    -not $playbackCoreCollectorSource.Contains('game_target') -or
    -not $playbackCoreCollectorSource.Contains('playback_armed_qpc_ticks') -or
    -not $playbackCoreCollectorSource.Contains('worker_release_verified_qpc_ticks') -or
    -not $playbackCoreCollectorSource.Contains("'DIAGNOSTIC_ONLY'") -or
    -not $playbackCoreCollectorSource.Contains("eligible_count = 0; measured_count = 0") -or
    -not $playbackCoreNativeSource.Contains('native_acceptance_lifecycle_evidence') -or
    -not $playbackCoreNativeSource.Contains('playback_armed_qpc_ticks') -or
    -not $playbackCoreNativeSource.Contains('worker_release_verified_qpc_ticks') -or
    -not $playbackCoreNativeSource.Contains('raw_up_syscall_count_source') -or
    $playbackCoreNativeSource.Contains('outcome.release_outcome.attempts') -or
    $playbackCoreBaselineNativeSource.Contains('lifecycle_measurement') -or
    $playbackCoreBaselineNativeSource.Contains('playback_armed_qpc_ticks') -or
    $playbackCoreBaselineNativeSource.Contains('worker_release_verified_qpc_ticks') -or
    $playbackCoreBaselineNativeSource.Contains('raw_up_syscall_count') -or
    $playbackCoreBaselineNativeSource.Contains('native_acceptance_lifecycle_evidence') -or
    -not $playbackCoreWorkerBootSource.Contains('playback_armed_qpc_ticks') -or
    -not $playbackCoreWorkerFinalizeSource.Contains('terminal_release_verified_qpc_ticks') -or
    -not $playbackCoreWorkerFinalizeSource.Contains('terminal_raw_up_syscall_count') -or
    -not $playbackCoreCaseRunnerSource.Contains('requires -OwnerApprovedReceiveOnlyRun')) {
    throw 'PlaybackCoreLifecycle exact-source provenance, worker event boundaries, actual raw-call evidence, or no-send dry-run boundary is incomplete'
}

$pclV1Profile = Get-PlaybackCoreLifecycleProfile
$pclV2Profile = Get-PlaybackCoreLifecycleProfile -Version 2
$pclV3Profile = Get-PlaybackCoreLifecycleProfile -Version 3
if ($pclV1Profile.Name -ne 'PlaybackCoreLifecycle' -or
    $pclV1Profile.BaselineRevision -ne '2544daa13eb23fdf03bd922e65b96fcffaba421c' -or
    $pclV1Profile.CandidateLineageBranch -ne 'refactor/playback-core-lifecycle' -or
    $pclV1Profile.SafetyScenarios -contains 'short-up-gap-stress' -or
    $pclV2Profile.Name -ne 'PlaybackCoreLifecycleV2' -or
    $pclV2Profile.WorkloadVersion -ne 'playback-core-lifecycle-v2' -or
    $pclV2Profile.BaselineRevision -ne '1d00c3b71fbf1f94003865bad6c4eba6d8dcd902' -or
    $pclV2Profile.BaselineTree -ne '25a20291b9cf84b1283804f451b33e3c4605507c' -or
    $pclV2Profile.CandidateLineageBranch -ne 'refactor/playback-remove-release-gap' -or
    $pclV2Profile.SafetyScenarios[-2] -ne 'short-up-gap-stress' -or
    $pclV2Profile.SafetyScenarios[-1] -ne 'pause-resume-active-cleanup' -or
    $pclV2Profile.HarnessManifestSchemaVersion -ne 2 -or
    $pclV2Profile.AuthoredPrefixParityContract -ne 'timestamp-kind-scan-mask-v1' -or
    $pclV3Profile.Name -ne 'PlaybackCoreLifecycleV3' -or
    $pclV3Profile.WorkloadVersion -ne 'playback-core-lifecycle-v3-cookie-0183-v1' -or
    $pclV3Profile.HarnessManifestSchemaVersion -ne 3 -or
    $pclV3Profile.BaselineRevision -ne '4bc4a8409d3250da012be6e92f6681e721c17f3a' -or
    $pclV3Profile.BaselineTree -ne '8e2ce7c115458d95b9e13aa144b3d6cc16c083a4' -or
    $pclV3Profile.LineageAnchorRevision -ne '1d00c3b71fbf1f94003865bad6c4eba6d8dcd902' -or
    $pclV3Profile.BaselineSuccessorParentRevision -ne $pclV2Profile.BaselineRevision -or
    $pclV3Profile.BaselineSuccessorRef -ne 'refs/heads/qualification/playback-core-lifecycle-v3-baseline' -or
    $pclV3Profile.BaselineSuccessorLockSha256 -ne 'baaf6e57419a1f28d9a880d655a48863676fe2fe0ddcd9a1467153912af13b68' -or
    $pclV3Profile.AuthoredPrefixParityContract -ne $pclV2Profile.AuthoredPrefixParityContract -or
    $pclV3Profile.SafetyScenarios.Count -ne $pclV2Profile.SafetyScenarios.Count -or
    (Get-PlaybackCoreLifecycleProfileForName -Name 'PlaybackCoreLifecycleV3').Version -ne 3) {
    throw 'PlaybackCoreLifecycleV2 changed the immutable v1 profile or failed to pin its strict PR-base identity and workload contract'
}

$pclV3BaselineSuccessor = Assert-PlaybackCoreLifecycleV3BaselineSuccessor -RepositoryRoot $playbackCoreRepoRoot
if ($pclV3BaselineSuccessor.revision -ne $pclV3Profile.BaselineSuccessorRevision -or
    $pclV3BaselineSuccessor.tree -ne $pclV3Profile.BaselineSuccessorTree -or
    $pclV3BaselineSuccessor.parent_revision -ne $pclV3Profile.LineageAnchorRevision -or
    ($pclV3BaselineSuccessor.changed_paths -join '|') -ne 'rust/Cargo.lock' -or
    $pclV3BaselineSuccessor.lock_sha256 -ne $pclV3Profile.BaselineSuccessorLockSha256) {
    throw 'PlaybackCoreLifecycleV3 baseline successor is not the clean, lock-only cookie upgrade from the immutable V2 anchor'
}
$pclV3BaselineShaRejected = $false
try { [void](Assert-PlaybackCoreLifecycleV3BaselineSuccessor -RepositoryRoot $playbackCoreRepoRoot -Revision ('0' * 40)) } catch { $pclV3BaselineShaRejected = $true }
if (-not $pclV3BaselineShaRejected) { throw 'PlaybackCoreLifecycleV3 accepted a baseline successor with the wrong revision' }
$pclV3BaselineTreeRejected = $false
try { [void](Assert-PlaybackCoreLifecycleV3BaselineSuccessor -RepositoryRoot $playbackCoreRepoRoot -ExpectedTree ('0' * 40)) } catch { $pclV3BaselineTreeRejected = $true }
if (-not $pclV3BaselineTreeRejected) { throw 'PlaybackCoreLifecycleV3 accepted a baseline successor with the wrong tree identity' }
$pclV3BaselineLockRejected = $false
try { [void](Assert-PlaybackCoreLifecycleV3BaselineSuccessor -RepositoryRoot $playbackCoreRepoRoot -ExpectedLockSha256 ('0' * 64)) } catch { $pclV3BaselineLockRejected = $true }
if (-not $pclV3BaselineLockRejected) { throw 'PlaybackCoreLifecycleV3 accepted a baseline successor with the wrong lock hash' }
$pclV3CandidateRevision = (& git -C $playbackCoreRepoRoot rev-parse HEAD).Trim()
$pclV3StaleRemoteRejected = $false
try {
    [void](Assert-PlaybackCoreLifecycleV3CandidateLineage -RepositoryRoot $playbackCoreRepoRoot `
        -CandidateRevision $pclV3CandidateRevision -RemoteTipRevision ('0' * 40))
} catch { $pclV3StaleRemoteRejected = $true }
if (-not $pclV3StaleRemoteRejected) { throw 'PlaybackCoreLifecycleV3 accepted a candidate that does not match the remote PR head' }
$pclV3WrongAnchorRejected = $false
try {
    [void](Assert-PlaybackCoreLifecycleV3CandidateLineage -RepositoryRoot $playbackCoreRepoRoot `
        -CandidateRevision '2544daa13eb23fdf03bd922e65b96fcffaba421c' -RemoteTipRevision '2544daa13eb23fdf03bd922e65b96fcffaba421c')
} catch { $pclV3WrongAnchorRejected = $true }
if (-not $pclV3WrongAnchorRejected) { throw 'PlaybackCoreLifecycleV3 accepted a candidate outside the V2 anchor lineage' }
$pclV3LineageRemoteTip = $pclV3CandidateRevision
$pclV3PairLineageAccepted = Assert-PlaybackCoreLifecyclePairLineage -RepositoryRoot $playbackCoreRepoRoot `
    -ProfileVersion 3 -CandidateRevision $pclV3CandidateRevision -RemoteTipRevision $pclV3LineageRemoteTip `
    -ControlRevision $pclV3Profile.BaselineSuccessorRevision -ControlTree $pclV3Profile.BaselineSuccessorTree `
    -ControlLockSha256 $pclV3Profile.BaselineSuccessorLockSha256
if (-not $pclV3PairLineageAccepted) { throw 'PlaybackCoreLifecycleV3 rejected its pinned B1 sibling and exact remote candidate pair' }
$pclV3WrongB1Rejected = $false
try {
    [void](Assert-PlaybackCoreLifecyclePairLineage -RepositoryRoot $playbackCoreRepoRoot `
        -ProfileVersion 3 -CandidateRevision $pclV3CandidateRevision -RemoteTipRevision $pclV3LineageRemoteTip `
        -ControlRevision $pclV3Profile.LineageAnchorRevision -ControlTree $pclV3Profile.BaselineSuccessorParentRevision `
        -ControlLockSha256 $pclV3Profile.BaselineSuccessorLockSha256)
} catch { $pclV3WrongB1Rejected = $true }
if (-not $pclV3WrongB1Rejected) { throw 'PlaybackCoreLifecycleV3 pair lineage accepted a control other than its exact pinned B1 revision/tree' }
$pclV3PairWrongAnchorRejected = $false
try {
    [void](Assert-PlaybackCoreLifecyclePairLineage -RepositoryRoot $playbackCoreRepoRoot `
        -ProfileVersion 3 -CandidateRevision '2544daa13eb23fdf03bd922e65b96fcffaba421c' `
        -RemoteTipRevision '2544daa13eb23fdf03bd922e65b96fcffaba421c' `
        -ControlRevision $pclV3Profile.BaselineSuccessorRevision -ControlTree $pclV3Profile.BaselineSuccessorTree `
        -ControlLockSha256 $pclV3Profile.BaselineSuccessorLockSha256)
} catch { $pclV3PairWrongAnchorRejected = $true }
if (-not $pclV3PairWrongAnchorRejected) { throw 'PlaybackCoreLifecycleV3 pair lineage accepted a candidate outside the immutable V2 anchor lineage' }
$pclV3PairStaleRemoteRejected = $false
try {
    [void](Assert-PlaybackCoreLifecyclePairLineage -RepositoryRoot $playbackCoreRepoRoot `
        -ProfileVersion 3 -CandidateRevision $pclV3CandidateRevision -RemoteTipRevision ('0' * 40) `
        -ControlRevision $pclV3Profile.BaselineSuccessorRevision -ControlTree $pclV3Profile.BaselineSuccessorTree `
        -ControlLockSha256 $pclV3Profile.BaselineSuccessorLockSha256)
} catch { $pclV3PairStaleRemoteRejected = $true }
if (-not $pclV3PairStaleRemoteRejected) { throw 'PlaybackCoreLifecycleV3 pair lineage accepted a stale remote branch tip' }
$pclV3LegacyNonAncestorRejected = $false
try {
    [void](Assert-PlaybackCoreLifecyclePairLineage -RepositoryRoot $playbackCoreRepoRoot `
        -ProfileVersion 2 -CandidateRevision '2544daa13eb23fdf03bd922e65b96fcffaba421c' `
        -RemoteTipRevision '2544daa13eb23fdf03bd922e65b96fcffaba421c' `
        -ControlRevision $pclV2Profile.BaselineRevision -ControlTree $pclV2Profile.BaselineTree)
} catch { $pclV3LegacyNonAncestorRejected = $true }
if (-not $pclV3LegacyNonAncestorRejected) { throw 'legacy PlaybackCoreLifecycle pair lineage no longer rejects a non-ancestor control' }
$pclV3GitFailureRejected = $false
try {
    [void](Test-DispatchGitAncestor -RepositoryRoot (Join-Path $env:TEMP ('missing-git-repo-' + [guid]::NewGuid().ToString('N'))) `
        -AncestorRevision $pclV3Profile.LineageAnchorRevision -DescendantRevision $pclV3CandidateRevision)
} catch { $pclV3GitFailureRejected = $_.Exception.Message -match 'git merge-base --is-ancestor failed with exit 128:' }
if (-not $pclV3GitFailureRejected) { throw 'PlaybackCoreLifecycle ancestry validation did not fail closed on a Git command error' }

function ConvertTo-PclV3CheckoutFixture {
    param($Identity)
    [ordered]@{
        root = $Identity.root
        top_level = $Identity.checkout.top_level
        git_dir = $Identity.checkout.git_dir
        git_common_dir = $Identity.checkout.git_common_dir
        revision = $Identity.revision
        tree = $Identity.tree
        branch = $Identity.branch
        clean = $Identity.clean
    }
}

$pclV3TempRoot = Join-Path $env:TEMP ('pcl-v3-manifest-contract-' + [guid]::NewGuid().ToString('N'))
$pclV3BaselineRoot = Join-Path $pclV3TempRoot 'baseline-worktree'
$pclV3LineageRepoRoot = Join-Path $pclV3TempRoot 'lineage-topology-repo'
New-Item -ItemType Directory -Path $pclV3TempRoot | Out-Null
try {
    & git clone --shared --no-checkout --quiet $playbackCoreRepoRoot $pclV3LineageRepoRoot
    if ($LASTEXITCODE -ne 0) { throw 'could not create an isolated Git topology for V3 merged-B1 lineage contract tests' }
    & git -C $pclV3LineageRepoRoot config user.name 'Qualification Contract'
    if ($LASTEXITCODE -ne 0) { throw 'could not configure the isolated V3 lineage topology identity' }
    & git -C $pclV3LineageRepoRoot config user.email 'qualification-contract@example.invalid'
    if ($LASTEXITCODE -ne 0) { throw 'could not configure the isolated V3 lineage topology identity' }
    & git -c core.autocrlf=false -c core.eol=lf -C $playbackCoreRepoRoot worktree add --detach $pclV3BaselineRoot $pclV3Profile.BaselineRevision | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'could not create the pinned V3 baseline worktree for manifest contract tests' }
    Import-Module (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Force
    Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_profiles.psm1') -Force
    $pclV3RunnerIdentity = Get-DispatchM3GitCheckoutIdentity $playbackCoreRepoRoot
    $pclV3BaselineIdentity = Get-DispatchM3GitCheckoutIdentity $pclV3BaselineRoot
    $pclV3MergeTree = (& git -C $pclV3LineageRepoRoot rev-parse ($pclV3RunnerIdentity.revision + '^{tree}')).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve candidate tree for the isolated V3 merged-B1 topology' }
    $pclV3MergedB1Revision = (& git -C $pclV3LineageRepoRoot commit-tree $pclV3MergeTree `
        -p $pclV3RunnerIdentity.revision -p $pclV3Profile.BaselineSuccessorRevision -m 'V3 merged B1 lineage contract').Trim()
    if ($LASTEXITCODE -ne 0 -or $pclV3MergedB1Revision -notmatch '^[0-9a-f]{40}$') {
        throw 'could not create the isolated merged-B1 candidate topology'
    }
    $pclV3MergedB1Rejected = $false
    try {
        [void](Assert-PlaybackCoreLifecyclePairLineage -RepositoryRoot $pclV3LineageRepoRoot `
            -ProfileVersion 3 -CandidateRevision $pclV3MergedB1Revision -RemoteTipRevision $pclV3MergedB1Revision `
            -ControlRevision $pclV3Profile.BaselineSuccessorRevision -ControlTree $pclV3Profile.BaselineSuccessorTree `
            -ControlLockSha256 $pclV3Profile.BaselineSuccessorLockSha256)
    } catch { $pclV3MergedB1Rejected = $_.Exception.Message.Contains('must remain a sibling') }
    if (-not $pclV3MergedB1Rejected) { throw 'PlaybackCoreLifecycleV3 accepted a candidate that had merged its pinned B1 control' }
    $pclV3BaselineLockPath = Join-Path $pclV3BaselineRoot 'rust\Cargo.lock'
    $pclV3OriginalBaselineLockBytes = [System.IO.File]::ReadAllBytes($pclV3BaselineLockPath)
    $pclV3BaselineLockText = [System.Text.Encoding]::UTF8.GetString($pclV3OriginalBaselineLockBytes)
    if ($pclV3BaselineLockText.Contains("`r")) { throw 'PlaybackCoreLifecycleV3 baseline contract worktree did not start with LF Cargo.lock bytes' }
    try {
        $pclV3CrlfLockBytes = [System.Text.Encoding]::UTF8.GetBytes($pclV3BaselineLockText.Replace("`n", "`r`n"))
        [System.IO.File]::WriteAllBytes($pclV3BaselineLockPath, $pclV3CrlfLockBytes)
        $pclV3CrlfLockRejected = $false
        $pclV3CrlfLockError = ''
        try { Assert-PlaybackCoreLifecycleV3LockfileBytes -RepositoryRoot $pclV3BaselineRoot -SourceRole Baseline | Out-Null } catch {
            $pclV3CrlfLockRejected = $true
            $pclV3CrlfLockError = $_.Exception.Message
        }
        if (-not $pclV3CrlfLockRejected -or
            -not $pclV3CrlfLockError.Contains('CR bytes') -or
            -not $pclV3CrlfLockError.Contains('checkout-index --force -- rust/Cargo.lock') -or
            -not $pclV3CrlfLockError.Contains($pclV3Profile.BaselineSuccessorLockSha256)) {
            throw 'PlaybackCoreLifecycleV3 did not clearly reject a CRLF lockfile checkout with LF repair instructions'
        }
    } finally {
        [System.IO.File]::WriteAllBytes($pclV3BaselineLockPath, $pclV3OriginalBaselineLockBytes)
    }
    if ((Assert-PlaybackCoreLifecycleV3LockfileBytes -RepositoryRoot $pclV3BaselineRoot -SourceRole Baseline) -ne $pclV3Profile.BaselineSuccessorLockSha256) {
        throw 'PlaybackCoreLifecycleV3 baseline lockfile byte preflight changed its pinned LF hash'
    }
    $pclV3RustcVersion = (& rustc -Vv) -join [Environment]::NewLine
    if ($LASTEXITCODE -ne 0) { throw 'could not identify rustc for V3 manifest contract fixtures' }
    $pclV3CargoVersion = (& cargo -V).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not identify cargo for V3 manifest contract fixtures' }
    $pclV3BuilderPath = Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1'
    $pclV3BuilderHash = (Get-FileHash -LiteralPath $pclV3BuilderPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $pclV3RunnerProvenance = @($pclV3Profile.RunnerProvenancePaths | ForEach-Object {
        $sourcePath = $_
        $fullPath = Join-Path $playbackCoreRepoRoot ($sourcePath.Replace('/', '\'))
        [pscustomobject]@{
            source_path = $sourcePath
            evidence_path = 'runner-provenance/' + $sourcePath.Replace('/', '\')
            sha256 = (Get-FileHash -LiteralPath $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    })
    function New-PclV3ManifestFixture {
        param([ValidateSet('Baseline', 'Candidate')] [string]$Role)
        $isBaseline = $Role -eq 'Baseline'
        $sourceIdentity = if ($isBaseline) { $pclV3BaselineIdentity } else { $pclV3RunnerIdentity }
        $sourceRoot = $sourceIdentity.root
        $lockPath = Join-Path $sourceRoot 'rust\Cargo.lock'
        $outputRoot = Join-Path $pclV3TempRoot ($Role.ToLowerInvariant() + '-output')
        $checkout = ConvertTo-PclV3CheckoutFixture $sourceIdentity
        $runnerCheckout = ConvertTo-PclV3CheckoutFixture $pclV3RunnerIdentity
        [pscustomobject][ordered]@{
            schema_version = 3
            qualification_profile = $pclV3Profile.Name
            workload_version = $pclV3Profile.WorkloadVersion
            source_role = $Role
            source_revision = $sourceIdentity.revision
            source_tree_sha = $sourceIdentity.tree
            source_branch = $sourceIdentity.branch
            source_tree_clean = $sourceIdentity.clean
            clean_build = $true
            base_source_revision = $pclV3Profile.BaselineRevision
            base_source_tree_sha = $pclV3Profile.BaselineTree
            baseline_source_revision = $pclV3Profile.BaselineRevision
            baseline_source_tree_sha = $pclV3Profile.BaselineTree
            candidate_source_revision = $pclV3RunnerIdentity.revision
            candidate_source_tree_sha = $pclV3RunnerIdentity.tree
            cargo_profile = $pclV3Profile.CargoProfile
            cargo_features = @($pclV3Profile.RequiredFeatures)
            qpc_read_counting = $false
            observer_mode = 'qualification-on'
            receiveonly_observer_mode = $pclV3Profile.ObserverMode
            receiveonly_observer_version = $pclV3Profile.ObserverVersion
            receiveonly_observer_schema_version = $pclV3Profile.ObserverSchemaVersion
            receiveonly_observer_event_schema_version = $pclV3Profile.ObserverEventSchemaVersion
            guard_mode = 'frozen'
            performance_timing_margin_us = $pclV3Profile.PerformanceTimingMarginUs
            safety_timing_margin_us = $pclV3Profile.SafetyTimingMarginUs
            target = $pclV3Profile.Target
            target_identity_status = 'NOT_RUN'
            target_identity = $pclV3Profile.TargetIdentity
            rustc_version = $pclV3RustcVersion
            cargo_version = $pclV3CargoVersion
            cargo_clean_exit_code = 0
            cargo_build_exit_code = 0
            binary_name = 'rt-native-acceptance.exe'
            binary_sha256 = 'a' * 64
            builder_script_sha256 = $pclV3BuilderHash
            source_root = $sourceRoot
            source_checkout_identity = $checkout
            runner_source_revision = $pclV3RunnerIdentity.revision
            runner_source_tree_sha = $pclV3RunnerIdentity.tree
            runner_source_branch = $pclV3RunnerIdentity.branch
            runner_source_root = $playbackCoreRepoRoot
            runner_source_clean = $pclV3RunnerIdentity.clean
            runner_source_checkout_identity = $runnerCheckout
            candidate_lineage_branch = $pclV3Profile.CandidateLineageBranch
            candidate_lineage_tip_revision = $pclV3RunnerIdentity.revision
            candidate_lineage_verified = $true
            lineage_anchor_revision = $pclV3Profile.LineageAnchorRevision
            baseline_successor_revision = $pclV3Profile.BaselineSuccessorRevision
            baseline_successor_tree_sha = $pclV3Profile.BaselineSuccessorTree
            baseline_successor_parent_revision = $pclV3Profile.BaselineSuccessorParentRevision
            baseline_successor_ref = $pclV3Profile.BaselineSuccessorRef
            baseline_successor_lock_sha256 = $pclV3Profile.BaselineSuccessorLockSha256
            output_root = $outputRoot
            cargo_target_dir = Join-Path $outputRoot 'cargo-target'
            cargo_lock_path = $lockPath
            cargo_lock_sha256 = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
            qualification_plan = [pscustomobject]@{
                pair_order = $pclV3Profile.PairOrder
                pair_count = $pclV3Profile.PairCount
                sample_seconds = $pclV3Profile.SampleSeconds
            }
            runner_script_provenance = $pclV3RunnerProvenance
        }
    }
    $pclV3ValidBaseline = New-PclV3ManifestFixture -Role Baseline
    $pclV3ValidCandidate = New-PclV3ManifestFixture -Role Candidate
    Assert-PlaybackCoreLifecycleManifestIdentity -Manifest $pclV3ValidBaseline -ExpectedRole Baseline -RepositoryRoot $playbackCoreRepoRoot | Out-Null
    Assert-PlaybackCoreLifecycleManifestIdentity -Manifest $pclV3ValidCandidate -ExpectedRole Candidate -RepositoryRoot $playbackCoreRepoRoot | Out-Null

    foreach ($mutation in @(
            [pscustomobject]@{ name = 'baseline SHA'; role = 'Baseline'; apply = { param($m) $m.source_revision = '0' * 40 } },
            [pscustomobject]@{ name = 'baseline tree'; role = 'Baseline'; apply = { param($m) $m.source_tree_sha = '0' * 40 } },
            [pscustomobject]@{ name = 'lock hash'; role = 'Baseline'; apply = { param($m) $m.cargo_lock_sha256 = '0' * 64 } },
            [pscustomobject]@{ name = 'dirty source'; role = 'Baseline'; apply = { param($m) $m.source_tree_clean = $false; $m.source_checkout_identity.clean = $false } },
            [pscustomobject]@{ name = 'toolchain'; role = 'Baseline'; apply = { param($m) $m.rustc_version = 'release: 0.0.0' } },
            [pscustomobject]@{ name = 'target'; role = 'Baseline'; apply = { param($m) $m.target = 'aarch64-pc-windows-msvc' } },
            [pscustomobject]@{ name = 'features'; role = 'Baseline'; apply = { param($m) $m.cargo_features = @('wrong-feature') } },
            [pscustomobject]@{ name = 'workload'; role = 'Baseline'; apply = { param($m) $m.workload_version = 'wrong-workload' } },
            [pscustomobject]@{ name = 'successor ref'; role = 'Baseline'; apply = { param($m) $m.baseline_successor_ref = 'refs/heads/moved-baseline' } },
            [pscustomobject]@{ name = 'candidate lock hash'; role = 'Candidate'; apply = { param($m) $m.cargo_lock_sha256 = '0' * 64 } }
        )) {
        $sourceFixture = if ($mutation.role -eq 'Baseline') { $pclV3ValidBaseline } else { $pclV3ValidCandidate }
        $mutatedFixture = $sourceFixture | ConvertTo-Json -Depth 16 | ConvertFrom-Json
        & $mutation.apply $mutatedFixture
        $rejected = $false
        try { Assert-PlaybackCoreLifecycleManifestIdentity -Manifest $mutatedFixture -ExpectedRole $mutation.role -RepositoryRoot $playbackCoreRepoRoot | Out-Null } catch { $rejected = $true }
        if (-not $rejected) { throw ('PlaybackCoreLifecycleV3 accepted a mismatched ' + $mutation.name) }
    }
    $pclV3WrongSchema = $pclV3ValidCandidate | ConvertTo-Json -Depth 16 | ConvertFrom-Json
    $pclV3WrongSchema.qualification_profile = 'PlaybackCoreLifecycleV2'
    $pclV3WrongSchema.schema_version = 2
    $pclV3WrongSchemaRejected = $false
    try { Assert-PlaybackCoreLifecycleManifestIdentity -Manifest $pclV3WrongSchema -ExpectedRole Candidate -RepositoryRoot $playbackCoreRepoRoot | Out-Null } catch { $pclV3WrongSchemaRejected = $true }
    if (-not $pclV3WrongSchemaRejected) { throw 'PlaybackCoreLifecycleV2 accepted a V3 manifest contract' }
} finally {
    $resolvedTempRoot = [System.IO.Path]::GetFullPath($pclV3TempRoot)
    $resolvedSystemTemp = [System.IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if (-not $resolvedTempRoot.StartsWith($resolvedSystemTemp, [System.StringComparison]::OrdinalIgnoreCase) -or
        -not $resolvedTempRoot.Split([System.IO.Path]::DirectorySeparatorChar)[-1].StartsWith('pcl-v3-manifest-contract-', [System.StringComparison]::Ordinal)) {
        throw 'refusing to remove an unexpected V3 qualification test directory'
    }
    if (Test-Path -LiteralPath $pclV3BaselineRoot) {
        & git -C $playbackCoreRepoRoot worktree remove --force $pclV3BaselineRoot | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'could not remove the temporary V3 baseline contract worktree' }
    }
    if (Test-Path -LiteralPath $pclV3TempRoot) { Remove-Item -LiteralPath $pclV3TempRoot -Recurse -Force }
}

$pclBaselineProjection = [pscustomobject]@{ details = [pscustomobject]@{ authored_packet_targets = @(
    [pscustomobject]@{ scheduled_us = 50000; up_mask = 0; down_mask = 1 },
    [pscustomobject]@{ scheduled_us = 67167; up_mask = 1; down_mask = 0 },
    [pscustomobject]@{ scheduled_us = 1050000; up_mask = 0; down_mask = 1 }
) } }
$pclCandidateProjection = [pscustomobject]@{ details = [pscustomobject]@{ authored_timestamp_group_targets = @(
    [pscustomobject]@{ scheduled_us = 50000; up_mask = 0; down_mask = 1 },
    [pscustomobject]@{ scheduled_us = 67167; up_mask = 1; down_mask = 0 },
    [pscustomobject]@{ scheduled_us = 1050000; up_mask = 0; down_mask = 1 }
) } }
$pclBaselinePairRow = [pscustomobject]@{ report = $pclBaselineProjection; workload = 'near-minimum-retrigger'; load = 'Quiet'; pair_id = 'near-minimum-retrigger/Quiet/01' }
$pclCandidatePairRow = [pscustomobject]@{ report = $pclCandidateProjection; workload = 'near-minimum-retrigger'; load = 'Quiet'; pair_id = 'near-minimum-retrigger/Quiet/01' }
[void](Assert-PlaybackCoreLifecyclePairedWorkloadParity -ControlRow $pclBaselinePairRow -CandidateRow $pclCandidatePairRow -SampleSeconds 1)
if ($pclBaselinePairRow.authored_workload_measured_event_count -ne 2 -or
    $pclBaselinePairRow.authored_workload_tail_group_count -ne 1 -or
    $pclBaselinePairRow.authored_workload_full_event_count -ne 3 -or
    $pclBaselinePairRow.authored_workload_tail_event_count -ne 1 -or
    $pclBaselinePairRow.authored_workload_measured_prefix_sha256 -ne $pclCandidatePairRow.authored_workload_measured_prefix_sha256 -or
    $pclBaselinePairRow.authored_workload_full_stream_sha256 -ne $pclCandidatePairRow.authored_workload_full_stream_sha256) {
    throw 'PlaybackCoreLifecycleV2 did not compare the full authored stream and retain its measured-prefix/tail evidence'
}
$pclCandidateProjection.details.authored_timestamp_group_targets[1].scheduled_us = 67168
$pclMismatchRejected = $false
try { [void](Assert-PlaybackCoreLifecyclePairedWorkloadParity -ControlRow $pclBaselinePairRow -CandidateRow $pclCandidatePairRow -SampleSeconds 1) } catch { $pclMismatchRejected = $true }
if (-not $pclMismatchRejected) { throw 'PlaybackCoreLifecycleV2 accepted a mismatched authored timestamp/kind/scan-mask prefix' }
$pclCandidateProjection.details.authored_timestamp_group_targets[1].scheduled_us = 67167
$pclCandidateProjection.details.authored_timestamp_group_targets[2].scheduled_us = 1050001
$pclTailMismatchRejected = $false
try { [void](Assert-PlaybackCoreLifecyclePairedWorkloadParity -ControlRow $pclBaselinePairRow -CandidateRow $pclCandidatePairRow -SampleSeconds 1) } catch { $pclTailMismatchRejected = $true }
if (-not $pclTailMismatchRejected) { throw 'PlaybackCoreLifecycleV2 accepted different authored tail padding after a matching measured prefix' }

Import-Module (Join-Path $PSScriptRoot 'lib/playback_core_lifecycle_collector.psm1') -Force
$pclValidSafetyIdentity = Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'stop-cleanup' -Workload 'safety-smoke' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0
$pclValidPerformanceIdentity = Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'mixed-up-down' -Workload 'mixed-up-down' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 15
if ($pclValidSafetyIdentity.configured_H_us -ne 17167 -or
    $pclValidPerformanceIdentity.workload -ne 'mixed-up-down') {
    throw 'PlaybackCoreLifecycle case identity did not preserve its 60 FPS/500 us workload configuration'
}
$pclInvalidCaseRejected = $false
try { [void](Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'timing-margin-sweep' -Workload 'boundary-60fps-margin-0' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0) } catch { $pclInvalidCaseRejected = $true }
if (-not $pclInvalidCaseRejected) { throw 'PlaybackCoreLifecycle accepted a case outside its exact profile matrix' }
$pclV1ShortGapRejected = $false
try { [void](Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'short-up-gap-stress' -Workload 'safety-smoke' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0) } catch { $pclV1ShortGapRejected = $true }
if (-not $pclV1ShortGapRejected -or
    (Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'short-up-gap-stress' -Workload 'safety-smoke' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0 -ProfileVersion 2).scenario -ne 'short-up-gap-stress') {
    throw 'PlaybackCoreLifecycleV2 did not isolate short-up-gap-stress to its candidate-only safety contract'
}
$pclV1ActivePauseRejected = $false
try { [void](Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'pause-resume-active-cleanup' -Workload 'safety-smoke' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0) } catch { $pclV1ActivePauseRejected = $true }
if (-not $pclV1ActivePauseRejected -or
    (Assert-PlaybackCoreLifecycleSessionIdentity -Scenario 'pause-resume-active-cleanup' -Workload 'safety-smoke' -TimingMarginUs 500 -GameFps 60 -SampleSeconds 0 -ProfileVersion 2).scenario -ne 'pause-resume-active-cleanup') {
    throw 'PlaybackCoreLifecycleV2 did not isolate active-key pause cleanup to its candidate-only safety contract'
}
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
    schema_version = 2; qpc_clock_source = 'Windows QueryPerformanceCounter'; qpc_frequency_hz = 10000000
    native_session_identity = [pscustomobject]@{ run_id = 'pcl-candidate-stop'; scenario = 'stop-cleanup' }
    playback_admission_event = 'system_power_ready_to_armed_publication'; playback_armed_qpc_ticks = 100
    acceptance_pre_arm_qpc_ticks_diagnostic = 90; stop_requested_qpc_ticks = 150
    worker_release_verified_qpc_ticks = 190; worker_joined_qpc_ticks = 210
    post_join_report_and_receiveonly_validation_completed_qpc_ticks = 10000
    release_sample_gate_proof_complete = $true; raw_up_syscall_count = 2
    raw_up_syscall_count_source = 'sum of worker TerminalSendAttemptEvidence.raw_calls'
    release_all_attempts = 9
}
function Copy-PclTestRows {
    param([object[]]$Rows)
    foreach ($sourceRow in $Rows) {
        $row = $sourceRow | Select-Object *
        foreach ($propertyName in @('lifecycle_measurement', 'target_identity')) {
            $property = $sourceRow.PSObject.Properties[$propertyName]
            if ($null -ne $property -and $null -ne $property.Value) {
                $row.$propertyName = $property.Value | ConvertTo-Json -Depth 16 | ConvertFrom-Json
            }
        }
        $row
    }
}
$pclProfile = Get-PlaybackCoreLifecycleProfile
$pclBaselineRevision = $pclProfile.BaselineRevision
$pclBaselineTree = $pclProfile.BaselineTree
$pclBaselineManifestHash = 'c' * 64
$pclBaselineBinaryHash = 'd' * 64
$pclRowsList = [System.Collections.Generic.List[object]]::new()
$pclSequence = 0
$pclRowsList.Add([pscustomobject]@{
    sequence_index = $pclSequence; run_id = 'pcl-candidate-stop'; role = 'candidate'
    source_revision = $pclRevision; source_tree_sha = $pclTree; manifest_sha256 = $pclManifestHash; harness_sha256 = $pclBinaryHash
    scenario = 'stop-cleanup'; workload = 'safety-smoke'; load = 'Quiet'; cell_id = 'pcl-safety-quiet'; pair_id = ''
    sample_seconds = 0; status = 'PASS'; case_exit_code = 0; lifecycle_measurement = $pclLifecycleMeasurement
    target_identity = $pclTargetIdentity; machine_name = $pclMachine; os_boot_time_utc = $pclBootUtc
    runner_pre_child_process_launch_qpc_ticks = 10; runner_pre_child_process_launch_qpc_frequency_hz = 10000000
})
$pclSequence++
foreach ($workload in $pclProfile.PerformanceWorkloads) {
    foreach ($load in $pclProfile.LoadCells) {
        $cellId = $workload + '/' + $load
        for ($pair = 1; $pair -le $pclProfile.PairCount; $pair++) {
            $pairOrder = if ($pair % 2 -eq 1) { 'AB' } else { 'BA' }
            $roles = if ($pairOrder -eq 'AB') { @('control', 'candidate') } else { @('candidate', 'control') }
            foreach ($role in $roles) {
                $isCandidate = $role -eq 'candidate'
                $runId = 'pcl-' + $workload + '-' + $load.ToLowerInvariant() + '-pair-' + $pair + '-' + $role
                $row = [ordered]@{
                    sequence_index = $pclSequence; run_id = $runId; role = $role
                    source_revision = if ($isCandidate) { $pclRevision } else { $pclBaselineRevision }
                    source_tree_sha = if ($isCandidate) { $pclTree } else { $pclBaselineTree }
                    manifest_sha256 = if ($isCandidate) { $pclManifestHash } else { $pclBaselineManifestHash }
                    harness_sha256 = if ($isCandidate) { $pclBinaryHash } else { $pclBaselineBinaryHash }
                    scenario = $workload; workload = $workload; load = $load; cell_id = $cellId
                    pair_id = $cellId + '/' + ('{0:D2}' -f $pair); pair_order = $pairOrder
                    sample_seconds = 15; status = 'PASS'; case_exit_code = 0
                    target_identity = $pclTargetIdentity; machine_name = $pclMachine; os_boot_time_utc = $pclBootUtc
                    lifecycle_measurement = $null
                    runner_pre_child_process_launch_qpc_ticks = 1000 + ($pclSequence * 100) + 50
                    runner_pre_child_process_launch_qpc_frequency_hz = 10000000
                }
                if ($isCandidate) {
                    $armedTicks = 1000 + ($pclSequence * 100)
                    $row.lifecycle_measurement = [pscustomobject]@{
                        schema_version = 2; qpc_clock_source = 'Windows QueryPerformanceCounter'; qpc_frequency_hz = 10000000
                        native_session_identity = [pscustomobject]@{ run_id = $runId; scenario = $workload }
                        playback_admission_event = 'system_power_ready_to_armed_publication'
                        playback_armed_qpc_ticks = $armedTicks; worker_joined_qpc_ticks = $armedTicks + 40
                    }
                }
                $pclRowsList.Add([pscustomobject]$row)
                $pclSequence++
            }
        }
    }
}
$pclRows = $pclRowsList.ToArray()
$pclV2Rows = @(Copy-PclTestRows $pclRows)
foreach ($pclV2Row in $pclV2Rows) {
    if ($pclV2Row.role -eq 'control') {
        $pclV2Row.source_revision = $pclV2Profile.BaselineRevision
        $pclV2Row.source_tree_sha = $pclV2Profile.BaselineTree
        if ($pclV2Row.sample_seconds -eq 15) {
            $armedTicks = 1000 + ([int]$pclV2Row.sequence_index * 100)
            $pclV2Row.lifecycle_measurement = [pscustomobject]@{
                schema_version = 2; qpc_clock_source = 'Windows QueryPerformanceCounter'; qpc_frequency_hz = 10000000
                native_session_identity = [pscustomobject]@{ run_id = $pclV2Row.run_id; scenario = $pclV2Row.scenario }
                playback_admission_event = 'system_power_ready_to_armed_publication'
                playback_armed_qpc_ticks = $armedTicks; worker_joined_qpc_ticks = $armedTicks + 40
            }
        }
    }
    if ($pclV2Row.sample_seconds -eq 15) {
        $pclV2Row | Add-Member -NotePropertyName authored_workload_measured_prefix_sha256 -NotePropertyValue ('e' * 64) -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_measured_event_count -NotePropertyValue 42 -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_measured_window_start_us -NotePropertyValue 50000 -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_measured_window_end_us -NotePropertyValue 15050000 -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_full_stream_sha256 -NotePropertyValue ('f' * 64) -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_full_event_count -NotePropertyValue 100 -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_tail_sha256 -NotePropertyValue ('a' * 64) -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_tail_event_count -NotePropertyValue 58 -Force
        $pclV2Row | Add-Member -NotePropertyName authored_workload_tail_group_count -NotePropertyValue 27 -Force
    }
}
function New-PclV2TestReport {
    param([Parameter(Mandatory)] [object[]]$Rows)
    New-PlaybackCoreLifecycleMeasurementReport -RunRows $Rows `
        -CandidateRevision $pclRevision -CandidateTree $pclTree `
        -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash `
        -ControlManifestSha256 $pclBaselineManifestHash -ControlBinarySha256 $pclBaselineBinaryHash `
        -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc -ProfileVersion 2
}
$pclSingleCellRows = @($pclRows | Where-Object { $_.cell_id -eq 'mixed-up-down/Quiet' -or $_.sequence_index -eq 0 })
$pclSingleCellRoleOrder = @($pclSingleCellRows | Where-Object { $_.sample_seconds -eq 15 } | ForEach-Object { $_.role }) -join ','
if ($pclSingleCellRoleOrder -ne 'control,candidate,candidate,control,control,candidate,candidate,control,control,candidate,candidate,control') {
    throw ('PlaybackCoreLifecycle fixture does not preserve the actual six-pair AB/BA role order: ' + $pclSingleCellRoleOrder)
}
$pclSingleCellReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclSingleCellRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclSingleCellReport.worker_joined_to_next_start_us.coverage.by_transition[0].observed_count -ne 3 -or
    $pclSingleCellReport.worker_joined_to_next_start_us.coverage.by_transition[0].measured_count -ne 3 -or
    $pclSingleCellReport.worker_joined_to_next_start_us.coverage.by_transition[1].observed_count -ne 3 -or
    $pclSingleCellReport.worker_joined_to_next_start_us.coverage.by_transition[1].unavailable_count -ne 3 -or
    $pclSingleCellReport.worker_joined_to_next_start_us.coverage.by_transition[1].eligible_count -ne 0 -or
    $pclSingleCellReport.worker_joined_to_next_start_us.status -ne 'INCOMPLETE') {
    throw 'one six-pair AB/BA cell did not yield exactly three measurable candidate→candidate and three unavailable candidate→baseline transitions'
}
$pclReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclReport.status -ne 'INCOMPLETE' -or
    $pclReport.stop_to_verified_release_us.status -ne 'COMPLETE' -or
    $pclReport.stop_to_verified_release_us.statistics.p50 -ne 4 -or
    $pclReport.worker_joined_to_next_start_us.status -ne 'INCOMPLETE' -or
    $pclReport.worker_joined_to_next_start_us.statistics.sample_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.statistics.p50 -ne 6 -or
    $pclReport.worker_joined_to_next_start_us.coverage.planned_transition_count -ne 60 -or
    $pclReport.worker_joined_to_next_start_us.coverage.eligible_transition_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.coverage.measured_transition_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.coverage.unavailable_transition_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 0 -or
    $pclReport.worker_joined_to_next_start_us.coverage.missing_planned_transition_count -ne 0 -or
    $pclReport.worker_joined_to_next_start_us.coverage.incomplete_transition_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.coverage.by_transition[0].observed_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.coverage.by_transition[0].measured_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.coverage.by_transition[1].observed_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.coverage.by_transition[1].unavailable_count -ne 30 -or
    $pclReport.worker_joined_to_next_start_us.samples[0].next_role -ne 'candidate' -or
    $pclReport.worker_joined_to_next_start_us.samples[0].prior_run_id -notmatch 'pair-1-candidate$' -or
    $pclReport.worker_joined_to_next_start_us.samples[0].next_run_id -notmatch 'pair-2-candidate$' -or
    $pclReport.worker_joined_to_runner_pre_next_process_launch_us.status -ne 'DIAGNOSTIC_ONLY' -or
    $pclReport.worker_joined_to_runner_pre_next_process_launch_us.statistics.sample_count -ne 60 -or
    $pclReport.raw_up_syscall_count.statistics.p50 -ne 2 -or
    $pclReport.source_identity.manifest_sha256 -ne $pclManifestHash -or
    $pclReport.Contains('qualification_profile') -or
    $pclReport.Contains('authored_prefix_parity_contract')) {
    throw 'PlaybackCoreLifecycle full six-pair role-order fixture did not preserve incomplete baseline coverage and real candidate Armed evidence'
}
$pclV2Report = New-PclV2TestReport -Rows $pclV2Rows
if ($pclV2Report.qualification_profile -ne 'PlaybackCoreLifecycleV2' -or
    $pclV2Report.workload_version -ne 'playback-core-lifecycle-v2' -or
    $pclV2Report.authored_prefix_parity_contract -ne 'timestamp-kind-scan-mask-v1' -or
    $pclV2Report.status -ne 'COMPLETE' -or
    $pclV2Report.worker_joined_to_next_start_us.status -ne 'COMPLETE' -or
    $pclV2Report.stop_to_verified_release_us.status -ne 'COMPLETE' -or
    $pclV2Report.raw_up_syscall_count.status -ne 'COMPLETE' -or
    $pclV2Report.stop_to_verified_release_us.samples.Count -ne 1 -or
    $pclV2Report.raw_up_syscall_count.samples.Count -ne 1 -or
    $pclV2Report.worker_joined_to_next_start_us.statistics.sample_count -ne 60 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.planned_transition_count -ne 60 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.eligible_transition_count -ne 60 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.measured_transition_count -ne 60 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.unavailable_transition_count -ne 0 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 0 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.missing_planned_transition_count -ne 0 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.incomplete_transition_count -ne 0 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.by_transition[0].eligible_count -ne 30 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.by_transition[0].measured_count -ne 30 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.by_transition[1].eligible_count -ne 30 -or
    $pclV2Report.worker_joined_to_next_start_us.coverage.by_transition[1].measured_count -ne 30) {
    throw 'PlaybackCoreLifecycleV2 did not measure all 60 candidate-to-candidate and candidate-to-baseline successor Armed transitions'
}
$pclSafetyScenarios = @('focus-loss', 'owner-mismatch', 'owner-process-termination', 'pause-resume', 'suspend-resume', 'stop-cleanup', 'skip-cleanup', 'supervisor-lease-expiry')
$pclV2SafetyScenarios = $pclSafetyScenarios + @('short-up-gap-stress', 'pause-resume-active-cleanup')
$pclPerformanceWorkloads = @('mixed-up-down', 'dense-alternating', 'canonical-max-chord', 'near-minimum-retrigger', 'long-idle')
$pclV1StageSummary = Get-PlaybackCoreLifecycleStageSummary -Stage Final -TimingMarginUs 500 -Pairs 6 -SampleSeconds 15 `
    -SafetyScenarios $pclSafetyScenarios -PerformanceWorkloads $pclPerformanceWorkloads -LoadCells @('Quiet', 'Contention')
$pclV2StageSummary = Get-PlaybackCoreLifecycleStageSummary -Stage Final -TimingMarginUs 500 -Pairs 6 -SampleSeconds 15 `
    -SafetyScenarios $pclV2SafetyScenarios -PerformanceWorkloads $pclPerformanceWorkloads -LoadCells @('Quiet', 'Contention') -ProfileVersion 2
if ($pclV1StageSummary.Contains('authored_prefix_parity_contract') -or
    $pclV1StageSummary.candidate_session_count -ne 68 -or
    $pclV2StageSummary.authored_prefix_parity_contract -ne 'timestamp-kind-scan-mask-v1' -or
    $pclV2StageSummary.authored_prefix_measured_window_seconds -ne 15 -or
    $pclV2StageSummary.candidate_session_count -ne 70 -or
    $pclV2StageSummary.planned_session_count -ne 130 -or
    $pclV2StageSummary.scenarios -notcontains 'short-up-gap-stress' -or
    $pclV2StageSummary.scenarios -notcontains 'pause-resume-active-cleanup') {
    throw 'PlaybackCoreLifecycleV2 changed the v1 report contract or omitted its candidate-only safety cases and full paired-session plan'
}
foreach ($controlRow in @($pclRows | Where-Object { $_.role -eq 'control' })) {
    if ($null -ne $controlRow.lifecycle_measurement) {
        throw 'realistic frozen-baseline fixture invented candidate lifecycle evidence on a control row'
    }
}

$pclV2BaselineSuccessor = @($pclV2Rows | Where-Object {
    $_.role -eq 'control' -and $_.sample_seconds -eq 15 -and $_.pair_order -eq 'BA'
})[0]
$pclV2BaselinePredecessor = @($pclV2Rows | Where-Object {
    $_.role -eq 'candidate' -and $_.sample_seconds -eq 15 -and $_.pair_id -eq $pclV2BaselineSuccessor.pair_id
})[0]
function Assert-PclV2TransitionIncomplete {
    param([Parameter(Mandatory)] $Report, [Parameter(Mandatory)] [string]$Case)
    if ($Report.status -ne 'INCOMPLETE' -or
        $Report.worker_joined_to_next_start_us.status -ne 'INCOMPLETE' -or
        [string]::IsNullOrWhiteSpace(($Report.incompleteness -join '; '))) {
        throw ('PlaybackCoreLifecycleV2 ' + $Case + ' did not make both the transition metric and total report INCOMPLETE with a reason')
    }
}
function Assert-PclV2StopEvidenceIncomplete {
    param([Parameter(Mandatory)] $Report, [Parameter(Mandatory)] [string]$Case)
    if ($Report.status -ne 'INCOMPLETE' -or
        $Report.stop_to_verified_release_us.status -ne 'INCOMPLETE' -or
        $Report.raw_up_syscall_count.status -ne 'INCOMPLETE' -or
        $Report.stop_to_verified_release_us.samples.Count -ne 0 -or
        $Report.raw_up_syscall_count.samples.Count -ne 0 -or
        -not (($Report.incompleteness -join '; ') -match 'pcl-candidate-stop')) {
        throw ('PlaybackCoreLifecycleV2 ' + $Case + ' did not exclude the stop row from both child metrics and mark the report INCOMPLETE with its run ID: report=' + $Report.status + ' stop=' + $Report.stop_to_verified_release_us.status + '/' + $Report.stop_to_verified_release_us.samples.Count + ' raw=' + $Report.raw_up_syscall_count.status + '/' + $Report.raw_up_syscall_count.samples.Count + ' reasons=' + ($Report.incompleteness -join '; '))
    }
}

foreach ($stopMutation in @('binary', 'machine', 'boot', 'qpc-frequency', 'schema', 'native-run-id', 'native-scenario')) {
    $pclV2StopRows = @(Copy-PclTestRows $pclV2Rows)
    $pclV2StopRow = @($pclV2StopRows | Where-Object { $_.run_id -eq 'pcl-candidate-stop' })[0]
    switch ($stopMutation) {
        'binary' { $pclV2StopRow.harness_sha256 = 'e' * 64 }
        'machine' { $pclV2StopRow.machine_name = 'different-machine' }
        'boot' { $pclV2StopRow.os_boot_time_utc = '2026-11-18T00:00:00.0000000Z' }
        'qpc-frequency' { $pclV2StopRow.lifecycle_measurement.qpc_frequency_hz = 9999999 }
        'schema' { $pclV2StopRow.lifecycle_measurement.schema_version = 1 }
        'native-run-id' { $pclV2StopRow.lifecycle_measurement.native_session_identity.run_id = 'unrelated-run' }
        'native-scenario' { $pclV2StopRow.lifecycle_measurement.native_session_identity.scenario = 'other-scenario' }
    }
    $pclV2StopReport = New-PclV2TestReport -Rows $pclV2StopRows
    Assert-PclV2StopEvidenceIncomplete -Report $pclV2StopReport -Case ('stop-cleanup ' + $stopMutation)
}

$pclV2MissingBaselineArmedRows = @(Copy-PclTestRows $pclV2Rows)
$pclV2MissingBaselineArmed = @($pclV2MissingBaselineArmedRows | Where-Object { $_.run_id -eq $pclV2BaselineSuccessor.run_id })[0]
[void]$pclV2MissingBaselineArmed.lifecycle_measurement.PSObject.Properties.Remove('playback_armed_qpc_ticks')
$pclV2MissingBaselineArmedReport = New-PclV2TestReport -Rows $pclV2MissingBaselineArmedRows
Assert-PclV2TransitionIncomplete -Report $pclV2MissingBaselineArmedReport -Case 'missing baseline Armed QPC'
if ($pclV2MissingBaselineArmedReport.worker_joined_to_next_start_us.statistics.sample_count -ne 59 -or
    $pclV2MissingBaselineArmedReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or
    $pclV2MissingBaselineArmedReport.worker_joined_to_next_start_us.coverage.incomplete_transition_count -ne 1) {
    throw 'PlaybackCoreLifecycleV2 missing baseline Armed fixture did not invalidate exactly one required sample'
}

$pclV2BaselineClockRows = @(Copy-PclTestRows $pclV2Rows)
$pclV2BaselineClock = @($pclV2BaselineClockRows | Where-Object { $_.run_id -eq $pclV2BaselineSuccessor.run_id })[0]
$pclV2BaselineClock.lifecycle_measurement.qpc_frequency_hz = 9999999
$pclV2BaselineClockReport = New-PclV2TestReport -Rows $pclV2BaselineClockRows
Assert-PclV2TransitionIncomplete -Report $pclV2BaselineClockReport -Case 'baseline QPC frequency mismatch'

$pclV2MalformedQpcRows = @(Copy-PclTestRows $pclV2Rows)
$pclV2MalformedQpc = @($pclV2MalformedQpcRows | Where-Object { $_.run_id -eq $pclV2BaselineSuccessor.run_id })[0]
$pclV2MalformedQpc.lifecycle_measurement.qpc_frequency_hz = 'not-a-frequency'
$pclV2MalformedQpcReport = New-PclV2TestReport -Rows $pclV2MalformedQpcRows
Assert-PclV2TransitionIncomplete -Report $pclV2MalformedQpcReport -Case 'malformed baseline QPC frequency'

$pclV2ClockSourceRows = @(Copy-PclTestRows $pclV2Rows)
$pclV2ClockSource = @($pclV2ClockSourceRows | Where-Object { $_.run_id -eq $pclV2BaselineSuccessor.run_id })[0]
$pclV2ClockSource.lifecycle_measurement.qpc_clock_source = 'different-clock'
$pclV2ClockSourceReport = New-PclV2TestReport -Rows $pclV2ClockSourceRows
Assert-PclV2TransitionIncomplete -Report $pclV2ClockSourceReport -Case 'baseline QPC clock source mismatch'

$pclV2MalformedJoinRows = @(Copy-PclTestRows $pclV2Rows)
$pclV2MalformedJoin = @($pclV2MalformedJoinRows | Where-Object { $_.run_id -eq $pclV2BaselinePredecessor.run_id })[0]
$pclV2MalformedJoin.lifecycle_measurement.worker_joined_qpc_ticks = 'not-a-timestamp'
$pclV2MalformedJoinReport = New-PclV2TestReport -Rows $pclV2MalformedJoinRows
Assert-PclV2TransitionIncomplete -Report $pclV2MalformedJoinReport -Case 'malformed candidate worker-join QPC'

foreach ($rowIdentityMutation in @('machine_name', 'os_boot_time_utc')) {
    $pclV2WrongRunnerRows = @(Copy-PclTestRows $pclV2Rows)
    $pclV2WrongRunnerRow = @($pclV2WrongRunnerRows | Where-Object { $_.run_id -eq $pclV2BaselineSuccessor.run_id })[0]
    if ($rowIdentityMutation -eq 'machine_name') { $pclV2WrongRunnerRow.machine_name = 'different-machine' }
    else { $pclV2WrongRunnerRow.os_boot_time_utc = '2026-11-18T00:00:00.0000000Z' }
    $pclV2WrongRunnerReport = New-PclV2TestReport -Rows $pclV2WrongRunnerRows
    Assert-PclV2TransitionIncomplete -Report $pclV2WrongRunnerReport -Case ('baseline ' + $rowIdentityMutation + ' mismatch')
}

foreach ($identityMutation in @('source_revision', 'source_tree_sha', 'manifest_sha256', 'harness_sha256')) {
    $pclV2BadIdentityRows = @(Copy-PclTestRows $pclV2Rows)
    $pclV2BadIdentity = @($pclV2BadIdentityRows | Where-Object { $_.run_id -eq $pclV2BaselineSuccessor.run_id })[0]
    if ($identityMutation -eq 'source_revision') { $pclV2BadIdentity.source_revision = 'f' * 40 }
    elseif ($identityMutation -eq 'source_tree_sha') { $pclV2BadIdentity.source_tree_sha = 'e' * 40 }
    elseif ($identityMutation -eq 'manifest_sha256') { $pclV2BadIdentity.manifest_sha256 = 'e' * 64 }
    else { $pclV2BadIdentity.harness_sha256 = 'e' * 64 }
    $pclV2BadIdentityReport = New-PclV2TestReport -Rows $pclV2BadIdentityRows
    Assert-PclV2TransitionIncomplete -Report $pclV2BadIdentityReport -Case ('baseline ' + $identityMutation + ' mismatch')
}

$pclV2BaselineChronologyRows = @(Copy-PclTestRows $pclV2Rows)
$pclV2BaselineChronology = @($pclV2BaselineChronologyRows | Where-Object { $_.run_id -eq $pclV2BaselineSuccessor.run_id })[0]
$pclV2BaselineChronology.lifecycle_measurement.playback_armed_qpc_ticks = $pclV2BaselinePredecessor.lifecycle_measurement.worker_joined_qpc_ticks - 1
$pclV2BaselineChronologyReport = New-PclV2TestReport -Rows $pclV2BaselineChronologyRows
Assert-PclV2TransitionIncomplete -Report $pclV2BaselineChronologyReport -Case 'Armed-before-Join chronology'

$pclV2InvalidSuccessorRows = @(Copy-PclTestRows $pclV2Rows)
$pclV2CandidateSuccessor = @($pclV2InvalidSuccessorRows | Where-Object {
    $_.role -eq 'candidate' -and $_.sample_seconds -eq 15 -and $_.pair_order -eq 'BA'
})[0]
$pclV2CandidateSuccessor.status = 'FAIL'
$pclV2CandidateSuccessor.case_exit_code = 1
$pclV2InvalidSuccessorReport = New-PclV2TestReport -Rows $pclV2InvalidSuccessorRows
Assert-PclV2TransitionIncomplete -Report $pclV2InvalidSuccessorReport -Case 'failed candidate successor'

foreach ($drainCompletionTicks in @(211, 10000, 900000000)) {
    $pclDelayedDrainRows = @(Copy-PclTestRows $pclRows)
    $pclDelayedDrainRows[0].lifecycle_measurement.post_join_report_and_receiveonly_validation_completed_qpc_ticks = $drainCompletionTicks
    $pclDelayedDrainReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclDelayedDrainRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
    if ($pclDelayedDrainReport.stop_to_verified_release_us.statistics.p50 -ne 4 -or
        $pclDelayedDrainReport.stop_to_verified_release_us.samples[0].worker_release_verified_qpc_ticks -ne 190) {
        throw ('post-join telemetry/ReceiveOnly drain delay changed the worker release interval: ' + $drainCompletionTicks)
    }
}

$pclMissingStartRows = @(Copy-PclTestRows $pclRows)
[void]$pclMissingStartRows[3].lifecycle_measurement.PSObject.Properties.Remove('playback_armed_qpc_ticks')
$pclMissingStartReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclMissingStartRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclMissingStartReport.status -ne 'INCOMPLETE' -or
    $pclMissingStartReport.worker_joined_to_next_start_us.status -ne 'INCOMPLETE' -or
    $pclMissingStartReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or
    $pclMissingStartReport.worker_joined_to_next_start_us.statistics.sample_count -ne 29 -or
    $pclMissingStartReport.worker_joined_to_runner_pre_next_process_launch_us.statistics.sample_count -ne 60) {
    throw 'missing actual next playback Start was substituted with the available child-process launch diagnostic'
}

$pclClockMismatchRows = @(Copy-PclTestRows $pclRows)
$pclClockMismatchRows[3].lifecycle_measurement.qpc_frequency_hz = 9999999
$pclClockMismatchReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclClockMismatchRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclClockMismatchReport.status -ne 'INCOMPLETE' -or $pclClockMismatchReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or $pclClockMismatchReport.worker_joined_to_next_start_us.statistics.sample_count -ne 29) {
    throw 'PlaybackCoreLifecycle accepted successor QPC frequency mismatch'
}
$pclMissingFrequencyRows = @(Copy-PclTestRows $pclRows)
$pclMissingFrequencyRows[3].lifecycle_measurement.qpc_frequency_hz = $null
$pclMissingFrequencyReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclMissingFrequencyRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclMissingFrequencyReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or $pclMissingFrequencyReport.worker_joined_to_next_start_us.statistics.sample_count -ne 29) {
    throw 'PlaybackCoreLifecycle accepted successor QPC evidence without a frequency'
}
$pclClockSourceMismatchRows = @(Copy-PclTestRows $pclRows)
$pclClockSourceMismatchRows[3].lifecycle_measurement.qpc_clock_source = 'different-clock'
$pclClockSourceMismatchReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclClockSourceMismatchRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclClockSourceMismatchReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or $pclClockSourceMismatchReport.worker_joined_to_next_start_us.statistics.sample_count -ne 29) {
    throw 'PlaybackCoreLifecycle accepted mismatched QPC clock provenance'
}

foreach ($invalidArmedTicks in @(0, 1000)) {
    $pclInvalidArmedRows = @(Copy-PclTestRows $pclRows)
    $pclInvalidArmedRows[3].lifecycle_measurement.playback_armed_qpc_ticks = $invalidArmedTicks
    $pclInvalidArmedReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclInvalidArmedRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
    if ($pclInvalidArmedReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or $pclInvalidArmedReport.worker_joined_to_next_start_us.statistics.sample_count -ne 29) {
        throw ('PlaybackCoreLifecycle accepted missing/invalid successor Armed QPC ticks: ' + $invalidArmedTicks)
    }
}

$pclWrongSuccessorIdentityRows = @(Copy-PclTestRows $pclRows)
$pclWrongSuccessorIdentityRows[3].lifecycle_measurement.native_session_identity.run_id = 'unrelated-run'
$pclWrongSuccessorIdentityReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongSuccessorIdentityRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclWrongSuccessorIdentityReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or $pclWrongSuccessorIdentityReport.worker_joined_to_next_start_us.statistics.sample_count -ne 29) {
    throw 'PlaybackCoreLifecycle accepted a successor Armed event bound to a different run ID'
}

$pclFabricatedBaselineRows = @(Copy-PclTestRows $pclRows)
$pclFabricatedBaselineMeasurement = $pclFabricatedBaselineRows[3].lifecycle_measurement | ConvertTo-Json -Depth 16 | ConvertFrom-Json
$pclFabricatedBaselineMeasurement.native_session_identity.run_id = $pclFabricatedBaselineRows[4].run_id
$pclFabricatedBaselineMeasurement.native_session_identity.scenario = $pclFabricatedBaselineRows[4].scenario
$pclFabricatedBaselineRows[4].lifecycle_measurement = $pclFabricatedBaselineMeasurement
$pclFabricatedBaselineReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclFabricatedBaselineRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclFabricatedBaselineReport.worker_joined_to_next_start_us.coverage.unavailable_transition_count -ne 29 -or
    $pclFabricatedBaselineReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or
    $pclFabricatedBaselineReport.worker_joined_to_next_start_us.statistics.sample_count -ne 30) {
    throw 'PlaybackCoreLifecycle treated fabricated schema-v2 lifecycle evidence as a frozen-baseline playback Start'
}

$pclMissingSuccessorRows = @(Copy-PclTestRows $pclRows)
$pclMissingSuccessorRows = @($pclMissingSuccessorRows | Where-Object { $_.sequence_index -ne 3 })
$pclMissingSuccessorReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclMissingSuccessorRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclMissingSuccessorReport.worker_joined_to_next_start_us.coverage.missing_planned_transition_count -ne 2 -or
    $pclMissingSuccessorReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -ne 1 -or
    $pclMissingSuccessorReport.worker_joined_to_next_start_us.coverage.incomplete_transition_count -ne 31 -or
    $pclMissingSuccessorReport.worker_joined_to_next_start_us.status -ne 'INCOMPLETE') {
    throw 'PlaybackCoreLifecycle silently dropped a missing timed successor row'
}

$pclMissingSuccessorEvidenceRows = @(Copy-PclTestRows $pclRows)
$pclMissingSuccessorEvidenceRows[3].lifecycle_measurement = $null
$pclMissingSuccessorEvidenceReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclMissingSuccessorEvidenceRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclMissingSuccessorEvidenceReport.worker_joined_to_next_start_us.coverage.invalid_transition_count -lt 1 -or
    $pclMissingSuccessorEvidenceReport.worker_joined_to_next_start_us.statistics.sample_count -ne 29) {
    throw 'PlaybackCoreLifecycle accepted a missing candidate successor lifecycle measurement'
}

$pclWrongRoleRows = @(Copy-PclTestRows $pclRows)
$pclWrongRoleRows[3].role = 'baseline'
$pclWrongRoleRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongRoleRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongRoleRejected = $true }
if (-not $pclWrongRoleRejected) { throw 'PlaybackCoreLifecycle accepted a role outside the exact candidate/control protocol' }

foreach ($identityMutation in @(
        [pscustomobject]@{ name = 'candidate source SHA'; property = 'source_revision'; value = 'f' * 40; row = 3 },
        [pscustomobject]@{ name = 'candidate source tree'; property = 'source_tree_sha'; value = 'e' * 40; row = 3 },
        [pscustomobject]@{ name = 'candidate manifest hash'; property = 'manifest_sha256'; value = 'e' * 64; row = 3 },
        [pscustomobject]@{ name = 'candidate binary hash'; property = 'harness_sha256'; value = 'f' * 64; row = 3 },
        [pscustomobject]@{ name = 'baseline source SHA'; property = 'source_revision'; value = 'e' * 40; row = 4 },
        [pscustomobject]@{ name = 'baseline source tree'; property = 'source_tree_sha'; value = 'f' * 40; row = 4 }
    )) {
    $pclIdentityRows = @(Copy-PclTestRows $pclRows)
    $pclIdentityRows[$identityMutation.row].($identityMutation.property) = $identityMutation.value
    $pclIdentityRejected = $false
    try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclIdentityRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclIdentityRejected = $true }
    if (-not $pclIdentityRejected) { throw ('PlaybackCoreLifecycle accepted mutated ' + $identityMutation.name) }
}

$pclWrongControlHashRows = @(Copy-PclTestRows $pclRows)
$pclWrongControlHashRows[4].manifest_sha256 = 'e' * 64
$pclWrongControlHashRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongControlHashRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongControlHashRejected = $true }
if (-not $pclWrongControlHashRejected) { throw 'PlaybackCoreLifecycle accepted a control manifest/binary hash that differs from the pinned baseline identity' }

$pclWrongBootRows = @(Copy-PclTestRows $pclRows)
$pclWrongBootRows[3].os_boot_time_utc = '2026-11-18T00:00:00.0000000Z'
$pclWrongBootRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongBootRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongBootRejected = $true }
if (-not $pclWrongBootRejected) { throw 'PlaybackCoreLifecycle accepted a successor from a different Windows boot identity' }
$pclWrongMachineRows = @(Copy-PclTestRows $pclRows)
$pclWrongMachineRows[3].machine_name = 'different-machine'
$pclWrongMachineRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongMachineRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongMachineRejected = $true }
if (-not $pclWrongMachineRejected) { throw 'PlaybackCoreLifecycle accepted a successor from a different machine identity' }

$pclReleaseProofMissingRows = @(Copy-PclTestRows $pclRows)
$pclReleaseProofMissingRows[0].lifecycle_measurement.release_sample_gate_proof_complete = $false
$pclReleaseProofMissingReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclReleaseProofMissingRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclReleaseProofMissingReport.status -ne 'INCOMPLETE' -or $pclReleaseProofMissingReport.stop_to_verified_release_us.status -ne 'INCOMPLETE') {
    throw 'PlaybackCoreLifecycle accepted a missing worker release/journal/receiver proof'
}
$pclReceiverInconclusiveRows = @(Copy-PclTestRows $pclRows)
$pclReceiverInconclusiveRows[0].status = 'INCONCLUSIVE'
$pclReceiverInconclusiveRows[0].case_exit_code = 1
$pclReceiverInconclusiveReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclReceiverInconclusiveRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclReceiverInconclusiveReport.status -ne 'INCOMPLETE' -or $pclReceiverInconclusiveReport.stop_to_verified_release_us.status -ne 'INCOMPLETE') {
    throw 'PlaybackCoreLifecycle accepted inconclusive ReceiveOnly release evidence'
}

$pclRawEvidenceMissingRows = @(Copy-PclTestRows $pclRows)
[void]$pclRawEvidenceMissingRows[0].lifecycle_measurement.PSObject.Properties.Remove('raw_up_syscall_count')
$pclRawEvidenceMissingReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclRawEvidenceMissingRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclRawEvidenceMissingReport.status -ne 'INCOMPLETE' -or $pclRawEvidenceMissingReport.raw_up_syscall_count.status -ne 'INCOMPLETE') {
    throw 'PlaybackCoreLifecycle invented a raw Up syscall count when raw evidence was missing'
}
$pclWrongTargetRows = @(Copy-PclTestRows $pclRows)
$pclWrongTargetRows[1].target_identity = $pclTargetIdentity | Select-Object *
$pclWrongTargetRows[1].target_identity.game_target = $true
$pclWrongTargetRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongTargetRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongTargetRejected = $true }
if (-not $pclWrongTargetRejected) { throw 'PlaybackCoreLifecycle collector accepted game_target=true' }
$pclWrongClassRows = @(Copy-PclTestRows $pclRows)
$pclWrongClassRows[0].target_identity.target_class = 'Sky'
$pclWrongClassRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongClassRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongClassRejected = $true }
if (-not $pclWrongClassRejected) { throw 'PlaybackCoreLifecycle collector accepted a Sky target class' }
$pclMissingGameTargetRows = @(Copy-PclTestRows $pclRows)
[void]$pclMissingGameTargetRows[0].target_identity.PSObject.Properties.Remove('game_target')
$pclMissingGameTargetRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclMissingGameTargetRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclMissingGameTargetRejected = $true }
if (-not $pclMissingGameTargetRejected) { throw 'PlaybackCoreLifecycle collector accepted missing game_target identity' }
$pclWrongHashRows = @(Copy-PclTestRows $pclRows)
$pclWrongHashRows[0].manifest_sha256 = 'e' * 64
$pclWrongHashRejected = $false
try { [void](New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclWrongHashRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc) } catch { $pclWrongHashRejected = $true }
if (-not $pclWrongHashRejected) { throw 'PlaybackCoreLifecycle collector accepted a candidate manifest hash mismatch' }
$pclIncompleteRows = @(Copy-PclTestRows $pclRows)
$pclIncompleteRows[0].lifecycle_measurement.release_sample_gate_proof_complete = $false
$pclIncompleteReport = New-PlaybackCoreLifecycleMeasurementReport -RunRows $pclIncompleteRows -CandidateRevision $pclRevision -CandidateTree $pclTree -CandidateManifestSha256 $pclManifestHash -CandidateBinarySha256 $pclBinaryHash -RunnerMachineName $pclMachine -RunnerOsBootTimeUtc $pclBootUtc
if ($pclIncompleteReport.status -ne 'INCOMPLETE' -or $pclIncompleteReport.stop_to_verified_release_us.status -ne 'INCOMPLETE') {
    throw 'PlaybackCoreLifecycle collector did not fail closed when verified-release proof was absent'
}

Write-Output ('PASS: DispatchCleanupM3 contract positives plus ' + $m3NegativeContractCount + ' negative cases; DispatchSessionRedesignM3 schema-v2 provenance inventory, mapping, timing, count, and fail-stop contracts pass; existing Scheduler, FrameSnapshot, DispatchCore, sink, and fail-stop contracts remain passing')
Write-Output ('PASS: PlaybackCoreLifecycle v1 frozen-baseline semantics remain planned=60 eligible=30 measured=30 unavailable=30 incomplete=30; v2 positive coverage=60/60 COMPLETE (30 candidate→candidate, 30 candidate→baseline); missing/malformed Armed, Join, QPC, source/tree/manifest/binary, machine/boot, chronology, and invalid-successor fixtures set both metric and report INCOMPLETE; v2 stop provenance fixtures exclude binary/machine/boot/QPC/schema/native-session identity failures from both Stop and raw-Up metrics; Stop→release and raw syscall evidence contracts pass')
Write-Output 'PASS: PowerShell parser, paired CSV/provenance, Smoke/Final fail-stop, and timestamp provenance contracts'
