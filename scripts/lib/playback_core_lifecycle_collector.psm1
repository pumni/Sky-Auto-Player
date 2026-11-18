function Get-PclCollectorProperty {
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

function Get-PclNearestRankSummary {
    param([double[]]$Values)
    $ordered = @($Values | Sort-Object)
    if ($ordered.Count -eq 0) {
        return [ordered]@{ sample_count = 0; p50 = $null; p95 = $null; p99 = $null; max = $null }
    }
    $pick = {
        param([double]$Percentile)
        $rank = [Math]::Max(1, [int][Math]::Ceiling(($Percentile / 100.0) * $ordered.Count))
        [double]$ordered[$rank - 1]
    }
    [ordered]@{
        sample_count = $ordered.Count
        p50 = & $pick 50
        p95 = & $pick 95
        p99 = & $pick 99
        max = [double]$ordered[-1]
    }
}

function Assert-PclReceiveOnlyTarget {
    param($Target, [string]$RunId)
    $gameTarget = Get-PclCollectorProperty $Target 'game_target'
    if ($null -eq $Target -or
        (Get-PclCollectorProperty $Target 'role') -ne 'ReceiveOnly' -or
        (Get-PclCollectorProperty $Target 'sink_kind') -ne 'SkyAutoPlayer.NativeAcceptanceSink' -or
        (Get-PclCollectorProperty $Target 'target_class') -ne 'project-owned-non-game-receive-only' -or
        (Get-PclCollectorProperty $Target 'target_name') -ne 'Sky Auto Player — Native Acceptance Sink' -or
        $null -eq $gameTarget -or $gameTarget -isnot [bool] -or $gameTarget -ne $false -or
        (Get-PclCollectorProperty $Target 'title') -ne 'Sky Auto Player — Native Acceptance Sink' -or
        (Get-PclCollectorProperty $Target 'process') -ne 'native_acceptance_sink.ps1' -or
        (Get-PclCollectorProperty $Target 'input_policy') -ne 'receives_only; benchmark must use SendInput' -or
        [long](Get-PclCollectorProperty $Target 'pid') -le 0 -or
        [long](Get-PclCollectorProperty $Target 'hwnd') -le 0 -or
        [long](Get-PclCollectorProperty $Target 'process_start_time_filetime') -le 0 -or
        [string]::IsNullOrWhiteSpace([string](Get-PclCollectorProperty $Target 'event_log_id'))) {
        throw ('PlaybackCoreLifecycle target identity is not the exact project-owned non-game ReceiveOnly sink: ' + $RunId)
    }
}

function New-PlaybackCoreLifecycleMeasurementReport {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [object[]]$RunRows,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-f]{40}$')] [string]$CandidateRevision,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-f]{40}$')] [string]$CandidateTree,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-f]{64}$')] [string]$CandidateManifestSha256,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-f]{64}$')] [string]$CandidateBinarySha256,
        [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string]$RunnerMachineName,
        [Parameter(Mandatory)] [ValidateNotNullOrEmpty()] [string]$RunnerOsBootTimeUtc
    )

    if ($RunRows.Count -eq 0) {
        return [ordered]@{
            schema_version = 1
            status = 'INCOMPLETE'
            source_identity = [ordered]@{ revision = $CandidateRevision; tree = $CandidateTree; manifest_sha256 = $CandidateManifestSha256; binary_sha256 = $CandidateBinarySha256 }
            target_identity = 'project-owned-non-game-receive-only; native capture not performed by this collector'
            stop_to_verified_release_us = [ordered]@{ status = 'INCOMPLETE'; sample_count = 0; p50 = $null; p95 = $null; p99 = $null; max = $null; samples = @() }
            worker_joined_to_next_start_us = [ordered]@{ status = 'INCOMPLETE'; sample_count = 0; p50 = $null; p95 = $null; p99 = $null; max = $null; samples = @() }
            raw_up_call_count = [ordered]@{ status = 'INCOMPLETE'; sample_count = 0; p50 = $null; p95 = $null; p99 = $null; max = $null; samples = @() }
            incompleteness = @('no qualification rows were supplied')
        }
    }

    $orderedRows = @($RunRows | Sort-Object { [int](Get-PclCollectorProperty $_ 'sequence_index') })
    $candidateRows = @($orderedRows | Where-Object { (Get-PclCollectorProperty $_ 'role') -eq 'candidate' })
    $allTargets = @{}
    foreach ($row in $orderedRows) {
        $runId = [string](Get-PclCollectorProperty $row 'run_id')
        if ([string]::IsNullOrWhiteSpace($runId)) { throw 'PlaybackCoreLifecycle collector row is missing run_id' }
        $target = Get-PclCollectorProperty $row 'target_identity'
        Assert-PclReceiveOnlyTarget -Target $target -RunId $runId
        $machineName = [string](Get-PclCollectorProperty $row 'machine_name')
        $bootTime = [string](Get-PclCollectorProperty $row 'os_boot_time_utc')
        if ($machineName -ne $RunnerMachineName -or $bootTime -ne $RunnerOsBootTimeUtc) {
            throw ('PlaybackCoreLifecycle row machine/boot identity differs from this collector run: ' + $runId)
        }
        $allTargets[$runId] = [ordered]@{
            target_class = Get-PclCollectorProperty $target 'target_class'
            target_name = Get-PclCollectorProperty $target 'target_name'
            game_target = [bool](Get-PclCollectorProperty $target 'game_target')
            pid = [long](Get-PclCollectorProperty $target 'pid')
            hwnd = [long](Get-PclCollectorProperty $target 'hwnd')
            process_start_time_filetime = [long](Get-PclCollectorProperty $target 'process_start_time_filetime')
            event_log_id = [string](Get-PclCollectorProperty $target 'event_log_id')
        }
    }

    foreach ($row in $candidateRows) {
        $runId = [string](Get-PclCollectorProperty $row 'run_id')
        if ((Get-PclCollectorProperty $row 'source_revision') -ne $CandidateRevision -or
            (Get-PclCollectorProperty $row 'source_tree_sha') -ne $CandidateTree -or
            (Get-PclCollectorProperty $row 'manifest_sha256') -ne $CandidateManifestSha256 -or
            (Get-PclCollectorProperty $row 'harness_sha256') -ne $CandidateBinarySha256) {
            throw ('PlaybackCoreLifecycle candidate row source/manifest/binary identity mismatch: ' + $runId)
        }
    }

    $incompleteness = [System.Collections.Generic.List[string]]::new()
    $stopSamples = [System.Collections.Generic.List[object]]::new()
    $rawUpSamples = [System.Collections.Generic.List[object]]::new()
    foreach ($row in @($candidateRows | Where-Object { (Get-PclCollectorProperty $_ 'scenario') -eq 'stop-cleanup' })) {
        $runId = [string](Get-PclCollectorProperty $row 'run_id')
        $measurement = Get-PclCollectorProperty $row 'lifecycle_measurement'
        $frequency = Get-PclCollectorProperty $measurement 'qpc_frequency_hz'
        $stop = Get-PclCollectorProperty $measurement 'stop_requested_qpc_ticks'
        $joined = Get-PclCollectorProperty $measurement 'worker_joined_qpc_ticks'
        $verified = Get-PclCollectorProperty $measurement 'verified_release_qpc_ticks'
        $rawUps = Get-PclCollectorProperty $measurement 'raw_up_call_count'
        $proof = Get-PclCollectorProperty $measurement 'verified_release_requires_join_clean_journal_and_validated_receiveonly_up'
        if ($null -eq $frequency -or [long]$frequency -le 0 -or $null -eq $rawUps -or [long]$rawUps -lt 0) {
            $incompleteness.Add('stop-cleanup run lacks QPC frequency or raw Up count: ' + $runId)
            continue
        }
        $rawUpSamples.Add([pscustomobject]@{ run_id = $runId; value = [long]$rawUps })
        if ([string](Get-PclCollectorProperty $row 'status') -ne 'PASS' -or
            [int](Get-PclCollectorProperty $row 'case_exit_code') -ne 0 -or
            -not [bool]$proof -or
            $null -eq $stop -or $null -eq $joined -or $null -eq $verified -or
            [long]$stop -le 0 -or [long]$joined -lt [long]$stop -or [long]$verified -lt [long]$joined) {
            $incompleteness.Add('stop-cleanup lacks joined, clean, verified-release proof: ' + $runId)
            continue
        }
        $duration = 1000000.0 * ([double]$verified - [double]$stop) / [double]$frequency
        $stopSamples.Add([pscustomobject]@{ run_id = $runId; source_revision = $CandidateRevision; source_tree_sha = $CandidateTree; manifest_sha256 = $CandidateManifestSha256; binary_sha256 = $CandidateBinarySha256; qpc_frequency_hz = [long]$frequency; stop_requested_qpc_ticks = [long]$stop; worker_joined_qpc_ticks = [long]$joined; verified_release_qpc_ticks = [long]$verified; verified_release_us = $duration; raw_up_call_count = [long]$rawUps })
    }
    if ($stopSamples.Count -eq 0) { $incompleteness.Add('no verified stop-to-release sample was collected from candidate stop-cleanup rows') }

    $joinedSamples = [System.Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt $orderedRows.Count - 1; $index++) {
        $previous = $orderedRows[$index]
        $next = $orderedRows[$index + 1]
        if ((Get-PclCollectorProperty $previous 'role') -ne 'candidate') { continue }
        $previousMeasurement = Get-PclCollectorProperty $previous 'lifecycle_measurement'
        $joined = Get-PclCollectorProperty $previousMeasurement 'worker_joined_qpc_ticks'
        $frequency = Get-PclCollectorProperty $previousMeasurement 'qpc_frequency_hz'
        $nextStart = Get-PclCollectorProperty $next 'case_launch_qpc_ticks'
        $nextFrequency = Get-PclCollectorProperty $next 'case_launch_qpc_frequency_hz'
        $previousId = [string](Get-PclCollectorProperty $previous 'run_id')
        $nextId = [string](Get-PclCollectorProperty $next 'run_id')
        if ($null -eq $joined -or $null -eq $frequency -or $null -eq $nextStart -or
            $null -eq $nextFrequency -or [long]$frequency -le 0 -or [long]$frequency -ne [long]$nextFrequency -or
            [long]$nextStart -lt [long]$joined) {
            $incompleteness.Add('candidate worker-join/next case-launch QPC interval is unavailable or invalid: ' + $previousId + ' -> ' + $nextId)
            continue
        }
        $duration = 1000000.0 * ([double]$nextStart - [double]$joined) / [double]$frequency
        $joinedSamples.Add([pscustomobject]@{ prior_run_id = $previousId; next_run_id = $nextId; prior_role = 'candidate'; next_role = Get-PclCollectorProperty $next 'role'; qpc_frequency_hz = [long]$frequency; worker_joined_qpc_ticks = [long]$joined; next_case_launch_qpc_ticks = [long]$nextStart; worker_joined_to_next_case_launch_us = $duration })
    }
    if ($joinedSamples.Count -eq 0) { $incompleteness.Add('no candidate worker-joined-to-next-case-launch interval was collected') }

    $stopValues = [double[]]@($stopSamples | ForEach-Object { [double]$_.verified_release_us })
    $joinedValues = [double[]]@($joinedSamples | ForEach-Object { [double]$_.worker_joined_to_next_case_launch_us })
    $rawUpValues = [double[]]@($rawUpSamples | ForEach-Object { [double]$_.value })
    $stopSummary = Get-PclNearestRankSummary $stopValues
    $joinedSummary = Get-PclNearestRankSummary $joinedValues
    $rawUpSummary = Get-PclNearestRankSummary $rawUpValues
    [ordered]@{
        schema_version = 1
        status = if ($incompleteness.Count -eq 0) { 'COMPLETE' } else { 'INCOMPLETE' }
        source_identity = [ordered]@{ revision = $CandidateRevision; tree = $CandidateTree; manifest_sha256 = $CandidateManifestSha256; binary_sha256 = $CandidateBinarySha256 }
        machine_identity = [ordered]@{ machine_name = $RunnerMachineName; os_boot_time_utc = $RunnerOsBootTimeUtc }
        target_identity = [ordered]@{ target_class = 'project-owned-non-game-receive-only'; target_name = 'Sky Auto Player — Native Acceptance Sink'; game_target = $false; runs = @($allTargets.Values) }
        stop_to_verified_release_us = [ordered]@{ status = if ($stopSamples.Count -gt 0) { 'COMPLETE' } else { 'INCOMPLETE' }; measurement = 'verified release tick minus Stop request tick; only PASS rows with joined worker, clean journal, and validated ReceiveOnly Up'; statistics = $stopSummary; samples = $stopSamples.ToArray() }
        worker_joined_to_next_start_us = [ordered]@{ status = if ($joinedSamples.Count -gt 0) { 'COMPLETE' } else { 'INCOMPLETE' }; measurement = 'next actual native case process launch tick minus the immediately preceding candidate worker join tick; includes inter-case harness setup'; statistics = $joinedSummary; samples = $joinedSamples.ToArray() }
        raw_up_call_count = [ordered]@{ status = if ($rawUpSamples.Count -gt 0) { 'COMPLETE' } else { 'INCOMPLETE' }; measurement = 'actual worker release outcome raw Up attempts for candidate stop-cleanup cases'; statistics = $rawUpSummary; samples = $rawUpSamples.ToArray() }
        incompleteness = $incompleteness.ToArray()
    }
}

Export-ModuleMember -Function New-PlaybackCoreLifecycleMeasurementReport
