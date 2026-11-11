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

$requiredFunctions = @(
    'Get-OptionalProperty',
    'Get-Percentiles',
    'Get-DeadlineCrossingToPreCallSample',
    'Get-TimingMetrics',
    'Invoke-NativeCaseFailStopSequence'
)
foreach ($name in $requiredFunctions) {
    $functionAst = $ast.Find({
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true)
    if ($null -eq $functionAst) { throw "missing runner function: $name" }
    Invoke-Expression $functionAst.Extent.Text
}

$runnerSource = Get-Content -LiteralPath $runnerPath -Raw
if ([regex]::Matches($runnerSource, 'Invoke-NativeCaseFailStopSequence').Count -lt 3) {
    throw 'Smoke and Final must both use the tested fail-stop sequence helper'
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

Write-Output 'PASS: PowerShell parser, Smoke/Final fail-stop, and timestamp provenance contracts'
