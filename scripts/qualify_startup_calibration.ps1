param(
    [ValidateRange(0, 3000)]
    [int]$TimingMarginUs = 500,
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location -LiteralPath $root
$caseRunner = Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1'
$harness = Join-Path $root 'rust\target\release\rt-native-acceptance.exe'
$workloads = @(
    'long-single-sequence',
    'canonical-single',
    'mixed-up-down',
    'canonical-max-chord',
    'near-minimum-retrigger'
)
$repetitions = 6
$outputRoot = if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    Join-Path $root '.benchmarks\wake-calibration-qualification'
} elseif ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
}
$runId = 'wake-calibration-' + (Get-Date -Format 'yyyyMMddTHHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$qualificationDirectory = Join-Path $outputRoot $runId
New-Item -ItemType Directory -Force -Path $qualificationDirectory | Out-Null
$buildLog = Join-Path $qualificationDirectory 'release-build.log'
$runOrderPath = Join-Path $qualificationDirectory 'run-order.json'
$summaryPath = Join-Path $qualificationDirectory 'summary.json'

function Get-Sha256 {
    param([string]$Path)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        [System.BitConverter]::ToString($sha.ComputeHash([System.IO.File]::ReadAllBytes($Path))).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Get-Distribution {
    param(
        [double[]]$Values,
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

function Get-TraceDistribution {
    param(
        [object[]]$Runs,
        [string]$ValueProperty,
        [string]$AvailabilityProperty,
        [switch]$Signed
    )
    $values = [System.Collections.Generic.List[double]]::new()
    $missing = 0L
    $dropped = 0L
    $truncated = 0L
    foreach ($run in $Runs) {
        $telemetry = $run.report.details.native_timing_telemetry
        if ($null -eq $telemetry) { $missing++; continue }
        $dropped += [long]$telemetry.dropped + [long]$telemetry.observer_queue_dropped
        if ($telemetry.truncated) { $truncated++ }
        $frequency = [double]$telemetry.qpc_frequency_hz
        foreach ($record in $telemetry.records) {
            if (-not $record.$AvailabilityProperty) { $missing++; continue }
            if ($frequency -le 0) { $missing++; continue }
            $ticks = [double]$record.$ValueProperty
            if ($Signed) {
                $values.Add(($ticks * 1000000.0) / $frequency)
            } else {
                $values.Add(($ticks * 1000000.0) / $frequency)
            }
        }
    }
    Get-Distribution -Values $values.ToArray() -Missing $missing -Dropped $dropped -TruncatedRuns $truncated
}

function Get-SessionDistribution {
    param([object[]]$Runs, [string]$Property)
    $values = [System.Collections.Generic.List[double]]::new()
    $missing = 0L
    foreach ($run in $Runs) {
        $value = $run.report.details.$Property
        if ($null -eq $value) { $missing++; continue }
        $values.Add([double]$value)
    }
    Get-Distribution -Values $values.ToArray() -Missing $missing
}

function Get-PairedP99Residual {
    param([object[]]$Runs)
    $pairRows = @()
    foreach ($pair in ($Runs | Group-Object pair_id)) {
        $baseline = @($pair.Group | Where-Object placement -eq 'baseline') | Select-Object -First 1
        $candidate = @($pair.Group | Where-Object placement -eq 'candidate') | Select-Object -First 1
        if ($null -eq $baseline -or $null -eq $candidate) { continue }
        $baselineValues = @()
        $candidateValues = @()
        foreach ($row in $baseline.report.details.native_timing_telemetry.records) {
            if ($row.residual_lateness_available) {
                $baselineValues += ([double]$row.residual_lateness_ticks * 1000000.0) / [double]$baseline.report.details.native_timing_telemetry.qpc_frequency_hz
            }
        }
        foreach ($row in $candidate.report.details.native_timing_telemetry.records) {
            if ($row.residual_lateness_available) {
                $candidateValues += ([double]$row.residual_lateness_ticks * 1000000.0) / [double]$candidate.report.details.native_timing_telemetry.qpc_frequency_hz
            }
        }
        if ($baselineValues.Count -eq 0 -or $candidateValues.Count -eq 0) { continue }
        $baselineP99 = (Get-Distribution -Values $baselineValues).p99
        $candidateP99 = (Get-Distribution -Values $candidateValues).p99
        $pairRows += [pscustomobject]@{
            pair_id = $pair.Name
            baseline_p99_residual_us = $baselineP99
            candidate_p99_residual_us = $candidateP99
            candidate_delta_us = $candidateP99 - $baselineP99
            candidate_improved = $candidateP99 -lt $baselineP99
            baseline_samples = $baselineValues.Count
            candidate_samples = $candidateValues.Count
        }
    }
    $deltas = @($pairRows | ForEach-Object { [double]$_.candidate_delta_us })
    [ordered]@{
        pairs = $pairRows
        median_candidate_delta_us = (Get-Distribution -Values $deltas).p50
        improving_pairs = @($pairRows | Where-Object candidate_improved).Count
        complete_pairs = $pairRows.Count
    }
}

function Get-PairedMetric {
    param([object[]]$Runs, [string]$Property)
    $pairRows = @()
    foreach ($pair in ($Runs | Group-Object pair_id)) {
        $baseline = @($pair.Group | Where-Object { $_.placement -eq 'baseline' }) | Select-Object -First 1
        $candidate = @($pair.Group | Where-Object { $_.placement -eq 'candidate' }) | Select-Object -First 1
        if ($null -eq $baseline -or $null -eq $candidate) { continue }
        $baselineValue = $baseline.report.details.$Property
        $candidateValue = $candidate.report.details.$Property
        if ($null -eq $baselineValue -or $null -eq $candidateValue) { continue }
        $pairRows += [pscustomobject]@{
            pair_id = $pair.Name
            baseline = [double]$baselineValue
            candidate = [double]$candidateValue
            candidate_delta = [double]$candidateValue - [double]$baselineValue
        }
    }
    $deltas = @($pairRows | ForEach-Object { [double]$_.candidate_delta })
    [ordered]@{
        pairs = $pairRows
        candidate_delta_us = Get-Distribution -Values $deltas
        complete_pairs = $pairRows.Count
    }
}

$buildCommand = 'cargo build --locked --manifest-path rust/Cargo.toml -p sky_player --features real-input-acceptance --bin rt-native-acceptance --release'
$head = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Could not read the source HEAD.' }
$trackedChanges = (& git -C $root status --porcelain --untracked-files=no) -join ''
if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace($trackedChanges)) {
    throw 'Commit the source before qualification so every paired run has one clean source identity.'
}
$buildOutput = @(& cargo build --locked --manifest-path (Join-Path $root 'rust\Cargo.toml') -p sky_player --features real-input-acceptance --bin rt-native-acceptance --release 2>&1)
$buildExit = $LASTEXITCODE
($buildOutput | ForEach-Object { [string]$_ }) -join [Environment]::NewLine | Set-Content -LiteralPath $buildLog -Encoding UTF8
if ($buildExit -ne 0 -or -not (Test-Path -LiteralPath $harness)) {
    throw "Release harness build failed with exit code $buildExit. See $buildLog"
}

$os = Get-CimInstance Win32_OperatingSystem
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$binaryHash = Get-Sha256 $harness
$runnerHash = Get-Sha256 $caseRunner
$rustVersion = (& rustc --version).Trim()
$powerScheme = (& powercfg /getactivescheme | Out-String).Trim()
$metadata = [ordered]@{
    run_id = $runId
    source_revision = $head
    source_tree_clean = $true
    build_command = $buildCommand
    build_exit_code = $buildExit
    harness_sha256 = $binaryHash
    case_runner_sha256 = $runnerHash
    qualification_runner_sha256 = Get-Sha256 $PSCommandPath
    rust_version = $rustVersion
    windows_caption = $os.Caption
    windows_version = $os.Version
    windows_build = $os.BuildNumber
    processor_name = $cpu.Name
    processor_cores = $cpu.NumberOfCores
    processor_logical_processors = $cpu.NumberOfLogicalProcessors
    qpc_frequency_source = 'native timing telemetry in each session report'
    power_scheme = $powerScheme
    timing_margin_us = $TimingMarginUs
    repetitions_per_cell = $repetitions
    order = 'alternating AB and BA per workload/load cell'
    quiet = 'no qualification contention workers requested'
    contention = 'two hidden pwsh workers at BelowNormal priority'
    workloads = $workloads
    raw_artifacts = 'per-session directories in .benchmarks/physical-native-cases'
    promotion_thresholds = 'docs/rt-dispatch-timing-evidence.md#startup-calibration-placement-qualification'
}
$metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $qualificationDirectory 'qualification.json') -Encoding UTF8

$runRows = [System.Collections.Generic.List[object]]::new()
$workloadIndex = 0
foreach ($scenario in $workloads) {
    foreach ($contention in @($false, $true)) {
        for ($pair = 1; $pair -le $repetitions; $pair++) {
            $pairId = '{0}-{1}-{2:D2}' -f $scenario, $(if ($contention) { 'contention' } else { 'quiet' }), $pair
            if (($pair + $workloadIndex) % 2 -eq 1) {
                $placements = @('baseline', 'candidate')
            } else {
                $placements = @('candidate', 'baseline')
            }
            foreach ($placement in $placements) {
                $caseArguments = @(
                    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $caseRunner,
                    '-Scenario', $scenario,
                    '-TimingMarginUs', [string]$TimingMarginUs,
                    '-CalibrationPlacement', (Get-Culture).TextInfo.ToTitleCase($placement)
                )
                if ($contention) { $caseArguments += '-CpuContention' }
                $started = [DateTimeOffset]::UtcNow.ToString('O')
                $caseOutput = @(& (Get-Command pwsh.exe).Source @caseArguments 2>&1)
                $caseExit = $LASTEXITCODE
                $ended = [DateTimeOffset]::UtcNow.ToString('O')
                $caseText = ($caseOutput | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
                Write-Output $caseText
                $evidenceLine = @($caseOutput | ForEach-Object { [string]$_ } | Where-Object { $_ -like 'EVIDENCE *' }) | Select-Object -Last 1
                if (-not $evidenceLine) { throw "Case runner omitted its evidence path for $pairId/$placement." }
                $caseDirectory = $evidenceLine.Substring('EVIDENCE '.Length).Trim()
                $reportPath = Join-Path $caseDirectory 'rt-native-acceptance.jsonl'
                if (-not (Test-Path -LiteralPath $reportPath)) { throw "Missing native report: $reportPath" }
                $report = (Get-Content -LiteralPath $reportPath -Raw).Trim() | ConvertFrom-Json
                $row = [pscustomobject]@{
                    pair_id = $pairId
                    scenario = $scenario
                    contention = [bool]$contention
                    placement = $placement
                    started_utc = $started
                    ended_utc = $ended
                    case_exit_code = $caseExit
                    status = $report.status
                    reason = $report.reason
                    evidence_directory = $caseDirectory
                    report = $report
                }
                $runRows.Add($row)
                [ordered]@{
                    pair_id = $pairId
                    scenario = $scenario
                    contention = [bool]$contention
                    placement = $placement
                    started_utc = $started
                    ended_utc = $ended
                    case_exit_code = $caseExit
                    status = $report.status
                    reason = $report.reason
                    evidence_directory = $caseDirectory
                } | ConvertTo-Json -Depth 5 -Compress | Add-Content -LiteralPath (Join-Path $qualificationDirectory 'run-order.jsonl') -Encoding UTF8
                [ordered]@{
                    qualification_run_id = $runId
                    pair_id = $pairId
                    scenario = $scenario
                    contention = [bool]$contention
                    placement = $placement
                    run_order = $runRows.Count
                } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $caseDirectory 'paired-run.json') -Encoding UTF8
                $caseManifest = Get-ChildItem -LiteralPath $caseDirectory -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name | ForEach-Object {
                    (Get-Sha256 $_.FullName) + '  ' + $_.Name
                }
                Set-Content -LiteralPath (Join-Path $caseDirectory 'SHA256SUMS.txt') -Value $caseManifest -Encoding UTF8
                if ($caseExit -ne 0 -or $report.status -ne 'PASS') {
                    throw "Acceptance run failed for $pairId/$placement with exit $caseExit ($($report.status)). Evidence: $caseDirectory"
                }
            }
        }
        $workloadIndex++
    }
}

$cellSummaries = @()
foreach ($cell in ($runRows | Group-Object scenario, contention, placement)) {
    $sample = $cell.Group[0]
    $runs = @($cell.Group)
    $traceMetrics = [ordered]@{
        authored_lateness_us = Get-TraceDistribution -Runs $runs -ValueProperty 'authored_lateness_ticks' -AvailabilityProperty 'authored_lateness_available' -Signed
        physical_floor_displacement_us = Get-TraceDistribution -Runs $runs -ValueProperty 'physical_floor_displacement_ticks' -AvailabilityProperty 'floor_displacement_available'
        residual_lateness_us = Get-TraceDistribution -Runs $runs -ValueProperty 'residual_lateness_ticks' -AvailabilityProperty 'residual_lateness_available' -Signed
        hol_lower_bound_us = Get-TraceDistribution -Runs $runs -ValueProperty 'hol_delay_lower_bound_ticks' -AvailabilityProperty 'hol_delay_available'
        sender_envelope_us = Get-TraceDistribution -Runs $runs -ValueProperty 'sendinput_envelope_ticks' -AvailabilityProperty 'sendinput_envelope_available'
    }
    $calibrationRuns = @($runs | Where-Object { $_.report.details.startup_calibration_executed })
    $cellSummaries += [ordered]@{
        scenario = $sample.scenario
        contention = $sample.contention
        placement = $sample.placement
        run_count = $runs.Count
        startup_ready_latency_us = Get-SessionDistribution -Runs $runs -Property 'startup_latency_us'
        calibration_duration_us = Get-SessionDistribution -Runs $runs -Property 'startup_calibration_duration_us'
        calibration_sample_count = Get-SessionDistribution -Runs $runs -Property 'startup_calibration_sample_count'
        calibration_executed_runs = $calibrationRuns.Count
        calibration_fallback_runs = $runs.Count - $calibrationRuns.Count
        calibration_execution_rate = if ($runs.Count -gt 0) { [double]$calibrationRuns.Count / $runs.Count } else { $null }
        calibration_fallback_rate = if ($runs.Count -gt 0) { [double]($runs.Count - $calibrationRuns.Count) / $runs.Count } else { $null }
        calibration_fallback_reasons = @($runs | Group-Object { $_.report.details.startup_calibration_result } | ForEach-Object { [ordered]@{ reason = $_.Name; count = $_.Count } })
        startup_wake_error_session_p50_us = Get-SessionDistribution -Runs $calibrationRuns -Property 'startup_wake_error_p50_us'
        startup_wake_error_session_p95_us = Get-SessionDistribution -Runs $calibrationRuns -Property 'startup_wake_error_p95_us'
        startup_wake_error_session_p99_us = Get-SessionDistribution -Runs $calibrationRuns -Property 'startup_wake_error_p99_us'
        startup_wake_error_session_max_us = Get-SessionDistribution -Runs $calibrationRuns -Property 'startup_wake_error_max_us'
        startup_wake_error_session_robust_us = Get-SessionDistribution -Runs $calibrationRuns -Property 'startup_wake_error_robust_us'
        effective_spin_threshold_us = Get-SessionDistribution -Runs $runs -Property 'effective_spin_threshold_us'
        worker_cpu_time_us = Get-SessionDistribution -Runs $runs -Property 'worker_cpu_time_us'
        process_cpu_time_us = Get-SessionDistribution -Runs $runs -Property 'process_cpu_time_us'
        spin_duty_cycle_ppm = Get-SessionDistribution -Runs $runs -Property 'spin_duty_cycle_ppm'
        acquired_priority_modes = @($runs | Group-Object { $_.report.details.rt_priority_acquired } | ForEach-Object { [ordered]@{ mode = $_.Name; count = $_.Count } })
        power_guard_active_runs = @($runs | Where-Object { $_.report.details.power_throttling_disabled }).Count
        dispatch_timing = $traceMetrics
        telemetry_attempted = (@($runs | ForEach-Object { [long]$_.report.details.native_timing_telemetry.attempted }) | Measure-Object -Sum).Sum
        telemetry_accepted = (@($runs | ForEach-Object { [long]$_.report.details.native_timing_telemetry.accepted }) | Measure-Object -Sum).Sum
        telemetry_dropped = (@($runs | ForEach-Object { [long]$_.report.details.native_timing_telemetry.dropped }) | Measure-Object -Sum).Sum
        observer_queue_dropped = (@($runs | ForEach-Object { [long]$_.report.details.native_timing_telemetry.observer_queue_dropped }) | Measure-Object -Sum).Sum
        telemetry_truncated_runs = @($runs | Where-Object { $_.report.details.native_timing_telemetry.truncated }).Count
    }
}

$pairedComparisons = @()
foreach ($cell in ($runRows | Group-Object scenario, contention)) {
    $sample = $cell.Group[0]
    $pairedComparisons += [ordered]@{
        scenario = $sample.scenario
        contention = $sample.contention
        residual_lateness = Get-PairedP99Residual -Runs @($cell.Group)
        startup_ready_latency = Get-PairedMetric -Runs @($cell.Group) -Property 'startup_latency_us'
        startup_wake_error_session_p95 = Get-PairedMetric -Runs @($cell.Group | Where-Object { $_.report.details.startup_calibration_executed }) -Property 'startup_wake_error_p95_us'
    }
}

$allPassed = @($runRows | Where-Object { $_.case_exit_code -ne 0 -or $_.status -ne 'PASS' }).Count -eq 0
$summary = [ordered]@{
    qualification = $metadata
    qualification_directory = $qualificationDirectory
    all_receive_only_acceptance_runs_passed = $allPassed
    comparison_method = 'paired per-run empirical nearest-rank quantiles; alternating run order'
    cells = $cellSummaries
    paired_comparisons = $pairedComparisons
    recommendation = 'not evaluated automatically; apply the predeclared gate and retain Baseline A if any required evidence is noisy, missing, or regressed'
}
$summary | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $summaryPath -Encoding UTF8

$manifest = Get-ChildItem -LiteralPath $qualificationDirectory -File | Where-Object Name -notin @('SHA256SUMS.txt') | Sort-Object Name | ForEach-Object {
    (Get-Sha256 $_.FullName) + '  ' + $_.Name
}
Set-Content -LiteralPath (Join-Path $qualificationDirectory 'SHA256SUMS.txt') -Value $manifest -Encoding UTF8
Write-Output "QUALIFICATION $runId"
Write-Output "SOURCE $head"
Write-Output "HARNESS_SHA256 $binaryHash"
Write-Output "SUMMARY $summaryPath"
exit 0
