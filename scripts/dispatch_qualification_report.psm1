function Get-ReportOptionalProperty {
    param($Object, [string]$Name)
    if ($null -eq $Object) { return $null }
    if ($Object -is [System.Collections.IDictionary]) {
        if ($Object.Contains($Name)) { return $Object[$Name] }
        return $null
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    $property.Value
}

function Get-QualificationReportRowLabel {
    param(
        [Parameter(Mandatory)] [string]$Profile,
        [Parameter(Mandatory)] [ValidateSet('control', 'candidate')] [string]$Role,
        $Manifest
    )

    if ($Profile -eq 'FrameSnapshot') {
        if ($Role -eq 'control') { return 'FrameSnapshot-Baseline' }
        return 'FrameSnapshot-Candidate'
    }
    if ($Profile -eq 'DispatchCleanupM3') {
        if ($Role -eq 'control') { return 'DispatchCleanupM3-Baseline' }
        return 'DispatchCleanupM3-Candidate'
    }
    if ($Profile -eq 'DispatchSessionRedesignM3') {
        if ($Role -eq 'control') { return 'DispatchSessionRedesignM3-Baseline' }
        return 'DispatchSessionRedesignM3-Candidate'
    }
    if ($Profile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) {
        if ($Role -eq 'control') { return ($Profile + '-Baseline') }
        return ($Profile + '-Candidate')
    }
    if ($Profile -eq 'Scheduler') {
        if ($Role -eq 'control') { return 'Scheduler-Baseline' }
        return 'Scheduler-Candidate'
    }
    if ([bool](Get-ReportOptionalProperty $Manifest 'c1_timed_verified')) { return 'C1-TIMED-R2' }
    if ($Role -eq 'candidate') { return 'Candidate-R2' }
    'C0'
}

function Get-QualificationReportSchemaVersion {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('DispatchCore', 'Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')]
        [string]$Profile
    )
    if ($Profile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) { return 2 }
    1
}

function Get-DispatchCleanupM3TimingFloors {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('canonical-single', 'canonical-chord', 'canonical-max-chord', 'hold', 'long-single-sequence', 'dense-alternating', 'chord-sweep', 'near-minimum-retrigger', 'rapid-retrigger', 'short-up-gap-stress', 'mixed-up-down', 'ambiguous-packet', 'preflight-user-held', 'modifier-held-final-boundary', 'modifier-held-after-owned', 'cleanup-full-release', 'focus-loss', 'target-hwnd-change', 'owner-mismatch', 'owner-query-failure', 'owner-process-termination', 'pause-resume', 'pause-resume-active-cleanup', 'suspend-resume', 'stop-cleanup', 'skip-cleanup', 'supervisor-lease-expiry', 'lease-age-watchdog-delayed', 'noncanonical-profile', 'timing-margin-sweep', 'long-idle', 'hol-interleaved')]
        [string]$Scenario,
        [Parameter(Mandatory)]
        [ValidateSet(60, 120)]
        [int]$GameFps,
        [Parameter(Mandatory)]
        [ValidateScript({ $_ -ge 0 -and $_ -le 3000 -and $_ % 100 -eq 0 })]
        [int]$TimingMarginUs
    )

    $frameUs = [long][Math]::Ceiling(1000000.0 / $GameFps)
    $holdFrames = if ($Scenario -eq 'hol-interleaved') { 2 } else { 1 }
    $holdUs = [long]($holdFrames * $frameUs + $TimingMarginUs)
    [pscustomobject]@{
        scenario = $Scenario
        game_fps = $GameFps
        timing_margin_us = $TimingMarginUs
        frame_us = $frameUs
        hold_frames = $holdFrames
        expected_hold_us = $holdUs
        configured_H_us = $holdUs
    }
}

function Get-DispatchSessionRedesignM3TimingFloors {
    param(
        [Parameter(Mandatory)] [ValidateSet('canonical-single', 'focus-loss', 'stop-cleanup', 'pause-resume', 'owner-process-termination', 'target-hwnd-change', 'near-minimum-retrigger', 'mixed-up-down')] [string]$Scenario,
        [Parameter(Mandatory)] [ValidateSet(60, 120)] [int]$GameFps,
        [Parameter(Mandatory)] [ValidateScript({ $_ -eq 500 })] [int]$TimingMarginUs
    )
    $frameUs = [long][Math]::Ceiling(1000000.0 / $GameFps)
    [pscustomobject]@{
        scenario = $Scenario
        game_fps = $GameFps
        timing_margin_us = $TimingMarginUs
        frame_us = $frameUs
        hold_frames = 1
        expected_hold_us = [long]($frameUs + $TimingMarginUs)
        configured_H_us = [long]($frameUs + $TimingMarginUs)
    }
}

function Assert-DispatchSessionRedesignM3SessionIdentity {
    param(
        [Parameter(Mandatory)] [string]$Scenario,
        [Parameter(Mandatory)] [string]$Workload,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [int]$GameFps,
        [int]$SampleSeconds
    )

    $canary = $Scenario -eq 'canonical-single' -and $Workload -eq 'canonical-single' -and
        $TimingMarginUs -eq 500 -and $GameFps -eq 60 -and $SampleSeconds -eq 0
    $safetyScenarios = @('focus-loss', 'stop-cleanup', 'pause-resume', 'owner-process-termination', 'target-hwnd-change')
    $standardSafety = $Scenario -in $safetyScenarios -and $Workload -eq 'safety-smoke' -and
        $TimingMarginUs -eq 500 -and $GameFps -eq 60 -and $SampleSeconds -eq 0
    $retriggerSafety = $Scenario -eq 'near-minimum-retrigger' -and $Workload -eq 'near-minimum-retrigger' -and
        $TimingMarginUs -eq 500 -and $GameFps -in @(60, 120) -and $SampleSeconds -eq 0
    $performance = $Scenario -eq 'mixed-up-down' -and $Workload -eq 'mixed-up-down' -and
        $TimingMarginUs -eq 500 -and $GameFps -eq 60 -and $SampleSeconds -eq 15
    if (-not ($canary -or $standardSafety -or $retriggerSafety -or $performance)) {
        throw ('DispatchSessionRedesignM3 does not authorize scenario/workload/margin/FPS/sample ' +
            $Scenario + '/' + $Workload + '/' + $TimingMarginUs + '/' + $GameFps + '/' + $SampleSeconds)
    }
    Get-DispatchSessionRedesignM3TimingFloors -Scenario $Scenario -GameFps $GameFps -TimingMarginUs $TimingMarginUs
}

function Assert-PlaybackCoreLifecycleSessionIdentity {
    param(
        [Parameter(Mandatory)] [string]$Scenario,
        [Parameter(Mandatory)] [string]$Workload,
        [Parameter(Mandatory)] [ValidateScript({ $_ -eq 500 })] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [ValidateSet(60)] [int]$GameFps,
        [int]$SampleSeconds,
        [ValidateSet(1, 2)] [int]$ProfileVersion = 1
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version $ProfileVersion
    $performance = $Workload -in $profile.PerformanceWorkloads -and
        $Scenario -eq $Workload -and $SampleSeconds -eq 15
    $safety = $Workload -eq 'safety-smoke' -and
        $Scenario -in $profile.SafetyScenarios -and $SampleSeconds -eq 0
    if (-not ($performance -or $safety)) {
        throw ('PlaybackCoreLifecycle does not authorize scenario/workload/margin/FPS/sample ' +
            $Scenario + '/' + $Workload + '/' + $TimingMarginUs + '/' + $GameFps + '/' + $SampleSeconds)
    }
    $frameUs = [long][Math]::Ceiling(1000000.0 / $GameFps)
    [pscustomobject]@{
        scenario = $Scenario
        workload = $Workload
        game_fps = $GameFps
        timing_margin_us = $TimingMarginUs
        frame_us = $frameUs
        hold_frames = 1
        expected_hold_us = [long]($frameUs + $TimingMarginUs)
        configured_H_us = [long]($frameUs + $TimingMarginUs)
    }
}

function Get-PlaybackCoreLifecycleMeasuredAuthoredPrefix {
    param(
        [Parameter(Mandatory)] $Report,
        [Parameter(Mandatory)] [ValidateRange(1, 86400)] [int]$SampleSeconds
    )
    $details = Get-ReportOptionalProperty $Report 'details'
    $rawGroups = Get-ReportOptionalProperty $details 'authored_timestamp_group_targets'
    if ($null -eq $rawGroups) { $rawGroups = Get-ReportOptionalProperty $details 'authored_packet_targets' }
    $groups = [System.Collections.Generic.List[object]]::new()
    foreach ($item in $rawGroups) {
        if ($item -is [Array]) {
            foreach ($nestedItem in $item) { $groups.Add($nestedItem) }
        } else {
            $groups.Add($item)
        }
    }
    if ($groups.Count -eq 0) { throw 'paired acceptance report is missing its authored timestamp-group projection' }
    $firstTimestampUs = [long](Get-ReportOptionalProperty $groups[0] 'scheduled_us')
    $windowEndUs = $firstTimestampUs + ([long]$SampleSeconds * 1000000L)
    $fullEvents = [System.Collections.Generic.List[object]]::new()
    $measuredEvents = [System.Collections.Generic.List[object]]::new()
    $tailEvents = [System.Collections.Generic.List[object]]::new()
    foreach ($group in $groups) {
        $timestampUs = [long](Get-ReportOptionalProperty $group 'scheduled_us')
        $upMask = [long](Get-ReportOptionalProperty $group 'up_mask')
        $downMask = [long](Get-ReportOptionalProperty $group 'down_mask')
        foreach ($event in @(
            if ($upMask -ne 0) { [ordered]@{ timestamp_us = $timestampUs; kind = 'up'; scan_mask = $upMask } }
            if ($downMask -ne 0) { [ordered]@{ timestamp_us = $timestampUs; kind = 'down'; scan_mask = $downMask } }
        )) {
            $fullEvents.Add($event)
            if ($timestampUs -ge $firstTimestampUs -and $timestampUs -lt $windowEndUs) {
                $measuredEvents.Add($event)
            } elseif ($timestampUs -ge $windowEndUs) {
                $tailEvents.Add($event)
            }
        }
    }
    if ($measuredEvents.Count -eq 0) { throw 'paired acceptance report has no authored events in the measured window' }
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $fullCanonical = ConvertTo-Json -InputObject $fullEvents.ToArray() -Depth 6 -Compress
        $measuredCanonical = ConvertTo-Json -InputObject $measuredEvents.ToArray() -Depth 6 -Compress
        $tailCanonical = ConvertTo-Json -InputObject $tailEvents.ToArray() -Depth 6 -Compress
        $fullDigest = [Convert]::ToHexString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($fullCanonical))).ToLowerInvariant()
        $measuredDigest = [Convert]::ToHexString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($measuredCanonical))).ToLowerInvariant()
        $tailDigest = [Convert]::ToHexString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($tailCanonical))).ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
    [pscustomobject]@{
        start_us = $firstTimestampUs
        end_us = $windowEndUs
        measured_event_count = $measuredEvents.Count
        measured_sha256 = $measuredDigest
        full_event_count = $fullEvents.Count
        full_sha256 = $fullDigest
        tail_event_count = $tailEvents.Count
        tail_sha256 = $tailDigest
        tail_group_count = @($groups | Where-Object { [long](Get-ReportOptionalProperty $_ 'scheduled_us') -ge $windowEndUs }).Count
    }
}

function Assert-PlaybackCoreLifecyclePairedWorkloadParity {
    param(
        [Parameter(Mandatory)] $ControlRow,
        [Parameter(Mandatory)] $CandidateRow,
        [Parameter(Mandatory)] [int]$SampleSeconds
    )
    $control = Get-PlaybackCoreLifecycleMeasuredAuthoredPrefix -Report $ControlRow.report -SampleSeconds $SampleSeconds
    $candidate = Get-PlaybackCoreLifecycleMeasuredAuthoredPrefix -Report $CandidateRow.report -SampleSeconds $SampleSeconds
    if ($control.start_us -ne $candidate.start_us -or $control.end_us -ne $candidate.end_us -or
        $control.measured_event_count -ne $candidate.measured_event_count -or
        $control.measured_sha256 -ne $candidate.measured_sha256 -or
        $control.full_event_count -ne $candidate.full_event_count -or
        $control.full_sha256 -ne $candidate.full_sha256 -or
        $control.tail_event_count -ne $candidate.tail_event_count -or
        $control.tail_group_count -ne $candidate.tail_group_count) {
        throw ('paired workload authored stream mismatch for ' + $ControlRow.workload + '/' + $ControlRow.load +
            ' pair ' + $ControlRow.pair_id + ': baseline full sha256=' + $control.full_sha256 +
            ' candidate full sha256=' + $candidate.full_sha256)
    }
    foreach ($entry in @(@{ row = $ControlRow; prefix = $control }, @{ row = $CandidateRow; prefix = $candidate })) {
        $entry.row | Add-Member -NotePropertyName authored_workload_measured_window_start_us -NotePropertyValue $entry.prefix.start_us -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_measured_window_end_us -NotePropertyValue $entry.prefix.end_us -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_measured_event_count -NotePropertyValue $entry.prefix.measured_event_count -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_measured_prefix_sha256 -NotePropertyValue $entry.prefix.measured_sha256 -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_full_event_count -NotePropertyValue $entry.prefix.full_event_count -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_full_stream_sha256 -NotePropertyValue $entry.prefix.full_sha256 -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_tail_event_count -NotePropertyValue $entry.prefix.tail_event_count -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_tail_sha256 -NotePropertyValue $entry.prefix.tail_sha256 -Force
        $entry.row | Add-Member -NotePropertyName authored_workload_tail_group_count -NotePropertyValue $entry.prefix.tail_group_count -Force
    }
    $true
}

function Get-DispatchSessionRedesignM3StageSummary {
    param(
        [Parameter(Mandatory)] [ValidateSet('Final')] [string]$Stage,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [int]$Pairs,
        [Parameter(Mandatory)] [int]$SampleSeconds,
        [string[]]$SafetyScenarios = @(),
        [string[]]$PerformanceWorkloads = @()
    )
    $profile = Get-DispatchSessionRedesignM3Profile
    if ($TimingMarginUs -ne 500 -or $Pairs -ne 6 -or $SampleSeconds -ne 15 -or
        (@($SafetyScenarios) -join '|') -ne ($profile.SafetyScenarios -join '|') -or
        (@($PerformanceWorkloads) -join '|') -ne 'mixed-up-down') {
        throw 'DispatchSessionRedesignM3 Final plan must match its seven candidate safety cases and paired mixed-up-down workload'
    }

    $fpsValues = @(60, 120)
    $frame60 = [long][Math]::Ceiling(1000000.0 / 60)
    $frame120 = [long][Math]::Ceiling(1000000.0 / 120)
    [ordered]@{
        stage = $Stage
        stage_kind = 'candidate Canary, candidate safety, then paired performance; fail-stop between phases'
        qualification_profile = $profile.Name
        workload_version = $profile.WorkloadVersion
        planned_session_count = 32
        candidate_session_count = 20
        control_session_count = 12
        candidate_role = 'Candidate'
        control_role = 'Baseline'
        role_scope = 'candidate Canary and safety; Baseline/Candidate paired performance'
        workloads = @('canonical-single') + @('safety-smoke', 'near-minimum-retrigger') + @('mixed-up-down')
        scenarios = @('canonical-single') + @($SafetyScenarios) + @('mixed-up-down')
        workload_summary = 'one Quiet canonical-single canary; seven Quiet safety cases; mixed-up-down Quiet/Contention paired A/B'
        load_cells = @('Quiet', 'Contention')
        game_fps = $null
        game_fps_values = $fpsValues
        game_fps_summary = 'varies by session (60, 120 FPS)'
        timing_margin_us = 500
        timing_margin_values_us = @(500)
        timing_margin_summary = '500 us'
        performance_timing_margin_us = 500
        safety_timing_margin_us = 500
        configured_H_us = $null
        configured_H_mode = 'varies_by_session'
        configured_H_summary = '60 FPS H=17,167 us; 120 FPS H=8,834 us'
        pairs = 6
        pair_order = 'six alternating AB/BA pairs per Quiet and Contention cell'
        timed_performance_sample_seconds = 15
        timed_performance_samples = $true
        sample_scope = '15-second mixed-up-down performance sessions only; Canary and safety sessions are untimed'
    }
}

function Assert-DispatchCleanupM3RunConfigTiming {
    param(
        [Parameter(Mandatory)] [string]$ExpectedScenario,
        [Parameter(Mandatory)] $RunConfig,
        [Parameter(Mandatory)] [string]$ExpectedWorkload,
        [Parameter(Mandatory)] [int]$ExpectedGameFps,
        [Parameter(Mandatory)] [int]$ExpectedTimingMarginUs
    )

    if ($RunConfig.qualification_profile -ne 'DispatchCleanupM3' -or
        $RunConfig.scenario -ne $ExpectedScenario -or
        $RunConfig.workload -ne $ExpectedWorkload -or
        [int]$RunConfig.game_fps -ne $ExpectedGameFps -or
        [int]$RunConfig.timing_margin_us -ne $ExpectedTimingMarginUs) {
        throw 'DispatchCleanupM3 run-config profile, scenario, or workload attribution mismatch'
    }

    foreach ($retiredField in @('expected_resume_cleanup_delay_us', 'resume_cleanup_delay_us')) {
        if ($null -ne (Get-ReportOptionalProperty $RunConfig $retiredField)) {
            throw ('DispatchCleanupM3 run-config contains retired timing field ' + $retiredField)
        }
    }

    $floors = Get-DispatchCleanupM3TimingFloors -Scenario $ExpectedScenario `
        -GameFps ([int]$RunConfig.game_fps) -TimingMarginUs ([int]$RunConfig.timing_margin_us)
    foreach ($field in @('expected_hold_us', 'configured_H_us')) {
        if ($null -eq $RunConfig.$field -or [long]$RunConfig.$field -ne $floors.configured_H_us) {
            throw ('DispatchCleanupM3 run-config ' + $field + ' does not match the native scenario hold floor')
        }
    }
    $floors
}

function Get-DispatchCleanupM3StageSummary {
    param(
        [Parameter(Mandatory)] [ValidateSet('Canary', 'Smoke', 'Compare', 'Final')] [string]$Stage,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [int]$Pairs,
        [Parameter(Mandatory)] [int]$SampleSeconds,
        [string[]]$SafetyScenarios = @(),
        [string[]]$PerformanceWorkloads = @()
    )

    $safetyCount = $SafetyScenarios.Count
    if ($Stage -in @('Smoke', 'Final') -and $safetyCount -eq 0) {
        throw ('DispatchCleanupM3 ' + $Stage + ' summary requires its planned safety scenarios')
    }
    $contexts = [System.Collections.Generic.List[object]]::new()
    $workloads = @()
    $scenarioNames = @()
    $loadCells = @()
    $roleScope = 'candidate-only'
    $pairCount = $null
    $pairOrder = 'not applicable; candidate-only safety stage'
    $timedPerformanceSampleSeconds = $null
    $plannedSessions = 0
    $candidateSessions = 0
    $controlSessions = 0

    switch ($Stage) {
        'Canary' {
            $scenario = 'canonical-single'
            $workloads = @('canonical-single')
            $scenarioNames = @($scenario)
            $loadCells = @('Quiet')
            $contexts.Add([pscustomobject]@{ scenario = $scenario; game_fps = 60; margin_us = $TimingMarginUs })
            $plannedSessions = 1
            $candidateSessions = 1
        }
        'Smoke' {
            $workloads = @('safety-smoke')
            $scenarioNames = @($SafetyScenarios)
            $loadCells = @('Quiet')
            foreach ($scenario in $SafetyScenarios) {
                $contexts.Add([pscustomobject]@{ scenario = $scenario; game_fps = 60; margin_us = $TimingMarginUs })
            }
            $plannedSessions = $safetyCount
            $candidateSessions = $safetyCount
        }
        'Compare' {
            if ($PerformanceWorkloads.Count -eq 0) { throw 'DispatchCleanupM3 Compare summary requires its performance workloads' }
            $workloads = @($PerformanceWorkloads)
            $scenarioNames = @($PerformanceWorkloads)
            $loadCells = @('Quiet', 'Contention')
            $roleScope = 'Baseline and Candidate performance pairs'
            $pairCount = $Pairs
            $pairOrder = 'alternating AB/BA; ' + $Pairs + ' pairs per workload/load cell'
            $timedPerformanceSampleSeconds = $SampleSeconds
            foreach ($scenario in $PerformanceWorkloads) {
                $contexts.Add([pscustomobject]@{ scenario = $scenario; game_fps = 60; margin_us = 0 })
            }
            $controlSessions = $PerformanceWorkloads.Count * 2 * $Pairs
            $candidateSessions = $controlSessions
            $plannedSessions = $controlSessions + $candidateSessions
        }
        'Final' {
            if ($PerformanceWorkloads.Count -eq 0) { throw 'DispatchCleanupM3 Final summary requires its performance workloads' }
            $workloads = @($PerformanceWorkloads) + @('safety-smoke', 'boundary-60fps-margin-0', 'boundary-120fps-margin-500')
            $scenarioNames = @($PerformanceWorkloads) + @($SafetyScenarios) + @('timing-margin-sweep')
            $loadCells = @('Quiet', 'Contention', 'boundary-Quiet')
            $roleScope = 'Baseline and Candidate performance pairs; Candidate-only safety/boundary cases'
            $pairCount = $Pairs
            $pairOrder = 'alternating AB/BA for performance workloads; ' + $Pairs + ' pairs per workload/load cell'
            $timedPerformanceSampleSeconds = $SampleSeconds
            foreach ($scenario in $PerformanceWorkloads) {
                $contexts.Add([pscustomobject]@{ scenario = $scenario; game_fps = 60; margin_us = 0 })
            }
            foreach ($scenario in $SafetyScenarios) {
                $contexts.Add([pscustomobject]@{ scenario = $scenario; game_fps = 60; margin_us = 500 })
            }
            $contexts.Add([pscustomobject]@{ scenario = 'timing-margin-sweep'; game_fps = 60; margin_us = 0 })
            $contexts.Add([pscustomobject]@{ scenario = 'timing-margin-sweep'; game_fps = 120; margin_us = 500 })
            $controlSessions = $PerformanceWorkloads.Count * 2 * $Pairs
            $candidateSessions = $controlSessions + ($safetyCount * 2) + 2
            $plannedSessions = $controlSessions + $candidateSessions
        }
    }

    $floorRows = @($contexts | ForEach-Object {
        Get-DispatchCleanupM3TimingFloors -Scenario $_.scenario -GameFps $_.game_fps -TimingMarginUs $_.margin_us
    })
    $fpsValues = @($contexts | ForEach-Object { [int]$_.game_fps } | Sort-Object -Unique)
    $marginValues = @($contexts | ForEach-Object { [int]$_.margin_us } | Sort-Object -Unique)
    $holdValues = @($floorRows | ForEach-Object { [long]$_.configured_H_us } | Sort-Object -Unique)
    $configuredH = if ($holdValues.Count -eq 1) { [long]$holdValues[0] } else { $null }
    $fpsSummary = if ($fpsValues.Count -eq 1) { [string]$fpsValues[0] + ' FPS' } else { 'varies by session (' + (($fpsValues | ForEach-Object { [string]$_ }) -join ', ') + ' FPS)' }
    $marginSummary = if ($Stage -eq 'Final') {
        '0 us performance; 500 us safety; boundary cases 0/500 us'
    } elseif ($marginValues.Count -eq 1) {
        [string]$marginValues[0] + ' us'
    } else {
        'varies by session (' + (($marginValues | ForEach-Object { [string]$_ }) -join ', ') + ' us)'
    }
    $floorSummary = if ($null -ne $configuredH) {
        'H=' + $configuredH + ' us'
    } else {
        'H varies by session; use per-session configured_H_us'
    }
    $workloadSummary = switch ($Stage) {
        'Canary' { 'canonical-single' }
        'Smoke' { ([string]$safetyCount) + ' candidate-only safety scenarios' }
        'Compare' { $PerformanceWorkloads -join ', ' }
        'Final' { 'five performance workloads; 21 safety scenarios; two boundary cases' }
    }
    $stageKind = switch ($Stage) {
        'Canary' { 'candidate-only safety Canary' }
        'Smoke' { 'safety-only; candidate-only' }
        'Compare' { 'paired performance' }
        'Final' { 'paired performance plus candidate-only safety and boundaries' }
    }
    $stageSummary = [ordered]@{
        stage = $Stage
        stage_kind = $stageKind
        qualification_profile = 'DispatchCleanupM3'
        workload_version = 'dispatch-runtime-cleanup-m3-v1'
        planned_session_count = $plannedSessions
        candidate_session_count = $candidateSessions
        control_session_count = $controlSessions
        candidate_role = 'Candidate'
        control_role = if ($controlSessions -gt 0) { 'Baseline' } else { $null }
        role_scope = $roleScope
        workloads = @($workloads)
        scenarios = @($scenarioNames)
        workload_summary = $workloadSummary
        load_cells = @($loadCells)
        game_fps = if ($fpsValues.Count -eq 1) { [int]$fpsValues[0] } else { $null }
        game_fps_values = @($fpsValues)
        game_fps_summary = $fpsSummary
        timing_margin_us = if ($marginValues.Count -eq 1) { [int]$marginValues[0] } else { $null }
        timing_margin_values_us = @($marginValues)
        timing_margin_summary = $marginSummary
        performance_timing_margin_us = 0
        safety_timing_margin_us = 500
        configured_H_us = $configuredH
        configured_H_mode = if ($null -ne $configuredH) { 'common' } else { 'varies_by_session' }
        configured_H_summary = $floorSummary
        pairs = $pairCount
        pair_order = $pairOrder
        timed_performance_sample_seconds = $timedPerformanceSampleSeconds
        timed_performance_samples = ($null -ne $timedPerformanceSampleSeconds -and $timedPerformanceSampleSeconds -gt 0)
        sample_scope = if ($timedPerformanceSampleSeconds) { 'performance workloads only; safety and boundary cases are untimed' } else { 'no timed performance samples' }
    }
    $stageSummary
}

function Get-PlaybackCoreLifecycleStageSummary {
    param(
        [Parameter(Mandatory)] [ValidateSet('Final')] [string]$Stage,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [Parameter(Mandatory)] [int]$Pairs,
        [Parameter(Mandatory)] [int]$SampleSeconds,
        [string[]]$SafetyScenarios = @(),
        [string[]]$PerformanceWorkloads = @(),
        [string[]]$LoadCells = @(),
        [ValidateSet(1, 2)] [int]$ProfileVersion = 1
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version $ProfileVersion
    $expectedSafetyScenarios = @(
        'focus-loss',
        'owner-mismatch',
        'owner-process-termination',
        'pause-resume',
        'suspend-resume',
        'stop-cleanup',
        'skip-cleanup',
        'supervisor-lease-expiry'
    )
    if ($ProfileVersion -eq 2) { $expectedSafetyScenarios += @('short-up-gap-stress', 'pause-resume-active-cleanup') }
    $expectedSafetyContract = $expectedSafetyScenarios -join '|'
    if ($TimingMarginUs -ne 500 -or $Pairs -ne 6 -or $SampleSeconds -ne 15 -or
        (@($SafetyScenarios) -join '|') -ne $expectedSafetyContract -or
        (@($PerformanceWorkloads) -join '|') -ne 'mixed-up-down|dense-alternating|canonical-max-chord|near-minimum-retrigger|long-idle' -or
        (@($LoadCells) -join '|') -ne 'Quiet|Contention') {
        throw ($profile.Name + ' Final plan must match its candidate safety matrix and six alternating A/B pairs across the five Quiet/Contention workloads')
    }
    $pairedPerRole = $PerformanceWorkloads.Count * $LoadCells.Count * $Pairs
    $candidateSessions = $pairedPerRole + $SafetyScenarios.Count
    $summary = [ordered]@{
        stage = $Stage
        stage_kind = 'six alternating AB/BA pairs per workload/load cell, followed by candidate-only safety cases; fail-stop between cases'
        qualification_profile = $profile.Name
        workload_version = $profile.WorkloadVersion
        planned_session_count = $candidateSessions + $pairedPerRole
        candidate_session_count = $candidateSessions
        control_session_count = $pairedPerRole
        candidate_role = 'Candidate'
        control_role = 'Baseline'
        role_scope = 'Baseline/Candidate paired performance; Candidate-only safety'
        workloads = @('safety-smoke') + @($PerformanceWorkloads)
        scenarios = @($SafetyScenarios) + @($PerformanceWorkloads)
        workload_summary = if ($ProfileVersion -eq 2) { 'ten candidate safety cases plus five paired workloads in Quiet and Contention' } else { 'eight candidate safety cases plus five paired workloads in Quiet and Contention' }
        load_cells = @($LoadCells)
        game_fps = 60
        game_fps_values = @(60)
        game_fps_summary = '60 FPS workload configuration'
        timing_margin_us = 500
        timing_margin_values_us = @(500)
        timing_margin_summary = '500 us'
        performance_timing_margin_us = 500
        safety_timing_margin_us = 500
        configured_H_us = $null
        configured_H_mode = 'per scenario workload profile'
        configured_H_summary = 'profile-defined production H at 60 FPS with a 500 us margin'
        pairs = $Pairs
        pair_order = 'AB/BA alternating'
        timed_performance_sample_seconds = $SampleSeconds
        timed_performance_samples = $pairedPerRole * 2
        sample_scope = '15 seconds for each candidate and baseline performance session; safety cases are untimed'
    }
    if ($ProfileVersion -eq 2) {
        $summary['authored_prefix_parity_contract'] = $profile.AuthoredPrefixParityContract
        $summary['authored_prefix_measured_window_seconds'] = $SampleSeconds
    }
    $summary
}

function New-QualificationReportProjection {
    param(
        [Parameter(Mandatory)] [ValidateSet('DispatchCore', 'Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')] [string]$Profile,
        [Parameter(Mandatory)] $Candidate,
        $Control,
        [Parameter(Mandatory)] [ValidateSet('Canary', 'Smoke', 'Compare', 'Final')] [string]$Stage,
        [Parameter(Mandatory)] [int]$Pairs,
        [Parameter(Mandatory)] [int]$SampleSeconds,
        [Parameter(Mandatory)] [int]$TimingMarginUs,
        [string[]]$SafetyScenarios = @(),
        [string[]]$PerformanceWorkloads = @()
    )

    $candidateLabel = switch ($Profile) {
        'FrameSnapshot' { 'FrameSnapshot-Candidate' }
        'DispatchCleanupM3' { 'DispatchCleanupM3-Candidate' }
        'DispatchSessionRedesignM3' { 'DispatchSessionRedesignM3-Candidate' }
        'PlaybackCoreLifecycle' { 'PlaybackCoreLifecycle-Candidate' }
        'PlaybackCoreLifecycleV2' { 'PlaybackCoreLifecycleV2-Candidate' }
        'Scheduler' { 'Scheduler-Candidate' }
        default { 'Candidate-R2' }
    }
    $controlLabel = if (-not $Control) {
        $null
    } elseif ($Profile -eq 'FrameSnapshot') {
        'FrameSnapshot-Baseline'
    } elseif ($Profile -eq 'DispatchCleanupM3') {
        'DispatchCleanupM3-Baseline'
    } elseif ($Profile -eq 'DispatchSessionRedesignM3') {
        'DispatchSessionRedesignM3-Baseline'
    } elseif ($Profile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) {
        $Profile + '-Baseline'
    } elseif ($Profile -eq 'Scheduler') {
        'C0-Scheduler-Baseline'
    } elseif ([bool](Get-ReportOptionalProperty $Control 'c1_timed_verified')) {
        'C1-TIMED-R2'
    } else {
        'C0'
    }
    $markdownControlLabel = if (-not $Control) {
        'not applicable'
    } elseif ($Profile -eq 'FrameSnapshot') {
        'FrameSnapshot-Baseline'
    } elseif ($Profile -eq 'DispatchCleanupM3') {
        'DispatchCleanupM3-Baseline'
    } elseif ($Profile -eq 'DispatchSessionRedesignM3') {
        'DispatchSessionRedesignM3-Baseline'
    } elseif ($Profile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) {
        $Profile + '-Baseline'
    } elseif ([bool](Get-ReportOptionalProperty $Control 'c1_timed_verified')) {
        'C1-TIMED-R2'
    } else {
        'C0'
    }
    $markdownCandidatePrefix = if ($Profile -eq 'FrameSnapshot') {
        'FrameSnapshot-Candidate '
    } elseif ($Profile -eq 'DispatchCleanupM3') {
        'DispatchCleanupM3-Candidate '
    } elseif ($Profile -eq 'DispatchSessionRedesignM3') {
        'DispatchSessionRedesignM3-Candidate '
    } elseif ($Profile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) {
        $Profile + '-Candidate '
    } else { '' }
    $candidateMargins = [ordered]@{
        performance_timing_margin_us = Get-ReportOptionalProperty $Candidate 'performance_timing_margin_us'
        safety_timing_margin_us = Get-ReportOptionalProperty $Candidate 'safety_timing_margin_us'
    }
    $controlMargins = if ($Control) {
        [ordered]@{
            performance_timing_margin_us = Get-ReportOptionalProperty $Control 'performance_timing_margin_us'
            safety_timing_margin_us = Get-ReportOptionalProperty $Control 'safety_timing_margin_us'
        }
    } else { $null }
    $shortStage = $Stage -in @('Canary', 'Smoke')
    $m3StageSummary = if ($Profile -eq 'DispatchCleanupM3') {
        Get-DispatchCleanupM3StageSummary -Stage $Stage -TimingMarginUs $TimingMarginUs -Pairs $Pairs `
            -SampleSeconds $SampleSeconds -SafetyScenarios $SafetyScenarios -PerformanceWorkloads $PerformanceWorkloads
    } elseif ($Profile -eq 'DispatchSessionRedesignM3') {
        Get-DispatchSessionRedesignM3StageSummary -Stage $Stage -TimingMarginUs $TimingMarginUs -Pairs $Pairs `
            -SampleSeconds $SampleSeconds -SafetyScenarios $SafetyScenarios -PerformanceWorkloads $PerformanceWorkloads
    } elseif ($Profile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) {
        Get-PlaybackCoreLifecycleStageSummary -Stage $Stage -TimingMarginUs $TimingMarginUs -Pairs $Pairs `
            -SampleSeconds $SampleSeconds -SafetyScenarios $SafetyScenarios -PerformanceWorkloads $PerformanceWorkloads `
            -LoadCells @('Quiet', 'Contention') -ProfileVersion $(if ($Profile -eq 'PlaybackCoreLifecycleV2') { 2 } else { 1 })
    } else { $null }
    $summaryParameters = if ($m3StageSummary) {
        [ordered]@{
            stage = $m3StageSummary.stage
            stage_kind = $m3StageSummary.stage_kind
            qualification_profile = $m3StageSummary.qualification_profile
            planned_session_count = $m3StageSummary.planned_session_count
            candidate_session_count = $m3StageSummary.candidate_session_count
            control_session_count = $m3StageSummary.control_session_count
            role_scope = $m3StageSummary.role_scope
            workloads = $m3StageSummary.workloads
            scenarios = $m3StageSummary.scenarios
            workload_summary = $m3StageSummary.workload_summary
            load_cells = $m3StageSummary.load_cells
            game_fps = $m3StageSummary.game_fps
            game_fps_values = $m3StageSummary.game_fps_values
            game_fps_summary = $m3StageSummary.game_fps_summary
            timing_margin_us = $m3StageSummary.timing_margin_us
            timing_margin_values_us = $m3StageSummary.timing_margin_values_us
            timing_margin_summary = $m3StageSummary.timing_margin_summary
            configured_H_us = $m3StageSummary.configured_H_us
            configured_H_mode = $m3StageSummary.configured_H_mode
            configured_H_summary = $m3StageSummary.configured_H_summary
            pairs = $m3StageSummary.pairs
            sample_seconds = $m3StageSummary.timed_performance_sample_seconds
            timed_performance_samples = $m3StageSummary.timed_performance_samples
            sample_scope = $m3StageSummary.sample_scope
            order = $m3StageSummary.pair_order
            performance_timing_margin_us = $candidateMargins.performance_timing_margin_us
            safety_timing_margin_us = $candidateMargins.safety_timing_margin_us
            quiet = 'no benchmark CPU workers'
            contention = 'two hidden pwsh busy workers at BelowNormal priority'
            sink = 'project-owned ReceiveOnly sink, identity verified by PID/HWND/start time/event-log ID'
            receiveonly_observer_mode = 'SchedulerQueuedV3Compat3'
            receiveonly_observer_version = 'scheduler-receiveonly-queue-v3-compat3'
        }
        if ($Profile -eq 'PlaybackCoreLifecycleV2') {
            $summaryParameters['authored_prefix_parity_contract'] = $m3StageSummary.authored_prefix_parity_contract
            $summaryParameters['authored_prefix_measured_window_seconds'] = $m3StageSummary.authored_prefix_measured_window_seconds
        }
    } else {
        [ordered]@{
            pairs = if ($shortStage) { 1 } else { $Pairs }
            sample_seconds = if ($shortStage) { $null } else { $SampleSeconds }
            timing_margin_us = $TimingMarginUs
            performance_timing_margin_us = $candidateMargins.performance_timing_margin_us
            safety_timing_margin_us = $candidateMargins.safety_timing_margin_us
            order = 'alternating AB/BA, configured pairs per cell'
            quiet = 'no benchmark CPU workers'
            contention = 'two hidden pwsh busy workers at BelowNormal priority'
            sink = 'project-owned ReceiveOnly sink, identity verified by PID/HWND/start time/event-log ID'
            receiveonly_observer_mode = if ($Profile -in @('Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) { 'SchedulerQueuedV3Compat3' } else { 'LegacyInline' }
            receiveonly_observer_version = if ($Profile -in @('Scheduler', 'FrameSnapshot', 'DispatchCleanupM3', 'DispatchSessionRedesignM3', 'PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) { 'scheduler-receiveonly-queue-v3-compat3' } else { $null }
        }
    }
    $manifestParameters = if ($m3StageSummary) {
        [ordered]@{
            stage = $m3StageSummary.stage
            stage_kind = $m3StageSummary.stage_kind
            qualification_profile = $m3StageSummary.qualification_profile
            pairs = $m3StageSummary.pairs
            sample_seconds = $m3StageSummary.timed_performance_sample_seconds
            timing_margin_us = $m3StageSummary.timing_margin_us
            performance_timing_margin_us = $candidateMargins.performance_timing_margin_us
            safety_timing_margin_us = $candidateMargins.safety_timing_margin_us
            workload_version = Get-ReportOptionalProperty $Candidate 'workload_version'
            observer_mode = Get-ReportOptionalProperty $Candidate 'observer_mode'
            guard_mode = Get-ReportOptionalProperty $Candidate 'guard_mode'
            quiet_load = 'no extra qualification workers'
            contention_load = 'two BelowNormal CPU workers'
            pair_order = if ($m3StageSummary.pairs -eq $null) { $null } else { $m3StageSummary.pair_order }
            planned_session_count = $m3StageSummary.planned_session_count
            candidate_session_count = $m3StageSummary.candidate_session_count
            control_session_count = $m3StageSummary.control_session_count
            role_scope = $m3StageSummary.role_scope
            workloads = $m3StageSummary.workloads
            scenarios = $m3StageSummary.scenarios
            load_cells = $m3StageSummary.load_cells
            game_fps = $m3StageSummary.game_fps
            game_fps_summary = $m3StageSummary.game_fps_summary
            timing_margin_values_us = $m3StageSummary.timing_margin_values_us
            timing_margin_summary = $m3StageSummary.timing_margin_summary
            configured_H_us = $m3StageSummary.configured_H_us
            configured_H_mode = $m3StageSummary.configured_H_mode
            configured_H_summary = $m3StageSummary.configured_H_summary
            timed_performance_samples = $m3StageSummary.timed_performance_samples
            sample_scope = $m3StageSummary.sample_scope
        }
        if ($Profile -eq 'PlaybackCoreLifecycleV2') {
            $manifestParameters['authored_prefix_parity_contract'] = $m3StageSummary.authored_prefix_parity_contract
            $manifestParameters['authored_prefix_measured_window_seconds'] = $m3StageSummary.authored_prefix_measured_window_seconds
        }
    } else {
        [ordered]@{
            pairs = if ($shortStage) { 1 } else { 6 }
            sample_seconds = if ($shortStage) { $null } else { 15 }
            timing_margin_us = $TimingMarginUs
            performance_timing_margin_us = $candidateMargins.performance_timing_margin_us
            safety_timing_margin_us = $candidateMargins.safety_timing_margin_us
            workload_version = Get-ReportOptionalProperty $Candidate 'workload_version'
            observer_mode = Get-ReportOptionalProperty $Candidate 'observer_mode'
            guard_mode = Get-ReportOptionalProperty $Candidate 'guard_mode'
            quiet_load = 'no extra qualification workers'
            contention_load = 'two BelowNormal CPU workers'
            pair_order = 'alternating AB/BA'
        }
    }

    $markdownStageLines = @()
    if ($m3StageSummary) {
        $m3ProfileName = if ($Profile -eq 'DispatchSessionRedesignM3') { 'DispatchSessionRedesignM3' } elseif ($Profile -in @('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2')) { $Profile } else { 'DispatchCleanupM3' }
        $markdownStageLines = [string[]]@(
            '- Qualification: ' + $m3ProfileName + ' / ' + $m3StageSummary.stage + ' (' + $m3StageSummary.stage_kind + '); workload version ' + $m3StageSummary.workload_version,
            '- Planned sessions: ' + $m3StageSummary.planned_session_count + '; candidate: ' + $m3StageSummary.candidate_session_count + '; baseline/control: ' + $m3StageSummary.control_session_count,
            '- Role scope: ' + $m3StageSummary.role_scope,
            '- Workloads: ' + $m3StageSummary.workload_summary + '; load cells: ' + ($m3StageSummary.load_cells -join ', '),
            '- FPS: ' + $m3StageSummary.game_fps_summary + '; timing margin: ' + $m3StageSummary.timing_margin_summary,
            '- Manifest margins: performance ' + $m3StageSummary.performance_timing_margin_us + ' us; safety ' + $m3StageSummary.safety_timing_margin_us + ' us.',
            '- Configured hold floor: ' + $m3StageSummary.configured_H_summary,
            '- Pairing: ' + $m3StageSummary.pair_order,
            '- Sampling: ' + $m3StageSummary.sample_scope + $(if ($m3StageSummary.timed_performance_sample_seconds) { '; ' + $m3StageSummary.timed_performance_sample_seconds + ' seconds per performance session' } else { '' })
        )
    }

    [pscustomobject]@{
        CandidateQualificationProfile = Get-ReportOptionalProperty $Candidate 'qualification_profile'
        ControlQualificationProfile = Get-ReportOptionalProperty $Control 'qualification_profile'
        CsvRoleLabels = [ordered]@{
            control = if ($Control) { Get-QualificationReportRowLabel -Profile $Profile -Role control -Manifest $Control } else { $null }
            candidate = Get-QualificationReportRowLabel -Profile $Profile -Role candidate -Manifest $Candidate
        }
        CandidateLabel = $candidateLabel
        ControlLabel = $controlLabel
        MarkdownControlLabel = $markdownControlLabel
        MarkdownCandidatePrefix = $markdownCandidatePrefix
        CandidateMargins = $candidateMargins
        ControlMargins = $controlMargins
        StageSummary = $m3StageSummary
        MarkdownStageLines = $markdownStageLines
        SummaryParameters = $summaryParameters
        ManifestParameters = $manifestParameters
    }
}

Export-ModuleMember -Function Get-QualificationReportRowLabel, Get-QualificationReportSchemaVersion, Get-DispatchCleanupM3TimingFloors, Assert-DispatchCleanupM3RunConfigTiming, Get-DispatchCleanupM3StageSummary, Get-DispatchSessionRedesignM3TimingFloors, Assert-DispatchSessionRedesignM3SessionIdentity, Get-PlaybackCoreLifecycleStageSummary, Assert-PlaybackCoreLifecycleSessionIdentity, Get-PlaybackCoreLifecycleMeasuredAuthoredPrefix, Assert-PlaybackCoreLifecyclePairedWorkloadParity, Get-DispatchSessionRedesignM3StageSummary, New-QualificationReportProjection
