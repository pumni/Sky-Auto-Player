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

foreach ($scriptName in @('build_dispatch_core_harness.ps1', 'run_native_acceptance_case.ps1', 'native_acceptance_sink.ps1')) {
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
        process = 'native_acceptance_sink.ps1'; input_policy = 'receives_only; benchmark must use SendInput'
        pid = 17; hwnd = 42; event_log_id = $observerReady.event_log_id
        process_start_time_filetime = $identityProcessStart
        observer_mode = $observerMode; observer_version = $observerVersion
        observer_queue_capacity = 32768; observer_qpc_frequency_hz = 10000000
    }
    Assert-SinkIdentity -Record $sinkReady -SessionId 1 -ExpectedObserverVersion $observerVersion

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
$finalSafetyBlock = [regex]::Match($runnerSource, '(?s)\$finalSafetyCases\s*=\s*@\((.*?)\n\s*\)\s*\n\s*\$sequenceResult', [System.Text.RegularExpressions.RegexOptions]::Singleline)
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
if ($frameBuilderSource -notmatch "ValidateSet\('DispatchCore', 'Scheduler', 'FrameSnapshot'\)" -or
    $acceptanceSource -notmatch "ValidateSet\('DispatchCore', 'Scheduler', 'FrameSnapshot'\)") {
    throw 'FrameSnapshot profile is missing from an immutable build or case-runner boundary'
}
$qualifierSource = Get-Content -LiteralPath $runnerPath -Raw
if ($qualifierSource -notmatch "ValidateSet\('DispatchCore', 'Scheduler', 'FrameSnapshot'\)") {
    throw 'FrameSnapshot profile is missing from qualification runner'
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
if ($qualifierSource -notmatch "ValidateSet\('Canary', 'Smoke', 'Compare', 'Final'\)" -or
    -not $qualifierSource.Contains("if (`$Stage -eq 'Canary' -and `$QualificationProfile -ne 'FrameSnapshot')")) {
    throw 'FrameSnapshot canary stage is missing its opt-in stage and profile boundary'
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

Write-Output 'PASS: PowerShell parser, paired CSV/provenance, Smoke/Final fail-stop, and timestamp provenance contracts'
