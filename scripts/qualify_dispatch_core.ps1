param(
    [Parameter(Mandatory)]
    [ValidateSet('Canary', 'Smoke', 'Compare', 'Final')]
    [string]$Stage,
    [ValidateSet('DispatchCore', 'Scheduler', 'FrameSnapshot')]
    [string]$QualificationProfile = 'DispatchCore',
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
$PSNativeCommandUseErrorActionPreference = $false
Set-StrictMode -Version Latest
Import-Module (Join-Path $PSScriptRoot 'dispatch_pair_screen.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_report.psm1') -Force

$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$qualifiedProfile = $QualificationProfile -in @('Scheduler', 'FrameSnapshot')
$baseRevision = if ($QualificationProfile -eq 'FrameSnapshot') { '8a800ff07a5dc362a5ee3aaa59204ff40b6ae327' } elseif ($QualificationProfile -eq 'Scheduler') { 'b9ed666e3ddddc4952831626f1654e1cc9071a10' } else { '843cc0c6a1392dfff0ca4b017551bd1ffb180579' }
$baseTree = if ($QualificationProfile -eq 'FrameSnapshot') { '48eec424c155e98fa1a81ad018bc832b6e6b964f' } elseif ($QualificationProfile -eq 'Scheduler') { 'f554c41b23a3969d02ed29842d4178f6143c9d86' } else { $null }
$expectedBranch = if ($QualificationProfile -eq 'FrameSnapshot') { 'refactor/frame-snapshot-dispatch' } elseif ($QualificationProfile -eq 'Scheduler') { 'refactor/scheduler-single-owner-20261112' } else { 'perf/dispatch-core-20261109' }
$expectedWorkloadVersion = if ($QualificationProfile -eq 'FrameSnapshot') { 'frame-snapshot-v1' } elseif ($QualificationProfile -eq 'Scheduler') { 'scheduler-single-owner-v1' } else { 'dispatch-core-646-a1' }
$caseRunner = Join-Path $PSScriptRoot 'run_native_acceptance_case.ps1'
$manifestBuilder = Join-Path $PSScriptRoot 'build_dispatch_core_harness.ps1'
$manifestValidator = Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1'
$reportProjectionModule = Join-Path $PSScriptRoot 'dispatch_qualification_report.psm1'
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
$stoppedAtFirstNativeFailure = $false

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

function Get-OptionalProperty {
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

function Get-CurrentSourceIdentity {
    $branch = ((& git -C $root branch --show-current) -join '').Trim()
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

function Assert-SchedulerManifestIdentity {
    param($Manifest, [string]$Role, [string]$ExpectedBaseRevision, [string]$ExpectedBaseTree)
    $expectedSourceRole = if ($Role -eq 'candidate') { 'Candidate' } else { 'Baseline' }
    if ($Manifest.qualification_profile -ne 'Scheduler' -or $Manifest.source_role -ne $expectedSourceRole -or
        -not [bool]$Manifest.source_tree_clean -or
        $Manifest.base_source_revision -ne $ExpectedBaseRevision -or
        $Manifest.base_source_tree_sha -ne $ExpectedBaseTree) {
        throw ($Role + ' Scheduler manifest profile, role, base SHA, or source cleanliness is invalid')
    }
}

function Assert-SchedulerSourceRoot {
    param($Manifest, [string]$Role)
    if ($QualificationProfile -ne 'Scheduler') { return }
    Assert-SchedulerManifestIdentity -Manifest $Manifest -Role $Role -ExpectedBaseRevision $baseRevision -ExpectedBaseTree $baseTree
    $sourceRoot = [System.IO.Path]::GetFullPath([string]$Manifest.source_root)
    if (-not (Test-Path -LiteralPath (Join-Path $sourceRoot 'rust\Cargo.toml') -PathType Leaf)) {
        throw ($Role + ' Scheduler source root is missing the Rust workspace')
    }
    $revision = (& git -C $sourceRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw ($Role + ' Scheduler source root is not a Git checkout') }
    $tree = (& git -C $sourceRoot rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw ($Role + ' Scheduler source tree could not be resolved') }
    $branch = ((& git -C $sourceRoot branch --show-current) -join '').Trim()
    if ($LASTEXITCODE -ne 0) { throw ($Role + ' Scheduler source branch could not be resolved') }
    $changes = (& git -C $sourceRoot status --porcelain --untracked-files=all) -join ''
    if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace($changes)) {
        throw ($Role + ' Scheduler source root is not clean')
    }
    if ($revision -ne $Manifest.source_revision -or $tree -ne $Manifest.source_tree_sha) {
        throw ($Role + ' Scheduler manifest source SHA/tree differs from its checkout')
    }
    if ($Role -eq 'candidate') {
        if ($sourceRoot -ne $root -or $branch -ne $expectedBranch) {
            throw 'Scheduler candidate source must be the clean scheduler branch checkout'
        }
        & git -C $sourceRoot merge-base --is-ancestor $baseRevision HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Scheduler candidate is not descended from the pinned base' }
    } elseif ($revision -ne $baseRevision -or $tree -ne $baseTree -or -not [string]::IsNullOrWhiteSpace($branch)) {
        throw 'Scheduler baseline must be a detached clean checkout at the pinned base SHA/tree'
    }
}

function Assert-FrameSnapshotSourceRoot {
    param($Manifest, [string]$Role)
    $sourceRole = if ($Role -eq 'candidate') { 'Candidate' } else { 'Baseline' }
    Assert-FrameSnapshotManifestIdentity -Manifest $Manifest -ExpectedRole $sourceRole
    $sourceRoot = [System.IO.Path]::GetFullPath([string]$Manifest.source_root)
    if (-not (Test-Path -LiteralPath (Join-Path $sourceRoot 'rust\Cargo.toml') -PathType Leaf)) {
        throw ($Role + ' FrameSnapshot source root is missing the Rust workspace')
    }
    $revision = (& git -C $sourceRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw ($Role + ' FrameSnapshot source root is not a Git checkout') }
    $tree = (& git -C $sourceRoot rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw ($Role + ' FrameSnapshot source tree could not be resolved') }
    $branch = ((& git -C $sourceRoot branch --show-current) -join '').Trim()
    if ($LASTEXITCODE -ne 0) { throw ($Role + ' FrameSnapshot source branch could not be resolved') }
    $changes = (& git -C $sourceRoot status --porcelain --untracked-files=all) -join ''
    if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace($changes)) {
        throw ($Role + ' FrameSnapshot source root is not clean')
    }
    if ($revision -ne $Manifest.source_revision -or $tree -ne $Manifest.source_tree_sha) {
        throw ($Role + ' FrameSnapshot manifest source SHA/tree differs from its checkout')
    }
    if ($Role -eq 'candidate') {
        if (-not [string]::Equals($sourceRoot, $root, [System.StringComparison]::OrdinalIgnoreCase) -or
            $branch -ne $expectedBranch) {
            throw 'FrameSnapshot candidate source must be the clean frame-snapshot branch checkout'
        }
        & git -C $sourceRoot merge-base --is-ancestor $baseRevision HEAD
        if ($LASTEXITCODE -ne 0) { throw 'FrameSnapshot candidate is not descended from the pinned base' }
    } elseif ($revision -ne $baseRevision -or $tree -ne $baseTree -or -not [string]::IsNullOrWhiteSpace($branch)) {
        throw 'FrameSnapshot baseline must be a detached clean checkout at the pinned base SHA/tree'
    }
}

function Read-And-VerifyManifest {
    param([string]$Path, [string]$Role)
    Assert-AbsolutePath $Path ($Role + ' manifest')
    Assert-OutsideRepository $Path ($Role + ' manifest')
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw ($Role + ' manifest does not exist: ' + $Path) }
    $manifest = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    $c1TimedVerified = if ($qualifiedProfile) {
        $false
    } else {
        Assert-C1TimedControlProvenance -Manifest $manifest -RepositoryRoot $root -Role $Role -AllowHistoricalControl:($Role -eq 'control')
    }
    Assert-SchedulerSourceRoot -Manifest $manifest -Role $Role
    if ($QualificationProfile -eq 'FrameSnapshot') {
        Assert-FrameSnapshotSourceRoot -Manifest $manifest -Role $Role
    }
    if ($qualifiedProfile) {
        Assert-ImmutableHarnessManifestIntegrity -Manifest $manifest -ManifestPath $Path | Out-Null
    }
    if ($manifest.guard_mode -eq 'adaptive') {
        throw ($Role + ' manifest uses retired adaptive guard mode (#646); only frozen is accepted for qualification')
    }
    if ($manifest.schema_version -ne 1 -or
        $manifest.source_revision -notmatch '^[0-9a-f]{40}$' -or
        $manifest.source_tree_sha -notmatch '^[0-9a-f]{40}$' -or
        $manifest.source_branch -ne $(if ($qualifiedProfile -and $Role -eq 'control') { '' } else { $expectedBranch }) -or
        ([bool]$manifest.source_tree_clean -ne (-not $c1TimedVerified)) -or
        -not [bool]$manifest.clean_build -or
        $manifest.cargo_profile -ne 'dist' -or
        @($manifest.cargo_features).Count -ne 1 -or
        @($manifest.cargo_features)[0] -ne 'real-input-acceptance' -or
        [bool]$manifest.qpc_read_counting -or
        $manifest.observer_mode -ne 'qualification-on' -or
        $manifest.guard_mode -ne 'frozen' -or
        $manifest.workload_version -ne $expectedWorkloadVersion -or
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
    $manifest | Add-Member -NotePropertyName c1_timed_verified -NotePropertyValue $c1TimedVerified -Force
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

function Get-DeadlineCrossingToPreCallSample {
    param($Record, [double]$QpcFrequencyHz)
    $crossingAvailable = [bool](Get-OptionalProperty $Record 'waiter_deadline_crossing_qpc_available')
    $crossingProvenance = Get-OptionalProperty $Record 'waiter_deadline_crossing_qpc_provenance'
    $preCallAvailable = [bool](Get-OptionalProperty $Record 'pre_call_qpc_available')
    $preCallProvenance = Get-OptionalProperty $Record 'pre_call_qpc_provenance'
    if (-not $crossingAvailable -or $crossingProvenance -ne 'measured' -or
        -not $preCallAvailable -or $preCallProvenance -ne 'measured' -or $QpcFrequencyHz -le 0) {
        return [pscustomobject]@{ available = $false; negative_order = $false; value_us = $null }
    }
    $delta = [double]$Record.pre_call_qpc_ticks - [double]$Record.waiter_deadline_crossing_qpc_ticks
    if ($delta -lt 0) {
        return [pscustomobject]@{ available = $false; negative_order = $true; value_us = $null }
    }
    [pscustomobject]@{
        available = $true
        negative_order = $false
        value_us = ($delta * 1000000.0) / $QpcFrequencyHz
    }
}

function Get-TimingMetrics {
    param($Report)
    $details = Get-OptionalProperty $Report 'details'
    $telemetry = Get-OptionalProperty $details 'native_timing_telemetry'
    if ($null -eq $telemetry) { return $null }
    $frequency = [double]$telemetry.qpc_frequency_hz
    if ($frequency -le 0) { return $null }
    $metricNames = @(
        'authored_lateness_per_chord_us', 'floor_displacement_us', 'residual_lateness_us',
        'waiter_to_pre_call_us', 'deadline_crossing_to_pre_call_us',
        'sender_envelope_per_syscall_us', 'timer_requested_delay_us',
        'timer_arm_to_return_us', 'timer_wake_residual_us', 'eligibility_minus_timer_arm_target_us',
        'realized_hold_H_us', 'realized_gap_G_us'
    )
    $values = [ordered]@{}
    foreach ($name in $metricNames) { $values[$name] = [System.Collections.Generic.List[double]]::new() }
    $kindValues = [ordered]@{}
    foreach ($kind in @('down_only', 'up_only', 'mixed')) {
        $kindValues[$kind] = [ordered]@{
            authored_lateness_us = [System.Collections.Generic.List[double]]::new()
            floor_displacement_us = [System.Collections.Generic.List[double]]::new()
            residual_lateness_us = [System.Collections.Generic.List[double]]::new()
            sender_envelope_per_syscall_us = [System.Collections.Generic.List[double]]::new()
        }
    }
    $widthValues = @{}
    $gapValues = [ordered]@{
        hot = [System.Collections.Generic.List[double]]::new()
        cold = [System.Collections.Generic.List[double]]::new()
    }
    $lastDownCompletion = [System.Collections.Generic.Dictionary[int, double]]::new()
    $lastUpCompletion = [System.Collections.Generic.Dictionary[int, double]]::new()
    $identityRecords = [System.Collections.Generic.List[object]]::new()
    $upSlotRecords = @(Get-OptionalProperty $telemetry 'up_slot_records' | Where-Object { $null -ne $_ })
    $upSlotRecordsDropped = [long](Get-OptionalProperty $telemetry 'up_slot_records_dropped')
    $upSlotAttributionMissingPackets = @($telemetry.records | Where-Object { [bool]$_.up_slot_attribution_missing }).Count
    $counts = [ordered]@{
        trace_records = @($telemetry.records).Count
        up_slot_sidecar_records = $upSlotRecords.Count
        up_slot_sidecar_records_dropped = $upSlotRecordsDropped
        up_slot_attribution_missing_packets = $upSlotAttributionMissingPackets
        down_only_chords = 0
        up_only_events = 0
        mixed_events = 0
        down_chord_syscalls = 0
        attempted_down_send_calls = 0
        completed_down_chords = 0
        failed_down_chords = 0
        sendinput_calls = 0
        timer_arm_samples = 0
        timer_return_samples = 0
        timer_generation_matched = 0
        timer_generation_mismatch = 0
        timer_pair_unavailable = 0
        timer_target_provenance_invalid = 0
        negative_order = 0
        missing_authored = 0
        missing_floor_displacement = 0
        missing_residual = 0
        missing_waiter_to_pre_call = 0
        missing_deadline_crossing_to_pre_call = 0
        missing_sender_envelope = 0
        missing_timer_arm = 0
        missing_timer_return = 0
        missing_H_pair = 0
        missing_G_pair = 0
        duplicate_event_identity = 0
    }
    $seenDownChords = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    $lastDownAuthored = $null
    $lastDownIdentity = $null

    foreach ($record in @($telemetry.records)) {
        $kindCode = [int]$record.kind
        $kindName = switch ($kindCode) { 0 { 'down_only' } 1 { 'up_only' } 2 { 'mixed' } default { $null } }
        $downMask = [int]$record.down_mask
        $upMask = [int]$record.up_mask
        $downWidth = 0
        for ($bit = 0; $bit -lt 15; $bit++) { if (($downMask -band (1 -shl $bit)) -ne 0) { $downWidth++ } }
        if ($kindCode -eq 0) { $counts.down_only_chords++ }
        if ($kindCode -eq 1) { $counts.up_only_events++ }
        if ($kindCode -eq 2) { $counts.mixed_events++ }
        $counts.sendinput_calls += [long]$record.send_attempts
        if ($kindCode -eq 0 -or $kindCode -eq 2) {
            $counts.down_chord_syscalls++
            if ([int]$record.send_attempts -gt 0) { $counts.attempted_down_send_calls += [int]$record.send_attempts }
            if ([int]$record.send_status -eq 0 -and ([int]$record.flags -band 1) -ne 0 -and [int]$record.sent_count -eq [int]$record.requested_count) {
                $counts.completed_down_chords++
            } elseif ([int]$record.send_attempts -gt 0) { $counts.failed_down_chords++ }
            if ($downWidth -gt 0) {
                $widthKey = 'width_' + $downWidth
                if (-not $widthValues.ContainsKey($widthKey)) {
                    $widthValues[$widthKey] = [ordered]@{
                        authored_lateness_per_chord_us = [System.Collections.Generic.List[double]]::new()
                        sender_envelope_per_syscall_us = [System.Collections.Generic.List[double]]::new()
                    }
                }
            }
            $packetIndex = if ([bool]$record.compiled_packet_index_available) { [string]$record.compiled_packet_index } else { 'none' }
            $identity = [string]$packetIndex + '/' + [string]$record.event_index
            if ($seenDownChords.Add($identity)) {
                $identityRecords.Add([ordered]@{
                    compiled_packet_index = if ([bool]$record.compiled_packet_index_available) { $record.compiled_packet_index } else { $null }
                    event_index = $record.event_index
                    source_action_index = $record.source_action_index
                    kind = $kindName
                    up_mask = $upMask
                    down_mask = $downMask
                    down_chord_width = $downWidth
                    send_attempts = $record.send_attempts
                })
                if ($null -ne $lastDownAuthored -and $identity -ne $lastDownIdentity) {
                    $gapTicks = [double]$record.authored_offset_ticks - [double]$lastDownAuthored
                    if ($gapTicks -lt 0) { $counts.negative_order++ }
                    else {
                        $gapUs = ($gapTicks * 1000000.0) / $frequency
                        if ($gapUs -lt 20000.0) { $gapValues.hot.Add($gapUs) } else { $gapValues.cold.Add($gapUs) }
                    }
                }
                $lastDownAuthored = [double]$record.authored_offset_ticks
                $lastDownIdentity = $identity
            } else { $counts.duplicate_event_identity++ }
        }

        if ([bool]$record.authored_lateness_available) {
            $value = ([double]$record.authored_lateness_ticks * 1000000.0) / $frequency
            $values.authored_lateness_per_chord_us.Add($value)
            if ($null -ne $kindName) { $kindValues[$kindName].authored_lateness_us.Add($value) }
            if (($kindCode -eq 0 -or $kindCode -eq 2) -and $downWidth -gt 0) {
                $valuesForWidth = $widthValues['width_' + $downWidth]
                if ($null -ne $valuesForWidth) { $valuesForWidth.authored_lateness_per_chord_us.Add($value) }
            }
        } else { $counts.missing_authored++ }

        if ([bool]$record.floor_displacement_available) {
            $value = ([double]$record.physical_floor_displacement_ticks * 1000000.0) / $frequency
            $values.floor_displacement_us.Add($value)
            if ($null -ne $kindName) { $kindValues[$kindName].floor_displacement_us.Add($value) }
        } else { $counts.missing_floor_displacement++ }

        if ([bool]$record.residual_lateness_available) {
            $value = ([double]$record.residual_lateness_ticks * 1000000.0) / $frequency
            $values.residual_lateness_us.Add($value)
            if ($null -ne $kindName) { $kindValues[$kindName].residual_lateness_us.Add($value) }
            if ($value -lt 0) { $counts.negative_order++ }
        } else { $counts.missing_residual++ }

        if ([bool]$record.waiter_return_qpc_available -and [bool]$record.pre_call_qpc_available) {
            $delta = [double]$record.pre_call_qpc_ticks - [double]$record.waiter_return_qpc_ticks
            if ($delta -lt 0) { $counts.negative_order++ }
            else { $values.waiter_to_pre_call_us.Add(($delta * 1000000.0) / $frequency) }
        } else { $counts.missing_waiter_to_pre_call++ }

        $crossingSample = Get-DeadlineCrossingToPreCallSample $record $frequency
        if ($crossingSample.available) {
            $values.deadline_crossing_to_pre_call_us.Add([double]$crossingSample.value_us)
        } elseif ($crossingSample.negative_order) {
            $counts.negative_order++
        } else {
            $counts.missing_deadline_crossing_to_pre_call++
        }

        if ([bool]$record.sendinput_envelope_available) {
            $value = ([double]$record.sendinput_envelope_ticks * 1000000.0) / $frequency
            $values.sender_envelope_per_syscall_us.Add($value)
            if ($null -ne $kindName) { $kindValues[$kindName].sender_envelope_per_syscall_us.Add($value) }
            if (($kindCode -eq 0 -or $kindCode -eq 2) -and $downWidth -gt 0) {
                $valuesForWidth = $widthValues['width_' + $downWidth]
                if ($null -ne $valuesForWidth) { $valuesForWidth.sender_envelope_per_syscall_us.Add($value) }
            }
        } else { $counts.missing_sender_envelope++ }

        $armQpcAvailable = [bool](Get-OptionalProperty $record 'timer_arm_qpc_available')
        $armTargetAvailable = [bool](Get-OptionalProperty $record 'timer_arm_target_qpc_available')
        $armGenerationAvailable = [bool](Get-OptionalProperty $record 'timer_arm_generation_available')
        $armQpcProvenance = Get-OptionalProperty $record 'timer_arm_qpc_provenance'
        $armTargetProvenance = Get-OptionalProperty $record 'timer_arm_target_qpc_provenance'
        $validArmTarget = $armTargetProvenance -eq 'derived_from_timer_arm'
        if ($armTargetAvailable -and -not $validArmTarget) { $counts.timer_target_provenance_invalid++ }
        $returnAvailable = [bool](Get-OptionalProperty $record 'timer_return_qpc_available')
        $returnGenerationAvailable = [bool](Get-OptionalProperty $record 'timer_return_generation_available')
        if ($armQpcAvailable -and $armTargetAvailable -and $armGenerationAvailable) {
            $counts.timer_arm_samples++
            $armDelay = [double]$record.timer_arm_target_qpc_ticks - [double]$record.timer_arm_qpc_ticks
            if ($armDelay -lt 0) { $counts.negative_order++ }
            elseif ($validArmTarget) { $values.timer_requested_delay_us.Add(($armDelay * 1000000.0) / $frequency) }
            if ($validArmTarget -and [bool]$record.physical_eligibility_qpc_available) {
                $values.eligibility_minus_timer_arm_target_us.Add((( [double]$record.physical_eligibility_qpc_ticks - [double]$record.timer_arm_target_qpc_ticks) * 1000000.0) / $frequency)
            }
        } else { $counts.missing_timer_arm++ }
        if ($returnAvailable -and $returnGenerationAvailable) { $counts.timer_return_samples++ } else { $counts.missing_timer_return++ }
        if ($armTargetAvailable -and $returnAvailable -and $armGenerationAvailable -and $returnGenerationAvailable) {
            if ([long]$record.timer_arm_generation -eq [long]$record.timer_return_generation) {
                $counts.timer_generation_matched++
                $wakeResidual = [double]$record.timer_return_qpc_ticks - [double]$record.timer_arm_target_qpc_ticks
                if ($validArmTarget) { $values.timer_wake_residual_us.Add(($wakeResidual * 1000000.0) / $frequency) }
                if ($armQpcAvailable -and $armQpcProvenance -eq 'measured') {
                    $armToReturn = [double]$record.timer_return_qpc_ticks - [double]$record.timer_arm_qpc_ticks
                    if ($armToReturn -lt 0) { $counts.negative_order++ }
                    else { $values.timer_arm_to_return_us.Add(($armToReturn * 1000000.0) / $frequency) }
                }
            } else { $counts.timer_generation_mismatch++ }
        } else { $counts.timer_pair_unavailable++ }

        $preAvailable = [bool]$record.pre_call_qpc_available
        $completionAvailable = [bool]$record.sendinput_completion_qpc_available
        $preQpc = if ($preAvailable) { [double]$record.pre_call_qpc_ticks } else { $null }
        $completionQpc = if ($completionAvailable) { [double]$record.sendinput_completion_qpc_ticks } else { $null }
        if ($null -ne $preQpc -and $null -ne $completionQpc) {
            for ($slot = 0; $slot -lt 15; $slot++) {
                $slotMask = 1 -shl $slot
                if (($upMask -band $slotMask) -ne 0) {
                    if ($lastDownCompletion.ContainsKey($slot)) {
                        $hold = $preQpc - $lastDownCompletion[$slot]
                        if ($hold -lt 0) { $counts.negative_order++ }
                        else { $values.realized_hold_H_us.Add(($hold * 1000000.0) / $frequency) }
                        [void]$lastDownCompletion.Remove($slot)
                    } else { $counts.missing_H_pair++ }
                    $lastUpCompletion[$slot] = $completionQpc
                }
                if (($downMask -band $slotMask) -ne 0) {
                    if ($lastUpCompletion.ContainsKey($slot)) {
                        $gap = $preQpc - $lastUpCompletion[$slot]
                        if ($gap -lt 0) { $counts.negative_order++ }
                        else { $values.realized_gap_G_us.Add(($gap * 1000000.0) / $frequency) }
                    } else { $counts.missing_G_pair++ }
                    $lastDownCompletion[$slot] = $completionQpc
                }
            }
        }
    }

    $perKind = [ordered]@{}
    foreach ($kind in $kindValues.Keys) {
        $perKind[$kind] = [ordered]@{
            authored_lateness_per_chord_us = Get-Percentiles $kindValues[$kind].authored_lateness_us.ToArray()
            floor_displacement_us = Get-Percentiles $kindValues[$kind].floor_displacement_us.ToArray()
            residual_lateness_us = Get-Percentiles $kindValues[$kind].residual_lateness_us.ToArray()
            sender_envelope_per_syscall_us = Get-Percentiles $kindValues[$kind].sender_envelope_per_syscall_us.ToArray()
        }
    }
    $widthMetrics = [ordered]@{}
    foreach ($key in ($widthValues.Keys | Sort-Object)) {
        $widthMetrics[$key] = [ordered]@{
            authored_lateness_per_chord_us = Get-Percentiles $widthValues[$key].authored_lateness_per_chord_us.ToArray()
            sender_envelope_per_syscall_us = Get-Percentiles $widthValues[$key].sender_envelope_per_syscall_us.ToArray()
        }
    }
    $allMetrics = [ordered]@{}
    foreach ($name in $metricNames) { $allMetrics[$name] = Get-Percentiles $values[$name].ToArray() }
    [ordered]@{
        summary_scope = 'packet_trace_records'
        up_slot_sidecar_consumed_by_summary = $false
        per_key_authored_up_p95_p99_supported = $false
        metrics = $allMetrics
        per_kind = $perKind
        down_chord_widths = $widthMetrics
        authored_gap_hot_cold_us = [ordered]@{
            hot_threshold_us = 20000
            hot = Get-Percentiles $gapValues.hot.ToArray()
            cold = Get-Percentiles $gapValues.cold.ToArray()
        }
        counts = $counts
        event_chord_identities = $identityRecords.ToArray()
        telemetry_accepted = [long]$telemetry.accepted
        telemetry_dropped = [long]$telemetry.dropped
        observer_queue_dropped = [long]$telemetry.observer_queue_dropped
        telemetry_truncated = [bool]$telemetry.truncated
        qpc_frequency_hz = [long]$frequency
        cpu = [ordered]@{
            worker_cpu_time_us = Get-OptionalProperty $details 'worker_cpu_time_us'
            process_cpu_time_us = Get-OptionalProperty $details 'process_cpu_time_us'
            spin_time_us = Get-OptionalProperty $details 'spin_time_us'
            spin_time_available = Get-OptionalProperty $details 'spin_time_available'
            spin_time_complete = Get-OptionalProperty $details 'spin_time_complete'
            spin_duty_cycle_ppm = Get-OptionalProperty $details 'spin_duty_cycle_ppm'
            spin_expected_samples = Get-OptionalProperty $details 'spin_observation_expected_samples'
            spin_dropped_samples = Get-OptionalProperty $details 'spin_observation_dropped_samples'
            spin_missing_samples = Get-OptionalProperty $details 'spin_observation_missing_samples'
        }
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
        '-QualificationProfile', $QualificationProfile,
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
    $commandText = $commandText.Replace($caseRunner, 'scripts/run_native_acceptance_case.ps1')
    $commandText = $commandText.Replace($HarnessManifest.resolved_binary_path, '<immutable-binary>')
    $commandText = $commandText.Replace($HarnessManifest.manifest_path, '<immutable-manifest>')
    $commandText = $commandText.Replace($sessionRoot, '<sessions-root>')
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
    $details = if ($null -ne $report) { Get-OptionalProperty $report 'details' } else { $null }
    $invocationPath = if ($caseDirectory) { Join-Path $caseDirectory 'invocation.json' } else { $null }
    $invocation = if ($invocationPath -and (Test-Path -LiteralPath $invocationPath -PathType Leaf)) { Get-Content -LiteralPath $invocationPath -Raw | ConvertFrom-Json } else { $null }
    $row = [pscustomobject]@{
        run_id = if ($null -ne $report) { [string]$report.run_id } else { $null }
        stage = $Stage
        cell_id = $CellId
        pair_id = $PairId
        pair_order = $Order
        role = $Role
        source_revision = $HarnessManifest.source_revision
        source_tree_sha = $HarnessManifest.source_tree_sha
        control_kind = Get-OptionalProperty $HarnessManifest 'control_kind'
        control_label = Get-QualificationReportRowLabel -Profile $QualificationProfile -Role $Role -Manifest $HarnessManifest
        base_source_revision = Get-OptionalProperty $HarnessManifest 'base_source_revision'
        base_source_tree_sha = Get-OptionalProperty $HarnessManifest 'base_source_tree_sha'
        base_source_archive_sha256 = Get-OptionalProperty $HarnessManifest 'base_source_archive_sha256'
        base_source_file_sha256 = Get-OptionalProperty $HarnessManifest 'base_source_file_sha256'
        overlay_sha256 = Get-OptionalProperty $HarnessManifest 'overlay_sha256'
        derived_source_file_sha256 = Get-OptionalProperty $HarnessManifest 'derived_source_file_sha256'
        source_changes_sha256 = Get-OptionalProperty $HarnessManifest 'source_changes_sha256'
        derived_source_inventory_sha256 = Get-OptionalProperty $HarnessManifest 'derived_source_inventory_sha256'
        harness_sha256 = $HarnessManifest.binary_sha256
        profile = $HarnessManifest.cargo_profile
        features = (@($HarnessManifest.cargo_features) -join ',')
        observer_mode = $HarnessManifest.observer_mode
        receiveonly_observer_mode = Get-OptionalProperty $invocation 'receiveonly_observer_mode'
        receiveonly_observer_version = Get-OptionalProperty $invocation 'receiveonly_observer_version'
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
        evidence_directory = if ($caseDirectory) { [System.IO.Path]::GetRelativePath($evidenceRoot, $caseDirectory) } else { $null }
        report_path = if ($reportPath) { [System.IO.Path]::GetRelativePath($evidenceRoot, $reportPath) } else { $null }
        expected_sink_event_count = Get-OptionalProperty $details 'expected_sink_event_count'
        observed_sink_event_count = Get-OptionalProperty $details 'observed_sink_event_count'
        sink_window_match = Get-OptionalProperty $invocation 'report_window_match'
        active_count = Get-OptionalProperty $details 'active_count'
        possibly_active_count = Get-OptionalProperty $details 'possibly_active_count'
        final_release_obligation_mask = Get-OptionalProperty $details 'final_release_obligation_mask'
        production_hold_floor_violation_count = Get-OptionalProperty $details 'production_hold_floor_violation_count'
        production_release_floor_violation_count = Get-OptionalProperty $details 'production_release_floor_violation_count'
        production_forensics_anomaly_count = Get-OptionalProperty $details 'production_forensics_anomaly_count'
        production_structural_anomaly_count = Get-OptionalProperty $details 'production_structural_anomaly_count'
        production_same_key_overlap_forensics_count = Get-OptionalProperty $details 'production_same_key_overlap_forensics_count'
        production_anchor_overwrite_count = Get-OptionalProperty $details 'production_anchor_overwrite_count'
        production_unmatched_up_count = Get-OptionalProperty $details 'production_unmatched_up_count'
        missed_down_boundaries = Get-OptionalProperty $details 'missed_down_boundaries'
        missed_down_keys = Get-OptionalProperty $details 'missed_down_keys'
        missed_physical_window_boundaries = Get-OptionalProperty $details 'missed_physical_window_boundaries'
        keys_dropped = Get-OptionalProperty $details 'keys_dropped'
        generation_total = Get-OptionalProperty $details 'generation_total'
        generation_activated = Get-OptionalProperty $details 'generation_activated'
        generation_released = Get-OptionalProperty $details 'generation_released'
        generation_cancelled = Get-OptionalProperty $details 'generation_cancelled'
        generation_scheduled = Get-OptionalProperty $details 'generation_scheduled'
        chord_split_events = Get-OptionalProperty $details 'chord_split_events'
        sendinput_partial_events = Get-OptionalProperty $details 'sendinput_partial_events'
        sendinput_zero_progress_failures = Get-OptionalProperty $details 'sendinput_zero_progress_failures'
        expected_down_key_count = Get-OptionalProperty $details 'expected_down_key_count'
        expected_up_key_count = Get-OptionalProperty $details 'expected_up_key_count'
        precision_packet_attempts = Get-OptionalProperty $details 'precision_packet_attempts'
        authored_lateness_samples = Get-OptionalProperty $details 'authored_lateness_samples'
        residual_lateness_samples = Get-OptionalProperty $details 'residual_lateness_samples'
        sendinput_envelope_samples = Get-OptionalProperty $details 'sendinput_envelope_samples'
        qpc_read_count_available = Get-OptionalProperty $details 'qpc_read_count_available'
        qpc_read_count = Get-OptionalProperty $details 'qpc_read_count'
        native_timing_telemetry_available = Get-OptionalProperty $details 'native_timing_telemetry_available'
        native_timing_telemetry_unavailable_reason = Get-OptionalProperty $details 'native_timing_telemetry_unavailable_reason'
        worker_cpu_time_us = Get-OptionalProperty $details 'worker_cpu_time_us'
        process_cpu_time_us = Get-OptionalProperty $details 'process_cpu_time_us'
        spin_time_us = Get-OptionalProperty $details 'spin_time_us'
        spin_time_available = Get-OptionalProperty $details 'spin_time_available'
        spin_time_complete = Get-OptionalProperty $details 'spin_time_complete'
        spin_duty_cycle_ppm = Get-OptionalProperty $details 'spin_duty_cycle_ppm'
        spin_expected_samples = Get-OptionalProperty $details 'spin_observation_expected_samples'
        spin_dropped_samples = Get-OptionalProperty $details 'spin_observation_dropped_samples'
        spin_missing_samples = Get-OptionalProperty $details 'spin_observation_missing_samples'
        telemetry_trace_records = if ($null -ne $metrics) { $metrics.counts.trace_records } else { $null }
        observed_down_chords = if ($null -ne $metrics) { $metrics.counts.down_chord_syscalls } else { $null }
        attempted_down_send_calls = if ($null -ne $metrics) { $metrics.counts.attempted_down_send_calls } else { $null }
        completed_down_chords = if ($null -ne $metrics) { $metrics.counts.completed_down_chords } else { $null }
        failed_down_chords = if ($null -ne $metrics) { $metrics.counts.failed_down_chords } else { $null }
        actual_sendinput_calls = if ($null -ne $metrics) { $metrics.counts.sendinput_calls } else { $null }
        telemetry_generation_mismatch = if ($null -ne $metrics) { $metrics.counts.timer_generation_mismatch } else { $null }
        telemetry_negative_order = if ($null -ne $metrics) { $metrics.counts.negative_order } else { $null }
        telemetry_dropped = if ($null -ne $metrics) { $metrics.telemetry_dropped } else { $null }
        observer_queue_dropped = if ($null -ne $metrics) { $metrics.observer_queue_dropped } else { $null }
        timing_metrics_json = if ($null -ne $metrics) { ConvertTo-Json -InputObject $metrics -Depth 12 -Compress } else { $null }
        timing_metrics = $metrics
        report = $report
        runner_output = if ($caseDirectory) { $text.Replace($caseDirectory, '<evidence-directory>').Replace($evidenceRoot, '<evidence-root>') } else { $text }
    }
    $script:runRows.Add($row)
    if ($caseExit -ne 0 -or $status -ne 'PASS') {
        $script:failureRows.Add([pscustomobject]@{ scenario = $Scenario; workload = $Workload; role = $Role; cell_id = $CellId; pair_id = $PairId; exit_code = $caseExit; status = $status; reason = $reason; evidence_directory = $caseDirectory })
        if ($caseExit -eq 2 -or $status -eq 'RUNNER_FAILURE' -or $status -eq 'MISSING_REPORT') { $script:exitCode = 2 } else { $script:exitCode = [Math]::Max($script:exitCode, 1) }
    }
    Write-Host ($Role + ' ' + $Scenario + ' [' + $Load + '] ' + $status + ' exit=' + $caseExit + $(if ($caseDirectory) { ' evidence=' + $caseDirectory } else { '' }))
    $row
}

function Invoke-NativeCaseFailStopSequence {
    param([object[]]$Cases, [scriptblock]$InvokeCase)
    $attemptedRows = [System.Collections.Generic.List[object]]::new()
    foreach ($case in $Cases) {
        # InvokeCase is synchronous: a native process completes before its row
        # is inspected, so fail-stop never interrupts an in-flight case.
        $row = & $InvokeCase $case
        if ($null -eq $row) { throw 'native case callback did not return a completed case row' }
        $attemptedRows.Add($row)
        if ([int](Get-OptionalProperty $row 'case_exit_code') -ne 0 -or
            [string](Get-OptionalProperty $row 'status') -ne 'PASS') {
            return [pscustomobject]@{
                attempted_rows = $attemptedRows.ToArray()
                stopped_at_first_native_failure = $true
            }
        }
    }
    [pscustomobject]@{
        attempted_rows = $attemptedRows.ToArray()
        stopped_at_first_native_failure = $false
    }
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
                    $completedCase = $script:runRows[$script:runRows.Count - 1]
                    if ($completedCase.case_exit_code -ne 0 -or $completedCase.status -ne 'PASS') { $script:stoppedAtFirstNativeFailure = $true; return }
                }
            }
        }
        $workloadIndex++
    }
}

function Get-MetricPath {
    param($Metrics, [string]$Path)
    $current = $Metrics
    foreach ($part in $Path.Split('.')) {
        $current = Get-OptionalProperty $current $part
        if ($null -eq $current) { return $null }
    }
    $current
}

function Get-ComparisonDescription {
param([string]$Profile, $Control, $Candidate)
if ($Profile -eq 'FrameSnapshot') { return 'FrameSnapshot baseline vs FrameSnapshot candidate; same frame-snapshot-v1 workload and zero-margin performance policy' }
if ($Profile -eq 'Scheduler') { return 'C0 Scheduler baseline vs Scheduler-Candidate' }
if ($Control -and $Control.guard_mode -eq 'frozen' -and $Candidate.guard_mode -eq 'adaptive') { return 'Frozen vs Adaptive' }
'C1-TIMED-R2 vs Candidate-R2; C0 not timed-qualified'
}

function Assert-QualificationTimingMargin {
    param(
        [ValidateSet('Canary', 'Smoke', 'Compare', 'Final')] [string]$RequestedStage,
        [ValidateSet('DispatchCore', 'Scheduler', 'FrameSnapshot')] [string]$Profile,
        [ValidateSet(0, 500)] [int]$MarginUs,
        [int]$PairCount,
        [int]$SampleDurationSeconds
    )
    if ($RequestedStage -in @('Canary', 'Smoke')) {
        if ($Profile -eq 'FrameSnapshot' -and $MarginUs -ne 500) {
            throw ('FrameSnapshot ' + $RequestedStage + ' is a candidate safety phase and requires 500 us')
        }
        return $true
    }
    $expectedMarginUs = if ($Profile -eq 'FrameSnapshot') { 0 } else { 500 }
    if ($PairCount -ne 6 -or $SampleDurationSeconds -ne 15 -or $MarginUs -ne $expectedMarginUs) {
        throw ($RequestedStage + ' requires exactly six paired runs, 15-second samples, and a ' + $expectedMarginUs + ' us timing margin for ' + $Profile)
    }
    $true
}

function Assert-SchedulerObserverPairMode {
    param($Control, $Candidate)
    if ($QualificationProfile -notin @('Scheduler', 'FrameSnapshot')) { return }
    $expectedVersion = 'scheduler-receiveonly-queue-v3-compat3'
    foreach ($row in @($Control, $Candidate)) {
        if ($row.receiveonly_observer_mode -ne 'SchedulerQueuedV3Compat3' -or
            $row.receiveonly_observer_version -ne $expectedVersion) {
            throw ($QualificationProfile + ' baseline and candidate must use SchedulerQueuedV3Compat3 with the same observer version')
        }
    }
}

function Get-PairedMetrics {
    $pairRows = [System.Collections.Generic.List[object]]::new()
    $metricPaths = @(
        'metrics.authored_lateness_per_chord_us', 'metrics.floor_displacement_us',
        'metrics.residual_lateness_us', 'metrics.waiter_to_pre_call_us',
        'metrics.sender_envelope_per_syscall_us', 'metrics.timer_arm_to_return_us',
        'metrics.timer_wake_residual_us',
        'metrics.realized_hold_H_us', 'metrics.realized_gap_G_us',
        'per_kind.down_only.authored_lateness_per_chord_us', 'per_kind.up_only.authored_lateness_per_chord_us',
        'per_kind.mixed.authored_lateness_per_chord_us', 'per_kind.down_only.residual_lateness_us',
        'per_kind.up_only.residual_lateness_us', 'per_kind.mixed.residual_lateness_us',
        'per_kind.down_only.sender_envelope_per_syscall_us',
        'per_kind.up_only.sender_envelope_per_syscall_us',
        'per_kind.mixed.sender_envelope_per_syscall_us'
    )
    foreach ($group in ($runRows | Where-Object { $_.pair_id } | Group-Object pair_id)) {
        $baseline = @($group.Group | Where-Object role -eq 'control') | Select-Object -First 1
        $next = @($group.Group | Where-Object role -eq 'candidate') | Select-Object -First 1
        if ($null -eq $baseline -or $null -eq $next) { continue }
        Assert-SchedulerObserverPairMode -Control $baseline -Candidate $next
        foreach ($metricPath in $metricPaths) {
            $controlMetric = if ($baseline.status -eq 'PASS') { Get-MetricPath $baseline.timing_metrics $metricPath } else { $null }
            $candidateMetric = if ($next.status -eq 'PASS') { Get-MetricPath $next.timing_metrics $metricPath } else { $null }
            $controlP50 = Get-OptionalProperty $controlMetric 'p50'
            $controlP95 = Get-OptionalProperty $controlMetric 'p95'
            $controlP99 = Get-OptionalProperty $controlMetric 'p99'
            $controlMax = Get-OptionalProperty $controlMetric 'max'
            $candidateP50 = Get-OptionalProperty $candidateMetric 'p50'
            $candidateP95 = Get-OptionalProperty $candidateMetric 'p95'
            $candidateP99 = Get-OptionalProperty $candidateMetric 'p99'
            $candidateMax = Get-OptionalProperty $candidateMetric 'max'
            $available = $null -ne $controlP95 -and $null -ne $controlP99 -and $null -ne $candidateP95 -and $null -ne $candidateP99
            $deltaP95 = if ($available) { [double]$candidateP95 - [double]$controlP95 } else { $null }
            $deltaP99 = if ($available) { [double]$candidateP99 - [double]$controlP99 } else { $null }
            $p95Limit = if ($available) { [Math]::Max(5.0, [double]$controlP95 * 0.10) } else { $null }
            $p99Limit = if ($available) { [Math]::Max(20.0, [double]$controlP99 * 0.10) } else { $null }
            $p95Regression = $available -and $deltaP95 -gt $p95Limit
            $p99Regression = $available -and $deltaP99 -gt $p99Limit
            $pairRows.Add([pscustomobject]@{
                cell_id = $baseline.cell_id
                pair_id = $group.Name
                order = $baseline.pair_order
                receiveonly_observer_version = if ($QualificationProfile -eq 'Scheduler') { $baseline.receiveonly_observer_version } else { $null }
                metric = $metricPath
                control_status = $baseline.status
                candidate_status = $next.status
                available = $available
                control_p50_us = $controlP50
                candidate_p50_us = $candidateP50
                delta_p50_us = if ($null -ne $controlP50 -and $null -ne $candidateP50) { [double]$candidateP50 - [double]$controlP50 } else { $null }
                control_p95_us = $controlP95
                candidate_p95_us = $candidateP95
                delta_p95_us = $deltaP95
                p95_regression_limit_us = $p95Limit
                p95_regression = $p95Regression
                control_p99_us = $controlP99
                candidate_p99_us = $candidateP99
                delta_p99_us = $deltaP99
                p99_regression_limit_us = $p99Limit
                p99_regression = $p99Regression
                control_max_us = $controlMax
                candidate_max_us = $candidateMax
                delta_max_us = if ($null -ne $controlMax -and $null -ne $candidateMax) { [double]$candidateMax - [double]$controlMax } else { $null }
                control_samples = Get-OptionalProperty $controlMetric 'sample_count'
                candidate_samples = Get-OptionalProperty $candidateMetric 'sample_count'
                favorable_p95 = $available -and $deltaP95 -le 0
                favorable_p99 = $available -and $deltaP99 -le 0
                regression_screen_triggered = $p95Regression -or $p99Regression
            })
        }
    }

    $cellRows = [System.Collections.Generic.List[object]]::new()
    foreach ($group in ($pairRows | Group-Object { $_.cell_id + '|' + $_.metric })) {
        $rows = @($group.Group)
        $availableRows = @($rows | Where-Object available)
        $p95Worse = @($availableRows | Where-Object p95_regression).Count
        $p99Worse = @($availableRows | Where-Object p99_regression).Count
        $triggered = @($availableRows | Where-Object regression_screen_triggered).Count
        $deltaP95 = @($availableRows | ForEach-Object { [double]$_.delta_p95_us })
        $deltaP99 = @($availableRows | ForEach-Object { [double]$_.delta_p99_us })
        $p95DeltaSummary = Get-Percentiles $deltaP95
        $p99DeltaSummary = Get-Percentiles $deltaP99
        $first = $rows | Select-Object -First 1
        $cellRows.Add([pscustomobject]@{
            cell_id = $first.cell_id
            metric = $first.metric
            attempted_pairs = $rows.Count
            available_pairs = $availableRows.Count
            favorable_p95_pairs = @($availableRows | Where-Object favorable_p95).Count
            unfavorable_p95_pairs = @($availableRows | Where-Object { -not $_.favorable_p95 }).Count
            favorable_p99_pairs = @($availableRows | Where-Object favorable_p99).Count
            unfavorable_p99_pairs = @($availableRows | Where-Object { -not $_.favorable_p99 }).Count
            p95_regression_pairs = $p95Worse
            p99_regression_pairs = $p99Worse
            pairs_with_any_tail_regression = $triggered
            screen_complete = $rows.Count -eq 6 -and $availableRows.Count -eq 6
            leader_review_prompt = Get-DispatchPairLeaderReviewPrompt -AttemptedPairs $rows.Count -AvailablePairs $availableRows.Count -P95RegressionPairs $p95Worse -P99RegressionPairs $p99Worse
            p95_delta_sample_count = $deltaP95.Count
            p95_delta_p50_us = Get-OptionalProperty $p95DeltaSummary 'p50'
            p95_delta_p95_us = Get-OptionalProperty $p95DeltaSummary 'p95'
            p95_delta_p99_us = Get-OptionalProperty $p95DeltaSummary 'p99'
            p95_delta_max_us = Get-OptionalProperty $p95DeltaSummary 'max'
            p99_delta_sample_count = $deltaP99.Count
            p99_delta_p50_us = Get-OptionalProperty $p99DeltaSummary 'p50'
            p99_delta_p95_us = Get-OptionalProperty $p99DeltaSummary 'p95'
            p99_delta_p99_us = Get-OptionalProperty $p99DeltaSummary 'p99'
            p99_delta_max_us = Get-OptionalProperty $p99DeltaSummary 'max'
            no_outlier_removed = $true
        })
    }
    [ordered]@{ paired_sessions = $pairRows.ToArray(); cell_summary = $cellRows.ToArray() }
}

try {
    if ($Stage -in @('Canary', 'Smoke') -and $ControlManifest) { throw ($Stage + ' accepts only -CandidateManifest; control manifests are reserved for Compare and Final') }
    if ($Stage -eq 'Canary' -and $QualificationProfile -ne 'FrameSnapshot') { throw 'Canary is available only for FrameSnapshot' }
    if ($Stage -in @('Compare', 'Final') -and -not $ControlManifest) { throw ($Stage + ' requires -ControlManifest') }
    Assert-QualificationTimingMargin -RequestedStage $Stage -Profile $QualificationProfile -MarginUs $TimingMarginUs -PairCount $Pairs -SampleDurationSeconds $SampleSeconds | Out-Null
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
        if (-not $qualifiedProfile -and $Stage -eq 'Compare' -and -not $control.c1_timed_verified -and -not ($control.guard_mode -eq 'frozen' -and $candidate.guard_mode -eq 'adaptive')) {
            throw 'Compare requires the leader-approved C1-TIMED control or a Frozen vs Adaptive comparison'
        }
        if ($Stage -eq 'Final' -and $control.c1_timed_verified) {
            throw 'C1-TIMED is authorized only for Compare A, not Final'
        }
        if ($control.rustc_version -ne $candidate.rustc_version -or $control.cargo_profile -ne $candidate.cargo_profile -or
            (@($control.cargo_features) -join ',') -ne (@($candidate.cargo_features) -join ',') -or
            $control.target -ne $candidate.target -or $control.workload_version -ne $candidate.workload_version -or
            ($qualifiedProfile -and ($control.qualification_profile -ne $candidate.qualification_profile -or
                $control.receiveonly_observer_mode -ne $candidate.receiveonly_observer_mode -or
                $control.receiveonly_observer_version -ne $candidate.receiveonly_observer_version)) -or
            ($QualificationProfile -eq 'FrameSnapshot' -and
                ($control.performance_timing_margin_us -ne 0 -or $candidate.performance_timing_margin_us -ne 0 -or
                    $control.safety_timing_margin_us -ne 500 -or $candidate.safety_timing_margin_us -ne 500))) {
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

    if ($Stage -eq 'Canary') {
        [void](Invoke-NativeCase -Scenario 'canonical-single' -Workload 'frame-snapshot-canary-v1' -Role 'candidate' -HarnessManifest $candidate -Load 'Quiet' -CellId 'frame-snapshot-canary' -PairId '' -Order '' -Margin $TimingMarginUs -Fps 60 -Sample 0 -Historical $false)
    } elseif ($Stage -eq 'Smoke') {
        $sequenceResult = Invoke-NativeCaseFailStopSequence -Cases $scenarios -InvokeCase {
            param($scenario)
            Invoke-NativeCase -Scenario $scenario -Workload 'safety-smoke' -Role 'candidate' -HarnessManifest $candidate -Load 'Quiet' -CellId 'safety-smoke' -PairId '' -Order '' -Margin $TimingMarginUs -Fps 60 -Sample 0 -Historical $false
        }
        if ($sequenceResult.stopped_at_first_native_failure) { $script:stoppedAtFirstNativeFailure = $true }
    } elseif ($Stage -eq 'Compare') {
        Invoke-PairedWorkloads $performanceWorkloads
    } else {
        Invoke-PairedWorkloads $performanceWorkloads
        if ($script:exitCode -eq 0) {
            $finalSafetyCases = @(
                foreach ($load in @('Quiet', 'Contention')) {
                    foreach ($scenario in $scenarios) {
                        [pscustomobject]@{ scenario = $scenario; workload = 'safety-smoke'; cell_id = 'final-safety-' + $load.ToLowerInvariant(); load = $load; margin = if ($QualificationProfile -eq 'FrameSnapshot') { $candidate.safety_timing_margin_us } else { $TimingMarginUs }; fps = 60 }
                    }
                }
                [pscustomobject]@{ scenario = 'timing-margin-sweep'; workload = 'boundary-60fps-margin-0'; cell_id = 'boundary-safety'; load = 'Quiet'; margin = 0; fps = 60 }
                [pscustomobject]@{ scenario = 'timing-margin-sweep'; workload = 'boundary-120fps-margin-500'; cell_id = 'boundary-safety'; load = 'Quiet'; margin = 500; fps = 120 }
            )
            $sequenceResult = Invoke-NativeCaseFailStopSequence -Cases $finalSafetyCases -InvokeCase {
                param($case)
                Invoke-NativeCase -Scenario $case.scenario -Workload $case.workload -Role 'candidate' -HarnessManifest $candidate -Load $case.load -CellId $case.cell_id -PairId '' -Order '' -Margin $case.margin -Fps $case.fps -Sample 0 -Historical $false
            }
            if ($sequenceResult.stopped_at_first_native_failure) { $script:stoppedAtFirstNativeFailure = $true }
        }
    }

    $source = Get-CurrentSourceIdentity
    $os = Get-CimInstance Win32_OperatingSystem
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $pairedMetrics = Get-PairedMetrics
    $reportProjection = New-QualificationReportProjection `
        -Profile $QualificationProfile `
        -Candidate $candidate `
        -Control $control `
        -Stage $Stage `
        -Pairs $Pairs `
        -SampleSeconds $SampleSeconds `
        -TimingMarginUs $TimingMarginUs
    $comparisonDescription = if ($Stage -eq 'Compare') {
        Get-ComparisonDescription -Profile $QualificationProfile -Control $control -Candidate $candidate
    } else { $null }
    $controlLabel = $reportProjection.ControlLabel
    $candidateLabel = $reportProjection.CandidateLabel
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
        comparison = $comparisonDescription
        qualification_profile = $QualificationProfile
        receiveonly_observer = [ordered]@{ mode = if ($qualifiedProfile) { 'SchedulerQueuedV3Compat3' } else { 'LegacyInline' }; version = if ($qualifiedProfile) { 'scheduler-receiveonly-queue-v3-compat3' } else { $null } }
        source = $source
        candidate = [ordered]@{ label = $candidateLabel; manifest_path = $candidate.manifest_path; manifest_sha256 = $candidate.manifest_sha256; qualification_profile = $reportProjection.CandidateQualificationProfile; source_role = Get-OptionalProperty $candidate 'source_role'; source_root = Get-OptionalProperty $candidate 'source_root'; source_revision = $candidate.source_revision; source_tree_sha = $candidate.source_tree_sha; base_source_revision = Get-OptionalProperty $candidate 'base_source_revision'; base_source_tree_sha = Get-OptionalProperty $candidate 'base_source_tree_sha'; binary_path = $candidate.resolved_binary_path; profile = $candidate.cargo_profile; features = @($candidate.cargo_features); target = $candidate.target; binary_sha256 = $candidate.binary_sha256; performance_timing_margin_us = $reportProjection.CandidateMargins.performance_timing_margin_us; safety_timing_margin_us = $reportProjection.CandidateMargins.safety_timing_margin_us }
        control = if ($control) { [ordered]@{ control_label = $controlLabel; manifest_path = $control.manifest_path; control_kind = Get-OptionalProperty $control 'control_kind'; manifest_sha256 = $control.manifest_sha256; qualification_profile = $reportProjection.ControlQualificationProfile; source_role = Get-OptionalProperty $control 'source_role'; source_root = Get-OptionalProperty $control 'source_root'; source_revision = $control.source_revision; source_tree_sha = $control.source_tree_sha; base_source_revision = Get-OptionalProperty $control 'base_source_revision'; base_source_tree_sha = Get-OptionalProperty $control 'base_source_tree_sha'; binary_path = $control.resolved_binary_path; profile = $control.cargo_profile; features = @($control.cargo_features); target = $control.target; binary_sha256 = $control.binary_sha256; performance_timing_margin_us = $reportProjection.ControlMargins.performance_timing_margin_us; safety_timing_margin_us = $reportProjection.ControlMargins.safety_timing_margin_us } } else { $null }
        parameters = $reportProjection.SummaryParameters
        environment = [ordered]@{ rustc_version = $rustcVersion.Trim(); windows_caption = $os.Caption; windows_version = $os.Version; windows_build = $os.BuildNumber; processor_name = $cpu.Name; processor_cores = $cpu.NumberOfCores; processor_logical_processors = $cpu.NumberOfLogicalProcessors; power_scheme = (& powercfg /getactivescheme | Out-String).Trim() }
        run_count = $runRows.Count
        passed_count = $passed
        failed_count = $failureRows.Count
        stopped_at_first_native_failure = $stoppedAtFirstNativeFailure
        hard_safety_review = $hardSafety
        performance_pairs = $pairedMetrics.paired_sessions
        performance_cells = $pairedMetrics.cell_summary
        measurement_definitions = [ordered]@{
            floor_displacement = 'physical eligibility/floor QPC minus authored target'
            residual_lateness = 'sender pre-call QPC minus physical eligibility/floor QPC'
            waiter_to_pre_call = 'sender pre-call QPC minus waiter-return QPC'
            sender_envelope = 'SendInput completion QPC minus sender pre-call QPC, per syscall'
            timer_arm_target = 'only records with derived_from_timer_arm provenance are compared; target is QPC arm sample plus the relative microsecond timeout passed to SetWaitableTimer'
            timer_arm_to_return = 'timer return QPC minus matching arm QPC; timer wake residual is return QPC minus requested timer target QPC'
            realized_H_G = 'per-key successful Down completion to authored Up pre-call (H), and successful Up completion to next Down pre-call (G)'
            per_key_authored_up = 'Up sidecar records are retained and counted, but this summary does not consume their per-slot authored offsets or floors; per-key authored Up p95/p99 are unsupported'
            packet_aggregates = 'p95/p99 for authored lateness, floor displacement, residual, waiter handoff, and sender envelope are packet trace record or per-syscall aggregates'
            hot_cold = 'authored Down-chord schedule gap; hot < 20000 us, cold >= 20000 us'
            paired_screen = 'candidate worse than max(5 us, 10% control) at p95 or max(20 us, 10% control) at p99 in at least 4 of 6 pairs prompts leader review'
        }
        remote_evidence = [ordered]@{ summary_and_manifest = 'PR report'; raw_bundle = 'PENDING user attachment; no existing workflow uploader used' }
        failures = $failureRows.ToArray()
        sessions = $runRows.ToArray()
        disposition = if ($stoppedAtFirstNativeFailure) { 'STOPPED_AT_FIRST_NATIVE_FAILURE' } elseif ($exitCode -eq 0) { 'collected; leader review required for performance gates' } else { 'one or more cases failed; leader review required' }
    }
    $summary | ConvertTo-Json -Depth 14 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'summary.json') -Encoding UTF8
    $runRows | Select-Object run_id,stage,cell_id,pair_id,pair_order,role,source_revision,source_tree_sha,control_kind,control_label,base_source_revision,base_source_tree_sha,base_source_archive_sha256,base_source_file_sha256,overlay_sha256,derived_source_file_sha256,source_changes_sha256,derived_source_inventory_sha256,harness_sha256,profile,features,observer_mode,receiveonly_observer_mode,receiveonly_observer_version,guard_mode,workload,scenario,load,game_fps,timing_margin_us,sample_seconds,started_utc,ended_utc,case_exit_code,status,reason,evidence_directory,expected_sink_event_count,observed_sink_event_count,sink_window_match,active_count,possibly_active_count,final_release_obligation_mask,production_hold_floor_violation_count,production_release_floor_violation_count,production_forensics_anomaly_count,production_structural_anomaly_count,production_same_key_overlap_forensics_count,production_anchor_overwrite_count,production_unmatched_up_count,missed_down_boundaries,missed_down_keys,missed_physical_window_boundaries,keys_dropped,chord_split_events,sendinput_partial_events,sendinput_zero_progress_failures,generation_total,generation_activated,generation_released,generation_cancelled,generation_scheduled,expected_down_key_count,expected_up_key_count,precision_packet_attempts,authored_lateness_samples,residual_lateness_samples,sendinput_envelope_samples,qpc_read_count_available,qpc_read_count,native_timing_telemetry_available,native_timing_telemetry_unavailable_reason,telemetry_trace_records,observed_down_chords,attempted_down_send_calls,completed_down_chords,failed_down_chords,actual_sendinput_calls,telemetry_generation_mismatch,telemetry_negative_order,telemetry_dropped,observer_queue_dropped,worker_cpu_time_us,process_cpu_time_us,spin_time_us,spin_time_available,spin_time_complete,spin_duty_cycle_ppm,spin_expected_samples,spin_dropped_samples,spin_missing_samples,timing_metrics_json | Export-Csv -LiteralPath (Join-Path $evidenceRoot 'sessions.csv') -NoTypeInformation -Encoding UTF8
    $pairedMetrics.paired_sessions | Export-Csv -LiteralPath (Join-Path $evidenceRoot 'paired-sessions.csv') -NoTypeInformation -Encoding UTF8
    $pairedMetrics.cell_summary | Export-Csv -LiteralPath (Join-Path $evidenceRoot 'paired-cells.csv') -NoTypeInformation -Encoding UTF8
    $commandRows | Set-Content -LiteralPath (Join-Path $evidenceRoot 'commands.txt') -Encoding UTF8
    $safetyMarkdown = @($safetyRows | ForEach-Object { '| ' + $_.scenario + ' | ' + $_.status + ' | ' + $_.case_exit_code + ' | ' + $_.reason.Replace('|', '\|') + ' |' })
    $performanceMarkdown = @($pairedMetrics.cell_summary | ForEach-Object {
        '| ' + $_.cell_id + ' | ' + $_.metric + ' | ' + $_.available_pairs + '/' + $_.attempted_pairs + ' | ' + $_.favorable_p95_pairs + '/' + $_.unfavorable_p95_pairs + ' | ' + $_.favorable_p99_pairs + '/' + $_.unfavorable_p99_pairs + ' | ' + $_.p95_regression_pairs + '/' + $_.p99_regression_pairs + ' | ' + $_.leader_review_prompt + ' |'
    })
    $markdown = @(
        '# Dispatch Core qualification ' + $Stage,
        '',
        $(if ($Stage -eq 'Compare') {
            '**Comparison: ' + $comparisonDescription + '.**'
        } else { $null }),
        '- Run: `' + $runId + '`',
        '- Source: `' + $head + '` tree `' + $tree + '` (clean)',
        ('- Candidate: ' + $reportProjection.MarkdownCandidatePrefix + '`' + $candidate.source_revision + '` / `' + $candidate.cargo_profile + '` / SHA-256 `' + $candidate.binary_sha256 + '`'),
        ('- Control: ' + $(if ($control) { $(if ($control.c1_timed_verified) { $reportProjection.MarkdownControlLabel + ' derived from frozen C0 A1; C0 not timed-qualified; ' } else { $reportProjection.MarkdownControlLabel + '; ' }) + '`' + $control.source_revision + '` / `' + $control.cargo_profile + '` / SHA-256 `' + $control.binary_sha256 + '`' } else { $reportProjection.MarkdownControlLabel })),
        '- Sessions: ' + $runRows.Count + '; PASS: ' + $passed + '; failed/inconclusive: ' + $failureRows.Count,
        '- Disposition: ' + $(if ($stoppedAtFirstNativeFailure) { 'STOPPED_AT_FIRST_NATIVE_FAILURE' } elseif ($exitCode -ne 0) { 'FAILED' } else { 'COMPLETED' }),
        '- Paired order: AB/BA alternating; six pairs per workload/load cell; 15 seconds per timed session.',
        '- Contention: two hidden PowerShell CPU workers at BelowNormal priority. Sink: identity-verified ReceiveOnly.',
        '- Percentiles use nearest-rank ceiling. No p99.9 is inferred.',
        '- Reported: per-chord authored lateness, floor displacement, residual lateness, waiter-to-pre-call, per-syscall sender envelope, per-key realized H/G, Down/Up/Mixed, chord width and authored hot/cold gap groups.',
        '- Timer target uses the arm QPC sample plus the relative timeout passed to the timer API; timer return is paired with the arm generation. Missing and negative-order samples remain counted.',
        '- Tail screen: candidate p95 worse than max(5 us, 10% control) or p99 worse than max(20 us, 10% control) in >=4/6 pairs in a cell prompts leader review; outliers are retained.',
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
        '| Cell | Metric | Available pairs | p95 favorable/unfavorable | p99 favorable/unfavorable | p95/p99 threshold pairs | Leader review prompt |',
        '|---|---|---:|---:|---:|---:|---:|'
    ) + $performanceMarkdown + @(
        '',
        'These are evidence summaries, not an automatic performance-gate verdict. No design, gate, or rollout selection is made by this runner.',
        ''
    )
    $markdown | Set-Content -LiteralPath (Join-Path $evidenceRoot 'summary.md') -Encoding UTF8
    $scriptHash = Get-Sha256 $PSCommandPath
    $caseRunnerHash = Get-Sha256 $caseRunner
    $builderHash = Get-Sha256 $manifestBuilder
    $validatorHash = Get-Sha256 $manifestValidator
    $reportProjectionHash = Get-Sha256 $reportProjectionModule
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $evidenceRoot 'qualify_dispatch_core.ps1')
    Copy-Item -LiteralPath $caseRunner -Destination (Join-Path $evidenceRoot 'run_native_acceptance_case.ps1')
    Copy-Item -LiteralPath $manifestBuilder -Destination (Join-Path $evidenceRoot 'build_dispatch_core_harness.ps1')
    Copy-Item -LiteralPath $manifestValidator -Destination (Join-Path $evidenceRoot 'dispatch_harness_manifest.psm1')
    Copy-Item -LiteralPath $reportProjectionModule -Destination (Join-Path $evidenceRoot 'dispatch_qualification_report.psm1')
    $commandRows.Insert(0, 'pwsh -NoProfile -File scripts/qualify_dispatch_core.ps1 -QualificationProfile ' + $QualificationProfile + ' -Stage ' + $Stage + ' -CandidateManifest <candidate-manifest>' + $(if ($controlManifestPath) { ' -ControlManifest <control-manifest> -Pairs 6 -SampleSeconds 15 -TimingMarginUs ' + $TimingMarginUs } else { '' }) + ' -OutputRoot <evidence-root>')
    $commandRows | Set-Content -LiteralPath (Join-Path $evidenceRoot 'commands.txt') -Encoding UTF8
    $metadata = [ordered]@{ runner_sha256 = $scriptHash; case_runner_sha256 = $caseRunnerHash; builder_sha256 = $builderHash; manifest_validator_sha256 = $validatorHash; report_projection_sha256 = $reportProjectionHash; source_revision = $head; source_tree_sha = $tree; source_tree_clean = $true; candidate_manifest_sha256 = $candidate.manifest_sha256; control_manifest_sha256 = if ($control) { $control.manifest_sha256 } else { $null }; comparison = $comparisonDescription }
    $metadata | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'provenance.json') -Encoding UTF8
    [ordered]@{
        schema_version = 1
        run_id = $runId
        stage = $Stage
        comparison = $comparisonDescription
        created_utc = [DateTimeOffset]::UtcNow.ToString('O')
        source_branch = $source.branch
        source_revision = $source.revision
        source_tree_sha = $source.tree
        source_tree_clean = $source.clean
        control_manifest = if ($control) { [ordered]@{ control_label = $controlLabel; path = $control.manifest_path; control_kind = Get-OptionalProperty $control 'control_kind'; sha256 = $control.manifest_sha256; qualification_profile = $reportProjection.ControlQualificationProfile; source_role = Get-OptionalProperty $control 'source_role'; source_revision = $control.source_revision; source_tree_sha = $control.source_tree_sha; base_source_revision = Get-OptionalProperty $control 'base_source_revision'; base_source_tree_sha = Get-OptionalProperty $control 'base_source_tree_sha'; binary_path = $control.resolved_binary_path; cargo_profile = $control.cargo_profile; cargo_features = @($control.cargo_features); target = $control.target; binary_sha256 = $control.binary_sha256; performance_timing_margin_us = $reportProjection.ControlMargins.performance_timing_margin_us; safety_timing_margin_us = $reportProjection.ControlMargins.safety_timing_margin_us } } else { $null }
        candidate_manifest = [ordered]@{ label = $candidateLabel; path = $candidate.manifest_path; sha256 = $candidate.manifest_sha256; qualification_profile = $reportProjection.CandidateQualificationProfile; source_role = Get-OptionalProperty $candidate 'source_role'; source_revision = $candidate.source_revision; source_tree_sha = $candidate.source_tree_sha; binary_path = $candidate.resolved_binary_path; cargo_profile = $candidate.cargo_profile; cargo_features = @($candidate.cargo_features); target = $candidate.target; binary_sha256 = $candidate.binary_sha256; performance_timing_margin_us = $reportProjection.CandidateMargins.performance_timing_margin_us; safety_timing_margin_us = $reportProjection.CandidateMargins.safety_timing_margin_us }
        parameters = $reportProjection.ManifestParameters
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
        manifest_validator_sha256 = $validatorHash
        report_projection_sha256 = $reportProjectionHash
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
