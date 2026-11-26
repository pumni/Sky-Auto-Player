param(
    [string]$RepositoryRoot = '.',
    [string]$OutputRoot,
    [ValidateSet('PlanOnly', 'PreflightOnly', 'Execute')]
    [string]$Mode = 'PlanOnly',
    [switch]$OwnerApprovedReceiveOnlyRun,
    [switch]$BuildHarness,
    [string]$HarnessManifestPath,
    [string[]]$Widths = @('1', '5', '15'),
    [string]$AuthorizedReceiverRole = 'ReceiveOnly',
    [switch]$AuthorizedGameTarget,
    [int]$Seed = 20261126,
    [switch]$AllowDirtyForContractTest
)

$ErrorActionPreference = 'Stop'

# Fail-closed authorization & contradiction checks BEFORE any build, receiver, or runner action.
if ($OwnerApprovedReceiveOnlyRun -and $Mode -in @('PlanOnly', 'PreflightOnly')) {
    throw "Fail-closed contradiction: -OwnerApprovedReceiveOnlyRun cannot be combined with -Mode $Mode."
}
if ($Mode -eq 'Execute') {
    if (-not $OwnerApprovedReceiveOnlyRun) {
        throw 'Fail-closed: -Mode Execute requires explicit -OwnerApprovedReceiveOnlyRun scoped authorization.'
    }
    if ($AuthorizedReceiverRole -ne 'ReceiveOnly') {
        throw "Fail-closed: unauthorized receiver role '$AuthorizedReceiverRole' (only 'ReceiveOnly' is permitted)."
    }
    if ($AuthorizedGameTarget) {
        throw 'Fail-closed: game_target=true is forbidden for ReceiveOnly qualification.'
    }
    throw 'Fail-closed: R2 native ReceiveOnly execution is LOCKED in R2-P preparation. Explicit Leader review and scoped owner authorization are required before unlocking execution.'
}
if ($AuthorizedReceiverRole -ne 'ReceiveOnly') {
    throw "Fail-closed: unauthorized receiver role '$AuthorizedReceiverRole' (only 'ReceiveOnly' is permitted)."
}
if ($AuthorizedGameTarget) {
    throw 'Fail-closed: game_target=true is forbidden for ReceiveOnly qualification.'
}

# Validate exact bounded pilot widths (1, 5, 15 only; no duplicates, no arbitrary widths)
$allowedWidths = @(1, 5, 15)
$parsedWidths = @()
foreach ($entry in $Widths) {
    foreach ($part in ($entry -split ',')) {
        $trimmed = $part.Trim()
        if ([string]::IsNullOrEmpty($trimmed)) { continue }
        $num = 0
        if (-not [int]::TryParse($trimmed, [ref]$num)) {
            throw "Fail-closed: invalid pilot chord width '$trimmed'."
        }
        $parsedWidths += $num
    }
}
if ($parsedWidths.Count -eq 0) {
    throw 'Fail-closed: at least one pilot width from {1, 5, 15} is required.'
}
$seenWidths = @{}
foreach ($w in $parsedWidths) {
    if ($w -notin $allowedWidths) {
        throw "Fail-closed: unsupported pilot chord width $w (only 1, 5, 15 are authorized)."
    }
    if ($seenWidths.ContainsKey($w)) {
        throw "Fail-closed: duplicate pilot chord width $w."
    }
    $seenWidths[$w] = $true
}
$Widths = $parsedWidths

$repoFull = [System.IO.Path]::GetFullPath($RepositoryRoot)
if ($OutputRoot) {
    $outFull = [System.IO.Path]::GetFullPath($OutputRoot)
    if ($outFull.StartsWith($repoFull.TrimEnd('\') + '\', [System.StringComparison]::OrdinalIgnoreCase) -or
        [string]::Equals($outFull, $repoFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Fail-closed: OutputRoot must be outside the repository checkout.'
    }
}

# Verify source identity, cleanliness, branch, and LF Cargo.lock bytes
$headSha = (git -C $repoFull rev-parse HEAD).Trim()
$treeSha = (git -C $repoFull rev-parse 'HEAD^{tree}').Trim()
$branch = (git -C $repoFull branch --show-current).Trim()
$status = (git -C $repoFull status --porcelain)
if (-not $AllowDirtyForContractTest -and -not [string]::IsNullOrWhiteSpace(($status -join "`n"))) {
    throw 'Fail-closed: repository checkout is dirty; clean committed source is required for source-bound preflight.'
}

$lockPath = Join-Path $repoFull 'rust/Cargo.lock'
if (-not (Test-Path -LiteralPath $lockPath)) {
    throw 'Fail-closed: rust/Cargo.lock is missing.'
}
$lockBytes = [System.IO.File]::ReadAllBytes($lockPath)
if ($lockBytes -contains [byte]13) {
    throw 'Fail-closed: rust/Cargo.lock contains CR bytes; LF bytes are required.'
}
$lockSha256 = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()

$sinkScriptPath = Join-Path $repoFull 'scripts/native_acceptance_sink.ps1'
$analyzerScriptPath = Join-Path $repoFull 'scripts/analyze_chord_reception.mjs'
$selfScriptPath = $PSCommandPath
$sinkScriptSha256 = (Get-FileHash -LiteralPath $sinkScriptPath -Algorithm SHA256).Hash.ToLowerInvariant()
$analyzerScriptSha256 = (Get-FileHash -LiteralPath $analyzerScriptPath -Algorithm SHA256).Hash.ToLowerInvariant()
$preflightScriptSha256 = (Get-FileHash -LiteralPath $selfScriptPath -Algorithm SHA256).Hash.ToLowerInvariant()

$nativeBuildStatus = 'NOT BUILT'
$nativeManifestStatus = 'NOT VERIFIED'
$binarySha256 = 'NOT_BUILT'
$binaryPath = $null

if ($BuildHarness) {
    if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
        throw 'Fail-closed: -BuildHarness requires an external -OutputRoot outside the repository.'
    }
    New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
    $targetDir = Join-Path $outFull 'target'
    Push-Location $repoFull
    try {
        & node scripts/toolchain-pin.mjs exec -- cargo build --locked --profile dist --target x86_64-pc-windows-msvc --target-dir $targetDir --manifest-path rust/Cargo.toml -p sky_player --features real-input-acceptance --bin rt-native-acceptance
        if ($LASTEXITCODE -ne 0) {
            throw "Fail-closed: pinned dist harness build failed with exit code $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
    $binaryPath = Join-Path $targetDir 'x86_64-pc-windows-msvc/dist/rt-native-acceptance.exe'
    if (-not (Test-Path -LiteralPath $binaryPath)) {
        throw "Fail-closed: expected harness binary missing at $binaryPath."
    }
    $binarySha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $nativeBuildStatus = 'BUILT_NONPHYSICAL_DIST'
    $manifestObj = [ordered]@{
        schema_version = 1
        pilot_identity = 'ChordWidthPilotV1'
        source_revision = $headSha
        source_tree = $treeSha
        branch = $branch
        cargo_lock_sha256 = $lockSha256
        target = 'x86_64-pc-windows-msvc'
        profile = 'dist'
        features = @('real-input-acceptance')
        binary_path = $binaryPath
        binary_sha256 = $binarySha256
        sink_script_sha256 = $sinkScriptSha256
        analyzer_script_sha256 = $analyzerScriptSha256
        preflight_script_sha256 = $preflightScriptSha256
    }
    $manifestOut = Join-Path $outFull 'chord-width-pilot-v1-manifest.json'
    ($manifestObj | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath $manifestOut -Encoding utf8NoBOM
    $HarnessManifestPath = $manifestOut
    $nativeManifestStatus = 'VERIFIED_LOCAL_SOURCE_BOUND'
} elseif (-not [string]::IsNullOrWhiteSpace($HarnessManifestPath)) {
    if (-not (Test-Path -LiteralPath $HarnessManifestPath)) {
        throw "Fail-closed: harness manifest not found at $HarnessManifestPath."
    }
    $manifest = Get-Content -LiteralPath $HarnessManifestPath -Raw | ConvertFrom-Json
    if ($manifest.pilot_identity -ne 'ChordWidthPilotV1') {
        throw "Fail-closed: manifest pilot_identity '$($manifest.pilot_identity)' != 'ChordWidthPilotV1'."
    }
    if ($manifest.source_revision -ne $headSha -or $manifest.source_tree -ne $treeSha) {
        throw "Fail-closed: stale manifest source identity ($($manifest.source_revision)/$($manifest.source_tree) != $headSha/$treeSha)."
    }
    if ($manifest.cargo_lock_sha256 -ne $lockSha256) {
        throw 'Fail-closed: manifest Cargo.lock SHA-256 does not match checkout.'
    }
    if (-not (Test-Path -LiteralPath $manifest.binary_path)) {
        throw "Fail-closed: manifest binary missing at $($manifest.binary_path)."
    }
    $actualBinHash = (Get-FileHash -LiteralPath $manifest.binary_path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualBinHash -ne $manifest.binary_sha256) {
        throw "Fail-closed: binary SHA-256 mismatch ($actualBinHash != $($manifest.binary_sha256))."
    }
    $binaryPath = $manifest.binary_path
    $binarySha256 = $actualBinHash
    $nativeBuildStatus = 'VERIFIED_FROM_MANIFEST'
    $nativeManifestStatus = 'VERIFIED_LOCAL_SOURCE_BOUND'
} elseif ($Mode -eq 'PreflightOnly') {
    throw 'Fail-closed: -Mode PreflightOnly requires either -BuildHarness or a verified -HarnessManifestPath.'
}

# Deterministic randomized session order (LCG seeded with $Seed)
$orderedWidths = @($Widths)
$rngState = [uint32]$Seed
for ($i = $orderedWidths.Count - 1; $i -gt 0; $i--) {
    $rngState = ([uint64]$rngState * 1664525UL + 1013904223UL) % 4294967296UL
    $j = [int]($rngState % [uint32]($i + 1))
    $tmp = $orderedWidths[$i]
    $orderedWidths[$i] = $orderedWidths[$j]
    $orderedWidths[$j] = $tmp
}

$sessions = @()
for ($idx = 0; $idx -lt $orderedWidths.Count; $idx++) {
    $w = $orderedWidths[$idx]
    $sessions += [ordered]@{
        session_index = $idx + 1
        scenario = 'chord-width-pilot-v1'
        workload_definition_identity = 'chord-width-pilot-v1'
        pilot_chord_width = $w
        load_condition = 'Quiet'
        game_fps = 60
        timing_margin_us = 500
        configured_hold_us = 17167
        cadence_us = 100000
        onset_count = 150
        sample_seconds = 15
        nominal_receiver_events = 2 * 150 * $w
        nominal_up_slot_entries = 150 * $w
        nominal_healthy_packets = 300
        observer_mode = 'SchedulerQueuedV2'
        timing_observer_for_qualification = $true
    }
}

$plan = [ordered]@{
    schema_version = 1
    pilot_identity = 'ChordWidthPilotV1'
    mode = $Mode
    source_revision = $headSha
    source_tree = $treeSha
    branch = $branch
    cargo_lock_sha256 = $lockSha256
    sink_script_sha256 = $sinkScriptSha256
    analyzer_script_sha256 = $analyzerScriptSha256
    preflight_script_sha256 = $preflightScriptSha256
    native_build_status = $nativeBuildStatus
    native_manifest_status = $nativeManifestStatus
    binary_sha256 = $binarySha256
    receiver_started = $false
    harness_started = $false
    no_native_invocation = $true
    sendinput_calls = 0
    owner_authorization_status = 'PENDING_EXPLICIT_SCOPED_APPROVAL'
    receiver_contract = [ordered]@{
        role = 'ReceiveOnly'
        game_target = $false
        observer_mode = 'SchedulerQueuedV2'
        runtime_pid = 'PENDING'
        runtime_hwnd = 'PENDING'
        runtime_process_start_time_filetime = 'PENDING'
        runtime_event_log_id = 'PENDING'
    }
    randomization_seed = $Seed
    planned_smoke_session_count = $sessions.Count
    planned_sessions = $sessions
    prospective_owner_approved_entry_point = 'pwsh -NoProfile -File scripts/prepare_chord_width_pilot_v1.ps1 -Mode Execute -OwnerApprovedReceiveOnlyRun -HarnessManifestPath <manifest> -OutputRoot <external-root> (LOCKED until explicit Leader review & scoped owner authorization)'
}

$json = $plan | ConvertTo-Json -Depth 8
if (-not [string]::IsNullOrWhiteSpace($OutputRoot)) {
    New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
    $outFile = Join-Path $outFull "chord-width-pilot-v1-$($Mode.ToLowerInvariant()).json"
    $json | Set-Content -LiteralPath $outFile -Encoding utf8NoBOM
}
Write-Output $json
