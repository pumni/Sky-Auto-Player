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
    [int]$Seed = 20261126
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = $PSScriptRoot
Import-Module (Join-Path $scriptRoot 'dispatch_harness_manifest.psm1') -Force
Import-Module (Join-Path $scriptRoot 'dispatch_qualification_profiles.psm1') -Force

$expectedBranch = 'refactor/completion-anchored-pulse-engine'
$frozenBaselineSha = 'e7293877a6ce58a387cb8edfe0a88399a85d2b15'
$frozenBaselineTree = '1e2aa8c1a6b1d48a46cd419d89536a1051510890'
$expectedObserverMode = 'SchedulerQueuedV3Compat3'
$expectedObserverVersion = 'scheduler-receiveonly-queue-v3-compat3'
$expectedObserverSchemaVersion = 3
$expectedObserverEventSchemaVersion = 3
$expectedTarget = 'x86_64-pc-windows-msvc'
$expectedRustVersion = '1.99.1'

function Get-FileSha256 {
    param([Parameter(Mandatory)] [string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-NoReparsePointInPath {
    param([Parameter(Mandatory)] [string]$Path, [Parameter(Mandatory)] [string]$Label)
    $current = [System.IO.Path]::GetFullPath($Path)
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw ("Fail-closed: $Label contains a reparse point / symlink at $current.")
            }
        }
        $parent = Split-Path -Path $current -Parent
        if ([string]::IsNullOrWhiteSpace($parent) -or $parent -eq $current) { break }
        $current = $parent
    }
}

function Assert-ValidPeHeader {
    param([Parameter(Mandatory)] [string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Fail-closed: binary missing at $Path."
    }
    $bytes = New-Object byte[] 2
    $fs = [System.IO.File]::OpenRead($Path)
    try {
        $n = $fs.Read($bytes, 0, 2)
    } finally {
        $fs.Dispose()
    }
    if ($n -lt 2 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
        throw "Fail-closed: binary at $Path does not have a valid Windows PE MZ header."
    }
}

function Invoke-GitStrict {
    param([Parameter(Mandatory)] [string]$RepoPath, [Parameter(Mandatory)] [string[]]$GitArgs, [Parameter(Mandatory)] [string]$FailureMessage)
    $output = & git -C $RepoPath @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw ("Fail-closed: $FailureMessage (git exit $LASTEXITCODE): " + (($output -join ' ').Trim()))
    }
    return $output
}

# 1. Fail-closed authorization, lock, and parameter contradiction checks BEFORE any Git, build, or runner action.
if ($OwnerApprovedReceiveOnlyRun -and $Mode -in @('PlanOnly', 'PreflightOnly')) {
    throw "Fail-closed contradiction: -OwnerApprovedReceiveOnlyRun cannot be combined with -Mode $Mode."
}
if ($Mode -eq 'PlanOnly' -and $BuildHarness) {
    throw 'Fail-closed contradiction: -Mode PlanOnly cannot be combined with -BuildHarness.'
}
if ($BuildHarness -and -not [string]::IsNullOrWhiteSpace($HarnessManifestPath)) {
    throw 'Fail-closed contradiction: -BuildHarness cannot be combined with -HarnessManifestPath.'
}
if ($AuthorizedReceiverRole -ne 'ReceiveOnly') {
    throw "Fail-closed: unauthorized receiver role '$AuthorizedReceiverRole' (only 'ReceiveOnly' is permitted)."
}
if ($AuthorizedGameTarget) {
    throw 'Fail-closed: game_target=true is forbidden for ReceiveOnly qualification.'
}
if ($Mode -eq 'Execute') {
    if (-not $OwnerApprovedReceiveOnlyRun) {
        throw 'Fail-closed: -Mode Execute requires explicit -OwnerApprovedReceiveOnlyRun scoped authorization.'
    }
    throw 'Fail-closed: ChordWidthPilotV1 native ReceiveOnly execution is LOCKED pending explicit Leader review and separate scoped owner authorization.'
}

# 2. Validate exact bounded pilot widths ({1, 5, 15} only; no duplicates, no arbitrary widths).
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

# 3. Validate repository root and external OutputRoot (with reparse-point protection).
$repoFull = Get-DispatchM3PathIdentity $RepositoryRoot
Assert-NoReparsePointInPath -Path $repoFull -Label 'RepositoryRoot'
$outFull = $null
if (-not [string]::IsNullOrWhiteSpace($OutputRoot)) {
    $outFull = Get-DispatchM3PathIdentity $OutputRoot
    Assert-NoReparsePointInPath -Path $outFull -Label 'OutputRoot'
    if (-not (Test-DispatchM3PathOutside -Path $outFull -RepositoryRoot $repoFull)) {
        throw 'Fail-closed: OutputRoot must be outside the repository checkout.'
    }
}

# 4. Strict Git source identity, cleanliness, branch, ancestry, and exact origin remote-tip verification.
$topLevel = Get-DispatchM3PathIdentity ((Invoke-GitStrict -RepoPath $repoFull -GitArgs @('rev-parse', '--show-toplevel') -FailureMessage 'could not resolve git top-level') -join '').Trim()
if (-not [string]::Equals($topLevel, $repoFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Fail-closed: RepositoryRoot ($repoFull) is not the git checkout root ($topLevel)."
}
$headSha = ((Invoke-GitStrict -RepoPath $repoFull -GitArgs @('rev-parse', 'HEAD') -FailureMessage 'could not resolve HEAD') -join '').Trim()
$treeSha = ((Invoke-GitStrict -RepoPath $repoFull -GitArgs @('rev-parse', 'HEAD^{tree}') -FailureMessage 'could not resolve HEAD^{tree}') -join '').Trim()
$branch = ((Invoke-GitStrict -RepoPath $repoFull -GitArgs @('branch', '--show-current') -FailureMessage 'could not resolve current branch') -join '').Trim()
if ($branch -ne $expectedBranch) {
    throw "Fail-closed: expected branch '$expectedBranch', got '$branch'."
}
$statusLines = Invoke-GitStrict -RepoPath $repoFull -GitArgs @('status', '--porcelain', '--untracked-files=all') -FailureMessage 'could not check git status'
if (-not [string]::IsNullOrWhiteSpace(($statusLines -join "`n"))) {
    throw 'Fail-closed: repository checkout is dirty (tracked or untracked changes present); clean committed source is required.'
}
& git -C $repoFull merge-base --is-ancestor $frozenBaselineSha HEAD
if ($LASTEXITCODE -ne 0) {
    throw "Fail-closed: HEAD ($headSha) does not descend from frozen production baseline $frozenBaselineSha."
}
$remoteRef = "refs/heads/$expectedBranch"
$remoteLines = @(Invoke-GitStrict -RepoPath $repoFull -GitArgs @('ls-remote', '--heads', 'origin', $remoteRef) -FailureMessage "could not query origin for $remoteRef" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
if ($remoteLines.Count -ne 1) {
    throw "Fail-closed: expected exactly 1 remote ref for origin $remoteRef, got $($remoteLines.Count)."
}
$remoteParts = ($remoteLines[0].Trim() -split '\s+')
if ($remoteParts.Count -lt 2 -or $remoteParts[1] -ne $remoteRef -or $remoteParts[0] -notmatch '^[0-9a-f]{40}$') {
    throw "Fail-closed: malformed ls-remote response for origin $remoteRef."
}
$remoteTipSha = $remoteParts[0]
if ($remoteTipSha -ne $headSha) {
    throw "Fail-closed: local HEAD ($headSha) does not match pushed origin/$expectedBranch tip ($remoteTipSha)."
}

# 5. Strict Cargo.lock LF byte check and pinned toolchain verification.
$lockPath = Join-Path $repoFull 'rust\Cargo.lock'
if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
    throw 'Fail-closed: rust/Cargo.lock is missing.'
}
$lockBytes = [System.IO.File]::ReadAllBytes($lockPath)
if ($lockBytes -contains [byte]13) {
    throw 'Fail-closed: rust/Cargo.lock contains CR bytes; LF bytes are required.'
}
$lockSha256 = Get-FileSha256 $lockPath

Push-Location $repoFull
try {
    $rustcVvLines = & node scripts/toolchain-pin.mjs exec -- rustc -Vv 2>&1
    if ($LASTEXITCODE -ne 0) { throw 'Fail-closed: pinned rustc -Vv failed.' }
    $cargoVerLines = & node scripts/toolchain-pin.mjs exec -- cargo --version 2>&1
    if ($LASTEXITCODE -ne 0) { throw 'Fail-closed: pinned cargo --version failed.' }
} finally {
    Pop-Location
}
$rustcVersion = (($rustcVvLines -join [Environment]::NewLine).Trim())
$cargoVersion = (($cargoVerLines -join ' ').Trim())
if ($rustcVersion -notmatch ('(?m)^release: ' + [regex]::Escape($expectedRustVersion) + '\r?$') -or
    $rustcVersion -notmatch ('(?m)^host: ' + [regex]::Escape($expectedTarget) + '\r?$')) {
    throw "Fail-closed: pinned rustc version/host mismatch ($rustcVersion)."
}
if ($cargoVersion -notmatch ('^cargo ' + [regex]::Escape($expectedRustVersion) + '(?:\s|$)')) {
    throw "Fail-closed: pinned cargo version mismatch ($cargoVersion)."
}

# 6. Script provenance hashes.
$sinkScriptPath = Join-Path $repoFull 'scripts\native_acceptance_sink.ps1'
$analyzerScriptPath = Join-Path $repoFull 'scripts\analyze_chord_reception.mjs'
$caseRunnerScriptPath = Join-Path $repoFull 'scripts\run_native_acceptance_case.ps1'
$selfScriptPath = Join-Path $repoFull 'scripts\prepare_chord_width_pilot_v1.ps1'
foreach ($sp in @($sinkScriptPath, $analyzerScriptPath, $caseRunnerScriptPath, $selfScriptPath)) {
    if (-not (Test-Path -LiteralPath $sp -PathType Leaf)) {
        throw "Fail-closed: required pilot script missing at $sp."
    }
}
$sinkScriptSha256 = Get-FileSha256 $sinkScriptPath
$analyzerScriptSha256 = Get-FileSha256 $analyzerScriptPath
$caseRunnerScriptSha256 = Get-FileSha256 $caseRunnerScriptPath
$preflightScriptSha256 = Get-FileSha256 $selfScriptPath

$nativeBuildStatus = 'NOT BUILT'
$nativeManifestStatus = 'NOT VERIFIED'
$buildProvenance = 'not_built'
$binaryProvenanceVerifiedByFreshBuild = $false
$binarySha256 = 'NOT_BUILT'
$binaryPath = $null
$manifestSha256 = 'NOT_VERIFIED'

# 7. Build or validate harness manifest.
if ($BuildHarness) {
    if ([string]::IsNullOrWhiteSpace($outFull)) {
        throw 'Fail-closed: -BuildHarness requires an external -OutputRoot outside the repository.'
    }
    New-Item -ItemType Directory -Force -Path $outFull | Out-Null
    $targetDir = Join-Path $outFull 'target'
    Push-Location $repoFull
    try {
        & node scripts/toolchain-pin.mjs exec -- cargo build --locked --profile dist --target $expectedTarget --target-dir $targetDir --manifest-path rust/Cargo.toml -p sky_player --features real-input-acceptance --bin rt-native-acceptance
        if ($LASTEXITCODE -ne 0) {
            throw "Fail-closed: pinned dist harness build failed with exit code $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
    $postBuildStatus = Invoke-GitStrict -RepoPath $repoFull -GitArgs @('status', '--porcelain', '--untracked-files=all') -FailureMessage 'could not recheck git status after build'
    if (-not [string]::IsNullOrWhiteSpace(($postBuildStatus -join "`n"))) {
        throw 'Fail-closed: repository checkout became dirty during harness build.'
    }
    $binaryPath = Join-Path $targetDir "$expectedTarget\dist\rt-native-acceptance.exe"
    Assert-ValidPeHeader -Path $binaryPath
    $binarySha256 = Get-FileSha256 $binaryPath
    $nativeBuildStatus = 'BUILT_NONPHYSICAL_DIST'
    $buildProvenance = 'fresh_pinned_dist_build'
    $binaryProvenanceVerifiedByFreshBuild = $true

    $manifestObj = [ordered]@{
        schema_version = 1
        qualification_profile = 'ChordWidthPilotV1'
        pilot_identity = 'ChordWidthPilotV1'
        workload_version = 'chord-width-pilot-v1'
        source_role = 'Candidate'
        source_root = $repoFull
        source_revision = $headSha
        source_tree_sha = $treeSha
        source_branch = $expectedBranch
        source_tree_clean = $true
        clean_build = $true
        base_source_revision = $frozenBaselineSha
        base_source_tree_sha = $frozenBaselineTree
        remote_tip_revision = $remoteTipSha
        cargo_lock_sha256 = $lockSha256
        cargo_profile = 'dist'
        cargo_features = @('real-input-acceptance')
        qpc_read_counting = $false
        observer_mode = 'qualification-on'
        guard_mode = 'frozen'
        receiveonly_observer_mode = $expectedObserverMode
        receiveonly_observer_version = $expectedObserverVersion
        receiveonly_observer_schema_version = $expectedObserverSchemaVersion
        receiveonly_observer_event_schema_version = $expectedObserverEventSchemaVersion
        target = $expectedTarget
        rustc_version = $rustcVersion
        cargo_version = $cargoVersion
        binary_name = 'rt-native-acceptance.exe'
        binary_path = $binaryPath
        binary_sha256 = $binarySha256
        build_provenance = $buildProvenance
        sink_script_sha256 = $sinkScriptSha256
        analyzer_script_sha256 = $analyzerScriptSha256
        case_runner_script_sha256 = $caseRunnerScriptSha256
        preflight_script_sha256 = $preflightScriptSha256
    }
    $manifestOut = Join-Path $outFull 'chord-width-pilot-v1-manifest.json'
    ($manifestObj | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath $manifestOut -Encoding utf8NoBOM
    $HarnessManifestPath = $manifestOut
    $manifestSha256 = Get-FileSha256 $HarnessManifestPath
    $nativeManifestStatus = 'VERIFIED_FRESH_PINNED_BUILD'
} elseif (-not [string]::IsNullOrWhiteSpace($HarnessManifestPath)) {
    $manifestFull = Get-DispatchM3PathIdentity $HarnessManifestPath
    Assert-NoReparsePointInPath -Path $manifestFull -Label 'HarnessManifestPath'
    if (-not (Test-DispatchM3PathOutside -Path $manifestFull -RepositoryRoot $repoFull)) {
        throw 'Fail-closed: HarnessManifestPath must be outside the repository checkout.'
    }
    if (-not (Test-Path -LiteralPath $manifestFull -PathType Leaf)) {
        throw "Fail-closed: harness manifest not found at $manifestFull."
    }
    $manifest = Get-Content -LiteralPath $manifestFull -Raw | ConvertFrom-Json
    $mFeatures = @((Get-ManifestProperty $manifest 'cargo_features'))
    if ((Get-ManifestProperty $manifest 'schema_version') -ne 1 -or
        (Get-ManifestProperty $manifest 'qualification_profile') -ne 'ChordWidthPilotV1' -or
        (Get-ManifestProperty $manifest 'pilot_identity') -ne 'ChordWidthPilotV1' -or
        (Get-ManifestProperty $manifest 'workload_version') -ne 'chord-width-pilot-v1' -or
        (Get-ManifestProperty $manifest 'source_role') -ne 'Candidate' -or
        -not [string]::Equals((Get-DispatchM3PathIdentity ([string](Get-ManifestProperty $manifest 'source_root'))), $repoFull, [System.StringComparison]::OrdinalIgnoreCase) -or
        (Get-ManifestProperty $manifest 'source_revision') -ne $headSha -or
        (Get-ManifestProperty $manifest 'source_tree_sha') -ne $treeSha -or
        (Get-ManifestProperty $manifest 'source_branch') -ne $expectedBranch -or
        -not [bool](Get-ManifestProperty $manifest 'source_tree_clean') -or
        -not [bool](Get-ManifestProperty $manifest 'clean_build') -or
        (Get-ManifestProperty $manifest 'base_source_revision') -ne $frozenBaselineSha -or
        (Get-ManifestProperty $manifest 'base_source_tree_sha') -ne $frozenBaselineTree -or
        (Get-ManifestProperty $manifest 'cargo_lock_sha256') -ne $lockSha256 -or
        (Get-ManifestProperty $manifest 'cargo_profile') -ne 'dist' -or
        $mFeatures.Count -ne 1 -or $mFeatures[0] -ne 'real-input-acceptance' -or
        [bool](Get-ManifestProperty $manifest 'qpc_read_counting') -or
        (Get-ManifestProperty $manifest 'observer_mode') -ne 'qualification-on' -or
        (Get-ManifestProperty $manifest 'guard_mode') -ne 'frozen' -or
        (Get-ManifestProperty $manifest 'receiveonly_observer_mode') -ne $expectedObserverMode -or
        (Get-ManifestProperty $manifest 'receiveonly_observer_version') -ne $expectedObserverVersion -or
        (Get-ManifestProperty $manifest 'receiveonly_observer_schema_version') -ne $expectedObserverSchemaVersion -or
        (Get-ManifestProperty $manifest 'receiveonly_observer_event_schema_version') -ne $expectedObserverEventSchemaVersion -or
        (Get-ManifestProperty $manifest 'target') -ne $expectedTarget -or
        (Get-ManifestProperty $manifest 'rustc_version') -ne $rustcVersion -or
        (Get-ManifestProperty $manifest 'cargo_version') -ne $cargoVersion -or
        (Get-ManifestProperty $manifest 'binary_name') -ne 'rt-native-acceptance.exe' -or
        (Get-ManifestProperty $manifest 'sink_script_sha256') -ne $sinkScriptSha256 -or
        (Get-ManifestProperty $manifest 'analyzer_script_sha256') -ne $analyzerScriptSha256 -or
        (Get-ManifestProperty $manifest 'case_runner_script_sha256') -ne $caseRunnerScriptSha256 -or
        (Get-ManifestProperty $manifest 'preflight_script_sha256') -ne $preflightScriptSha256) {
        throw 'Fail-closed: harness manifest schema, identity, toolchain, observer protocol, features, or script provenance mismatch.'
    }
    $mBinPath = Get-DispatchM3PathIdentity ([string](Get-ManifestProperty $manifest 'binary_path'))
    Assert-NoReparsePointInPath -Path $mBinPath -Label 'Manifest binary_path'
    if (-not (Test-DispatchM3PathOutside -Path $mBinPath -RepositoryRoot $repoFull)) {
        throw 'Fail-closed: manifest binary_path must be outside the repository checkout.'
    }
    Assert-ValidPeHeader -Path $mBinPath
    $actualBinHash = Get-FileSha256 $mBinPath
    if ($actualBinHash -ne [string](Get-ManifestProperty $manifest 'binary_sha256')) {
        throw "Fail-closed: binary SHA-256 mismatch ($actualBinHash != $(Get-ManifestProperty $manifest 'binary_sha256'))."
    }
    $HarnessManifestPath = $manifestFull
    $manifestSha256 = Get-FileSha256 $HarnessManifestPath
    $binaryPath = $mBinPath
    $binarySha256 = $actualBinHash
    $nativeBuildStatus = 'REUSED_MANIFEST_HASH_VERIFIED_ONLY'
    $buildProvenance = 'reused_manifest_hash_verified_only_not_rebuilt'
    $binaryProvenanceVerifiedByFreshBuild = $false
    $nativeManifestStatus = 'HASH_AND_SCHEMA_VERIFIED_NOT_REBUILT'
} elseif ($Mode -eq 'PreflightOnly') {
    throw 'Fail-closed: -Mode PreflightOnly requires either -BuildHarness or a verified -HarnessManifestPath.'
}

# 8. Deterministic randomized session order (LCG seeded with $Seed).
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
        qualification_profile = 'ChordWidthPilotV1'
        scenario = 'chord-width-pilot-v1'
        workload_definition_identity = 'chord-width-pilot-v1'
        pilot_chord_width = $w
        admission_variant = 'control-a'
        calibration_placement = 'baseline'
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
        observer_mode = $expectedObserverMode
        observer_version = $expectedObserverVersion
        event_schema_version = $expectedObserverEventSchemaVersion
        timing_observer_for_qualification = $true
    }
}

# 9. In PreflightOnly, invoke run_native_acceptance_case.ps1 -PreflightOnly for every planned session.
$caseRunnerPreflightResults = @()
if ($Mode -eq 'PreflightOnly') {
    $preflightProbeRoot = if ($outFull) { Join-Path $outFull 'case-runner-preflight-probe' } else { Join-Path ([System.IO.Path]::GetTempPath()) ('sky-pilot-preflight-' + [guid]::NewGuid().ToString('N').Substring(0, 8)) }
    foreach ($sess in $sessions) {
        $probeDir = Join-Path $preflightProbeRoot ("session-$($sess.session_index)-w$($sess.pilot_chord_width)")
        if (Test-Path -LiteralPath $probeDir) {
            throw "Fail-closed: preflight probe directory unexpectedly exists at $probeDir."
        }
        $caseOut = & pwsh -NoProfile -File $caseRunnerScriptPath `
            -QualificationProfile 'ChordWidthPilotV1' `
            -Scenario 'chord-width-pilot-v1' `
            -Workload 'chord-width-pilot-v1' `
            -PilotChordWidth ([int]$sess.pilot_chord_width) `
            -GameFps 60 `
            -TimingMarginUs 500 `
            -SampleSeconds 15 `
            -TimingObserverForQualification `
            -HarnessPath $binaryPath `
            -HarnessManifestPath $HarnessManifestPath `
            -OutputRoot $probeDir `
            -PreflightOnly 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw ("Fail-closed: run_native_acceptance_case.ps1 -PreflightOnly failed for width $($sess.pilot_chord_width): " + (($caseOut -join ' ').Trim()))
        }
        $caseText = ($caseOut -join "`n").Trim()
        if ($caseText -notmatch 'CASE_PREFLIGHT_ONLY PASS' -or
            $caseText -notmatch 'receiver_started=false' -or
            $caseText -notmatch 'harness_started=false' -or
            $caseText -notmatch 'output_created=false') {
            throw "Fail-closed: unexpected case-runner preflight output: $caseText"
        }
        if (Test-Path -LiteralPath $probeDir) {
            throw "Fail-closed: case-runner -PreflightOnly created output directory $probeDir."
        }
        $caseRunnerPreflightResults += [ordered]@{
            session_index = $sess.session_index
            pilot_chord_width = $sess.pilot_chord_width
            status = 'CASE_PREFLIGHT_ONLY PASS'
            output_created = $false
            receiver_started = $false
            harness_started = $false
        }
    }
}

$plan = [ordered]@{
    schema_version = 1
    qualification_profile = 'ChordWidthPilotV1'
    pilot_identity = 'ChordWidthPilotV1'
    mode = $Mode
    source_revision = $headSha
    source_tree = $treeSha
    branch = $branch
    remote_tip_revision = $remoteTipSha
    frozen_baseline_revision = $frozenBaselineSha
    cargo_lock_sha256 = $lockSha256
    rustc_version = $rustcVersion
    cargo_version = $cargoVersion
    sink_script_sha256 = $sinkScriptSha256
    analyzer_script_sha256 = $analyzerScriptSha256
    case_runner_script_sha256 = $caseRunnerScriptSha256
    preflight_script_sha256 = $preflightScriptSha256
    native_build_status = $nativeBuildStatus
    native_manifest_status = $nativeManifestStatus
    build_provenance = $buildProvenance
    binary_provenance_verified_by_fresh_build = $binaryProvenanceVerifiedByFreshBuild
    manifest_sha256 = $manifestSha256
    binary_sha256 = $binarySha256
    receiver_started = $false
    harness_started = $false
    no_native_invocation = $true
    sendinput_calls = 0
    owner_authorization_status = 'PENDING_EXPLICIT_SCOPED_APPROVAL'
    execution_lock_status = 'LOCKED'
    receiver_contract = [ordered]@{
        role = 'ReceiveOnly'
        game_target = $false
        observer_mode = $expectedObserverMode
        observer_version = $expectedObserverVersion
        event_schema_version = $expectedObserverEventSchemaVersion
        observer_queue_capacity = 32768
        runtime_pid = 'PENDING'
        runtime_hwnd = 'PENDING'
        runtime_process_start_time_filetime = 'PENDING'
        runtime_event_log_id = 'PENDING'
    }
    randomization_seed = $Seed
    planned_smoke_session_count = $sessions.Count
    planned_sessions = $sessions
    case_runner_preflight_results = $caseRunnerPreflightResults
    prospective_owner_approved_entry_point = 'pwsh -NoProfile -File scripts/run_native_acceptance_case.ps1 -QualificationProfile ChordWidthPilotV1 -Scenario chord-width-pilot-v1 -Workload chord-width-pilot-v1 -PilotChordWidth <1|5|15> -GameFps 60 -TimingMarginUs 500 -SampleSeconds 15 -TimingObserverForQualification -HarnessPath <bin> -HarnessManifestPath <manifest> -OutputRoot <external-root> -OwnerApprovedReceiveOnlyRun (LOCKED until explicit Leader review & scoped owner authorization)'
}

$json = $plan | ConvertTo-Json -Depth 8
if (-not [string]::IsNullOrWhiteSpace($outFull)) {
    New-Item -ItemType Directory -Force -Path $outFull | Out-Null
    $outFile = Join-Path $outFull "chord-width-pilot-v1-$($Mode.ToLowerInvariant()).json"
    $json | Set-Content -LiteralPath $outFile -Encoding utf8NoBOM
}
Write-Output $json
