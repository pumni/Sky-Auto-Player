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

foreach ($scriptName in @('build_dispatch_core_harness.ps1', 'run_native_acceptance_case.ps1')) {
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

$requiredFunctions = @(
    'Get-OptionalProperty',
    'Get-Percentiles',
    'Get-MetricPath',
    'Get-ComparisonDescription',
    'Get-PairedMetrics',
    'Get-DeadlineCrossingToPreCallSample',
    'Get-TimingMetrics',
    'Invoke-NativeCaseFailStopSequence',
    'Assert-SchedulerManifestIdentity'
)
foreach ($name in $requiredFunctions) {
    $functionAst = $ast.Find({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true)
    if ($null -eq $functionAst) { throw "missing runner function: $name" }
    Invoke-Expression $functionAst.Extent.Text
}
Import-Module (Join-Path $PSScriptRoot 'dispatch_pair_screen.psm1') -Force

$runnerSource = Get-Content -LiteralPath $runnerPath -Raw
if ([regex]::Matches($runnerSource, 'Invoke-NativeCaseFailStopSequence').Count -lt 3) {
    throw 'Smoke and Final must both use the tested fail-stop sequence helper'
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
        role = 'control'; status = 'PASS'
        timing_metrics = [pscustomobject]@{
            metrics = [pscustomobject]@{ authored_lateness_per_chord_us = $controlMetric }
        }
    }
    $runRows += [pscustomobject]@{
        cell_id = 'Dispatch/Quiet'; pair_id = ('Dispatch/Quiet/{0:D2}' -f ($index + 1))
        pair_order = if ($index % 2 -eq 0) { 'AB' } else { 'BA' }
        role = 'candidate'; status = 'PASS'
        timing_metrics = [pscustomobject]@{
            metrics = [pscustomobject]@{ authored_lateness_per_chord_us = $candidateMetric }
        }
    }
}
$pairedMetrics = Get-PairedMetrics
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
