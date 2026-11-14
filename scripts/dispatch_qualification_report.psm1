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
    if ($Profile -eq 'Scheduler') {
        if ($Role -eq 'control') { return 'Scheduler-Baseline' }
        return 'Scheduler-Candidate'
    }
    if ([bool](Get-ReportOptionalProperty $Manifest 'c1_timed_verified')) { return 'C1-TIMED-R2' }
    if ($Role -eq 'candidate') { return 'Candidate-R2' }
    'C0'
}

function New-QualificationReportProjection {
    param(
        [Parameter(Mandatory)] [ValidateSet('DispatchCore', 'Scheduler', 'FrameSnapshot')] [string]$Profile,
        [Parameter(Mandatory)] $Candidate,
        $Control,
        [Parameter(Mandatory)] [ValidateSet('Canary', 'Smoke', 'Compare', 'Final')] [string]$Stage,
        [Parameter(Mandatory)] [int]$Pairs,
        [Parameter(Mandatory)] [int]$SampleSeconds,
        [Parameter(Mandatory)] [int]$TimingMarginUs
    )

    $candidateLabel = switch ($Profile) {
        'FrameSnapshot' { 'FrameSnapshot-Candidate' }
        'Scheduler' { 'Scheduler-Candidate' }
        default { 'Candidate-R2' }
    }
    $controlLabel = if (-not $Control) {
        $null
    } elseif ($Profile -eq 'FrameSnapshot') {
        'FrameSnapshot-Baseline'
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
    } elseif ([bool](Get-ReportOptionalProperty $Control 'c1_timed_verified')) {
        'C1-TIMED-R2'
    } else {
        'C0'
    }
    $markdownCandidatePrefix = if ($Profile -eq 'FrameSnapshot') {
        'FrameSnapshot-Candidate '
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
    $summaryParameters = [ordered]@{
        pairs = if ($shortStage) { 1 } else { $Pairs }
        sample_seconds = if ($shortStage) { $null } else { $SampleSeconds }
        timing_margin_us = $TimingMarginUs
        performance_timing_margin_us = $candidateMargins.performance_timing_margin_us
        safety_timing_margin_us = $candidateMargins.safety_timing_margin_us
        order = 'alternating AB/BA, three pairs each per cell'
        quiet = 'no benchmark CPU workers'
        contention = 'two hidden pwsh busy workers at BelowNormal priority'
        sink = 'project-owned ReceiveOnly sink, identity verified by PID/HWND/start time/event-log ID'
        receiveonly_observer_mode = if ($Profile -in @('Scheduler', 'FrameSnapshot')) { 'SchedulerQueuedV3Compat3' } else { 'LegacyInline' }
        receiveonly_observer_version = if ($Profile -in @('Scheduler', 'FrameSnapshot')) { 'scheduler-receiveonly-queue-v3-compat3' } else { $null }
    }
    $manifestParameters = [ordered]@{
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

    [pscustomobject]@{
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
        SummaryParameters = $summaryParameters
        ManifestParameters = $manifestParameters
    }
}

Export-ModuleMember -Function Get-QualificationReportRowLabel, New-QualificationReportProjection
