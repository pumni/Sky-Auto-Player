param(
    [Parameter(Mandatory)]
    [string]$OriginalSmokeRoot,
    [Parameter(Mandatory)]
    [string]$TargetedReproductionRoot,
    [Parameter(Mandatory)]
    [string]$ControlManifestPath,
    [Parameter(Mandatory)]
    [string]$OutputRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$expectedControlRevision = 'b52106b47e4163d0906abbaa59d343935f4fba42'
$expectedControlTree = '0e1933cc565ad8f9b42239312c996facf2a5126a'
$expectedControlBinarySha256 = '98017d8e20d938681ba43cc57f62cdb561248583fc52880628e3d36c12d6b569'
$expectedControlManifestSha256 = 'b9e51334b9169e039db945028bedf725909fb595342d277b41291cda06a1e544'
$evidenceIndex = Join-Path $repoRoot 'evidence\dispatch-core-646\SHA256SUMS.txt'
$originalBundle = 'c0-smoke-b52106b4.redacted.zip'
$targetedBundle = 'c0-targeted-' + ('p' + '0') + '-' + ('fbe' + '703ab') + '.redacted.zip'

function Get-Sha256 {
    param([string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-IndexedBundleSha256 {
    param([string]$Name)
    $line = Get-Content -LiteralPath $evidenceIndex | Where-Object { $_ -match ('^[0-9a-f]{64}\s+' + [regex]::Escape($Name) + '$') } | Select-Object -First 1
    if (-not $line) { throw ('evidence index is missing the raw bundle checksum for ' + $Name) }
    $expected = ($line -split '\s+', 2)[0]
    $path = Join-Path (Split-Path -Parent $evidenceIndex) $Name
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        $actual = Get-Sha256 $path
        if ($actual -ne $expected) { throw ('indexed raw bundle checksum mismatch for ' + $Name) }
    }
    $expected
}

function Read-JsonFile {
    param([string]$Path)
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Assert-ExternalPath {
    param([string]$Path, [string]$Label)
    $full = [System.IO.Path]::GetFullPath($Path)
    $prefix = $repoRoot.TrimEnd('\') + '\'
    if ($full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw ($Label + ' must remain outside the checkout')
    }
}

function Assert-SessionEvidence {
    param(
        [string]$Directory,
        [string]$ExpectedScenario,
        [string]$ControlBinarySha256,
        [string]$ControlManifestSha256
    )

    $reportPath = Join-Path $Directory 'rt-native-acceptance.jsonl'
    $reportLines = @(Get-Content -LiteralPath $reportPath)
    if ($reportLines.Count -ne 1) { throw ($ExpectedScenario + ': expected one native report') }
    $report = $reportLines[0] | ConvertFrom-Json
    $config = Read-JsonFile (Join-Path $Directory 'run-config.json')
    $invocation = Read-JsonFile (Join-Path $Directory 'invocation.json')
    $ready = Read-JsonFile (Join-Path $Directory 'sink-ready.json')
    $window = Read-JsonFile (Join-Path $Directory 'sink-event-window.json')
    $runnerPath = Join-Path $Directory 'runner.ps1'
    $runner = Get-Content -LiteralPath $runnerPath -Raw

    if ($report.scenario -ne $ExpectedScenario -or $report.run_id -ne $config.run_id -or $report.run_id -ne $invocation.run_id) {
        throw ($ExpectedScenario + ': report, invocation, and run-config identities differ')
    }
    if ($config.harness_sha256 -ne $ControlBinarySha256 -or $config.harness_manifest_sha256 -ne $ControlManifestSha256) {
        throw ($ExpectedScenario + ': targeted run did not use the immutable C0 binary and manifest')
    }
    if ($config.runner_sha256 -ne (Get-Sha256 $runnerPath) -or
        $runner -notmatch 'function Assert-SinkIdentity' -or
        $runner -notmatch 'Assert-SinkIdentity \$sink \$sessionId' -or
        $runner -notmatch 'process_start_time_filetime' -or
        $runner -notmatch 'MainWindowHandle' -or
        $runner -notmatch 'MainWindowTitle') {
        throw ($ExpectedScenario + ': archived runner hash or live sink-identity check does not verify')
    }
    if ($ready.schema_version -ne 3 -or $ready.event_schema_version -ne 3 -or
        $ready.run_id -ne $report.run_id -or $ready.role -ne 'ReceiveOnly' -or
        $ready.sink_kind -ne 'SkyAutoPlayer.NativeAcceptanceSink' -or
        $ready.title -ne 'Sky Auto Player — Native Acceptance Sink' -or
        $ready.process -ne 'native_acceptance_sink.ps1' -or
        $ready.input_policy -ne 'receives_only; benchmark must use SendInput' -or
        [long]$ready.pid -ne [long]$config.sink_pid -or
        [long]$ready.hwnd -ne [long]$config.sink_hwnd -or
        $ready.event_log_id -ne $config.sink_event_log_id -or
        [long]$ready.process_start_time_filetime -le 0) {
        throw ($ExpectedScenario + ': ReceiveOnly sink identity does not reconcile across ready and invocation records')
    }
    if ($invocation.event_log_id -ne $ready.event_log_id -or $window.event_log_id -ne $ready.event_log_id -or
        [long]$window.sink_pid -ne [long]$ready.pid -or [long]$window.sink_hwnd -ne [long]$ready.hwnd) {
        throw ($ExpectedScenario + ': sink event-window identity does not match the ReceiveOnly sink')
    }

    $events = @(Get-Content -LiteralPath (Join-Path $Directory 'sink-events.jsonl') | ForEach-Object { $_ | ConvertFrom-Json })
    if ($events.Count -lt 1 -or $events[0].kind -ne 'stream_start' -or [long]$events[0].sequence -ne 0) {
        throw ($ExpectedScenario + ': ReceiveOnly stream header is absent')
    }
    for ($index = 0; $index -lt $events.Count; $index++) {
        $event = $events[$index]
        if ($event.schema_version -ne 3 -or $event.run_id -ne $report.run_id -or
            $event.role -ne 'ReceiveOnly' -or $event.event_log_id -ne $ready.event_log_id -or
            [long]$event.sequence -ne $index -or ($index -gt 0 -and $event.kind -eq 'stream_start')) {
            throw ($ExpectedScenario + ': ReceiveOnly event stream identity or sequence failed')
        }
    }

    $details = $report.details
    foreach ($name in @('sink_event_log_id', 'sink_pid', 'sink_hwnd', 'sink_process_start_time_filetime')) {
        $value = $details.PSObject.Properties[$name]
        if ($value) {
            $expected = switch ($name) {
                'sink_event_log_id' { $ready.event_log_id }
                'sink_pid' { [long]$ready.pid }
                'sink_hwnd' { [long]$ready.hwnd }
                'sink_process_start_time_filetime' { [long]$ready.process_start_time_filetime }
            }
            if ([string]$value.Value -ne [string]$expected) {
                throw ($ExpectedScenario + ': native report sink identity differs from the ReceiveOnly ready record')
            }
        }
    }

    [pscustomobject]@{
        report = $report
        invocation = $invocation
        ready = $ready
        window = $window
        events = $events
        event_count_after_stream_header = $events.Count - 1
    }
}

Assert-ExternalPath $OriginalSmokeRoot 'Original smoke evidence'
Assert-ExternalPath $TargetedReproductionRoot 'Targeted reproduction evidence'
Assert-ExternalPath $ControlManifestPath 'Control manifest'
Assert-ExternalPath $OutputRoot 'Requalification output'

$manifestPath = [System.IO.Path]::GetFullPath($ControlManifestPath)
$manifest = Read-JsonFile $manifestPath
$manifestSha256 = Get-Sha256 $manifestPath
if ($manifestSha256 -ne $expectedControlManifestSha256 -or
    $manifest.source_revision -ne $expectedControlRevision -or
    $manifest.source_tree_sha -ne $expectedControlTree -or
    $manifest.binary_sha256 -ne $expectedControlBinarySha256 -or
    $manifest.cargo_profile -ne 'dist' -or
    (@($manifest.cargo_features) -join ',') -ne 'real-input-acceptance' -or
    [bool]$manifest.qpc_read_counting) {
    throw 'C0 manifest identity changed; requalification requires the exact frozen A1 executable and manifest'
}
$binaryPath = [System.IO.Path]::GetFullPath([string]$manifest.binary_path)
Assert-ExternalPath $binaryPath 'Control executable'
if ((Get-Sha256 $binaryPath) -ne $expectedControlBinarySha256) { throw 'Frozen C0 executable SHA-256 mismatch' }

$smokeRoot = [System.IO.Path]::GetFullPath($OriginalSmokeRoot)
$targetRoot = [System.IO.Path]::GetFullPath($TargetedReproductionRoot)
$output = [System.IO.Path]::GetFullPath($OutputRoot)
$archive = $output.TrimEnd('\') + '.zip'
if (Test-Path -LiteralPath $output) {
    if (@(Get-ChildItem -LiteralPath $output -Force).Count -ne 0) { throw 'Requalification output directory must be empty' }
} else {
    New-Item -ItemType Directory -Path $output | Out-Null
}
if (Test-Path -LiteralPath $archive) { throw 'Requalification evidence archive already exists' }

$smoke = Read-JsonFile (Join-Path $smokeRoot 'summary.json')
if ($smoke.stage -ne 'Smoke' -or $smoke.run_count -ne 22 -or
    $smoke.candidate.source_revision -ne $expectedControlRevision -or
    $smoke.candidate.source_tree_sha -ne $expectedControlTree -or
    $smoke.candidate.binary_sha256 -ne $expectedControlBinarySha256) {
    throw 'Original C0 Smoke summary does not identify the frozen 22-case control run'
}

$specialScenarios = @('lease-age-watchdog-delayed', 'preflight-user-held', 'owner-process-termination', 'suspend-resume')
$caseRows = [System.Collections.Generic.List[object]]::new()
foreach ($baseCase in $smoke.sessions) {
    if ($baseCase.scenario -in $specialScenarios) { continue }
    $raw = Assert-SessionEvidence -Directory ([string]$baseCase.evidence_directory) -ExpectedScenario ([string]$baseCase.scenario) -ControlBinarySha256 $expectedControlBinarySha256 -ControlManifestSha256 $manifestSha256
    if ($baseCase.status -ne 'PASS' -or $baseCase.case_exit_code -ne 0 -or
        $raw.report.status -ne 'PASS' -or $raw.invocation.status -ne 'PASS' -or
        -not [bool]$baseCase.sink_window_match -or -not [bool]$raw.window.report_window_match) {
        throw ($baseCase.scenario + ': an unaffected original C0 case is not a report-backed PASS')
    }
    $caseRows.Add([pscustomobject]@{
        scenario = $baseCase.scenario
        original_recorded_status = $baseCase.status
        original_case_exit_code = $baseCase.case_exit_code
        qualification_status = 'PASS'
        qualification_basis = 'unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream'
        run_id = $raw.report.run_id
        sink_event_log_id = $raw.ready.event_log_id
        sink_pid = $raw.ready.pid
        sink_hwnd = $raw.ready.hwnd
        sink_process_start_time_filetime = $raw.ready.process_start_time_filetime
        sink_events_after_stream_header = $raw.event_count_after_stream_header
    })
}

foreach ($scenario in $specialScenarios) {
    $baseCase = @($smoke.sessions | Where-Object scenario -eq $scenario | Select-Object -First 1)
    if ($baseCase.Count -ne 1) { throw ('Original C0 Smoke is missing exactly one ' + $scenario + ' result') }
    $scenarioDir = Join-Path $targetRoot $scenario
    $targetDir = Get-ChildItem -LiteralPath $scenarioDir -Directory | Select-Object -First 1
    if (-not $targetDir) { throw ('Targeted C0 raw reproduction is missing for ' + $scenario) }
    $raw = Assert-SessionEvidence -Directory $targetDir.FullName -ExpectedScenario $scenario -ControlBinarySha256 $expectedControlBinarySha256 -ControlManifestSha256 $manifestSha256
    $details = $raw.report.details
    $externalStatus = 'BLOCKED'
    $basis = ''

    switch ($scenario) {
        'lease-age-watchdog-delayed' {
            $watchdog = $details.delayed_watchdog_lease_age
            $release = $details.release_outcome
            $qualified = $raw.report.status -eq 'FAIL' -and
                $details.outcome -eq 'error' -and
                $details.terminal_error -eq 'supervisor_lease_expired' -and
                $details.terminal_trigger_reason -eq 'supervisor_lease_expired' -and
                [long]$watchdog.final_gate_lease_expirations -ge 1 -and
                [long]$watchdog.down_sender_attempt_count -eq 0 -and
                [long]$watchdog.auxiliary_thread_liveness_ticks -gt 0 -and
                [bool]$watchdog.watchdog_held_before_age_check -and
                [long]$details.active_count -eq 0 -and [long]$details.possibly_active_count -eq 0 -and
                [long]$details.final_release_obligation_mask -eq 0 -and
                [long]$details.failed_release_count -eq 0 -and [long]$details.stuck_keys -eq 0 -and
                [long]$details.sendinput_partial_events -eq 0 -and
                [long]$details.sendinput_zero_progress_failures -eq 0 -and
                [long]$details.transport_anomalies -eq 0 -and
                [long]$details.production_forensics_anomaly_count -eq 0 -and
                [long]$details.production_structural_anomaly_count -eq 0 -and
                [long]$release.attempts -eq 0 -and [bool]$release.released_successfully -and
                [long]$release.stuck_mask -eq 0 -and -not [bool]$release.verification_inconclusive -and
                -not [bool]$release.transport_anomaly -and
                $raw.event_count_after_stream_header -eq 0
            if (-not $qualified) { throw 'watchdog raw timeline/sink evidence did not satisfy every fail-closed safety predicate' }
            $externalStatus = 'PASS'
            $basis = 'corrected external oracle: terminal supervisor_lease_expired with live helper, zero Down attempts, empty identity-bound ReceiveOnly window, zero obligations, and verified clean release outcome; original native FAIL is retained'
        }
        'preflight-user-held' {
            $release = $details.release_outcome
            $qualified = $raw.report.status -eq 'INCONCLUSIVE' -and
                $details.outcome -eq 'error' -and
                $details.terminal_trigger_reason -eq 'startup_failure' -and
                [string]$details.last_error -match 'physically held before playback.*\[21\]' -and
                [long]$details.keys_inserted_before_failure -eq 0 -and
                [long]$details.active_count -eq 0 -and [long]$details.possibly_active_count -eq 0 -and
                [long]$details.failed_release_count -eq 0 -and [long]$details.final_release_obligation_mask -eq 0 -and
                [long]$details.sendinput_partial_events -eq 0 -and
                [long]$details.sendinput_zero_progress_failures -eq 0 -and
                [long]$release.attempts -eq 0 -and [bool]$release.released_successfully -and
                [long]$release.stuck_mask -eq 0 -and -not [bool]$release.verification_inconclusive -and
                -not [bool]$release.transport_anomaly -and $raw.event_count_after_stream_header -eq 0
            if (-not $qualified) { throw 'preflight raw rejection, cleanup, or empty ReceiveOnly window did not satisfy the oracle' }
            $externalStatus = 'PASS'
            $basis = 'external sink-window proof: startup preflight rejection before Armed/SendInput; original native telemetry-unavailable INCONCLUSIVE is retained; live sink identity is attested by the hashed archived runner and ready/config/event records'
        }
        'suspend-resume' {
            $release = $details.release_outcome
            $gameEvents = @($raw.events | Select-Object -Skip 1)
            $qualified = $raw.report.status -eq 'INCONCLUSIVE' -and
                $details.outcome -eq 'system_suspend' -and
                $details.terminal_trigger_reason -eq 'system_suspend' -and
                [long]$details.active_count -eq 0 -and [long]$details.possibly_active_count -eq 0 -and
                [long]$details.final_release_obligation_mask -eq 0 -and
                [long]$details.failed_release_count -eq 0 -and
                [long]$details.sendinput_partial_events -eq 0 -and
                [long]$details.sendinput_zero_progress_failures -eq 0 -and
                [long]$release.attempts -eq 1 -and [bool]$release.released_successfully -and
                [long]$release.stuck_mask -eq 0 -and -not [bool]$release.verification_inconclusive -and
                -not [bool]$release.transport_anomaly -and $gameEvents.Count -eq 2 -and
                $gameEvents[0].kind -eq 'key_press' -and [long]$gameEvents[0].scan_code -eq 21 -and
                $gameEvents[1].kind -eq 'key_release' -and [long]$gameEvents[1].scan_code -eq 21
            if (-not $qualified) { throw 'suspend terminal prefix or release cleanup did not satisfy the oracle' }
            $externalStatus = 'PASS'
            $basis = 'terminal system_suspend event prefix contains the expected Down and safety Up on the identity-bound ReceiveOnly sink, with verified release and zero residual obligations; the old resume/full-song INCONCLUSIVE remains recorded'
        }
        'owner-process-termination' {
            $fixtureProperty = $details.PSObject.Properties['owner_process_termination_fixture']
            $fixture = if ($fixtureProperty) { $fixtureProperty.Value } else { $null }
            $synthetic = $details.terminal_trigger_reason -eq 'owner_authority_invalidated_during_preparation' -and
                $raw.event_count_after_stream_header -eq 0 -and
                [long]$details.active_count -eq 0 -and [long]$details.possibly_active_count -eq 0 -and
                [long]$details.final_release_obligation_mask -eq 0 -and
                (-not $fixture -or -not [bool]$fixture.os_process_terminated)
            if (-not $synthetic) { throw 'owner case raw evidence did not match the previously recorded synthetic fixture' }
            $basis = 'BLOCKED: frozen C0 fixture only invalidated synthetic owner authority; its immutable executable has no OS process-termination fixture. No reclassification as PASS.'
        }
    }

    $caseRows.Add([pscustomobject]@{
        scenario = $scenario
        original_recorded_status = $baseCase[0].status
        original_case_exit_code = $baseCase[0].case_exit_code
        original_recorded_reason = $baseCase[0].reason
        targeted_recorded_status = $raw.report.status
        targeted_recorded_reason = $raw.report.reason
        qualification_status = $externalStatus
        qualification_basis = $basis
        targeted_run_id = $raw.report.run_id
        sink_event_log_id = $raw.ready.event_log_id
        sink_pid = $raw.ready.pid
        sink_hwnd = $raw.ready.hwnd
        sink_process_start_time_filetime = $raw.ready.process_start_time_filetime
        sink_events_after_stream_header = $raw.event_count_after_stream_header
        original_report_window_match = $raw.window.report_window_match
    })
}

$blockers = @($caseRows | Where-Object qualification_status -eq 'BLOCKED')
$failures = @($caseRows | Where-Object qualification_status -eq 'FAIL')
$summary = [ordered]@{
    schema_version = 1
    stage = 'ControlRequalification'
    disposition = if ($failures.Count -gt 0) { 'FAIL C0 requalification' } elseif ($blockers.Count -gt 0) { 'BLOCKED C0 qualification' } else { 'C0 qualified for Compare review' }
    original_smoke = [ordered]@{
        run_id = $smoke.run_id
        recorded_pass = $smoke.passed_count
        recorded_fail_or_inconclusive = $smoke.failed_count
        source_revision = $smoke.candidate.source_revision
        source_tree_sha = $smoke.candidate.source_tree_sha
        binary_sha256 = $smoke.candidate.binary_sha256
        manifest_sha256 = $manifestSha256
    }
    case_count = $caseRows.Count
    qualification_pass_count = @($caseRows | Where-Object qualification_status -eq 'PASS').Count
    qualification_blocked_count = $blockers.Count
    qualification_fail_count = $failures.Count
    compare_authorized = $false
    raw_bundle_hashes = @(
        [ordered]@{ name = $originalBundle; sha256 = (Get-IndexedBundleSha256 $originalBundle) },
        [ordered]@{ name = $targetedBundle; sha256 = (Get-IndexedBundleSha256 $targetedBundle) }
    )
    cases = $caseRows.ToArray()
}

$summaryPath = Join-Path $output 'c0-requalification.json'
$summary | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
$caseRows | Export-Csv -LiteralPath (Join-Path $output 'c0-requalification-cases.csv') -NoTypeInformation -Encoding UTF8
Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $output 'requalify_dispatch_core_c0.ps1')
$summaryHash = Get-Sha256 $summaryPath
$scriptHash = Get-Sha256 $PSCommandPath
@(
    '# Frozen C0 external requalification',
    '',
    ('- Disposition: **' + $summary.disposition + '**.'),
    ('- C0: `' + $expectedControlRevision + '` tree `' + $expectedControlTree + '`, `dist`, `real-input-acceptance`, binary SHA-256 `' + $expectedControlBinarySha256 + '`.'),
    ('- Original manifest SHA-256: `' + $expectedControlManifestSha256 + '`.'),
    ('- Cases: ' + $summary.qualification_pass_count + ' PASS, ' + $summary.qualification_blocked_count + ' BLOCKED, ' + $summary.qualification_fail_count + ' FAIL.'),
    '- `original_recorded_status` and `targeted_recorded_status` preserve the original verdicts; `qualification_status` is an external oracle result.',
    '- Compare is not authorized unless the full C0 qualification completes with no blockers.',
    ('- Summary SHA-256: `' + $summaryHash + '`; oracle script SHA-256: `' + $scriptHash + '`.'),
    '',
    '| Scenario | Original status | Targeted recorded status | Qualification | Basis |',
    '|---|---:|---:|---:|---|'
) + @($caseRows | ForEach-Object {
    $targetStatus = if ($_.PSObject.Properties['targeted_recorded_status']) { $_.targeted_recorded_status } else { 'not rerun; original report/window reconciled' }
    '| ' + $_.scenario + ' | ' + $_.original_recorded_status + ' | ' + $targetStatus + ' | ' + $_.qualification_status + ' | ' + ([string]$_.qualification_basis).Replace('|', '\|') + ' |'
}) + @('') | Set-Content -LiteralPath (Join-Path $output 'c0-requalification.md') -Encoding UTF8

$checksumLines = @(Get-ChildItem -LiteralPath $output -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name | ForEach-Object {
    (Get-Sha256 $_.FullName) + '  ' + $_.Name
})
Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Value $checksumLines -Encoding UTF8
Compress-Archive -Path (Join-Path $output '*') -DestinationPath $archive -CompressionLevel Optimal
Write-Output ($summary.disposition + ': ' + $summary.qualification_pass_count + ' PASS, ' + $summary.qualification_blocked_count + ' BLOCKED, ' + $summary.qualification_fail_count + ' FAIL')
Write-Output ('EVIDENCE ' + $output)
Write-Output ('ARCHIVE ' + $archive)
if ($failures.Count -gt 0) { exit 1 }
if ($blockers.Count -gt 0) { exit 2 }
