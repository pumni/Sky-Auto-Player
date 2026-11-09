param(
    [Parameter(Mandatory)]
    [ValidateSet('Smoke', 'Compare', 'Final')]
    [string]$Stage,
    [Parameter(Mandatory)]
    [string]$CandidateManifest,
    [string]$ControlManifest,
    [ValidateRange(6, 6)]
    [int]$Pairs = 6,
    [ValidateRange(15, 15)]
    [int]$SampleSeconds = 15,
    [ValidateSet(0, 500)]
    [int]$TimingMarginUs = 500,
    [Parameter(Mandatory)]
    [string]$OutputRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$baseRevision = '843cc0c6a1392dfff0ca4b017551bd1ffb180579'
$expectedBranch = 'perf/dispatch-core-20261109'
$caseRunner = Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1'
$manifestBuilder = Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1'
$candidateManifestPath = [System.IO.Path]::GetFullPath($CandidateManifest)
$controlManifestPath = if ($ControlManifest) { [System.IO.Path]::GetFullPath($ControlManifest) } else { $null }
$evidenceRoot = [System.IO.Path]::GetFullPath($OutputRoot)
$zipPath = $evidenceRoot.TrimEnd('\') + '.zip'
$runId = 'dispatch-core-' + $Stage.ToLowerInvariant() + '-' + (Get-Date -Format 'yyyyMMddTHHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$sessionRoot = Join-Path $evidenceRoot 'sessions'
$runRows = [System.Collections.Generic.List[object]]::new()
$failureRows = [System.Collections.Generic.List[object]]::new()
$commandRows = [System.Collections.Generic.List[string]]::new()
$candidate = $null
$control = $null
$head = $null
$tree = $null
$exitCode = 0

function Get-Sha256 {
    param([string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-OutsideRepository {
    param([string]$Path, [string]$Label)
    $full = [System.IO.Path]::GetFullPath($Path)
    $rootFull = [System.IO.Path]::GetFullPath($root).TrimEnd('\')
    $rootPrefix = $rootFull + '\'
    if ([string]::Equals($full.TrimEnd('\'), $rootFull, [System.StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw ($Label + ' must be outside the source checkout')
    }
}

function Assert-AbsolutePath {
    param([string]$Path, [string]$Label)
    if (-not [System.IO.Path]::IsPathRooted($Path)) { throw ($Label + ' must be an absolute path') }
}

function Get-CurrentSourceIdentity {
    $branch = (& git -C $root branch --show-current).Trim()
    if ($LASTEXITCODE -ne 0 -or $branch -ne $expectedBranch) { throw ('expected branch ' + $expectedBranch + ', got ' + $branch) }
    $script:head = (& git -C $root rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve current source revision' }
    $script:tree = (& git -C $root rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve current source tree' }
    $changes = (& git -C $root status --porcelain --untracked-files=all) -join ''
    if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace($changes)) { throw 'qualification requires a clean source checkout' }
    & git -C $root merge-base --is-ancestor $baseRevision HEAD
    if ($LASTEXITCODE -ne 0) { throw ('current revision is not descended from ' + $baseRevision) }
    [ordered]@{ branch = $branch; revision = $script:head; tree = $script:tree; clean = $true }
}

function Read-And-VerifyManifest {
    param([string]$Path, [string]$Role)
    Assert-AbsolutePath $Path ($Role + ' manifest')
    Assert-OutsideRepository $Path ($Role + ' manifest')
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw ($Role + ' manifest does not exist: ' + $Path) }
    $manifest = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or
        $manifest.source_revision -notmatch '^[0-9a-f]{40}$' -or
        $manifest.source_tree_sha -notmatch '^[0-9a-f]{40}$' -or
        $manifest.source_branch -ne $expectedBranch -or
        -not [bool]$manifest.source_tree_clean -or
        -not [bool]$manifest.clean_build -or
        $manifest.cargo_profile -ne 'dist' -or
        @($manifest.cargo_features).Count -ne 1 -or
        @($manifest.cargo_features)[0] -ne 'real-input-acceptance' -or
        [bool]$manifest.qpc_read_counting -or
        $manifest.observer_mode -ne 'qualification-on' -or
        $manifest.guard_mode -ne 'frozen' -or
        $manifest.workload_version -ne 'dispatch-core-646-a1' -or
        $manifest.target -ne 'x86_64-pc-windows-msvc' -or
        $manifest.cargo_clean_exit_code -ne 0 -or
        $manifest.cargo_build_exit_code -ne 0 -or
        $manifest.binary_name -ne 'rt-native-acceptance.exe') {
        throw ($Role + ' manifest profile, provenance, build, or feature identity is invalid')
    }
    $binary = [System.IO.Path]::GetFullPath([string]$manifest.binary_path)
    Assert-OutsideRepository $binary ($Role + ' immutable harness')
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf) -or (Get-Sha256 $binary) -ne $manifest.binary_sha256) {
        throw ($Role + ' immutable harness binary is missing or its SHA-256 does not match')
    }
    $manifest | Add-Member -NotePropertyName manifest_path -NotePropertyValue $Path -Force
    $manifest | Add-Member -NotePropertyName manifest_sha256 -NotePropertyValue (Get-Sha256 $Path) -Force
    $manifest | Add-Member -NotePropertyName resolved_binary_path -NotePropertyValue $binary -Force
    $manifest
}

function Get-Percentiles {
    param([double[]]$Values)
    $ordered = @($Values | Sort-Object)
    $count = $ordered.Count
    $quantile = {
        param([double]$Probability)
        if ($count -eq 0) { return $null }
        $index = [Math]::Max(0, [int][Math]::Ceiling($Probability * $count) - 1)
        [double]$ordered[$index]
    }
    [ordered]@{
        p50 = & $quantile 0.50
        p95 = & $quantile 0.95
        p99 = & $quantile 0.99
        max = if ($count -gt 0) { [double]$ordered[-1] } else { $null }
        sample_count = $count
        quantile = 'nearest_rank_ceiling'
    }
}

function Get-TimingMetrics {
    param($Report)
    $telemetry = $Report.details.native_timing_telemetry
    if ($null -eq $telemetry) { return $null }
    $frequency = [double]$telemetry.qpc_frequency_hz
    if ($frequency -le 0) { return $null }
    $authored = [System.Collections.Generic.List[double]]::new()
    $downAuthored = [System.Collections.Generic.List[double]]::new()
    $upAuthored = [System.Collections.Generic.List[double]]::new()
    $downChordEnvelope = [System.Collections.Generic.List[double]]::new()
    $allDownEnvelope = [System.Collections.Generic.List[double]]::new()
    $allUpEnvelope = [System.Collections.Generic.List[double]]::new()
    $timerArmToReturn = [System.Collections.Generic.List[double]]::new()
    $eligibilityToWaiterReturn = [System.Collections.Generic.List[double]]::new()
    foreach ($record in @($telemetry.records)) {
        if ([bool]$record.authored_lateness_available) {
            $value = ([double]$record.authored_lateness_ticks * 1000000.0) / $frequency
            $authored.Add($value)
            if ([int]$record.kind -eq 0) { $downAuthored.Add($value) }
            if ([int]$record.kind -eq 1) { $upAuthored.Add($value) }
        }
        if ([bool]$record.sendinput_envelope_available) {
            $envelope = ([double]$record.sendinput_envelope_ticks * 1000000.0) / $frequency
            if ([int]$record.kind -eq 0) {
                $allDownEnvelope.Add($envelope)
                $mask = [int]$record.down_mask
                if ($mask -ne 0 -and (($mask -band ($mask - 1)) -ne 0)) { $downChordEnvelope.Add($envelope) }
            }
            if ([int]$record.kind -eq 1) { $allUpEnvelope.Add($envelope) }
        }
        if ([bool]$record.timer_arm_target_qpc_available -and [bool]$record.timer_return_qpc_available) {
            $delta = [double]$record.timer_return_qpc_ticks - [double]$record.timer_arm_target_qpc_ticks
            if ($delta -ge 0) { $timerArmToReturn.Add(($delta * 1000000.0) / $frequency) }
        }
        if ([bool]$record.physical_eligibility_qpc_available -and [bool]$record.waiter_return_qpc_available) {
            $delta = [double]$record.waiter_return_qpc_ticks - [double]$record.physical_eligibility_qpc_ticks
            if ($delta -ge 0) { $eligibilityToWaiterReturn.Add(($delta * 1000000.0) / $frequency) }
        }
    }
    [ordered]@{
        authored_lateness_us = Get-Percentiles $authored.ToArray()
        down_authored_lateness_us = Get-Percentiles $downAuthored.ToArray()
        up_authored_lateness_us = Get-Percentiles $upAuthored.ToArray()
        down_chord_send_envelope_us = Get-Percentiles $downChordEnvelope.ToArray()
        down_send_envelope_us = Get-Percentiles $allDownEnvelope.ToArray()
        up_send_envelope_us = Get-Percentiles $allUpEnvelope.ToArray()
        timer_arm_to_return_us = Get-Percentiles $timerArmToReturn.ToArray()
        physical_eligibility_to_waiter_return_us = Get-Percentiles $eligibilityToWaiterReturn.ToArray()
        telemetry_records = @($telemetry.records).Count
        telemetry_accepted = [long]$telemetry.accepted
        telemetry_dropped = [long]$telemetry.dropped
        observer_queue_dropped = [long]$telemetry.observer_queue_dropped
        telemetry_truncated = [bool]$telemetry.truncated
        qpc_frequency_hz = [long]$frequency
    }
}

function Invoke-NativeCase {
    param(
        [string]$Scenario,
        [string]$Workload,
        [string]$Role,
        $HarnessManifest,
        [string]$Load,
        [string]$CellId,
        [string]$PairId,
        [string]$Order,
        [int]$Margin,
        [int]$Fps,
        [int]$Sample,
        [bool]$Historical
    )
    if ((Get-Sha256 $HarnessManifest.resolved_binary_path) -ne $HarnessManifest.binary_sha256 -or
        (Get-Sha256 $HarnessManifest.manifest_path) -ne $HarnessManifest.manifest_sha256) {
        throw ($Role + ' immutable manifest/binary changed before ' + $Scenario)
    }
    $args = @(
        '-NoProfile', '-File', $caseRunner,
        '-Scenario', $Scenario,
        '-TimingMarginUs', [string]$Margin,
        '-CalibrationPlacement', 'Baseline',
        '-TimingObserverForQualification',
        '-HarnessPath', $HarnessManifest.resolved_binary_path,
        '-HarnessManifestPath', $HarnessManifest.manifest_path,
        '-OutputRoot', $sessionRoot,
        '-GameFps', [string]$Fps
    )
    if ($Sample -gt 0) { $args += @('-SampleSeconds', [string]$Sample) }
    if ($Load -eq 'Contention') { $args += '-CpuContention' }
    if ($Historical) { $args += '-HistoricalSourceAllowed' }
    $commandText = 'pwsh -NoProfile -File scripts/run_native_acceptance_case.ps1 ' + (($args | ForEach-Object { '"' + ([string]$_).Replace('"', '\"') + '"' }) -join ' ')
    $script:commandRows.Add($commandText)
    $started = [DateTimeOffset]::UtcNow.ToString('O')
    $output = @(& (Get-Command pwsh.exe).Source @args 2>&1)
    $caseExit = $LASTEXITCODE
    $ended = [DateTimeOffset]::UtcNow.ToString('O')
    $text = ($output | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
    $evidenceLine = @($output | ForEach-Object { [string]$_ } | Where-Object { $_ -like 'EVIDENCE *' }) | Select-Object -Last 1
    $caseDirectory = if ($evidenceLine) { $evidenceLine.Substring('EVIDENCE '.Length).Trim() } else { $null }
    $report = $null
    $reportPath = if ($caseDirectory) { Join-Path $caseDirectory 'rt-native-acceptance.jsonl' } else { $null }
    if ($reportPath -and (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        try { $report = (Get-Content -LiteralPath $reportPath -Raw).Trim() | ConvertFrom-Json } catch { $report = $null }
    }
    $status = if ($null -ne $report) { [string]$report.status } elseif ($caseExit -eq 2) { 'RUNNER_FAILURE' } else { 'MISSING_REPORT' }
    $reason = if ($null -ne $report) { [string]$report.reason } elseif ($text) { $text } else { 'native runner produced no report or failure text' }
    $metrics = if ($null -ne $report) { Get-TimingMetrics $report } else { $null }
    $details = if ($null -ne $report) { $report.details } else { $null }
    $row = [pscustomobject]@{
        run_id = if ($null -ne $report) { [string]$report.run_id } else { $null }
        stage = $Stage
        cell_id = $CellId
        pair_id = $PairId
        pair_order = $Order
        role = $Role
        source_revision = $HarnessManifest.source_revision
        source_tree_sha = $HarnessManifest.source_tree_sha
        harness_sha256 = $HarnessManifest.binary_sha256
        profile = $HarnessManifest.cargo_profile
        features = (@($HarnessManifest.cargo_features) -join ',')
        observer_mode = $HarnessManifest.observer_mode
        guard_mode = $HarnessManifest.guard_mode
        workload = $Workload
        scenario = $Scenario
        load = $Load
        game_fps = $Fps
        timing_margin_us = $Margin
        sample_seconds = if ($Sample -gt 0) { $Sample } else { $null }
        started_utc = $started
        ended_utc = $ended
        case_exit_code = $caseExit
        status = $status
        reason = $reason
        evidence_directory = $caseDirectory
        report_path = $reportPath
        expected_sink_event_count = if ($null -ne $details) { $details.expected_sink_event_count } else { $null }
        observed_sink_event_count = if ($null -ne $details) { $details.observed_sink_event_count } else { $null }
        sink_window_match = if ($null -ne $details) { $details.sink_event_window_matches_report } else { $null }
        active_count = if ($null -ne $details) { $details.active_count } else { $null }
        possibly_active_count = if ($null -ne $details) { $details.possibly_active_count } else { $null }
        final_release_obligation_mask = if ($null -ne $details) { $details.final_release_obligation_mask } else { $null }
        production_hold_floor_violation_count = if ($null -ne $details) { $details.production_hold_floor_violation_count } else { $null }
        production_release_floor_violation_count = if ($null -ne $details) { $details.production_release_floor_violation_count } else { $null }
        production_forensics_anomaly_count = if ($null -ne $details) { $details.production_forensics_anomaly_count } else { $null }
        production_structural_anomaly_count = if ($null -ne $details) { $details.production_structural_anomaly_count } else { $null }
        production_same_key_overlap_forensics_count = if ($null -ne $details) { $details.production_same_key_overlap_forensics_count } else { $null }
        production_anchor_overwrite_count = if ($null -ne $details) { $details.production_anchor_overwrite_count } else { $null }
        production_unmatched_up_count = if ($null -ne $details) { $details.production_unmatched_up_count } else { $null }
        missed_down_boundaries = if ($null -ne $details) { $details.missed_down_boundaries } else { $null }
        missed_down_keys = if ($null -ne $details) { $details.missed_down_keys } else { $null }
        missed_physical_window_boundaries = if ($null -ne $details) { $details.missed_physical_window_boundaries } else { $null }
        keys_dropped = if ($null -ne $details) { $details.keys_dropped } else { $null }
        chord_split_events = if ($null -ne $details) { $details.chord_split_events } else { $null }
        sendinput_partial_events = if ($null -ne $details) { $details.sendinput_partial_events } else { $null }
        sendinput_zero_progress_failures = if ($null -ne $details) { $details.sendinput_zero_progress_failures } else { $null }
        worker_cpu_time_us = if ($null -ne $details) { $details.worker_cpu_time_us } else { $null }
        process_cpu_time_us = if ($null -ne $details) { $details.process_cpu_time_us } else { $null }
        spin_time_us = if ($null -ne $details) { $details.spin_time_us } else { $null }
        spin_time_available = if ($null -ne $details) { $details.spin_time_available } else { $null }
        spin_time_complete = if ($null -ne $details) { $details.spin_time_complete } else { $null }
        spin_duty_cycle_ppm = if ($null -ne $details) { $details.spin_duty_cycle_ppm } else { $null }
        spin_expected_samples = if ($null -ne $details) { $details.spin_observation_expected_samples } else { $null }
        spin_dropped_samples = if ($null -ne $details) { $details.spin_observation_dropped_samples } else { $null }
        spin_missing_samples = if ($null -ne $details) { $details.spin_observation_missing_samples } else { $null }
        timing_metrics = $metrics
        report = $report
        runner_output = $text
    }
    $script:runRows.Add($row)
    if ($caseExit -ne 0 -or $status -ne 'PASS') {
        $script:failureRows.Add([pscustomobject]@{ scenario = $Scenario; workload = $Workload; role = $Role; cell_id = $CellId; pair_id = $PairId; exit_code = $caseExit; status = $status; reason = $reason; evidence_directory = $caseDirectory })
        if ($caseExit -eq 2 -or $status -eq 'RUNNER_FAILURE' -or $status -eq 'MISSING_REPORT') { $script:exitCode = 2 } else { $script:exitCode = [Math]::Max($script:exitCode, 1) }
    }
    Write-Output ($Role + ' ' + $Scenario + ' [' + $Load + '] ' + $status + ' exit=' + $caseExit + $(if ($caseDirectory) { ' evidence=' + $caseDirectory } else { '' }))
    $row
}

function Invoke-PairedWorkloads {
    param([string[]]$Workloads)
    $workloadIndex = 0
    foreach ($workload in $Workloads) {
        foreach ($load in @('Quiet', 'Contention')) {
            $cellId = $workload + '/' + $load
            for ($pair = 1; $pair -le $Pairs; $pair++) {
                $pairId = $cellId + '/' + ('{0:D2}' -f $pair)
                $order = if ($pair % 2 -eq 1) { 'AB' } else { 'BA' }
                if ($order -eq 'AB') { $roles = @('control', 'candidate') } else { $roles = @('candidate', 'control') }
                foreach ($role in $roles) {
                    $isControl = $role -eq 'control'
                    $manifest = if ($isControl) { $control } else { $candidate }
                    $manifestPath = if ($isControl) { $controlManifestPath } else { $candidateManifestPath }
                    [void](Invoke-NativeCase -Scenario $workload -Workload $workload -Role $role -HarnessManifest $manifest -Load $load -CellId $cellId -PairId $pairId -Order $order -Margin $TimingMarginUs -Fps 60 -Sample $SampleSeconds -Historical $isControl)
                }
            }
        }
        $workloadIndex++
    }
}

function Get-PairedMetrics {
    $pairsOut = [System.Collections.Generic.List[object]]::new()
    foreach ($group in ($runRows | Where-Object { $_.pair_id -and $_.status -eq 'PASS' } | Group-Object pair_id)) {
        $baseline = @($group.Group | Where-Object role -eq 'control') | Select-Object -First 1
        $next = @($group.Group | Where-Object role -eq 'candidate') | Select-Object -First 1
        if ($null -eq $baseline -or $null -eq $next -or $null -eq $baseline.timing_metrics -or $null -eq $next.timing_metrics) { continue }
        $controlMetric = $baseline.timing_metrics.authored_lateness_us
        $candidateMetric = $next.timing_metrics.authored_lateness_us
        if ($null -eq $controlMetric.p99 -or $null -eq $candidateMetric.p99) { continue }
        $pairsOut.Add([pscustomobject]@{
            cell_id = $baseline.cell_id
            pair_id = $group.Name
            order = $baseline.pair_order
            control_p50_us = $controlMetric.p50
            candidate_p50_us = $candidateMetric.p50
            delta_p50_us = [double]$candidateMetric.p50 - [double]$controlMetric.p50
            control_p95_us = $controlMetric.p95
            candidate_p95_us = $candidateMetric.p95
            delta_p95_us = [double]$candidateMetric.p95 - [double]$controlMetric.p95
            control_p99_us = $controlMetric.p99
            candidate_p99_us = $candidateMetric.p99
            delta_p99_us = [double]$candidateMetric.p99 - [double]$controlMetric.p99
            control_max_us = $controlMetric.max
            candidate_max_us = $candidateMetric.max
            delta_max_us = [double]$candidateMetric.max - [double]$controlMetric.max
            control_samples = $controlMetric.sample_count
            candidate_samples = $candidateMetric.sample_count
            favorable_p99 = [double]$candidateMetric.p99 - [double]$controlMetric.p99 -lt 0
        })
    }
    $pairsOut.ToArray()
}

try {
    if ($Stage -eq 'Smoke' -and $ControlManifest) { throw 'Smoke accepts only -CandidateManifest; control manifests are reserved for Compare and Final' }
    if ($Stage -in @('Compare', 'Final') -and -not $ControlManifest) { throw ($Stage + ' requires -ControlManifest') }
    if ($Stage -ne 'Smoke' -and ($Pairs -ne 6 -or $SampleSeconds -ne 15 -or $TimingMarginUs -ne 500)) { throw 'Compare and Final require exactly six paired runs, 15-second samples, and a 500 us timing margin' }
    Assert-AbsolutePath $CandidateManifest 'candidate manifest'
    Assert-AbsolutePath $OutputRoot 'output root'
    Assert-OutsideRepository $evidenceRoot 'output root'
    if (Test-Path -LiteralPath $evidenceRoot) {
        if ((Get-ChildItem -LiteralPath $evidenceRoot -Force | Measure-Object).Count -ne 0) { throw 'output root must be absent or empty' }
    } else {
        New-Item -ItemType Directory -Path $evidenceRoot | Out-Null
    }
    if (Test-Path -LiteralPath $zipPath) { throw ('evidence archive already exists: ' + $zipPath) }
    $source = Get-CurrentSourceIdentity
    $rustcVersion = (& rustc -Vv) -join [Environment]::NewLine
    if ($LASTEXITCODE -ne 0) { throw 'could not identify active rustc version' }
    $candidate = Read-And-VerifyManifest $candidateManifestPath 'candidate'
    if ($candidate.source_revision -ne $head -or $candidate.source_tree_sha -ne $tree) { throw 'candidate manifest must name the current clean source revision and tree' }
    if ($candidate.rustc_version -ne $rustcVersion.Trim()) { throw 'candidate manifest rustc identity differs from the active toolchain' }
    if ($Stage -in @('Compare', 'Final')) {
        Assert-AbsolutePath $ControlManifest 'control manifest'
        $control = Read-And-VerifyManifest $controlManifestPath 'control'
        if ($control.rustc_version -ne $candidate.rustc_version -or $control.cargo_profile -ne $candidate.cargo_profile -or
            (@($control.cargo_features) -join ',') -ne (@($candidate.cargo_features) -join ',') -or
            $control.target -ne $candidate.target -or $control.workload_version -ne $candidate.workload_version) {
            throw 'control and candidate manifests do not have matching compiler, target, profile, feature, and workload identities'
        }
        & git -C $root merge-base --is-ancestor $control.source_revision HEAD
        if ($LASTEXITCODE -ne 0) { throw 'control source revision is not an ancestor of the candidate checkout' }
    }

    $scenarios = @(
        'canonical-single', 'canonical-max-chord', 'near-minimum-retrigger', 'release-gap-stress',
        'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary',
        'modifier-held-after-owned', 'focus-loss', 'target-hwnd-change', 'owner-mismatch',
        'owner-query-failure', 'owner-process-termination', 'pause-resume', 'pause-resume-release-floor',
        'suspend-resume', 'stop-cleanup', 'skip-cleanup', 'supervisor-lease-expiry',
        'lease-age-watchdog-delayed', 'noncanonical-profile'
    )
    $performanceWorkloads = if ($Stage -eq 'Compare') {
        @('dense-alternating', 'mixed-up-down', 'canonical-max-chord')
    } else {
        @('dense-alternating', 'mixed-up-down', 'canonical-max-chord', 'near-minimum-retrigger', 'long-idle')
    }

    if ($Stage -eq 'Smoke') {
        foreach ($scenario in $scenarios) {
            [void](Invoke-NativeCase -Scenario $scenario -Workload 'safety-smoke' -Role 'candidate' -HarnessManifest $candidate -Load 'Quiet' -CellId 'safety-smoke' -PairId '' -Order '' -Margin $TimingMarginUs -Fps 60 -Sample 0 -Historical $false)
        }
    } elseif ($Stage -eq 'Compare') {
        Invoke-PairedWorkloads $performanceWorkloads
    } else {
        Invoke-PairedWorkloads $performanceWorkloads
        foreach ($scenario in $scenarios) {
            [void](Invoke-NativeCase -Scenario $scenario -Workload 'safety-smoke' -Role 'candidate' -HarnessManifest $candidate -Load 'Quiet' -CellId 'final-safety-smoke' -PairId '' -Order '' -Margin $TimingMarginUs -Fps 60 -Sample 0 -Historical $false)
        }
        [void](Invoke-NativeCase -Scenario 'timing-margin-sweep' -Workload 'boundary-60fps-margin-0' -Role 'candidate' -HarnessManifest $candidate -Load 'Quiet' -CellId 'boundary-safety' -PairId '' -Order '' -Margin 0 -Fps 60 -Sample 0 -Historical $false)
        [void](Invoke-NativeCase -Scenario 'timing-margin-sweep' -Workload 'boundary-120fps-margin-500' -Role 'candidate' -HarnessManifest $candidate -Load 'Quiet' -CellId 'boundary-safety' -PairId '' -Order '' -Margin 500 -Fps 120 -Sample 0 -Historical $false)
    }

    $source = Get-CurrentSourceIdentity
    $os = Get-CimInstance Win32_OperatingSystem
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $pairedMetrics = Get-PairedMetrics
    $passed = @($runRows | Where-Object status -eq 'PASS').Count
    $safetyRows = @($runRows | Where-Object { $_.workload -like '*safety*' -or $_.cell_id -eq 'boundary-safety' })
    $hardSafety = [ordered]@{
        invalid_or_duplicate_down = '0 expected by PASS verdict; event-window reconciliation and production forensics are recorded per run'
        unexplained_missing_chord = '0 expected by PASS verdict; chord_split_events, missed boundaries, SendInput counters, and event-window counts are recorded per run'
        hold_release_floor_violation_count = [long](@($safetyRows | ForEach-Object { [long]($_.production_hold_floor_violation_count ?? 0) } | Measure-Object -Sum).Sum)
        hold_release_floor_violation_release_count = [long](@($safetyRows | ForEach-Object { [long]($_.production_release_floor_violation_count ?? 0) } | Measure-Object -Sum).Sum)
        production_forensics_anomaly_count = [long](@($safetyRows | ForEach-Object { [long]($_.production_forensics_anomaly_count ?? 0) } | Measure-Object -Sum).Sum)
        residual_release_obligations = @($safetyRows | Where-Object { $null -ne $_.final_release_obligation_mask -and [long]$_.final_release_obligation_mask -ne 0 }).Count
        rt_allocation = 'covered by focused rt_dispatch_no_alloc test; not instrumented by the native ReceiveOnly runner'
    }
    $summary = [ordered]@{
        schema_version = 1
        run_id = $runId
        stage = $Stage
        source = $source
        candidate = [ordered]@{ manifest_path = $candidate.manifest_path; manifest_sha256 = $candidate.manifest_sha256; source_revision = $candidate.source_revision; source_tree_sha = $candidate.source_tree_sha; profile = $candidate.cargo_profile; features = @($candidate.cargo_features); binary_sha256 = $candidate.binary_sha256 }
        control = if ($control) { [ordered]@{ manifest_path = $control.manifest_path; manifest_sha256 = $control.manifest_sha256; source_revision = $control.source_revision; source_tree_sha = $control.source_tree_sha; profile = $control.cargo_profile; features = @($control.cargo_features); binary_sha256 = $control.binary_sha256 } } else { $null }
        parameters = [ordered]@{ pairs = if ($Stage -eq 'Smoke') { 1 } else { $Pairs }; sample_seconds = if ($Stage -eq 'Smoke') { $null } else { $SampleSeconds }; timing_margin_us = $TimingMarginUs; order = 'alternating AB/BA, three pairs each per cell'; quiet = 'no benchmark CPU workers'; contention = 'two hidden pwsh busy workers at BelowNormal priority'; sink = 'project-owned ReceiveOnly sink, identity verified by PID/HWND/start time/event-log ID' }
        environment = [ordered]@{ rustc_version = $rustcVersion.Trim(); windows_caption = $os.Caption; windows_version = $os.Version; windows_build = $os.BuildNumber; processor_name = $cpu.Name; processor_cores = $cpu.NumberOfCores; processor_logical_processors = $cpu.NumberOfLogicalProcessors; power_scheme = (& powercfg /getactivescheme | Out-String).Trim() }
        run_count = $runRows.Count
        passed_count = $passed
        failed_count = $failureRows.Count
        hard_safety_review = $hardSafety
        performance_pairs = $pairedMetrics
        failures = $failureRows.ToArray()
        sessions = $runRows.ToArray()
        disposition = if ($exitCode -eq 0) { 'collected; leader review required for performance gates' } else { 'one or more cases failed; leader review required' }
    }
    $summary | ConvertTo-Json -Depth 14 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'summary.json') -Encoding UTF8
    $runRows | Select-Object run_id,stage,cell_id,pair_id,pair_order,role,source_revision,source_tree_sha,harness_sha256,profile,features,observer_mode,guard_mode,workload,scenario,load,game_fps,timing_margin_us,sample_seconds,started_utc,ended_utc,case_exit_code,status,reason,evidence_directory,expected_sink_event_count,observed_sink_event_count,sink_window_match,active_count,possibly_active_count,final_release_obligation_mask,production_hold_floor_violation_count,production_release_floor_violation_count,production_forensics_anomaly_count,production_structural_anomaly_count,production_same_key_overlap_forensics_count,production_anchor_overwrite_count,production_unmatched_up_count,missed_down_boundaries,missed_down_keys,missed_physical_window_boundaries,keys_dropped,chord_split_events,sendinput_partial_events,sendinput_zero_progress_failures,worker_cpu_time_us,process_cpu_time_us,spin_time_us,spin_time_available,spin_time_complete,spin_duty_cycle_ppm,spin_expected_samples,spin_dropped_samples,spin_missing_samples | Export-Csv -LiteralPath (Join-Path $evidenceRoot 'sessions.csv') -NoTypeInformation -Encoding UTF8
    $commandRows | Set-Content -LiteralPath (Join-Path $evidenceRoot 'commands.txt') -Encoding UTF8
    $safetyMarkdown = @($safetyRows | ForEach-Object { '| ' + $_.scenario + ' | ' + $_.status + ' | ' + $_.case_exit_code + ' | ' + $_.reason.Replace('|', '\|') + ' |' })
    $performanceMarkdown = @($pairedMetrics | ForEach-Object { '| ' + $_.cell_id + ' | ' + $_.pair_id + ' | ' + $_.control_p99_us + ' | ' + $_.candidate_p99_us + ' | ' + $_.delta_p99_us + ' | ' + $_.control_samples + '/' + $_.candidate_samples + ' |' })
    $markdown = @(
        '# Dispatch Core qualification ' + $Stage,
        '',
        '- Run: `' + $runId + '`',
        '- Source: `' + $head + '` tree `' + $tree + '` (clean)',
        '- Candidate: `' + $candidate.source_revision + '` / `' + $candidate.cargo_profile + '` / SHA-256 `' + $candidate.binary_sha256 + '`',
        ('- Control: ' + $(if ($control) { '`' + $control.source_revision + '` / `' + $control.cargo_profile + '` / SHA-256 `' + $control.binary_sha256 + '`' } else { 'not applicable' })),
        '- Sessions: ' + $runRows.Count + '; PASS: ' + $passed + '; failed/inconclusive: ' + $failureRows.Count,
        '- Paired order: AB/BA alternating; six pairs per workload/load cell; 15 seconds per timed session.',
        '- Contention: two hidden PowerShell CPU workers at BelowNormal priority. Sink: identity-verified ReceiveOnly.',
        '- Percentiles use nearest-rank ceiling; p99 is shown with sample count. No p99.9 is inferred.',
        '- Native RT allocation is not instrumented in the sink runner; see focused no-allocation test result in the accompanying command record.',
        '',
        '## Safety cases',
        '',
        '| Scenario | Verdict | Exit | Reason |',
        '|---|---:|---:|---|'
    ) + $safetyMarkdown + @(
        '',
        '## Paired performance observations',
        '',
        '| Cell | Pair | Control p99 authored lateness (us) | Candidate p99 (us) | Delta (us) | Samples C0/final |',
        '|---|---:|---:|---:|---:|---:|'
    ) + $performanceMarkdown + @(
        '',
        'These are evidence summaries, not an automatic performance-gate verdict. No design, gate, or rollout selection is made by this runner.',
        ''
    )
    $markdown | Set-Content -LiteralPath (Join-Path $evidenceRoot 'summary.md') -Encoding UTF8
    $scriptHash = Get-Sha256 $PSCommandPath
    $caseRunnerHash = Get-Sha256 $caseRunner
    $builderHash = Get-Sha256 $manifestBuilder
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $evidenceRoot 'qualify_dispatch_core.ps1')
    Copy-Item -LiteralPath $caseRunner -Destination (Join-Path $evidenceRoot 'run_native_acceptance_case.ps1')
    Copy-Item -LiteralPath $manifestBuilder -Destination (Join-Path $evidenceRoot 'build_dispatch_core_harness.ps1')
    $commandRows.Insert(0, 'pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 -Stage ' + $Stage + ' -CandidateManifest "' + $candidateManifestPath + '"' + $(if ($controlManifestPath) { ' -ControlManifest "' + $controlManifestPath + '" -Pairs 6 -SampleSeconds 15 -TimingMarginUs ' + $TimingMarginUs } else { '' }) + ' -OutputRoot "' + $evidenceRoot + '"')
    $commandRows | Set-Content -LiteralPath (Join-Path $evidenceRoot 'commands.txt') -Encoding UTF8
    $metadata = [ordered]@{ runner_sha256 = $scriptHash; case_runner_sha256 = $caseRunnerHash; builder_sha256 = $builderHash; source_revision = $head; source_tree_sha = $tree; source_tree_clean = $true; candidate_manifest_sha256 = $candidate.manifest_sha256; control_manifest_sha256 = if ($control) { $control.manifest_sha256 } else { $null } }
    $metadata | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'provenance.json') -Encoding UTF8
    [ordered]@{
        schema_version = 1
        run_id = $runId
        stage = $Stage
        created_utc = [DateTimeOffset]::UtcNow.ToString('O')
        source_branch = $source.branch
        source_revision = $source.revision
        source_tree_sha = $source.tree
        source_tree_clean = $source.clean
        control_manifest = if ($control) { [ordered]@{ path = $control.manifest_path; sha256 = $control.manifest_sha256; source_revision = $control.source_revision; source_tree_sha = $control.source_tree_sha; cargo_profile = $control.cargo_profile; cargo_features = @($control.cargo_features); binary_sha256 = $control.binary_sha256 } } else { $null }
        candidate_manifest = [ordered]@{ path = $candidate.manifest_path; sha256 = $candidate.manifest_sha256; source_revision = $candidate.source_revision; source_tree_sha = $candidate.source_tree_sha; cargo_profile = $candidate.cargo_profile; cargo_features = @($candidate.cargo_features); binary_sha256 = $candidate.binary_sha256 }
        parameters = [ordered]@{ pairs = if ($Stage -eq 'Smoke') { 1 } else { 6 }; sample_seconds = if ($Stage -eq 'Smoke') { $null } else { 15 }; timing_margin_us = $TimingMarginUs; workload_version = $candidate.workload_version; observer_mode = $candidate.observer_mode; guard_mode = $candidate.guard_mode; quiet_load = 'no extra qualification workers'; contention_load = 'two BelowNormal CPU workers'; pair_order = 'alternating AB/BA' }
        rustc_version = $rustcVersion.Trim()
        windows_caption = $os.Caption
        windows_version = $os.Version
        windows_build = $os.BuildNumber
        processor_name = $cpu.Name
        processor_cores = $cpu.NumberOfCores
        processor_logical_processors = $cpu.NumberOfLogicalProcessors
        power_scheme = (& powercfg /getactivescheme | Out-String).Trim()
        runner_sha256 = $scriptHash
        case_runner_sha256 = $caseRunnerHash
        builder_sha256 = $builderHash
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'manifest.json') -Encoding UTF8
} catch {
    $failureRows.Add([pscustomobject]@{ scenario = 'runner'; workload = $null; role = $null; cell_id = $null; pair_id = $null; exit_code = 2; status = 'RUNNER_FAILURE'; reason = $_.Exception.Message; evidence_directory = $evidenceRoot })
    $exitCode = 2
    Write-Output ('RUNNER FAILURE: ' + $_.Exception.Message)
} finally {
    if (Test-Path -LiteralPath $evidenceRoot) {
        if (-not (Test-Path -LiteralPath (Join-Path $evidenceRoot 'summary.json'))) {
            [ordered]@{ schema_version = 1; run_id = $runId; stage = $Stage; source_revision = $head; source_tree_sha = $tree; run_count = $runRows.Count; failures = $failureRows.ToArray(); disposition = 'runner failed before completing protocol' } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'summary.json') -Encoding UTF8
            $failureRows | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'failures.json') -Encoding UTF8
            $commandRows | Set-Content -LiteralPath (Join-Path $evidenceRoot 'commands.txt') -Encoding UTF8
        }
        $hashLines = @(Get-ChildItem -LiteralPath $evidenceRoot -Recurse -File | Where-Object { $_.Name -ne 'SHA256SUMS.txt' } | Sort-Object FullName | ForEach-Object {
            (Get-Sha256 $_.FullName) + '  ' + [System.IO.Path]::GetRelativePath($evidenceRoot, $_.FullName)
        })
        Set-Content -LiteralPath (Join-Path $evidenceRoot 'SHA256SUMS.txt') -Value $hashLines -Encoding UTF8
        if (-not (Test-Path -LiteralPath $zipPath)) {
            try { Get-ChildItem -LiteralPath $evidenceRoot -Force | Compress-Archive -DestinationPath $zipPath -CompressionLevel Optimal -ErrorAction Stop } catch { Write-Output ('ARCHIVE FAILURE: ' + $_.Exception.Message); $exitCode = 2 }
        }
    }
}

Write-Output ('SUMMARY ' + (Join-Path $evidenceRoot 'summary.md'))
Write-Output ('EVIDENCE ' + $evidenceRoot)
if (Test-Path -LiteralPath $zipPath) { Write-Output ('ARCHIVE ' + $zipPath) }
exit $exitCode
