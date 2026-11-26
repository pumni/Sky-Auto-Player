Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$srcPrepareScript = Join-Path $repoRoot 'scripts\prepare_chord_width_pilot_v1.ps1'
$srcCaseRunner = Join-Path $repoRoot 'scripts\run_native_acceptance_case.ps1'
$srcAnalyzer = Join-Path $repoRoot 'scripts\analyze_chord_reception.mjs'
$srcSink = Join-Path $repoRoot 'scripts\native_acceptance_sink.ps1'

function Assert-CommandFails {
    param([string]$Label, [scriptblock]$Action)
    $failed = $false
    try {
        & $Action | Out-Null
        if ($LASTEXITCODE -ne 0) { $failed = $true }
    } catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Expected failure for: $Label"
    }
    Write-Output "PASS (fail-closed): $Label"
}

# 1. Verify -AllowDirtyForContractTest bypass parameter has been completely removed.
$prepareCmd = Get-Command $srcPrepareScript
if ($prepareCmd.Parameters.ContainsKey('AllowDirtyForContractTest')) {
    throw 'RP-R2 failure: -AllowDirtyForContractTest must not exist on prepare_chord_width_pilot_v1.ps1'
}
Write-Output 'PASS: -AllowDirtyForContractTest removed'

# 2. Create an isolated temp git repo + local bare origin remote descending from the frozen baseline.
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('sky-pilot-isolated-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$bareRemote = Join-Path $tempRoot 'origin.git'
$workRepo = Join-Path $tempRoot 'repo'
$extOut = Join-Path $tempRoot 'external-out'
$fakeBinDir = Join-Path $tempRoot 'fake-bin'

New-Item -ItemType Directory -Force -Path $tempRoot, $extOut, $fakeBinDir | Out-Null

try {
    & git init --bare $bareRemote 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'git init --bare failed' }

    & git clone --no-hardlinks --shared $repoRoot $workRepo 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'git clone failed' }

    & git -C $workRepo checkout -B 'refactor/completion-anchored-pulse-engine' 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'git checkout branch failed' }

    # Copy current working-tree scripts under test into the isolated repo and commit cleanly.
    Copy-Item -LiteralPath $srcPrepareScript -Destination (Join-Path $workRepo 'scripts\prepare_chord_width_pilot_v1.ps1') -Force
    Copy-Item -LiteralPath $srcCaseRunner -Destination (Join-Path $workRepo 'scripts\run_native_acceptance_case.ps1') -Force
    Copy-Item -LiteralPath $srcAnalyzer -Destination (Join-Path $workRepo 'scripts\analyze_chord_reception.mjs') -Force
    Copy-Item -LiteralPath $srcSink -Destination (Join-Path $workRepo 'scripts\native_acceptance_sink.ps1') -Force

    & git -C $workRepo config user.email 'pilot-test@example.invalid'
    & git -C $workRepo config user.name 'Pilot Contract Test'
    & git -C $workRepo add scripts/prepare_chord_width_pilot_v1.ps1 scripts/run_native_acceptance_case.ps1 scripts/analyze_chord_reception.mjs scripts/native_acceptance_sink.ps1 2>&1 | Out-Null
    $stagedDiff = (& git -C $workRepo diff --cached --name-only) -join ''
    if (-not [string]::IsNullOrWhiteSpace($stagedDiff)) {
        & git -C $workRepo commit -m 'test: stage pilot scripts for isolated preflight test' 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'git commit in isolated repo failed' }
    }

    & git -C $workRepo remote set-url origin $bareRemote 2>&1 | Out-Null
    & git -C $workRepo push origin 'refactor/completion-anchored-pulse-engine' 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'git push to local bare origin failed' }

    $isoPrepare = Join-Path $workRepo 'scripts\prepare_chord_width_pilot_v1.ps1'
    $isoCaseRunner = Join-Path $workRepo 'scripts\run_native_acceptance_case.ps1'

    $headSha = (& git -C $workRepo rev-parse HEAD).Trim()
    $treeSha = (& git -C $workRepo rev-parse 'HEAD^{tree}').Trim()
    $lockSha = (Get-FileHash -LiteralPath (Join-Path $workRepo 'rust\Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
    $sinkSha = (Get-FileHash -LiteralPath (Join-Path $workRepo 'scripts\native_acceptance_sink.ps1') -Algorithm SHA256).Hash.ToLowerInvariant()
    $analyzerSha = (Get-FileHash -LiteralPath (Join-Path $workRepo 'scripts\analyze_chord_reception.mjs') -Algorithm SHA256).Hash.ToLowerInvariant()
    $caseRunnerSha = (Get-FileHash -LiteralPath $isoCaseRunner -Algorithm SHA256).Hash.ToLowerInvariant()
    $prepareSha = (Get-FileHash -LiteralPath $isoPrepare -Algorithm SHA256).Hash.ToLowerInvariant()

    Push-Location $workRepo
    try {
        $rustcVersion = ((& node scripts/toolchain-pin.mjs exec -- rustc -Vv) -join [Environment]::NewLine).Trim()
        $cargoVersion = ((& node scripts/toolchain-pin.mjs exec -- cargo --version) -join ' ').Trim()
    } finally {
        Pop-Location
    }

    # Create a controlled fake PE-prefixed binary outside the repo for nonphysical manifest/runner preflight.
    $fakeBin = Join-Path $fakeBinDir 'rt-native-acceptance.exe'
    $peBytes = [byte[]]@(0x4D, 0x5A, 0x90, 0x00, 0x03, 0x00, 0x00, 0x00)
    [System.IO.File]::WriteAllBytes($fakeBin, $peBytes)
    $fakeBinSha = (Get-FileHash -LiteralPath $fakeBin -Algorithm SHA256).Hash.ToLowerInvariant()

    function Write-Manifest {
        param([string]$Path, [hashtable]$Overrides = @{})
        $m = [ordered]@{
            schema_version = 1
            qualification_profile = 'ChordWidthPilotV1'
            pilot_identity = 'ChordWidthPilotV1'
            workload_version = 'chord-width-pilot-v1'
            source_role = 'Candidate'
            source_root = $workRepo
            source_revision = $headSha
            source_tree_sha = $treeSha
            source_branch = 'refactor/completion-anchored-pulse-engine'
            source_tree_clean = $true
            clean_build = $true
            base_source_revision = 'e7293877a6ce58a387cb8edfe0a88399a85d2b15'
            base_source_tree_sha = '1e2aa8c1a6b1d48a46cd419d89536a1051510890'
            remote_tip_revision = $headSha
            cargo_lock_sha256 = $lockSha
            cargo_profile = 'dist'
            cargo_features = @('real-input-acceptance')
            qpc_read_counting = $false
            observer_mode = 'qualification-on'
            guard_mode = 'frozen'
            receiveonly_observer_mode = 'SchedulerQueuedV3Compat3'
            receiveonly_observer_version = 'scheduler-receiveonly-queue-v3-compat3'
            receiveonly_observer_schema_version = 3
            receiveonly_observer_event_schema_version = 3
            target = 'x86_64-pc-windows-msvc'
            rustc_version = $rustcVersion
            cargo_version = $cargoVersion
            binary_name = 'rt-native-acceptance.exe'
            binary_path = $fakeBin
            binary_sha256 = $fakeBinSha
            build_provenance = 'reused_manifest_hash_verified_only_not_rebuilt'
            sink_script_sha256 = $sinkSha
            analyzer_script_sha256 = $analyzerSha
            case_runner_script_sha256 = $caseRunnerSha
            preflight_script_sha256 = $prepareSha
        }
        foreach ($k in $Overrides.Keys) { $m[$k] = $Overrides[$k] }
        ($m | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath $Path -Encoding utf8NoBOM
    }

    $validManifest = Join-Path $fakeBinDir 'valid-manifest.json'
    Write-Manifest -Path $validManifest

    # --- Positive 1: PlanOnly on clean pushed isolated repo ---
    $planJson = & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -OutputRoot (Join-Path $extOut 'plan')
    if ($LASTEXITCODE -ne 0) { throw 'PlanOnly failed on clean pushed isolated repo' }
    $plan = $planJson | ConvertFrom-Json
    if ($plan.receiver_started -ne $false -or $plan.harness_started -ne $false -or $plan.sendinput_calls -ne 0 -or
        $plan.receiver_contract.observer_mode -ne 'SchedulerQueuedV3Compat3' -or
        $plan.receiver_contract.event_schema_version -ne 3 -or
        $plan.planned_smoke_session_count -ne 3) {
        throw 'PlanOnly output contract mismatch'
    }
    Write-Output 'PASS: PlanOnly clean pushed isolated repo (Compat3 schema 3)'

    # --- Positive 2: PreflightOnly end-to-end through run_native_acceptance_case.ps1 -PreflightOnly ---
    $preJson = & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PreflightOnly -HarnessManifestPath $validManifest -OutputRoot (Join-Path $extOut 'preflight')
    if ($LASTEXITCODE -ne 0) { throw "PreflightOnly failed on valid manifest: $preJson" }
    $pre = $preJson | ConvertFrom-Json
    if ($pre.receiver_started -ne $false -or $pre.harness_started -ne $false -or $pre.sendinput_calls -ne 0 -or
        $pre.build_provenance -ne 'reused_manifest_hash_verified_only_not_rebuilt' -or
        $pre.binary_provenance_verified_by_fresh_build -ne $false -or
        $pre.case_runner_preflight_results.Count -ne 3) {
        throw 'PreflightOnly output contract mismatch'
    }
    Write-Output 'PASS: PreflightOnly end-to-end via run_native_acceptance_case.ps1 -PreflightOnly (3 sessions, zero native launch)'

    # --- Negative checks: authorization, locks, contradictions, widths, paths ---
    Assert-CommandFails 'Execute without approval' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode Execute }
    Assert-CommandFails 'Execute locked even with OwnerApprovedReceiveOnlyRun' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode Execute -OwnerApprovedReceiveOnlyRun }
    Assert-CommandFails 'PlanOnly + OwnerApprovedReceiveOnlyRun contradiction' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -OwnerApprovedReceiveOnlyRun }
    Assert-CommandFails 'PreflightOnly + OwnerApprovedReceiveOnlyRun contradiction' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PreflightOnly -OwnerApprovedReceiveOnlyRun -HarnessManifestPath $validManifest }
    Assert-CommandFails 'PlanOnly + BuildHarness contradiction' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -BuildHarness -OutputRoot (Join-Path $extOut 'bad') }
    Assert-CommandFails 'BuildHarness + HarnessManifestPath contradiction' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PreflightOnly -BuildHarness -HarnessManifestPath $validManifest -OutputRoot (Join-Path $extOut 'bad2') }
    Assert-CommandFails 'Unauthorized receiver role' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -AuthorizedReceiverRole 'Game' }
    Assert-CommandFails 'AuthorizedGameTarget=true' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -AuthorizedGameTarget }
    Assert-CommandFails 'Unsupported width 3' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -Widths 3 }
    Assert-CommandFails 'Duplicate width 5,5' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -Widths 5,5 }
    Assert-CommandFails 'OutputRoot inside repo' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly -OutputRoot (Join-Path $workRepo 'inside-out') }

    # --- Negative checks: manifest & binary tampering ---
    $nonPeBin = Join-Path $fakeBinDir 'not-a-pe.exe'
    [System.IO.File]::WriteAllText($nonPeBin, 'not a pe binary')
    $nonPeManifest = Join-Path $fakeBinDir 'non-pe-manifest.json'
    Write-Manifest -Path $nonPeManifest -Overrides @{
        binary_path = $nonPeBin
        binary_sha256 = (Get-FileHash -LiteralPath $nonPeBin -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    Assert-CommandFails 'Non-PE binary rejected even with matching hash' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PreflightOnly -HarnessManifestPath $nonPeManifest }

    $badBinHashManifest = Join-Path $fakeBinDir 'bad-bin-hash.json'
    Write-Manifest -Path $badBinHashManifest -Overrides @{ binary_sha256 = ('0' * 64) }
    Assert-CommandFails 'Tampered binary SHA-256' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PreflightOnly -HarnessManifestPath $badBinHashManifest }

    $badScriptHashManifest = Join-Path $fakeBinDir 'bad-script-hash.json'
    Write-Manifest -Path $badScriptHashManifest -Overrides @{ analyzer_script_sha256 = ('f' * 64) }
    Assert-CommandFails 'Tampered script hash in manifest' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PreflightOnly -HarnessManifestPath $badScriptHashManifest }

    $badObsManifest = Join-Path $fakeBinDir 'bad-obs.json'
    Write-Manifest -Path $badObsManifest -Overrides @{ receiveonly_observer_mode = 'SchedulerQueuedV2'; receiveonly_observer_event_schema_version = 4 }
    Assert-CommandFails 'Wrong observer protocol/schema in manifest' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PreflightOnly -HarnessManifestPath $badObsManifest }

    # --- Negative checks: direct run_native_acceptance_case.ps1 locks & bounds ---
    Assert-CommandFails 'Direct case-runner ChordWidthPilotV1 execution locked' {
        & pwsh -NoProfile -File $isoCaseRunner -QualificationProfile 'ChordWidthPilotV1' -Scenario 'chord-width-pilot-v1' -Workload 'chord-width-pilot-v1' -PilotChordWidth 5 -GameFps 60 -TimingMarginUs 500 -SampleSeconds 15 -TimingObserverForQualification -HarnessPath $fakeBin -HarnessManifestPath $validManifest -OutputRoot (Join-Path $extOut 'direct-exec') -OwnerApprovedReceiveOnlyRun
    }
    Assert-CommandFails 'Direct case-runner ChordWidthPilotV1 rejects CpuContention' {
        & pwsh -NoProfile -File $isoCaseRunner -QualificationProfile 'ChordWidthPilotV1' -Scenario 'chord-width-pilot-v1' -Workload 'chord-width-pilot-v1' -PilotChordWidth 5 -GameFps 60 -TimingMarginUs 500 -SampleSeconds 15 -TimingObserverForQualification -CpuContention -HarnessPath $fakeBin -HarnessManifestPath $validManifest -OutputRoot (Join-Path $extOut 'direct-cpu') -PreflightOnly
    }

    # --- Negative checks: Git cleanliness, branch, remote tip ---
    # Dirty untracked file
    $untracked = Join-Path $workRepo 'untracked-test.tmp'
    Set-Content -LiteralPath $untracked -Value 'dirty'
    Assert-CommandFails 'Dirty untracked file rejected' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly }
    Remove-Item -LiteralPath $untracked -Force

    # Dirty tracked file
    Add-Content -LiteralPath (Join-Path $workRepo 'README.md') -Value "`n# dirty"
    Assert-CommandFails 'Dirty tracked file rejected' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly }
    & git -C $workRepo checkout -- README.md 2>&1 | Out-Null

    # Unpushed local commit (remote tip mismatch)
    & git -C $workRepo commit --allow-empty -m 'test: unpushed commit' 2>&1 | Out-Null
    Assert-CommandFails 'Unpushed local commit (origin tip mismatch) rejected' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly }
    & git -C $workRepo reset --hard HEAD~1 2>&1 | Out-Null

    # Wrong branch
    & git -C $workRepo checkout -b 'wrong-branch' 2>&1 | Out-Null
    Assert-CommandFails 'Wrong branch rejected' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly }
    & git -C $workRepo checkout 'refactor/completion-anchored-pulse-engine' 2>&1 | Out-Null

    # Missing remote ref
    & git -C $workRepo push origin --delete 'refactor/completion-anchored-pulse-engine' 2>&1 | Out-Null
    Assert-CommandFails 'Missing origin branch ref rejected' { & pwsh -NoProfile -File $isoPrepare -RepositoryRoot $workRepo -Mode PlanOnly }

    Write-Output 'ALL CHORD_WIDTH_PILOT_V1 ISOLATED PREFLIGHT & CASE-RUNNER CONTRACT TESTS PASSED'
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
