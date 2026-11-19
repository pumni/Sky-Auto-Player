Import-Module (Join-Path $PSScriptRoot '..\dispatch_qualification_profiles.psm1')

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

function Test-PclAdjacentTimedRoleTransition {
    param($Prior, $Next, [string]$PriorRole, [string]$NextRole, [string[]]$Workloads, [string[]]$Loads, [int]$PairCount)
    if ([int](Get-PclCollectorProperty $Prior 'sample_seconds') -ne 15 -or
        [int](Get-PclCollectorProperty $Next 'sample_seconds') -ne 15) { return $false }
    $cell = [string](Get-PclCollectorProperty $Prior 'cell_id')
    $workload = [string](Get-PclCollectorProperty $Prior 'workload')
    $load = [string](Get-PclCollectorProperty $Prior 'load')
    if ([string]::IsNullOrWhiteSpace($cell) -or $cell -ne ($workload + '/' + $load) -or
        $cell -ne [string](Get-PclCollectorProperty $Next 'cell_id') -or
        $workload -notin $Workloads -or $load -notin $Loads -or
        $workload -ne (Get-PclCollectorProperty $Next 'workload') -or
        $load -ne (Get-PclCollectorProperty $Next 'load')) { return $false }
    $priorPairId = [string](Get-PclCollectorProperty $Prior 'pair_id')
    $nextPairId = [string](Get-PclCollectorProperty $Next 'pair_id')
    $priorOrder = [string](Get-PclCollectorProperty $Prior 'pair_order')
    $nextOrder = [string](Get-PclCollectorProperty $Next 'pair_order')
    $pairPattern = '^' + [regex]::Escape($cell) + '/([0-9]{2})$'
    if ($PriorRole -eq 'candidate' -and $NextRole -eq 'candidate') {
        if ($priorOrder -ne 'AB' -or $nextOrder -ne 'BA' -or
            $priorPairId -notmatch $pairPattern) { return $false }
        $priorPairNumber = [int]$Matches[1]
        if ($nextPairId -notmatch $pairPattern) { return $false }
        $nextPairNumber = [int]$Matches[1]
        return $priorPairNumber -ge 1 -and $nextPairNumber -le $PairCount -and
            $priorPairNumber % 2 -eq 1 -and $nextPairNumber -eq ($priorPairNumber + 1)
    }
    if ($PriorRole -eq 'candidate' -and $NextRole -eq 'control') {
        if ($priorPairId -notmatch $pairPattern) { return $false }
        $pairNumber = [int]$Matches[1]
        return $pairNumber -ge 1 -and $pairNumber -le $PairCount -and $pairNumber % 2 -eq 0 -and
            $priorOrder -eq 'BA' -and $nextOrder -eq 'BA' -and $priorPairId -eq $nextPairId
    }
    return $false
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

    $profile = Get-PlaybackCoreLifecycleProfile
    if ($profile.PairOrder -ne 'AB/BA alternating' -or $profile.PairCount -ne 6) {
        throw 'PlaybackCoreLifecycle Start coverage expects the frozen six-pair alternating AB/BA order'
    }
    $expectedTransitionsPerRolePair = [int]($profile.LoadCells.Count * $profile.PerformanceWorkloads.Count * ($profile.PairCount / 2))
    $plannedCandidatePredecessorTransitions = 2 * $expectedTransitionsPerRolePair
    $plannedRoleTransitions = @{}
    foreach ($workload in $profile.PerformanceWorkloads) {
        foreach ($load in $profile.LoadCells) {
            $cellId = $workload + '/' + $load
            for ($pair = 1; $pair -lt $profile.PairCount; $pair += 2) {
                $priorPairId = $cellId + '/' + ('{0:D2}' -f $pair)
                $nextPairId = $cellId + '/' + ('{0:D2}' -f ($pair + 1))
                $plannedRoleTransitions[('candidate|candidate|' + $priorPairId + '|' + $nextPairId)] = 'candidate-to-candidate'
                $plannedRoleTransitions[('candidate|control|' + $nextPairId + '|' + $nextPairId)] = 'candidate-to-control'
            }
        }
    }

    if ($RunRows.Count -eq 0) {
        return [ordered]@{
            schema_version = 2
            status = 'INCOMPLETE'
            source_identity = [ordered]@{ revision = $CandidateRevision; tree = $CandidateTree; manifest_sha256 = $CandidateManifestSha256; binary_sha256 = $CandidateBinarySha256 }
            target_identity = 'project-owned-non-game-receive-only; native capture not performed by this collector'
            stop_to_verified_release_us = [ordered]@{ status = 'INCOMPLETE'; sample_count = 0; p50 = $null; p95 = $null; p99 = $null; max = $null; samples = @() }
            worker_joined_to_next_start_us = [ordered]@{
                status = 'INCOMPLETE'
                measurement = 'immediately following timed session Armed publication QPC minus candidate worker join QPC'
                statistics = Get-PclNearestRankSummary @()
                coverage = [ordered]@{
                    planned_transition_count = $plannedCandidatePredecessorTransitions
                    eligible_transition_count = 0; measured_transition_count = 0
                    unavailable_transition_count = 0; invalid_transition_count = 0
                    missing_planned_transition_count = $plannedCandidatePredecessorTransitions
                    incomplete_transition_count = $plannedCandidatePredecessorTransitions
                    by_transition = @(
                        [ordered]@{ prior_role = 'candidate'; next_role = 'candidate'; planned_count = $expectedTransitionsPerRolePair; observed_count = 0; eligible_count = 0; measured_count = 0; unavailable_count = 0; invalid_count = 0; missing_count = $expectedTransitionsPerRolePair },
                        [ordered]@{ prior_role = 'candidate'; next_role = 'control'; planned_count = $expectedTransitionsPerRolePair; observed_count = 0; eligible_count = 0; measured_count = 0; unavailable_count = 0; invalid_count = 0; missing_count = $expectedTransitionsPerRolePair }
                    )
                    unavailable_transitions = @(); invalid_transitions = @()
                    protocol_decision_required = 'Leader/owner must decide whether same-candidate-only successor coverage is acceptable; otherwise the mandatory metric remains INCOMPLETE under the immutable baseline protocol.'
                }
                samples = @()
            }
            worker_joined_to_runner_pre_next_process_launch_us = [ordered]@{ status = 'DIAGNOSTIC_ONLY'; sample_count = 0; p50 = $null; p95 = $null; p99 = $null; max = $null; samples = @() }
            raw_up_syscall_count = [ordered]@{ status = 'INCOMPLETE'; sample_count = 0; p50 = $null; p95 = $null; p99 = $null; max = $null; samples = @() }
            incompleteness = @('no qualification rows were supplied')
        }
    }

    $orderedRows = @($RunRows | Sort-Object { [int](Get-PclCollectorProperty $_ 'sequence_index') })
    $candidateRows = @($orderedRows | Where-Object { (Get-PclCollectorProperty $_ 'role') -eq 'candidate' })
    $controlRows = @($orderedRows | Where-Object { (Get-PclCollectorProperty $_ 'role') -eq 'control' })
    $allTargets = @{}
    $seenRunIds = @{}
    foreach ($row in $orderedRows) {
        $runId = [string](Get-PclCollectorProperty $row 'run_id')
        if ([string]::IsNullOrWhiteSpace($runId)) { throw 'PlaybackCoreLifecycle collector row is missing run_id' }
        if ($seenRunIds.ContainsKey($runId)) { throw ('PlaybackCoreLifecycle collector has duplicate run_id: ' + $runId) }
        $seenRunIds[$runId] = $true
        $role = [string](Get-PclCollectorProperty $row 'role')
        if ($role -notin @('candidate', 'control')) { throw ('PlaybackCoreLifecycle collector row has an unsupported role: ' + $runId) }
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

    $candidateQpcSource = $null
    $candidateQpcFrequency = $null
    $candidateQpcInvalidRunIds = @{}
    foreach ($row in $candidateRows) {
        if ([int](Get-PclCollectorProperty $row 'sample_seconds') -ne 15 -and
            (Get-PclCollectorProperty $row 'scenario') -ne 'stop-cleanup') { continue }
        $runId = [string](Get-PclCollectorProperty $row 'run_id')
        $measurement = Get-PclCollectorProperty $row 'lifecycle_measurement'
        $clockSource = [string](Get-PclCollectorProperty $measurement 'qpc_clock_source')
        $frequency = Get-PclCollectorProperty $measurement 'qpc_frequency_hz'
        if ((Get-PclCollectorProperty $measurement 'schema_version') -ne 2 -or
            $clockSource -ne 'Windows QueryPerformanceCounter' -or $null -eq $frequency -or [long]$frequency -le 0) {
            $candidateQpcInvalidRunIds[$runId] = $true
            continue
        }
        if ($null -eq $candidateQpcSource) {
            $candidateQpcSource = $clockSource
            $candidateQpcFrequency = [long]$frequency
        } elseif ($clockSource -ne $candidateQpcSource -or [long]$frequency -ne $candidateQpcFrequency) {
            $candidateQpcInvalidRunIds[$runId] = $true
        }
    }

    $controlIdentity = $null
    foreach ($row in $controlRows) {
        $runId = [string](Get-PclCollectorProperty $row 'run_id')
        $revision = [string](Get-PclCollectorProperty $row 'source_revision')
        $tree = [string](Get-PclCollectorProperty $row 'source_tree_sha')
        $manifest = [string](Get-PclCollectorProperty $row 'manifest_sha256')
        $binary = [string](Get-PclCollectorProperty $row 'harness_sha256')
        if ($revision -ne $profile.BaselineRevision -or $tree -ne $profile.BaselineTree -or
            $manifest -notmatch '^[0-9a-f]{64}$' -or $binary -notmatch '^[0-9a-f]{64}$') {
            throw ('PlaybackCoreLifecycle control row is not bound to the immutable baseline manifest/binary identity: ' + $runId)
        }
        $identity = [pscustomobject]@{ revision = $revision; tree = $tree; manifest_sha256 = $manifest; binary_sha256 = $binary }
        if ($null -eq $controlIdentity) {
            $controlIdentity = $identity
        } elseif ($identity.revision -ne $controlIdentity.revision -or $identity.tree -ne $controlIdentity.tree -or
            $identity.manifest_sha256 -ne $controlIdentity.manifest_sha256 -or $identity.binary_sha256 -ne $controlIdentity.binary_sha256) {
            throw ('PlaybackCoreLifecycle control manifest/binary identity drifted from the first pinned baseline row: ' + $runId)
        }
    }

    $incompleteness = [System.Collections.Generic.List[string]]::new()
    $stopSamples = [System.Collections.Generic.List[object]]::new()
    $rawUpSamples = [System.Collections.Generic.List[object]]::new()
    $joinedSamples = [System.Collections.Generic.List[object]]::new()
    $joinedUnavailable = [System.Collections.Generic.List[object]]::new()
    $joinedInvalid = [System.Collections.Generic.List[object]]::new()
    $launchDiagnosticSamples = [System.Collections.Generic.List[object]]::new()
    $stopMetricIncomplete = $false
    $rawUpMetricIncomplete = $false
    $startMetricIncomplete = $false
    $observedCandidateToCandidate = 0
    $observedCandidateToControl = 0
    $eligibleCandidateToCandidate = 0
    $seenPlannedRoleTransitions = @{}
    foreach ($row in @($candidateRows | Where-Object { (Get-PclCollectorProperty $_ 'scenario') -eq 'stop-cleanup' })) {
        $runId = [string](Get-PclCollectorProperty $row 'run_id')
        $measurement = Get-PclCollectorProperty $row 'lifecycle_measurement'
        $frequency = Get-PclCollectorProperty $measurement 'qpc_frequency_hz'
        $stop = Get-PclCollectorProperty $measurement 'stop_requested_qpc_ticks'
        $joined = Get-PclCollectorProperty $measurement 'worker_joined_qpc_ticks'
        $verified = Get-PclCollectorProperty $measurement 'worker_release_verified_qpc_ticks'
        $rawSyscalls = Get-PclCollectorProperty $measurement 'raw_up_syscall_count'
        $proof = Get-PclCollectorProperty $measurement 'release_sample_gate_proof_complete'
        $sessionIdentity = Get-PclCollectorProperty $measurement 'native_session_identity'
        $clockSource = Get-PclCollectorProperty $measurement 'qpc_clock_source'
        $rawSource = Get-PclCollectorProperty $measurement 'raw_up_syscall_count_source'
        if ($null -ne $frequency -and [long]$frequency -gt 0 -and
            $clockSource -eq 'Windows QueryPerformanceCounter' -and
            $rawSource -eq 'sum of worker TerminalSendAttemptEvidence.raw_calls' -and
            $null -ne $rawSyscalls -and [long]$rawSyscalls -ge 0 -and
            (Get-PclCollectorProperty $sessionIdentity 'run_id') -eq $runId) {
            $rawUpSamples.Add([pscustomobject]@{ run_id = $runId; source_revision = Get-PclCollectorProperty $row 'source_revision'; source_tree_sha = Get-PclCollectorProperty $row 'source_tree_sha'; manifest_sha256 = Get-PclCollectorProperty $row 'manifest_sha256'; binary_sha256 = Get-PclCollectorProperty $row 'harness_sha256'; value = [long]$rawSyscalls; evidence_source = $rawSource })
        } else {
            $rawUpMetricIncomplete = $true
            $incompleteness.Add('stop-cleanup run lacks identity-bound QPC or actual raw Up syscall evidence: ' + $runId)
        }

        if ([string](Get-PclCollectorProperty $row 'status') -ne 'PASS' -or
            [int](Get-PclCollectorProperty $row 'case_exit_code') -ne 0 -or
            -not [bool]$proof -or
            (Get-PclCollectorProperty $sessionIdentity 'scenario') -ne 'stop-cleanup' -or
            $null -eq $frequency -or [long]$frequency -le 0 -or
            $clockSource -ne 'Windows QueryPerformanceCounter' -or
            $null -eq $stop -or $null -eq $joined -or $null -eq $verified -or
            [long]$stop -le 0 -or [long]$verified -lt [long]$stop -or [long]$joined -lt [long]$verified) {
            $stopMetricIncomplete = $true
            $incompleteness.Add('stop-cleanup lacks identity-bound worker release, joined, clean journal, or ReceiveOnly proof: ' + $runId)
            continue
        }
        $duration = 1000000.0 * ([double]$verified - [double]$stop) / [double]$frequency
        $stopSamples.Add([pscustomobject]@{ run_id = $runId; source_revision = $CandidateRevision; source_tree_sha = $CandidateTree; manifest_sha256 = $CandidateManifestSha256; binary_sha256 = $CandidateBinarySha256; qpc_frequency_hz = [long]$frequency; stop_requested_qpc_ticks = [long]$stop; worker_release_verified_qpc_ticks = [long]$verified; worker_joined_qpc_ticks = [long]$joined; stop_to_worker_release_verified_us = $duration; raw_up_syscall_count = if ($null -ne $rawSyscalls) { [long]$rawSyscalls } else { $null }; post_join_report_and_receiveonly_validation_completed_qpc_ticks = Get-PclCollectorProperty $measurement 'post_join_report_and_receiveonly_validation_completed_qpc_ticks' })
    }
    if ($stopSamples.Count -eq 0) { $stopMetricIncomplete = $true; $incompleteness.Add('no verified stop-to-release sample was collected from candidate stop-cleanup rows') }

    for ($index = 0; $index -lt $orderedRows.Count - 1; $index++) {
        $previous = $orderedRows[$index]
        $next = $orderedRows[$index + 1]
        if ((Get-PclCollectorProperty $previous 'role') -ne 'candidate' -or
            [int](Get-PclCollectorProperty $previous 'sample_seconds') -ne 15 -or
            [int](Get-PclCollectorProperty $next 'sample_seconds') -ne 15) { continue }
        $previousMeasurement = Get-PclCollectorProperty $previous 'lifecycle_measurement'
        $nextMeasurement = Get-PclCollectorProperty $next 'lifecycle_measurement'
        $joined = Get-PclCollectorProperty $previousMeasurement 'worker_joined_qpc_ticks'
        $frequency = Get-PclCollectorProperty $previousMeasurement 'qpc_frequency_hz'
        $nextStart = Get-PclCollectorProperty $nextMeasurement 'playback_armed_qpc_ticks'
        $nextFrequency = Get-PclCollectorProperty $nextMeasurement 'qpc_frequency_hz'
        $previousId = [string](Get-PclCollectorProperty $previous 'run_id')
        $nextId = [string](Get-PclCollectorProperty $next 'run_id')
        $previousSessionIdentity = Get-PclCollectorProperty $previousMeasurement 'native_session_identity'
        $nextSessionIdentity = Get-PclCollectorProperty $nextMeasurement 'native_session_identity'
        $previousClock = Get-PclCollectorProperty $previousMeasurement 'qpc_clock_source'
        $nextClock = Get-PclCollectorProperty $nextMeasurement 'qpc_clock_source'
        $previousRole = [string](Get-PclCollectorProperty $previous 'role')
        $nextRole = [string](Get-PclCollectorProperty $next 'role')
        $transitionKey = $previousRole + '|' + $nextRole + '|' +
            [string](Get-PclCollectorProperty $previous 'pair_id') + '|' + [string](Get-PclCollectorProperty $next 'pair_id')
        $expectedRoleTransition = (Test-PclAdjacentTimedRoleTransition -Prior $previous -Next $next -PriorRole $previousRole -NextRole $nextRole `
            -Workloads $profile.PerformanceWorkloads -Loads $profile.LoadCells -PairCount $profile.PairCount) -and
            $plannedRoleTransitions.ContainsKey($transitionKey) -and -not $seenPlannedRoleTransitions.ContainsKey($transitionKey)
        if ($expectedRoleTransition) { $seenPlannedRoleTransitions[$transitionKey] = $plannedRoleTransitions[$transitionKey] }
        $previousRevision = [string](Get-PclCollectorProperty $previous 'source_revision')
        $previousTree = [string](Get-PclCollectorProperty $previous 'source_tree_sha')
        $previousManifest = [string](Get-PclCollectorProperty $previous 'manifest_sha256')
        $previousBinary = [string](Get-PclCollectorProperty $previous 'harness_sha256')
        $nextRevision = [string](Get-PclCollectorProperty $next 'source_revision')
        $nextTree = [string](Get-PclCollectorProperty $next 'source_tree_sha')
        $nextManifest = [string](Get-PclCollectorProperty $next 'manifest_sha256')
        $nextBinary = [string](Get-PclCollectorProperty $next 'harness_sha256')
        if ($nextRole -eq 'candidate') { $observedCandidateToCandidate++ }
        elseif ($nextRole -eq 'control') { $observedCandidateToControl++ }

        # Preserve the parent-side process launch measurement as a diagnostic
        # only. It is never used to complete the playback Start metric.
        $launchTick = Get-PclCollectorProperty $next 'runner_pre_child_process_launch_qpc_ticks'
        $launchFrequency = Get-PclCollectorProperty $next 'runner_pre_child_process_launch_qpc_frequency_hz'
        if ($null -ne $joined -and $null -ne $frequency -and $null -ne $launchTick -and
            $null -ne $launchFrequency -and [long]$frequency -gt 0 -and
            [long]$frequency -eq [long]$launchFrequency -and [long]$launchTick -ge [long]$joined) {
            $launchDiagnosticSamples.Add([pscustomobject]@{
                prior_run_id = $previousId; next_run_id = $nextId; prior_role = $previousRole; next_role = $nextRole
                qpc_frequency_hz = [long]$frequency; worker_joined_qpc_ticks = [long]$joined
                runner_pre_child_process_launch_qpc_ticks = [long]$launchTick
                worker_joined_to_runner_pre_next_process_launch_us = 1000000.0 * ([double]$launchTick - [double]$joined) / [double]$frequency
            })
        }

        $previousBoundaryValid =
            (Get-PclCollectorProperty $previousMeasurement 'schema_version') -eq 2 -and
            (Get-PclCollectorProperty $previousSessionIdentity 'run_id') -eq $previousId -and
            (Get-PclCollectorProperty $previousSessionIdentity 'scenario') -eq (Get-PclCollectorProperty $previous 'scenario') -and
            $previousRevision -eq $CandidateRevision -and $previousTree -eq $CandidateTree -and
            $previousManifest -eq $CandidateManifestSha256 -and $previousBinary -eq $CandidateBinarySha256 -and
            $previousClock -eq 'Windows QueryPerformanceCounter' -and
            -not $candidateQpcInvalidRunIds.ContainsKey($previousId) -and
            $null -ne $joined -and $null -ne $frequency -and [long]$joined -gt 0 -and [long]$frequency -gt 0

        $candidatePairSchemaEligible =
            $expectedRoleTransition -and $previousRole -eq 'candidate' -and $nextRole -eq 'candidate' -and
            (Get-PclCollectorProperty $previousMeasurement 'schema_version') -eq 2 -and
            (Get-PclCollectorProperty $nextMeasurement 'schema_version') -eq 2 -and
            (Get-PclCollectorProperty $previousSessionIdentity 'run_id') -eq $previousId -and
            (Get-PclCollectorProperty $previousSessionIdentity 'scenario') -eq (Get-PclCollectorProperty $previous 'scenario') -and
            (Get-PclCollectorProperty $nextSessionIdentity 'run_id') -eq $nextId -and
            (Get-PclCollectorProperty $nextSessionIdentity 'scenario') -eq (Get-PclCollectorProperty $next 'scenario') -and
            $previousRevision -eq $CandidateRevision -and $previousTree -eq $CandidateTree -and
            $previousManifest -eq $CandidateManifestSha256 -and $previousBinary -eq $CandidateBinarySha256 -and
            $nextRevision -eq $CandidateRevision -and $nextTree -eq $CandidateTree -and
            $nextManifest -eq $CandidateManifestSha256 -and $nextBinary -eq $CandidateBinarySha256
        if ($candidatePairSchemaEligible) {
            $eligibleCandidateToCandidate++
        }

        if ($nextRole -eq 'control') {
            $nextMeasurement = Get-PclCollectorProperty $next 'lifecycle_measurement'
            if ($expectedRoleTransition -and $nextRevision -eq $profile.BaselineRevision -and
                $nextTree -eq $profile.BaselineTree -and $null -eq $nextMeasurement) {
                $joinedUnavailable.Add([pscustomobject]@{
                    prior_run_id = $previousId; next_run_id = $nextId
                    prior_role = $previousRole; next_role = $nextRole
                    prior_source_revision = $previousRevision; prior_source_tree_sha = $previousTree
                    prior_manifest_sha256 = $previousManifest; prior_binary_sha256 = $previousBinary
                    next_source_revision = $nextRevision; next_source_tree_sha = $nextTree
                    next_manifest_sha256 = $nextManifest; next_binary_sha256 = $nextBinary
                    prior_boundary_valid = $previousBoundaryValid
                    reason = if ($previousBoundaryValid) {
                        'immutable baseline native acceptance source emits no lifecycle_measurement or worker Armed QPC field'
                    } else {
                        'immutable baseline emits no worker Armed QPC field, and the candidate predecessor join/QPC boundary is also invalid'
                    }
                })
            } else {
                $joinedInvalid.Add([pscustomobject]@{
                    prior_run_id = $previousId; next_run_id = $nextId
                    prior_role = $previousRole; next_role = $nextRole
                    reason = 'candidate→control transition violates AB/BA adjacency, lacks valid candidate predecessor identity, or is inconsistent with the pinned baseline schema/source'
                })
            }
            $startMetricIncomplete = $true
            continue
        }

        if ($nextRole -ne 'candidate') {
            $joinedInvalid.Add([pscustomobject]@{
                prior_run_id = $previousId; next_run_id = $nextId
                prior_role = $previousRole; next_role = $nextRole
                reason = 'successor role is not part of the frozen candidate/control protocol'
            })
            $startMetricIncomplete = $true
            continue
        }

        $rowIdentitiesValid =
            $previousRole -eq 'candidate' -and $nextRole -eq 'candidate' -and
            $previousRevision -eq $CandidateRevision -and $previousTree -eq $CandidateTree -and
            $previousManifest -eq $CandidateManifestSha256 -and $previousBinary -eq $CandidateBinarySha256 -and
            $nextRevision -eq $CandidateRevision -and $nextTree -eq $CandidateTree -and
            $nextManifest -eq $CandidateManifestSha256 -and $nextBinary -eq $CandidateBinarySha256
        $validNativeBoundary =
            (Get-PclCollectorProperty $nextMeasurement 'schema_version') -eq 2 -and
            (Get-PclCollectorProperty $nextSessionIdentity 'run_id') -eq $nextId -and
            (Get-PclCollectorProperty $nextSessionIdentity 'scenario') -eq (Get-PclCollectorProperty $next 'scenario') -and
            (Get-PclCollectorProperty $nextMeasurement 'playback_admission_event') -eq 'system_power_ready_to_armed_publication' -and
            $previousClock -eq 'Windows QueryPerformanceCounter' -and $nextClock -eq $previousClock -and
            -not $candidateQpcInvalidRunIds.ContainsKey($nextId) -and
            [long]$frequency -eq $candidateQpcFrequency -and [long]$nextFrequency -eq $candidateQpcFrequency
        if (-not $expectedRoleTransition -or -not $previousBoundaryValid -or -not $rowIdentitiesValid -or -not $validNativeBoundary -or
            $null -eq $nextStart -or $null -eq $nextFrequency -or [long]$frequency -ne [long]$nextFrequency -or
            [long]$nextStart -le 0 -or [long]$nextStart -lt [long]$joined) {
            $startMetricIncomplete = $true
            $joinedInvalid.Add([pscustomobject]@{
                prior_run_id = $previousId; next_run_id = $nextId
                prior_role = $previousRole; next_role = $nextRole
                reason = 'candidate→candidate successor lacks valid schema-v2 worker QPC, identity, chronology, or source provenance'
            })
            $incompleteness.Add('candidate worker-join/next playback Armed QPC interval is unavailable, mismatched, or invalid: ' + $previousId + ' -> ' + $nextId)
        } else {
            $duration = 1000000.0 * ([double]$nextStart - [double]$joined) / [double]$frequency
            $joinedSamples.Add([pscustomobject]@{
                prior_run_id = $previousId; next_run_id = $nextId; prior_role = $previousRole; next_role = $nextRole
                prior_source_revision = $previousRevision; prior_source_tree_sha = $previousTree
                prior_manifest_sha256 = $previousManifest; prior_binary_sha256 = $previousBinary
                next_source_revision = $nextRevision; next_source_tree_sha = $nextTree
                next_manifest_sha256 = $nextManifest; next_binary_sha256 = $nextBinary
                qpc_clock_source = $previousClock; qpc_frequency_hz = [long]$frequency
                worker_joined_qpc_ticks = [long]$joined; next_playback_armed_qpc_ticks = [long]$nextStart
                worker_joined_to_next_playback_start_us = $duration
            })
        }
    }
    $seenCandidateToCandidate = @($seenPlannedRoleTransitions.Values | Where-Object { $_ -eq 'candidate-to-candidate' }).Count
    $seenCandidateToControl = @($seenPlannedRoleTransitions.Values | Where-Object { $_ -eq 'candidate-to-control' }).Count
    $missingCandidateToCandidate = [Math]::Max(0, $expectedTransitionsPerRolePair - $seenCandidateToCandidate)
    $missingCandidateToControl = [Math]::Max(0, $expectedTransitionsPerRolePair - $seenCandidateToControl)
    $incompleteStartTransitionCount = [Math]::Max(0, $plannedCandidatePredecessorTransitions - $joinedSamples.Count)
    if ($joinedSamples.Count -eq 0) { $startMetricIncomplete = $true; $incompleteness.Add('no candidate worker-joined-to-next-playback-Start interval was collected') }
    if ($joinedUnavailable.Count -gt 0) {
        $startMetricIncomplete = $true
        $incompleteness.Add('worker-joined-to-next-Start is INCOMPLETE: ' + $joinedSamples.Count + ' measured candidate→candidate transitions; ' + $joinedUnavailable.Count + ' candidate→frozen-baseline transitions are unavailable because the baseline emits no worker Armed QPC')
    }
    if ($missingCandidateToCandidate -gt 0 -or $missingCandidateToControl -gt 0) {
        $startMetricIncomplete = $true
        $incompleteness.Add('worker-joined-to-next-Start observed transition coverage differs from the frozen six-pair AB/BA plan')
    }
    if ($rawUpSamples.Count -eq 0) { $rawUpMetricIncomplete = $true; $incompleteness.Add('no actual raw Up syscall count was collected') }

    $stopValues = [double[]]@($stopSamples | ForEach-Object { [double]$_.stop_to_worker_release_verified_us })
    $joinedValues = [double[]]@($joinedSamples | ForEach-Object { [double]$_.worker_joined_to_next_playback_start_us })
    $rawUpValues = [double[]]@($rawUpSamples | ForEach-Object { [double]$_.value })
    $launchDiagnosticValues = [double[]]@($launchDiagnosticSamples | ForEach-Object { [double]$_.worker_joined_to_runner_pre_next_process_launch_us })
    $stopSummary = Get-PclNearestRankSummary $stopValues
    $joinedSummary = Get-PclNearestRankSummary $joinedValues
    $rawUpSummary = Get-PclNearestRankSummary $rawUpValues
    $launchDiagnosticSummary = Get-PclNearestRankSummary $launchDiagnosticValues
    [ordered]@{
        schema_version = 2
        status = if ($incompleteness.Count -eq 0) { 'COMPLETE' } else { 'INCOMPLETE' }
        source_identity = [ordered]@{ revision = $CandidateRevision; tree = $CandidateTree; manifest_sha256 = $CandidateManifestSha256; binary_sha256 = $CandidateBinarySha256 }
        machine_identity = [ordered]@{ machine_name = $RunnerMachineName; os_boot_time_utc = $RunnerOsBootTimeUtc }
        target_identity = [ordered]@{ target_class = 'project-owned-non-game-receive-only'; target_name = 'Sky Auto Player — Native Acceptance Sink'; game_target = $false; runs = @($allTargets.Values) }
        stop_to_verified_release_us = [ordered]@{ status = if ($stopSamples.Count -gt 0 -and -not $stopMetricIncomplete) { 'COMPLETE' } else { 'INCOMPLETE' }; measurement = 'worker-recorded terminal release verification QPC minus accepted Stop request QPC; subsequent ReceiveOnly Up validation gates sample eligibility but its drain/report delay is excluded'; statistics = $stopSummary; samples = $stopSamples.ToArray() }
        worker_joined_to_next_start_us = [ordered]@{
            status = if ($joinedSamples.Count -gt 0 -and -not $startMetricIncomplete -and $incompleteStartTransitionCount -eq 0) { 'COMPLETE' } else { 'INCOMPLETE' }
            measurement = 'immediately following timed session Armed publication QPC minus candidate worker join QPC, with exact row/session identities and matching Windows QPC source/frequency/machine/boot'
            statistics = $joinedSummary
            coverage = [ordered]@{
                planned_transition_count = $plannedCandidatePredecessorTransitions
                eligible_transition_count = $eligibleCandidateToCandidate
                measured_transition_count = $joinedSamples.Count
                unavailable_transition_count = $joinedUnavailable.Count
                invalid_transition_count = $joinedInvalid.Count
                missing_planned_transition_count = $missingCandidateToCandidate + $missingCandidateToControl
                incomplete_transition_count = $incompleteStartTransitionCount
                by_transition = @(
                    [ordered]@{
                        prior_role = 'candidate'; next_role = 'candidate'
                        planned_count = $expectedTransitionsPerRolePair; observed_count = $observedCandidateToCandidate
                        eligible_count = $eligibleCandidateToCandidate
                        measured_count = @($joinedSamples | Where-Object { $_.next_role -eq 'candidate' }).Count
                        unavailable_count = 0
                        invalid_count = @($joinedInvalid | Where-Object { $_.next_role -eq 'candidate' }).Count
                        missing_count = $missingCandidateToCandidate
                    },
                    [ordered]@{
                        prior_role = 'candidate'; next_role = 'control'
                        planned_count = $expectedTransitionsPerRolePair; observed_count = $observedCandidateToControl
                        eligible_count = 0; measured_count = 0
                        unavailable_count = $joinedUnavailable.Count
                        invalid_count = @($joinedInvalid | Where-Object { $_.next_role -eq 'control' }).Count
                        missing_count = $missingCandidateToControl
                    }
                )
                unavailable_transitions = $joinedUnavailable.ToArray()
                invalid_transitions = $joinedInvalid.ToArray()
                control_identity = if ($null -ne $controlIdentity) { $controlIdentity } else { $null }
                protocol_decision_required = 'The frozen plan requires immediate-next-Start coverage across paired candidate/control rows. Leader/owner must explicitly decide whether candidate→candidate-only coverage may replace full immediate-successor coverage; until then this required metric remains INCOMPLETE.'
            }
            samples = $joinedSamples.ToArray()
        }
        worker_joined_to_runner_pre_next_process_launch_us = [ordered]@{ status = 'DIAGNOSTIC_ONLY'; measurement = 'parent runner QPC sampled immediately before starting the next native case process; not a playback Start/admission timestamp'; statistics = $launchDiagnosticSummary; samples = $launchDiagnosticSamples.ToArray() }
        raw_up_syscall_count = [ordered]@{ status = if ($rawUpSamples.Count -gt 0 -and -not $rawUpMetricIncomplete) { 'COMPLETE' } else { 'INCOMPLETE' }; measurement = 'sum of worker TerminalSendAttemptEvidence.raw_calls for candidate stop-cleanup runs; ReleaseAllOutcome.attempts is not used'; statistics = $rawUpSummary; samples = $rawUpSamples.ToArray() }
        incompleteness = $incompleteness.ToArray()
    }
}

Export-ModuleMember -Function New-PlaybackCoreLifecycleMeasurementReport
