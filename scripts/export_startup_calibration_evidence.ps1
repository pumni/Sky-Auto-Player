[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$QualificationDirectory,
    [Parameter(Mandatory = $true)]
    [string]$CaseRoot,
    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'lib/startup_calibration_projection.ps1')

function Resolve-RepositoryPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ([System.IO.Path]::IsPathRooted($Path)) { return [System.IO.Path]::GetFullPath($Path) }
    [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $Path))
}

function Get-Sha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-VerifiedManifest {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $manifestPath = Join-Path $Directory 'SHA256SUMS.txt'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Missing checksum manifest: $manifestPath"
    }
    $entries = [System.Collections.Generic.List[object]]::new()
    $verifiedFiles = 0
    foreach ($line in Get-Content -LiteralPath $manifestPath) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ($line -notmatch '^([0-9a-fA-F]{64})\s{2}(.+)$') {
            throw "Malformed checksum entry in $manifestPath"
        }
        $expectedHash = $Matches[1].ToLowerInvariant()
        $relativeName = $Matches[2]
        if ([System.IO.Path]::GetFileName($relativeName) -ne $relativeName) {
            throw "Checksum entry must name a file in its evidence directory: $relativeName"
        }
        $filePath = Join-Path $Directory $relativeName
        if (-not (Test-Path -LiteralPath $filePath -PathType Leaf)) {
            throw "Manifest file is missing: $relativeName"
        }
        $actualHash = Get-Sha256 -Path $filePath
        if ($actualHash -ne $expectedHash) {
            throw "Checksum mismatch for $relativeName"
        }
        $entries.Add([ordered]@{ file = $relativeName; sha256 = $expectedHash })
        $verifiedFiles++
    }
    [pscustomobject]@{
        sha256 = Get-Sha256 -Path $manifestPath
        entries = $entries.ToArray()
        verified_file_count = $verifiedFiles
    }
}

function Get-Distribution {
    param(
        [Parameter(Mandatory = $true)][double[]]$Values,
        [long]$Missing = 0,
        [long]$Dropped = 0,
        [long]$TruncatedRuns = 0
    )

    $ordered = @($Values | Sort-Object)
    $count = $ordered.Count
    $getQuantile = {
        param([double]$Probability)
        if ($count -eq 0) { return $null }
        $index = [Math]::Max(0, [Math]::Ceiling($Probability * $count) - 1)
        return [double]$ordered[$index]
    }
    [ordered]@{
        p50 = & $getQuantile 0.50
        p95 = & $getQuantile 0.95
        p99 = & $getQuantile 0.99
        p99_9 = & $getQuantile 0.999
        max = if ($count -gt 0) { [double]$ordered[-1] } else { $null }
        sample_count = $count
        missing_count = $Missing
        dropped_count = $Dropped
        truncated_run_count = $TruncatedRuns
        quantile = 'nearest_rank_ceiling'
    }
}

function Get-CellDistribution {
    param([object]$Distribution)
    if ($null -eq $Distribution) { return $null }
    [ordered]@{
        p50 = $Distribution.p50
        p95 = $Distribution.p95
        p99 = $Distribution.p99
        p99_9 = $Distribution.p99_9
        max = $Distribution.max
        sample_count = $Distribution.sample_count
        missing_count = $Distribution.missing_count
        dropped_count = $Distribution.dropped_count
        truncated_run_count = $Distribution.truncated_run_count
        quantile = $Distribution.quantile
    }
}

$qualificationRoot = (Resolve-Path -LiteralPath (Resolve-RepositoryPath -Path $QualificationDirectory)).Path
$caseRootPath = (Resolve-Path -LiteralPath (Resolve-RepositoryPath -Path $CaseRoot)).Path
$qualificationManifest = Get-VerifiedManifest -Directory $qualificationRoot
if (@(Get-ChildItem -LiteralPath $qualificationRoot -File | Where-Object Name -ne 'SHA256SUMS.txt').Count -ne $qualificationManifest.verified_file_count) {
    throw 'Qualification directory contents do not match the manifest file set.'
}
$metadataPath = Join-Path $qualificationRoot 'qualification.json'
$summaryPath = Join-Path $qualificationRoot 'summary.json'
$runOrderPath = Join-Path $qualificationRoot 'run-order.jsonl'
$metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
$summary = Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json
$harnessPath = Join-Path $repositoryRoot 'rust\target\release\rt-native-acceptance.exe'
if (-not (Test-Path -LiteralPath $harnessPath -PathType Leaf)) {
    throw 'The measured Release acceptance binary is required to verify the recorded binary hash.'
}
$actualHarnessHash = Get-Sha256 -Path $harnessPath
if ($actualHarnessHash -ne $metadata.harness_sha256) {
    throw 'The local Release acceptance binary does not match the measured binary hash.'
}
$sourceRunOrder = @(
    Get-Content -LiteralPath $runOrderPath |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
        ForEach-Object { $_ | ConvertFrom-Json }
)

if (-not $metadata.source_tree_clean -or $metadata.build_exit_code -ne 0) {
    throw 'The qualification must identify a clean source tree and a successful harness build.'
}
if ($sourceRunOrder.Count -ne 120 -or -not $summary.all_receive_only_acceptance_runs_passed) {
    throw "Expected 120 passing source rows; found $($sourceRunOrder.Count)."
}

$runLedger = [System.Collections.Generic.List[object]]::new()
$sessionReports = [System.Collections.Generic.List[object]]::new()
$caseFileCount = 0
$verifiedCaseCount = 0
$qpcFrequencies = [System.Collections.Generic.HashSet[long]]::new()
$runNumber = 0
foreach ($row in $sourceRunOrder) {
    $runNumber++
    if ($row.case_exit_code -ne 0 -or $row.status -ne 'PASS') {
        throw "Run-order row $runNumber did not pass."
    }
    $caseName = [System.IO.Path]::GetFileName([string]$row.evidence_directory)
    if ([string]::IsNullOrWhiteSpace($caseName)) { throw "Run $runNumber has no case directory name." }
    $caseDirectory = Join-Path $caseRootPath $caseName
    if (-not (Test-Path -LiteralPath $caseDirectory -PathType Container)) {
        throw "Missing case evidence directory: $caseName"
    }
    $caseManifest = Get-VerifiedManifest -Directory $caseDirectory
    $verifiedCaseCount++
    if (@(Get-ChildItem -LiteralPath $caseDirectory -File | Where-Object Name -ne 'SHA256SUMS.txt').Count -ne $caseManifest.verified_file_count) {
        throw "Case directory contents do not match the manifest file set: $caseName"
    }
    $caseFileCount += $caseManifest.verified_file_count
    $reportPath = Join-Path $caseDirectory 'rt-native-acceptance.jsonl'
    $sinkWindowPath = Join-Path $caseDirectory 'sink-event-window.json'
    $sinkEventsPath = Join-Path $caseDirectory 'sink-events.jsonl'
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    if ($report.status -ne 'PASS' -or $report.scenario -ne $row.scenario) {
        throw "Acceptance report disagrees with the run-order row for $caseName."
    }
    $telemetry = $report.details.native_timing_telemetry
    if ($null -eq $telemetry -or $telemetry.qpc_frequency_hz -le 0) {
        throw "Missing valid native timing telemetry for $caseName."
    }
    [void]$qpcFrequencies.Add([long]$telemetry.qpc_frequency_hz)
    $sessionReports.Add([pscustomobject]@{
        placement = [string]$row.placement
        report = $report
    })
    $artifacts = [ordered]@{
        case_manifest_sha256 = $caseManifest.sha256
        acceptance_report_sha256 = Get-Sha256 -Path $reportPath
        receive_only_event_window_sha256 = Get-Sha256 -Path $sinkWindowPath
        receive_only_event_stream_sha256 = Get-Sha256 -Path $sinkEventsPath
    }
    $runLedger.Add([ordered]@{
        sequence = $runNumber
        pair_id = [string]$row.pair_id
        workload = [string]$row.scenario
        load = if ($row.contention) { 'controlled_contention' } else { 'quiet' }
        placement = [string]$row.placement
        status = [string]$row.status
        exit_code = [int]$row.case_exit_code
        artifacts = $artifacts
    })
}

if ($qpcFrequencies.Count -ne 1) {
    throw "The accepted sessions do not share one QPC frequency: $($qpcFrequencies -join ',')."
}
$qpcFrequency = @($qpcFrequencies)[0]
$pairIds = @($runLedger | ForEach-Object { $_.pair_id } | Select-Object -Unique)
if ($pairIds.Count -ne 60) { throw "Expected 60 paired comparisons; found $($pairIds.Count)." }
$placementCounts = @{
    baseline = @($runLedger | Where-Object placement -eq 'baseline').Count
    candidate = @($runLedger | Where-Object placement -eq 'candidate').Count
}
if ($placementCounts.baseline -ne 60 -or $placementCounts.candidate -ne 60) {
    throw 'The accepted run ledger does not contain 60 sessions per placement.'
}

$pairMetrics = @{}
foreach ($group in $summary.paired_comparisons) {
    foreach ($pair in $group.residual_lateness.pairs) {
        if (-not $pairMetrics.ContainsKey($pair.pair_id)) {
            $pairMetrics[$pair.pair_id] = [ordered]@{
                residual = $null
                ready = $null
                wake = $null
            }
        }
        $pairMetrics[$pair.pair_id].residual = $pair
    }
    foreach ($pair in $group.startup_ready_latency.pairs) {
        $pairMetrics[$pair.pair_id].ready = $pair
    }
    foreach ($pair in $group.startup_wake_error_session_p95.pairs) {
        $pairMetrics[$pair.pair_id].wake = $pair
    }
}
$pairs = [System.Collections.Generic.List[object]]::new()
foreach ($pairId in $pairIds) {
    $pairRuns = @($runLedger | Where-Object pair_id -eq $pairId)
    if ($pairRuns.Count -ne 2 -or @($pairRuns | ForEach-Object { $_.placement } | Select-Object -Unique).Count -ne 2 -or
        $null -eq $pairMetrics[$pairId].residual -or
        $null -eq $pairMetrics[$pairId].ready -or $null -eq $pairMetrics[$pairId].wake) {
        throw "Incomplete paired evidence for $pairId."
    }
    $pairRuns = @($pairRuns | Sort-Object sequence)
    $metrics = $pairMetrics[$pairId]
    $pairs.Add([ordered]@{
        pair_id = $pairId
        workload = $pairRuns[0].workload
        load = $pairRuns[0].load
        run_order = @($pairRuns | ForEach-Object { $_.placement })
        run_sequences = @($pairRuns | ForEach-Object { $_.sequence })
        outcome = if (@($pairRuns | Where-Object { $_.status -ne 'PASS' }).Count -eq 0) { 'PASS' } else { 'FAIL' }
        baseline = [ordered]@{
            ready_latency_us = $metrics.ready.baseline
            startup_wake_error_p95_us = $metrics.wake.baseline
            residual_lateness_p99_us = $metrics.residual.baseline_p99_residual_us
            residual_sample_count = $metrics.residual.baseline_samples
        }
        candidate = [ordered]@{
            ready_latency_us = $metrics.ready.candidate
            startup_wake_error_p95_us = $metrics.wake.candidate
            residual_lateness_p99_us = $metrics.residual.candidate_p99_residual_us
            residual_sample_count = $metrics.residual.candidate_samples
        }
        candidate_minus_baseline = [ordered]@{
            ready_latency_us = $metrics.ready.candidate_delta
            startup_wake_error_p95_us = $metrics.wake.candidate_delta
            residual_lateness_p99_us = $metrics.residual.candidate_delta_us
        }
        candidate_residual_improved = [bool]$metrics.residual.candidate_improved
    })
}
$pairOrderCounts = @($pairs | ForEach-Object { ($_.run_order -join ',') } | Group-Object)
if ($pairOrderCounts.Count -ne 2 -or
    @($pairOrderCounts | Where-Object { $_.Name -eq 'baseline,candidate' -and $_.Count -eq 30 }).Count -ne 1 -or
    @($pairOrderCounts | Where-Object { $_.Name -eq 'candidate,baseline' -and $_.Count -eq 30 }).Count -ne 1) {
    throw 'Expected 30 baseline/candidate and 30 candidate/baseline pairs.'
}

$cells = [System.Collections.Generic.List[object]]::new()
foreach ($cell in $summary.cells) {
    $load = if ($cell.contention) { 'controlled_contention' } else { 'quiet' }
    $cellRuns = @($runLedger | Where-Object {
        $_.workload -eq $cell.scenario -and $_.load -eq $load -and $_.placement -eq $cell.placement
    })
    $cells.Add([ordered]@{
        workload = [string]$cell.scenario
        load = $load
        placement = [string]$cell.placement
        outcome = if ($cellRuns.Count -eq 6 -and @($cellRuns | Where-Object status -ne 'PASS').Count -eq 0) { 'PASS' } else { 'INCOMPLETE_OR_FAILED' }
        session_count = $cell.run_count
        passed_sessions = @($cellRuns | Where-Object status -eq 'PASS').Count
        calibration = [ordered]@{
            executed_sessions = $cell.calibration_executed_runs
            fallback_sessions = $cell.calibration_fallback_runs
            execution_rate = $cell.calibration_execution_rate
            fallback_rate = $cell.calibration_fallback_rate
            sample_count = Get-CellDistribution -Distribution $cell.calibration_sample_count
        }
        session_metrics = [ordered]@{
            ready_latency_us = Get-CellDistribution -Distribution $cell.startup_ready_latency_us
            calibration_duration_us = Get-CellDistribution -Distribution $cell.calibration_duration_us
            startup_wake_error_session_p50_us = Get-CellDistribution -Distribution $cell.startup_wake_error_session_p50_us
            startup_wake_error_session_p95_us = Get-CellDistribution -Distribution $cell.startup_wake_error_session_p95_us
            startup_wake_error_session_p99_us = Get-CellDistribution -Distribution $cell.startup_wake_error_session_p99_us
            startup_wake_error_session_max_us = Get-CellDistribution -Distribution $cell.startup_wake_error_session_max_us
            effective_spin_threshold_us = Get-CellDistribution -Distribution $cell.effective_spin_threshold_us
            worker_cpu_time_us = Get-CellDistribution -Distribution $cell.worker_cpu_time_us
            process_cpu_time_us = Get-CellDistribution -Distribution $cell.process_cpu_time_us
            spin_duty_cycle_ppm = Get-CellDistribution -Distribution $cell.spin_duty_cycle_ppm
        }
        acquired_priority_modes = @($cell.startup_acquired_priority_modes)
        power_guard_acquired_sessions = $cell.startup_power_guard_acquired_runs
        dispatch_metrics = [ordered]@{
            authored_lateness_us = Get-CellDistribution -Distribution $cell.dispatch_timing.authored_lateness_us
            physical_floor_displacement_us = Get-CellDistribution -Distribution $cell.dispatch_timing.physical_floor_displacement_us
            residual_lateness_us = Get-CellDistribution -Distribution $cell.dispatch_timing.residual_lateness_us
            hol_lower_bound_us = Get-CellDistribution -Distribution $cell.dispatch_timing.hol_lower_bound_us
            sender_envelope_us = Get-CellDistribution -Distribution $cell.dispatch_timing.sender_envelope_us
        }
        telemetry = [ordered]@{
            attempted = $cell.telemetry_attempted
            accepted = $cell.telemetry_accepted
            dropped = $cell.telemetry_dropped
            observer_queue_dropped = $cell.observer_queue_dropped
            truncated_sessions = $cell.telemetry_truncated_runs
        }
    })
}
if ($cells.Count -ne 20 -or @($cells | Where-Object outcome -ne 'PASS').Count -ne 0) {
    throw 'Expected 20 complete passing workload/load/placement cells.'
}

$sessionMetricSources = [ordered]@{
    ready_latency_us = 'startup_latency_us'
    calibration_duration_us = 'startup_calibration_duration_us'
    calibration_sample_count = 'startup_calibration_sample_count'
    startup_wake_error_session_p50_us = 'startup_wake_error_p50_us'
    startup_wake_error_session_p95_us = 'startup_wake_error_p95_us'
    startup_wake_error_session_p99_us = 'startup_wake_error_p99_us'
    startup_wake_error_session_max_us = 'startup_wake_error_max_us'
    effective_spin_threshold_us = 'effective_spin_threshold_us'
    worker_cpu_time_us = 'worker_cpu_time_us'
    process_cpu_time_us = 'process_cpu_time_us'
    spin_duty_cycle_ppm = 'spin_duty_cycle_ppm'
}
$globalSessionMetrics = [ordered]@{}
$traceDefinitions = [ordered]@{
    authored_lateness_us = [ordered]@{ value = 'authored_lateness_ticks'; available = 'authored_lateness_available'; signed = $true }
    physical_floor_displacement_us = [ordered]@{ value = 'physical_floor_displacement_ticks'; available = 'floor_displacement_available'; signed = $false }
    residual_lateness_us = [ordered]@{ value = 'residual_lateness_ticks'; available = 'residual_lateness_available'; signed = $true }
    hol_lower_bound_us = [ordered]@{ value = 'hol_delay_lower_bound_ticks'; available = 'hol_delay_available'; signed = $false }
    sender_envelope_us = [ordered]@{ value = 'sendinput_envelope_ticks'; available = 'sendinput_envelope_available'; signed = $false }
}
$globalTraceMetrics = [ordered]@{}
foreach ($placement in @('baseline', 'candidate')) {
    $globalSessionMetrics[$placement] = [ordered]@{}
    foreach ($metricName in $sessionMetricSources.Keys) {
        $globalSessionMetrics[$placement][$metricName] = [ordered]@{
            values = [System.Collections.Generic.List[double]]::new(); missing = 0
        }
    }
    $globalTraceMetrics[$placement] = [ordered]@{}
    foreach ($metricName in $traceDefinitions.Keys) {
        $globalTraceMetrics[$placement][$metricName] = [ordered]@{
            values = [System.Collections.Generic.List[double]]::new(); missing = 0; dropped = 0; truncated = 0
        }
    }
}
$aggregateCounters = [ordered]@{
    calibration_executed_sessions = 0
    calibration_fallback_sessions = 0
    calibration_samples = 0
    telemetry_attempted = 0
    telemetry_accepted = 0
    telemetry_dropped = 0
    observer_queue_dropped = 0
    telemetry_truncated_sessions = 0
    transport_anomalies = 0
    final_release_obligation_mask_sum = 0
    release_failures = 0
    receive_only_events_expected = 0
    receive_only_events_observed = 0
}
foreach ($session in $sessionReports) {
    $placement = $session.placement
    $report = $session.report
    $details = $report.details
    foreach ($metricName in $sessionMetricSources.Keys) {
        $sourceName = $sessionMetricSources[$metricName]
        $value = $details.PSObject.Properties[$sourceName].Value
        if ($null -eq $value) {
            $globalSessionMetrics[$placement][$metricName].missing++
        } else {
            $globalSessionMetrics[$placement][$metricName].values.Add([double]$value)
        }
    }
    if ($details.startup_calibration_executed) {
        $aggregateCounters.calibration_executed_sessions++
        $aggregateCounters.calibration_samples += [long]$details.startup_calibration_sample_count
    } else {
        $aggregateCounters.calibration_fallback_sessions++
    }
    $telemetry = $details.native_timing_telemetry
    $aggregateCounters.telemetry_attempted += [long]$telemetry.attempted
    $aggregateCounters.telemetry_accepted += [long]$telemetry.accepted
    $aggregateCounters.telemetry_dropped += [long]$telemetry.dropped
    $aggregateCounters.observer_queue_dropped += [long]$telemetry.observer_queue_dropped
    if ($telemetry.truncated) { $aggregateCounters.telemetry_truncated_sessions++ }
    $aggregateCounters.transport_anomalies += [long]$details.transport_anomalies
    $aggregateCounters.final_release_obligation_mask_sum += [long]$details.final_release_obligation_mask
    $aggregateCounters.release_failures += [long]$details.failed_release_count
    $aggregateCounters.receive_only_events_expected += [long]$details.expected_sink_event_count
    $aggregateCounters.receive_only_events_observed += [long]$details.observed_sink_event_count
    $frequency = [double]$telemetry.qpc_frequency_hz
    foreach ($metricName in $traceDefinitions.Keys) {
        $bucket = $globalTraceMetrics[$placement][$metricName]
        $bucket.dropped += [long]$telemetry.dropped + [long]$telemetry.observer_queue_dropped
        if ($telemetry.truncated) { $bucket.truncated++ }
        foreach ($record in $telemetry.records) {
            $definition = $traceDefinitions[$metricName]
            if (-not $record.($definition.available) -or $frequency -le 0) {
                $bucket.missing++
                continue
            }
            $ticks = [double]$record.($definition.value)
            $bucket.values.Add(($ticks * 1000000.0) / $frequency)
        }
    }
}
$globalDistributions = [ordered]@{ session = [ordered]@{}; dispatch = [ordered]@{} }
foreach ($placement in @('baseline', 'candidate')) {
    $globalDistributions.session[$placement] = [ordered]@{}
    foreach ($metricName in $sessionMetricSources.Keys) {
        $bucket = $globalSessionMetrics[$placement][$metricName]
        $globalDistributions.session[$placement][$metricName] = Get-Distribution -Values $bucket.values.ToArray() -Missing $bucket.missing
    }
    $globalDistributions.dispatch[$placement] = [ordered]@{}
    foreach ($metricName in $traceDefinitions.Keys) {
        $bucket = $globalTraceMetrics[$placement][$metricName]
        $globalDistributions.dispatch[$placement][$metricName] = Get-Distribution `
            -Values $bucket.values.ToArray() `
            -Missing $bucket.missing `
            -Dropped $bucket.dropped `
            -TruncatedRuns $bucket.truncated
    }
}

$promotionPolicy = @(
    'In both load conditions, paired median p99 residual lateness must improve by at least 10% and 100 us in at least three of five workload groups.',
    'At least five of six pairs in each counted group must improve.',
    'Pooled session p95 startup wake error must improve by at least 10% and 50 us.',
    'Ready p95 increase is capped at 1 ms; no paired Ready increase may exceed 2 ms.',
    'Calibration fallback may increase by at most one percentage point.',
    'Worker and process CPU median increases are capped at 5%; spin duty increase at two percentage points; acquired scheduling guards must not regress.'
)
$evidence = [ordered]@{
    schema = 'sky-auto-player/wake-calibration-qualification-evidence/v1'
    qualification = [ordered]@{
        run_id = [string]$metadata.run_id
        measured_source_sha = [string]$metadata.source_revision
        measured_source_tree_clean = [bool]$metadata.source_tree_clean
        harness_build_command = [string]$metadata.build_command
        harness_build_exit_code = [int]$metadata.build_exit_code
        harness_sha256 = [string]$metadata.harness_sha256
        harness_binary_hash_verified = $true
        rust_version = [string]$metadata.rust_version
        platform = 'Windows 11 x64'
        qpc_frequency_hz = $qpcFrequency
        timing_margin_us = [int]$metadata.timing_margin_us
        sessions_per_cell_and_placement = [int]$metadata.repetitions_per_cell
        workloads = @($metadata.workloads)
        load_conditions = [ordered]@{
            quiet = [string]$metadata.quiet
            controlled_contention = [string]$metadata.contention
        }
        observer = [string]$metadata.observer
    }
    distribution_method = [ordered]@{
        quantile = 'nearest_rank_ceiling'
        paired_residual = 'Compute nearest-rank p99 within each session, then Candidate B minus Baseline A for the pair.'
        qpc_conversion = 'ticks * 1000000 / qpc_frequency_hz'
        low_count_note = 'Per-cell session distributions have n=6; p99.9 equals the maximum and is not a service guarantee.'
    }
    manifest_verification = [ordered]@{
        qualification_manifest_sha256 = $qualificationManifest.sha256
        qualification_inputs_verified = @($qualificationManifest.entries)
        qualification_input_file_count = $qualificationManifest.verified_file_count
        case_manifests_verified = $verifiedCaseCount
        case_artifact_files_verified = $caseFileCount
        raw_reports_and_sink_traces_committed = $false
        exporter_sha256 = Get-Sha256 -Path $PSCommandPath
    }
    outcomes = [ordered]@{
        receive_only_sessions = $runLedger.Count
        passed_sessions = @($runLedger | Where-Object status -eq 'PASS').Count
        baseline_sessions = $placementCounts.baseline
        candidate_sessions = $placementCounts.candidate
        complete_pairs = $pairs.Count
        incomplete_pairs = 60 - $pairs.Count
        pair_orders = @($pairOrderCounts | ForEach-Object { [ordered]@{ order = $_.Name; count = $_.Count } })
        aggregate_counts = $aggregateCounters
    }
    aggregate_distributions = $globalDistributions
    per_cell = $cells.ToArray()
    per_pair = $pairs.ToArray()
    run_order = $runLedger.ToArray()
    promotion_gate = [ordered]@{
        predefined_thresholds = $promotionPolicy
        recommendation = 'retain_baseline_a'
        candidate_qualified = $false
        rationale = @(
            'Ready latency p95 improved, but startup wake-error p95 and aggregate dispatch residual-lateness p99 regressed.',
            'Paired residual p99 changes did not meet the required per-workload improvement magnitude and consistency.',
            'Coarse CPU counters and recorded zero spin duty do not establish the CPU/spin cost gate.'
        )
    }
    measurement_limits = @(
        'Packet timing values use sampled QPC boundaries around the sender; they are not game-side timing.',
        'ReceiveOnly results establish event receipt by the test sink process only.',
        'No Sky note reception or audio onset was measured.',
        'Windows CPU time counters were quantized in 15.625 ms steps; zero spin duty is recorded telemetry, not proof that no spin occurred.',
        'Raw per-session reports and sink traces remain in the ignored local benchmark directory; this committed projection contains sanitized outcomes and their hashes.'
    )
}

$resolvedOutput = Resolve-RepositoryPath -Path $OutputPath
$outputDirectory = Split-Path -Parent $resolvedOutput
if (-not (Test-Path -LiteralPath $outputDirectory -PathType Container)) {
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
}
$json = $evidence | ConvertTo-Json -Depth 24
foreach ($forbidden in @('processor_name', 'processor_cores', 'windows_build', 'power_scheme', 'evidence_directory', 'sink_hwnd', 'sink_pid', 'process_start_time_filetime', 'started_utc', 'ended_utc')) {
    if ($json.Contains($forbidden)) { throw "Sanitized evidence unexpectedly contains $forbidden." }
}
# Custom OutputPath values remain supported; add a matching eol=lf attribute when checkout-time byte identity is required.
$outputHash = Write-StartupCalibrationProjection -SerializedJson $json -OutputPath $resolvedOutput
Write-Output "EVIDENCE $resolvedOutput"
Write-Output "SHA256 $outputHash"
Write-Output "VERIFIED_CASES $verifiedCaseCount"
Write-Output "VERIFIED_CASE_FILES $caseFileCount"
