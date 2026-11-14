Set-StrictMode -Version Latest

function Get-DispatchReportField {
    param(
        [Parameter(Mandatory)] [object] $Object,
        [Parameter(Mandatory)] [string] $Name
    )

    if ($Object -is [System.Collections.IDictionary]) {
        if ($Object.Contains($Name)) { return $Object[$Name] }
        return $null
    }
    return $Object.PSObject.Properties[$Name]?.Value
}

function Assert-DispatchReportField {
    param(
        [Parameter(Mandatory)] [object] $Object,
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [string] $Path
    )

    $value = Get-DispatchReportField -Object $Object -Name $Name
    if ($null -eq $value) { throw "dispatch lifecycle report missing $Path.$Name" }
    return $value
}

function Assert-DispatchLiveAccounting {
    param([Parameter(Mandatory)] [object] $Branch, [Parameter(Mandatory)] [string] $Path)

    if ((Assert-DispatchReportField $Branch 'generation_accounting_owner' $Path) -cne 'live_scheduler') {
        throw "$Path generation accounting owner must be live_scheduler"
    }
    $stages = Assert-DispatchReportField $Branch 'accounting_stages' $Path
    if ($stages -isnot [System.Collections.IEnumerable] -or $stages -is [string]) {
        throw "$Path.accounting_stages must be an array"
    }
    $stageCount = 0
    foreach ($stage in $stages) {
        $stageCount++
        if ((Assert-DispatchReportField $stage 'generation_accounting_owner' "$Path.accounting_stages") -cne 'live_scheduler') {
            throw "$Path contains mixed generation accounting owners"
        }
        foreach ($name in @('phase', 'activated', 'released', 'cancelled', 'active', 'scheduled', 'total')) {
            [void](Assert-DispatchReportField $stage $name "$Path.accounting_stages")
        }
    }
    if ($stageCount -lt 3) { throw "$Path must include staged accounting evidence" }
}

function Test-DispatchLifecycleReport {
    param([Parameter(Mandatory)] [object] $Report)

    $version = Assert-DispatchReportField $Report 'schema_version' 'report'
    if ([int]$version -eq 1) {
        [void](Assert-DispatchReportField $Report 'source_revision' 'historical v1 report')
        [void](Assert-DispatchReportField $Report 'transport_kind' 'historical v1 report')
        return [pscustomobject]@{ SchemaVersion = 1; EvidenceClass = 'historical' }
    }
    if ([int]$version -ne 2) { throw "unsupported dispatch lifecycle schema_version '$version'" }

    if ((Assert-DispatchReportField $Report 'report_kind' 'report') -cne 'dispatch_lifecycle') {
        throw 'schema v2 report_kind must be dispatch_lifecycle'
    }
    if ((Assert-DispatchReportField $Report 'result' 'report') -cne 'PASS') {
        throw 'schema v2 lifecycle result must be PASS'
    }

    $provenance = Assert-DispatchReportField $Report 'provenance' 'report'
    foreach ($name in @('source_sha', 'source_tree_sha')) {
        $value = [string](Assert-DispatchReportField $provenance $name 'provenance')
        if ($value -notmatch '^[0-9a-fA-F]{40}$') { throw "provenance.$name must be a full Git object ID" }
    }
    if ((Assert-DispatchReportField $provenance 'execution' 'provenance') -cne 'offline_test_support') {
        throw 'schema v2 execution must identify offline test-support provenance'
    }
    if ((Assert-DispatchReportField $provenance 'transport_kind' 'provenance') -cne 'deterministic_mock') {
        throw 'schema v2 transport must be deterministic_mock'
    }
    if ([long](Assert-DispatchReportField $provenance 'native_sendinput_calls' 'provenance') -ne 0) {
        throw 'offline lifecycle report must record zero native SendInput calls'
    }

    $timing = Assert-DispatchReportField $Report 'timing' 'report'
    foreach ($name in @('fps', 'frame_us', 'hold_frames', 'timing_margin_us', 'min_hold_us', 'min_release_gap_us', 'qpc_frequency_hz', 'min_release_gap_ticks')) {
        [void](Assert-DispatchReportField $timing $name 'timing')
    }
    $fps = [long]$timing.fps
    $frameUs = [long]$timing.frame_us
    $holdFrames = [long]$timing.hold_frames
    $marginUs = [long]$timing.timing_margin_us
    $holdUs = [long]$timing.min_hold_us
    $gapUs = [long]$timing.min_release_gap_us
    $frequency = [System.Numerics.BigInteger]::Parse([string]$timing.qpc_frequency_hz)
    if ($fps -le 0 -or $holdFrames -le 0 -or $marginUs -lt 0 -or $frequency -le 0) {
        throw 'timing values must be positive and margin nonnegative'
    }
    $expectedFrameUs = [long][math]::Ceiling(1000000.0 / $fps)
    $expectedHoldUs = $holdFrames * $expectedFrameUs + $marginUs
    $expectedGapUs = $expectedFrameUs + $marginUs
    if ($frameUs -ne $expectedFrameUs -or $holdUs -ne $expectedHoldUs -or $gapUs -ne $expectedGapUs) {
        throw 'timing schedule does not match the materialized T/H/G contract'
    }
    $expectedTicks = [System.Numerics.BigInteger]::Divide(
        ($frequency * [System.Numerics.BigInteger]$gapUs + 999999),
        1000000
    )
    if ([System.Numerics.BigInteger]::Parse([string]$timing.min_release_gap_ticks) -ne $expectedTicks) {
        throw 'timing.min_release_gap_ticks does not equal ceil(QPC frequency × G_us / 1,000,000)'
    }

    $live = Assert-DispatchReportField $Report 'live_scheduler' 'report'
    if ((Assert-DispatchReportField $live 'generation_accounting_owner' 'live_scheduler') -cne 'live_scheduler') {
        throw 'top-level generation accounting owner must be live_scheduler'
    }
    $normal = Assert-DispatchReportField $live 'no_pause_control' 'live_scheduler'
    if ([bool](Assert-DispatchReportField $normal 'pause_requested' 'no_pause_control')) {
        throw 'no_pause_control must be an actual no-pause negative control'
    }
    if ([long](Assert-DispatchReportField $normal 'sender_attempts' 'no_pause_control') -ne 4 -or
        [long](Assert-DispatchReportField $normal 'final_release_obligation_mask' 'no_pause_control') -ne 0 -or
        -not [bool](Assert-DispatchReportField $normal 'passed' 'no_pause_control')) {
        throw 'no_pause_control sender/obligation result is incomplete'
    }
    Assert-DispatchLiveAccounting -Branch $normal -Path 'no_pause_control'
    $normalFinal = @($normal.accounting_stages | Where-Object { $_.phase -eq 'after_final_up' })
    if ($normalFinal.Count -ne 1 -or [long]$normalFinal[0].activated -ne 2 -or
        [long]$normalFinal[0].released -ne 2 -or [long]$normalFinal[0].active -ne 0 -or
        [long]$normalFinal[0].cancelled -ne 0) {
        throw 'no_pause_control must finish two live scheduler generations as 2 activated / 2 released'
    }

    $paused = Assert-DispatchReportField $live 'pause_cleanup_resume' 'live_scheduler'
    if (-not [bool](Assert-DispatchReportField $paused 'pause_requested' 'pause_cleanup_resume') -or
        -not [bool](Assert-DispatchReportField $paused 'passed' 'pause_cleanup_resume') -or
        [long](Assert-DispatchReportField $paused 'sender_attempts' 'pause_cleanup_resume') -ne 2 -or
        [long](Assert-DispatchReportField $paused 'final_release_obligation_mask' 'pause_cleanup_resume') -ne 0) {
        throw 'pause_cleanup_resume does not prove pause, cleanup and resumed sender completion'
    }
    Assert-DispatchLiveAccounting -Branch $paused -Path 'pause_cleanup_resume'
    if ([long]$paused.min_release_gap_us -ne $gapUs -or
        [long]$paused.min_release_gap_ticks -ne [long]$timing.min_release_gap_ticks -or
        [long]$paused.resume_barrier_qpc_ticks -ne ([long]$paused.cleanup_completion_qpc_ticks + [long]$paused.min_release_gap_ticks) -or
        [long]$paused.candidate_packet_not_before_qpc_ticks -ne [long]$paused.resume_barrier_qpc_ticks -or
        [string]$paused.g_minus_one_decision -cne 'wait' -or
        [string]$paused.exact_g_decision -cne 'send') {
        throw 'pause_cleanup_resume does not preserve the verified completion + G barrier and G-1/G decisions'
    }

    return [pscustomobject]@{ SchemaVersion = 2; EvidenceClass = 'live_scheduler' }
}
