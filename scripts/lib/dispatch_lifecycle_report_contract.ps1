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

function ConvertTo-DispatchInteger {
    param(
        [Parameter(Mandatory)] [AllowNull()] [object] $Value,
        [Parameter(Mandatory)] [string] $Path
    )

    if ($null -eq $Value -or $Value -is [bool]) {
        throw "$Path must be an integer"
    }
    try {
        $number = [decimal]::Parse(
            [string]$Value,
            [System.Globalization.NumberStyles]::Float,
            [System.Globalization.CultureInfo]::InvariantCulture
        )
    }
    catch {
        throw "$Path must be an integer"
    }
    if ($number -ne [decimal]::Truncate($number)) { throw "$Path must be an integer" }
    return [System.Numerics.BigInteger]::Parse(
        $number.ToString('0', [System.Globalization.CultureInfo]::InvariantCulture),
        [System.Globalization.CultureInfo]::InvariantCulture
    )
}

function Assert-DispatchReportSourceIdentity {
    param(
        [Parameter(Mandatory)] [object] $Report,
        [Parameter(Mandatory)] [string] $SourceSha,
        [Parameter(Mandatory)] [string] $SourceTreeSha
    )

    $provenance = Assert-DispatchReportField $Report 'provenance' 'report'
    if ([string](Assert-DispatchReportField $provenance 'source_sha' 'provenance') -cne $SourceSha -or
        [string](Assert-DispatchReportField $provenance 'source_tree_sha' 'provenance') -cne $SourceTreeSha) {
        throw 'lifecycle report provenance does not match the clean source checkout used by the runner'
    }
}

function Assert-DispatchLiveAccounting {
    param(
        [Parameter(Mandatory)] [object] $Branch,
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [object[]] $ExpectedStages
    )

    if ((Assert-DispatchReportField $Branch 'generation_accounting_owner' $Path) -cne 'live_scheduler') {
        throw "$Path generation accounting owner must be live_scheduler"
    }
    $stages = Assert-DispatchReportField $Branch 'accounting_stages' $Path
    if ($stages -isnot [System.Collections.IEnumerable] -or $stages -is [string]) {
        throw "$Path.accounting_stages must be an array"
    }
    $actualStages = @($stages)
    if ($actualStages.Count -ne $ExpectedStages.Count) {
        throw "$Path must contain exactly $($ExpectedStages.Count) ordered accounting phases"
    }

    $accountingFields = @('total', 'scheduled', 'activated', 'released', 'cancelled', 'active')
    for ($index = 0; $index -lt $ExpectedStages.Count; $index++) {
        $stage = $actualStages[$index]
        $stagePath = "$Path.accounting_stages[$index]"
        if ((Assert-DispatchReportField $stage 'generation_accounting_owner' $stagePath) -cne 'live_scheduler') {
            throw "$stagePath has a non-live generation accounting owner"
        }
        $phase = [string](Assert-DispatchReportField $stage 'phase' $stagePath)
        if ($phase -cne [string]$ExpectedStages[$index].phase) {
            throw "$Path accounting phase $index must be '$($ExpectedStages[$index].phase)', found '$phase'"
        }

        $values = @{}
        foreach ($name in $accountingFields) {
            $values[$name] = ConvertTo-DispatchInteger `
                -Value (Assert-DispatchReportField $stage $name $stagePath) `
                -Path "$stagePath.$name"
            if ($values[$name] -lt 0) { throw "$stagePath.$name must be nonnegative" }
            $expected = [System.Numerics.BigInteger]$ExpectedStages[$index][$name]
            if ($values[$name] -ne $expected) {
                throw "$stagePath.$name expected $expected, found $($values[$name])"
            }
        }
        if ($values.total -ne ($values.scheduled + $values.activated)) {
            throw "$stagePath violates total = scheduled + activated"
        }
        if ($values.activated -ne ($values.active + $values.released + $values.cancelled)) {
            throw "$stagePath violates activated = active + released + cancelled"
        }
    }
}

function Assert-DispatchPacketEvidence {
    param(
        [Parameter(Mandatory)] [object] $Branch,
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string[]] $ExpectedMasks
    )

    $packets = Assert-DispatchReportField $Branch 'packets' $Path
    if ($packets -isnot [System.Collections.IEnumerable] -or $packets -is [string]) {
        throw "$Path.packets must be an array of captured production sends"
    }
    $actualPackets = @($packets)
    if ($actualPackets.Count -ne $ExpectedMasks.Count) {
        throw "$Path must capture exactly $($ExpectedMasks.Count) musical sender attempts"
    }
    $validatedPackets = [System.Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt $ExpectedMasks.Count; $index++) {
        $packet = $actualPackets[$index]
        $packetPath = "$Path.packets[$index]"
        $masks = $ExpectedMasks[$index].Split(':')
        $expectedUp = [System.Numerics.BigInteger]::Parse($masks[0])
        $expectedDown = [System.Numerics.BigInteger]::Parse($masks[1])
        $up = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'up_mask' $packetPath) "$packetPath.up_mask"
        $down = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'down_mask' $packetPath) "$packetPath.down_mask"
        $requested = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'requested_mask' $packetPath) "$packetPath.requested_mask"
        $confirmed = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'confirmed_mask' $packetPath) "$packetPath.confirmed_mask"
        $inserted = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'first_inserted' $packetPath) "$packetPath.first_inserted"
        $attempts = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'attempts' $packetPath) "$packetPath.attempts"
        $started = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'started_qpc_ticks' $packetPath) "$packetPath.started_qpc_ticks"
        $completed = ConvertTo-DispatchInteger (Assert-DispatchReportField $packet 'completed_qpc_ticks' $packetPath) "$packetPath.completed_qpc_ticks"
        $expectedMask = $expectedUp -bor $expectedDown
        if ($up -ne $expectedUp -or $down -ne $expectedDown -or
            $requested -ne $expectedMask -or $confirmed -ne $expectedMask -or
            $inserted -ne 1 -or $attempts -ne 1 -or $started -le 0 -or
            $completed -lt $started -or
            (Assert-DispatchReportField $packet 'worker_outcome' $packetPath) -cne 'dispatched') {
            throw "$packetPath does not prove the expected dispatched packet and completed mock transport"
        }
        [void]$validatedPackets.Add([pscustomobject]@{
            UpMask = $up
            DownMask = $down
            Started = $started
            Completed = $completed
        })
    }

    $senderAttempts = ConvertTo-DispatchInteger `
        (Assert-DispatchReportField $Branch 'sender_attempts' $Path) "$Path.sender_attempts"
    $musicalAttempts = ConvertTo-DispatchInteger `
        (Assert-DispatchReportField $Branch 'musical_sender_attempts' $Path) "$Path.musical_sender_attempts"
    if ($senderAttempts -ne $ExpectedMasks.Count -or $musicalAttempts -ne $senderAttempts) {
        throw "$Path sender counters must describe only captured musical packet sends"
    }
    return $validatedPackets.ToArray()
}

function Assert-DispatchPacketTimingFloor {
    param(
        [Parameter(Mandatory)] [object] $PreviousPacket,
        [Parameter(Mandatory)] [object] $NextPacket,
        [Parameter(Mandatory)] [System.Numerics.BigInteger] $FloorTicks,
        [Parameter(Mandatory)] [string] $Path
    )

    $earliestStart = $PreviousPacket.Completed + $FloorTicks
    if ($NextPacket.Started -lt $earliestStart) {
        throw "$Path packet started before the previous completion + floor ($FloorTicks QPC ticks)"
    }
}

function Assert-DispatchCleanupCapture {
    param(
        [Parameter(Mandatory)] [object] $Cleanup,
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [long] $ExpectedScanCode
    )

    $packets = Assert-DispatchReportField $Cleanup 'packets' $Path
    if ($packets -isnot [System.Collections.IEnumerable] -or $packets -is [string]) {
        throw "$Path.packets must be an array of mock cleanup captures"
    }
    $actualPackets = @($packets)
    $upEvents = [System.Numerics.BigInteger]::Zero
    $downEvents = [System.Numerics.BigInteger]::Zero
    $upAttempts = 0
    $downAttempts = 0
    foreach ($packet in $actualPackets) {
        $keyUp = [bool](Assert-DispatchReportField $packet 'key_up' $Path)
        if ($packet -is [System.Collections.IDictionary]) {
            $scanCodes = $packet['scan_codes']
        }
        else {
            $scanCodes = $packet.PSObject.Properties['scan_codes']?.Value
        }
        if ($null -eq $scanCodes) { throw "$Path missing scan_codes" }
        if ($scanCodes -isnot [System.Collections.IEnumerable] -or $scanCodes -is [string]) {
            throw "$Path scan_codes must be an array"
        }
        $eventCount = @($scanCodes).Count
        if ($eventCount -le 0) { throw "$Path captured an empty cleanup packet" }
        $scanCodeValues = @($scanCodes)
        if ($eventCount -ne 1 -or
            (ConvertTo-DispatchInteger $scanCodeValues[0] "$Path.scan_codes[0]") -ne $ExpectedScanCode) {
            throw "$Path must capture the fixture slot scan code 0x$($ExpectedScanCode.ToString('X2')) exactly once"
        }
        if ($keyUp) {
            $upAttempts++
            $upEvents += $eventCount
        }
        else {
            $downAttempts++
            $downEvents += $eventCount
        }
    }

    if ((ConvertTo-DispatchInteger (Assert-DispatchReportField $Cleanup 'attempts' $Path) "$Path.attempts") -ne $actualPackets.Count -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $Cleanup 'up_attempts' $Path) "$Path.up_attempts") -ne $upAttempts -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $Cleanup 'down_attempts' $Path) "$Path.down_attempts") -ne $downAttempts -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $Cleanup 'up_events' $Path) "$Path.up_events") -ne $upEvents -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $Cleanup 'down_events' $Path) "$Path.down_events") -ne $downEvents -or
        $downAttempts -ne 0 -or $upAttempts -le 0) {
        throw "$Path cleanup capture must account for Up-only attempts and no Down events"
    }
}

function Test-DispatchLifecycleReport {
    param([Parameter(Mandatory)] [object] $Report)

    $version = ConvertTo-DispatchInteger (Assert-DispatchReportField $Report 'schema_version' 'report') 'schema_version'
    if ($version -eq 1) {
        [void](Assert-DispatchReportField $Report 'source_revision' 'historical v1 report')
        [void](Assert-DispatchReportField $Report 'transport_kind' 'historical v1 report')
        return [pscustomobject]@{ SchemaVersion = 1; EvidenceClass = 'historical' }
    }
    if ($version -ne 2) { throw "unsupported dispatch lifecycle schema_version '$version'" }

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
    if ((ConvertTo-DispatchInteger (Assert-DispatchReportField $provenance 'native_sendinput_calls' 'provenance') 'provenance.native_sendinput_calls') -ne 0) {
        throw 'offline lifecycle report must record zero native SendInput calls'
    }

    $timing = Assert-DispatchReportField $Report 'timing' 'report'
    foreach ($name in @('fps', 'frame_us', 'hold_frames', 'timing_margin_us', 'min_hold_us', 'min_release_gap_us', 'qpc_frequency_hz', 'min_release_gap_ticks')) {
        [void](Assert-DispatchReportField $timing $name 'timing')
    }
    $fps = ConvertTo-DispatchInteger $timing.fps 'timing.fps'
    $frameUs = ConvertTo-DispatchInteger $timing.frame_us 'timing.frame_us'
    $holdFrames = ConvertTo-DispatchInteger $timing.hold_frames 'timing.hold_frames'
    $marginUs = ConvertTo-DispatchInteger $timing.timing_margin_us 'timing.timing_margin_us'
    $holdUs = ConvertTo-DispatchInteger $timing.min_hold_us 'timing.min_hold_us'
    $gapUs = ConvertTo-DispatchInteger $timing.min_release_gap_us 'timing.min_release_gap_us'
    $frequency = ConvertTo-DispatchInteger $timing.qpc_frequency_hz 'timing.qpc_frequency_hz'
    $gapTicks = ConvertTo-DispatchInteger $timing.min_release_gap_ticks 'timing.min_release_gap_ticks'
    if ($fps -le 0 -or $holdFrames -ne 1 -or $marginUs -lt 0 -or $frequency -le 0) {
        throw 'timing values must be positive with nonnegative margin and schema-v2 hold_frames exactly 1'
    }
    $expectedFrameUs = [System.Numerics.BigInteger]::Divide((1000000 + $fps - 1), $fps)
    $expectedHoldUs = $holdFrames * $expectedFrameUs + $marginUs
    $expectedGapUs = $expectedFrameUs + $marginUs
    if ($frameUs -ne $expectedFrameUs -or $holdUs -ne $expectedHoldUs -or $gapUs -ne $expectedGapUs) {
        throw 'timing schedule does not match the materialized T/H/G contract'
    }
    $expectedTicks = [System.Numerics.BigInteger]::Divide(
        ($frequency * $gapUs + 999999),
        1000000
    )
    $holdTicks = [System.Numerics.BigInteger]::Divide(
        ($frequency * $holdUs + 999999),
        1000000
    )
    if ($gapTicks -ne $expectedTicks) {
        throw 'timing.min_release_gap_ticks does not equal ceil(QPC frequency × G_us / 1,000,000)'
    }

    $live = Assert-DispatchReportField $Report 'live_scheduler' 'report'
    if ((Assert-DispatchReportField $live 'generation_accounting_owner' 'live_scheduler') -cne 'live_scheduler') {
        throw 'top-level generation accounting owner must be live_scheduler'
    }
    $normal = Assert-DispatchReportField $live 'no_pause_control' 'live_scheduler'
    if ([bool](Assert-DispatchReportField $normal 'pause_requested' 'no_pause_control') -or
        -not [bool](Assert-DispatchReportField $normal 'passed' 'no_pause_control') -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $normal 'final_release_obligation_mask' 'no_pause_control') 'no_pause_control.final_release_obligation_mask') -ne 0 -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $normal 'cleanup_sender_attempts' 'no_pause_control') 'no_pause_control.cleanup_sender_attempts') -ne 0 -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $normal 'total_mock_send_attempts' 'no_pause_control') 'no_pause_control.total_mock_send_attempts') -ne 4) {
        throw 'no_pause_control must pass without pause and finish with zero release obligation'
    }
    Assert-DispatchLiveAccounting -Branch $normal -Path 'no_pause_control' -ExpectedStages @(
        @{ phase = 'before_first_down'; total = 2; scheduled = 2; activated = 0; released = 0; cancelled = 0; active = 0 },
        @{ phase = 'after_first_down'; total = 2; scheduled = 1; activated = 1; released = 0; cancelled = 0; active = 1 },
        @{ phase = 'after_first_up'; total = 2; scheduled = 1; activated = 1; released = 1; cancelled = 0; active = 0 },
        @{ phase = 'after_second_down'; total = 2; scheduled = 0; activated = 2; released = 1; cancelled = 0; active = 1 },
        @{ phase = 'after_final_up'; total = 2; scheduled = 0; activated = 2; released = 2; cancelled = 0; active = 0 }
    )
    $normalPackets = @(Assert-DispatchPacketEvidence -Branch $normal -Path 'no_pause_control' -ExpectedMasks @('0:1', '1:0', '0:1', '1:0'))
    Assert-DispatchPacketTimingFloor -PreviousPacket $normalPackets[0] -NextPacket $normalPackets[1] -FloorTicks $holdTicks -Path 'no_pause_control Down→Up H'
    Assert-DispatchPacketTimingFloor -PreviousPacket $normalPackets[1] -NextPacket $normalPackets[2] -FloorTicks $gapTicks -Path 'no_pause_control Up→Down G'
    Assert-DispatchPacketTimingFloor -PreviousPacket $normalPackets[2] -NextPacket $normalPackets[3] -FloorTicks $holdTicks -Path 'no_pause_control Down→Up H (second generation)'

    $paused = Assert-DispatchReportField $live 'pause_cleanup_resume' 'live_scheduler'
    if (-not [bool](Assert-DispatchReportField $paused 'pause_requested' 'pause_cleanup_resume') -or
        -not [bool](Assert-DispatchReportField $paused 'passed' 'pause_cleanup_resume') -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'final_release_obligation_mask' 'pause_cleanup_resume') 'pause_cleanup_resume.final_release_obligation_mask') -ne 0) {
        throw 'pause_cleanup_resume must prove pause, cleanup, resumed send and zero final obligation'
    }
    Assert-DispatchLiveAccounting -Branch $paused -Path 'pause_cleanup_resume' -ExpectedStages @(
        @{ phase = 'before_first_down'; total = 2; scheduled = 2; activated = 0; released = 0; cancelled = 0; active = 0 },
        @{ phase = 'after_initial_down'; total = 2; scheduled = 1; activated = 1; released = 0; cancelled = 0; active = 1 },
        @{ phase = 'after_pause_cleanup'; total = 2; scheduled = 1; activated = 1; released = 0; cancelled = 1; active = 0 },
        @{ phase = 'after_resumed_down'; total = 2; scheduled = 0; activated = 2; released = 0; cancelled = 1; active = 1 },
        @{ phase = 'after_final_cleanup'; total = 2; scheduled = 0; activated = 2; released = 0; cancelled = 2; active = 0 }
    )
    $pausedPackets = @(Assert-DispatchPacketEvidence -Branch $paused -Path 'pause_cleanup_resume' -ExpectedMasks @('0:1', '0:1'))

    $cleanup = Assert-DispatchReportField $paused 'cleanup_captures' 'pause_cleanup_resume'
    $pauseCleanup = Assert-DispatchReportField $cleanup 'pause' 'cleanup_captures'
    $finalCleanup = Assert-DispatchReportField $cleanup 'final' 'cleanup_captures'
    Assert-DispatchCleanupCapture -Cleanup $pauseCleanup -Path 'pause_cleanup_resume.cleanup_captures.pause' -ExpectedScanCode 21
    Assert-DispatchCleanupCapture -Cleanup $finalCleanup -Path 'pause_cleanup_resume.cleanup_captures.final' -ExpectedScanCode 21
    $cleanupAttempts = ConvertTo-DispatchInteger (Assert-DispatchReportField $cleanup 'total_attempts' 'cleanup_captures') 'cleanup_captures.total_attempts'
    $pauseCaptureCompletion = ConvertTo-DispatchInteger (Assert-DispatchReportField $pauseCleanup 'completion_qpc_ticks' 'cleanup_captures.pause') 'cleanup_captures.pause.completion_qpc_ticks'
    $finalCaptureCompletion = ConvertTo-DispatchInteger (Assert-DispatchReportField $finalCleanup 'completion_qpc_ticks' 'cleanup_captures.final') 'cleanup_captures.final.completion_qpc_ticks'
    if ($cleanupAttempts -ne 2 -or
        (ConvertTo-DispatchInteger $pauseCleanup.up_attempts 'cleanup_captures.pause.up_attempts') -ne 1 -or
        (ConvertTo-DispatchInteger $finalCleanup.up_attempts 'cleanup_captures.final.up_attempts') -ne 1 -or
        (ConvertTo-DispatchInteger $pauseCleanup.up_events 'cleanup_captures.pause.up_events') -ne 1 -or
        (ConvertTo-DispatchInteger $finalCleanup.up_events 'cleanup_captures.final.up_events') -ne 1 -or
        $pauseCaptureCompletion -le 0 -or $finalCaptureCompletion -le 0 -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'cleanup_sender_attempts' 'pause_cleanup_resume') 'pause_cleanup_resume.cleanup_sender_attempts') -ne $cleanupAttempts -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'total_mock_send_attempts' 'pause_cleanup_resume') 'pause_cleanup_resume.total_mock_send_attempts') -ne 4) {
        throw 'pause_cleanup_resume must distinguish two captured cleanup Up attempts from two musical sends'
    }

    $cleanupQpc = ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'cleanup_completion_qpc_ticks' 'pause_cleanup_resume') 'pause_cleanup_resume.cleanup_completion_qpc_ticks'
    $resumeBarrier = ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'resume_barrier_qpc_ticks' 'pause_cleanup_resume') 'pause_cleanup_resume.resume_barrier_qpc_ticks'
    $candidateNotBefore = ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'candidate_packet_not_before_qpc_ticks' 'pause_cleanup_resume') 'pause_cleanup_resume.candidate_packet_not_before_qpc_ticks'
    $senderStart = ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'resumed_sender_start_qpc_ticks' 'pause_cleanup_resume') 'pause_cleanup_resume.resumed_sender_start_qpc_ticks'
    if ($cleanupQpc -le 0 -or $pauseCaptureCompletion -ne $cleanupQpc -or
        $pausedPackets[0].Completed -gt $pauseCaptureCompletion -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'min_release_gap_us' 'pause_cleanup_resume') 'pause_cleanup_resume.min_release_gap_us') -ne $gapUs -or
        (ConvertTo-DispatchInteger (Assert-DispatchReportField $paused 'min_release_gap_ticks' 'pause_cleanup_resume') 'pause_cleanup_resume.min_release_gap_ticks') -ne $gapTicks -or
        $resumeBarrier -ne ($cleanupQpc + $gapTicks) -or
        $candidateNotBefore -ne $resumeBarrier -or
        $senderStart -ne $pausedPackets[1].Started -or
        $pausedPackets[1].Started -lt $resumeBarrier -or
        $finalCaptureCompletion -lt $pausedPackets[1].Completed -or
        [string](Assert-DispatchReportField $paused 'g_minus_one_decision' 'pause_cleanup_resume') -cne 'wait' -or
        [string](Assert-DispatchReportField $paused 'exact_g_decision' 'pause_cleanup_resume') -cne 'send') {
        throw 'pause_cleanup_resume does not prove cleanup completion + G, G-1 wait, exact-G send and sender start at/after the barrier'
    }

    return [pscustomobject]@{ SchemaVersion = 2; EvidenceClass = 'live_scheduler' }
}
