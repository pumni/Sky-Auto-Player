function Get-DispatchSessionRedesignM3Profile {
    $safetyCases = @(
        [pscustomobject]@{ Id = 'focus-loss'; Scenario = 'focus-loss'; Workload = 'safety-smoke'; Load = 'Quiet'; GameFps = 60; TimingMarginUs = 500; SampleSeconds = 0 },
        [pscustomobject]@{ Id = 'stop-cleanup'; Scenario = 'stop-cleanup'; Workload = 'safety-smoke'; Load = 'Quiet'; GameFps = 60; TimingMarginUs = 500; SampleSeconds = 0 },
        [pscustomobject]@{ Id = 'pause-resume'; Scenario = 'pause-resume'; Workload = 'safety-smoke'; Load = 'Quiet'; GameFps = 60; TimingMarginUs = 500; SampleSeconds = 0 },
        [pscustomobject]@{ Id = 'owner-process-termination'; Scenario = 'owner-process-termination'; Workload = 'safety-smoke'; Load = 'Quiet'; GameFps = 60; TimingMarginUs = 500; SampleSeconds = 0 },
        [pscustomobject]@{ Id = 'target-hwnd-change'; Scenario = 'target-hwnd-change'; Workload = 'safety-smoke'; Load = 'Quiet'; GameFps = 60; TimingMarginUs = 500; SampleSeconds = 0 },
        [pscustomobject]@{ Id = 'near-minimum-retrigger@60'; Scenario = 'near-minimum-retrigger'; Workload = 'near-minimum-retrigger'; Load = 'Quiet'; GameFps = 60; TimingMarginUs = 500; SampleSeconds = 0 },
        [pscustomobject]@{ Id = 'near-minimum-retrigger@120'; Scenario = 'near-minimum-retrigger'; Workload = 'near-minimum-retrigger'; Load = 'Quiet'; GameFps = 120; TimingMarginUs = 500; SampleSeconds = 0 }
    )
    [pscustomobject]@{
        Name = 'DispatchSessionRedesignM3'
        WorkloadVersion = 'dispatch-session-redesign-m3-v1'
        HarnessManifestSchemaVersion = 2
        BaselineRevision = 'd0f65e7c412041a6d502f81f469872198db7adc5'
        BaselineTree = '19aae9e41bc667a2f991001ed105b6ffdebb8bc6'
        CandidateLineageBranch = 'refactor/dispatch-session-redesign'
        Target = 'x86_64-pc-windows-msvc'
        CargoProfile = 'dist'
        RequiredFeatures = @('real-input-acceptance')
        RustVersion = '1.99.1'
        CargoVersion = '1.99.1'
        ObserverMode = 'SchedulerQueuedV3Compat3'
        ObserverVersion = 'scheduler-receiveonly-queue-v3-compat3'
        ObserverSchemaVersion = 3
        ObserverEventSchemaVersion = 3
        PerformanceTimingMarginUs = 500
        SafetyTimingMarginUs = 500
        CanaryCase = [pscustomobject]@{ Id = 'canonical-single'; Scenario = 'canonical-single'; Workload = 'canonical-single'; Load = 'Quiet'; GameFps = 60; TimingMarginUs = 500; SampleSeconds = 0 }
        SafetyCases = $safetyCases
        SafetyScenarios = @($safetyCases | ForEach-Object { $_.Id })
        PerformanceCase = [pscustomobject]@{ Scenario = 'mixed-up-down'; Workload = 'mixed-up-down'; LoadCells = @('Quiet', 'Contention'); GameFps = 60; TimingMarginUs = 500; PairCount = 6; SampleSeconds = 15; PairOrder = 'AB/BA alternating' }
        RunnerProvenancePaths = @(
            'scripts/build_dispatch_core_harness.ps1',
            'scripts/dispatch_harness_manifest.psm1',
            'scripts/dispatch_qualification_profiles.psm1',
            'scripts/qualify_dispatch_core.ps1',
            'scripts/run_native_acceptance_case.ps1',
            'scripts/dispatch_qualification_report.psm1',
            'scripts/dispatch_qualification_orchestration.psm1',
            'scripts/dispatch_pair_screen.psm1',
            'scripts/native_acceptance_sink.ps1',
            'scripts/test_qualify_dispatch_core_contracts.ps1',
            'docs/playback-verification.md'
        )
    }
}

function Get-PlaybackCoreLifecycleProfile {
    param([ValidateSet(1, 2, 3)] [int]$Version = 1)
    $performanceWorkloads = @(
        'mixed-up-down',
        'dense-alternating',
        'canonical-max-chord',
        'near-minimum-retrigger',
        'long-idle'
    )
    $safetyScenarios = @(
        'focus-loss',
        'owner-mismatch',
        'owner-process-termination',
        'pause-resume',
        'suspend-resume',
        'stop-cleanup',
        'skip-cleanup',
        'supervisor-lease-expiry'
    )
    if ($Version -ge 2) { $safetyScenarios += @('short-up-gap-stress', 'pause-resume-active-cleanup') }
    [pscustomobject]@{
        Version = $Version
        Name = switch ($Version) { 1 { 'PlaybackCoreLifecycle' } 2 { 'PlaybackCoreLifecycleV2' } 3 { 'PlaybackCoreLifecycleV3' } }
        WorkloadVersion = switch ($Version) { 1 { 'playback-core-lifecycle-v1' } 2 { 'playback-core-lifecycle-v2' } 3 { 'playback-core-lifecycle-v3-cookie-0183-v1' } }
        HarnessManifestSchemaVersion = $Version
        BaselineRevision = switch ($Version) { 1 { '2544daa13eb23fdf03bd922e65b96fcffaba421c' } 2 { '1d00c3b71fbf1f94003865bad6c4eba6d8dcd902' } 3 { '4bc4a8409d3250da012be6e92f6681e721c17f3a' } }
        BaselineTree = switch ($Version) { 1 { '155882714df131ddc9b9985133a1f0611736b7cd' } 2 { '25a20291b9cf84b1283804f451b33e3c4605507c' } 3 { '8e2ce7c115458d95b9e13aa144b3d6cc16c083a4' } }
        LineageAnchorRevision = if ($Version -eq 3) { '1d00c3b71fbf1f94003865bad6c4eba6d8dcd902' } else { $null }
        BaselineSuccessorRevision = if ($Version -eq 3) { '4bc4a8409d3250da012be6e92f6681e721c17f3a' } else { $null }
        BaselineSuccessorTree = if ($Version -eq 3) { '8e2ce7c115458d95b9e13aa144b3d6cc16c083a4' } else { $null }
        BaselineSuccessorParentRevision = if ($Version -eq 3) { '1d00c3b71fbf1f94003865bad6c4eba6d8dcd902' } else { $null }
        BaselineSuccessorRef = if ($Version -eq 3) { 'refs/heads/qualification/playback-core-lifecycle-v3-baseline' } else { $null }
        BaselineSuccessorLockSha256 = if ($Version -eq 3) { 'baaf6e57419a1f28d9a880d655a48863676fe2fe0ddcd9a1467153912af13b68' } else { $null }
        AuthoredPrefixParityContract = if ($Version -ge 2) { 'timestamp-kind-scan-mask-v1' } else { $null }
        CandidateLineageBranch = if ($Version -eq 1) { 'refactor/playback-core-lifecycle' } else { 'refactor/playback-remove-release-gap' }
        Target = 'x86_64-pc-windows-msvc'
        CargoProfile = 'dist'
        RequiredFeatures = @('real-input-acceptance')
        RustVersion = '1.99.1'
        CargoVersion = '1.99.1'
        ObserverMode = 'SchedulerQueuedV3Compat3'
        ObserverVersion = 'scheduler-receiveonly-queue-v3-compat3'
        ObserverSchemaVersion = 3
        ObserverEventSchemaVersion = 3
        PerformanceTimingMarginUs = 500
        SafetyTimingMarginUs = 500
        PairCount = 6
        SampleSeconds = 15
        PairOrder = 'AB/BA alternating'
        LoadCells = @('Quiet', 'Contention')
        PerformanceWorkloads = $performanceWorkloads
        SafetyScenarios = $safetyScenarios
        Statistics = @('p50', 'p95', 'p99', 'max')
        Metrics = @(
            'eligible-to-pre-call-us',
            'wait-to-pre-call-us',
            'pre-call-to-completion-us',
            'stop-to-worker-release-verified-us',
            'worker-cpu-us',
            'worker-spin-us',
            'raw-up-syscall-count',
            'worker-joined-to-next-playback-start-us'
        )
        TargetIdentity = [ordered]@{
            target_class = 'project-owned-non-game-receive-only'
            target_name = 'Sky Auto Player — Native Acceptance Sink'
            game_target = $false
            capture_status = 'NOT_RUN'
            required_at_run = 'owner-approved exact project-owned ReceiveOnly HWND, PID, and retained process identity'
        }
        RunnerProvenancePaths = @(
            'scripts/build_dispatch_core_harness.ps1',
            'scripts/dispatch_harness_manifest.psm1',
            'scripts/dispatch_qualification_profiles.psm1',
            'scripts/qualify_dispatch_core.ps1',
            'scripts/run_native_acceptance_case.ps1',
            'scripts/dispatch_qualification_report.psm1',
            'scripts/dispatch_qualification_orchestration.psm1',
            'scripts/dispatch_pair_screen.psm1',
            'scripts/native_acceptance_sink.ps1',
            'scripts/lib/playback_core_lifecycle_collector.psm1',
            'scripts/test_qualify_dispatch_core_contracts.ps1',
            'docs/playback-verification.md'
        )
    }
}

function Get-PlaybackCoreLifecycleProfileForName {
    param([Parameter(Mandatory)] [ValidateSet('PlaybackCoreLifecycle', 'PlaybackCoreLifecycleV2', 'PlaybackCoreLifecycleV3')] [string]$Name)
    $version = switch ($Name) { 'PlaybackCoreLifecycle' { 1 } 'PlaybackCoreLifecycleV2' { 2 } 'PlaybackCoreLifecycleV3' { 3 } }
    Get-PlaybackCoreLifecycleProfile -Version $version
}

function Get-DispatchCleanupM3Profile {
    [pscustomobject]@{
        Name = 'DispatchCleanupM3'
        WorkloadVersion = 'dispatch-runtime-cleanup-m3-v1'
        BaselineRevision = '2277ac34a21a57a8d5ec313bba55ad78fc60794f'
        BaselineTree = '9a8637129dbb7a05be4f9386da188413b45f9774'
        CandidateRevision = '74c2b2618af50881f15eacd27ab722d316954743'
        CandidateTree = '902c772442706b326e020db6d36daedd60632b02'
        CandidateLineageBranch = 'refactor/dispatch-runtime-cleanup'
        Target = 'x86_64-pc-windows-msvc'
        CargoProfile = 'dist'
        RequiredFeatures = @('real-input-acceptance')
        RustVersion = '1.99.1'
        CargoVersion = '1.99.1'
        ObserverMode = 'SchedulerQueuedV3Compat3'
        ObserverVersion = 'scheduler-receiveonly-queue-v3-compat3'
        ObserverSchemaVersion = 3
        ObserverEventSchemaVersion = 3
        PerformanceTimingMarginUs = 0
        SafetyTimingMarginUs = 500
        RunnerProvenancePaths = @(
            'scripts/build_dispatch_core_harness.ps1',
            'scripts/dispatch_harness_manifest.psm1',
            'scripts/dispatch_qualification_profiles.psm1',
            'scripts/qualify_dispatch_core.ps1',
            'scripts/run_native_acceptance_case.ps1',
            'scripts/dispatch_qualification_report.psm1',
            'scripts/dispatch_pair_screen.psm1',
            'scripts/native_acceptance_sink.ps1',
            'scripts/test_qualify_dispatch_core_contracts.ps1',
            'docs/playback-verification.md'
        )
    }
}

function Get-DispatchM3PathIdentity {
    param([Parameter(Mandatory)] [string]$Path)
    [System.IO.Path]::GetFullPath($Path).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
}

function Test-DispatchM3PathOutside {
    param([Parameter(Mandatory)] [string]$Path, [Parameter(Mandatory)] [string]$RepositoryRoot)
    $fullPath = Get-DispatchM3PathIdentity $Path
    $fullRoot = Get-DispatchM3PathIdentity $RepositoryRoot
    -not ([string]::Equals($fullPath, $fullRoot, [System.StringComparison]::OrdinalIgnoreCase) -or
        $fullPath.StartsWith($fullRoot + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase))
}

function Get-DispatchM3GitCheckoutIdentity {
    param([Parameter(Mandatory)] [string]$Path)
    $fullPath = Get-DispatchM3PathIdentity $Path
    $revision = (& git -C $fullPath rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve checkout HEAD' }
    $tree = (& git -C $fullPath rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve checkout tree' }
    $branch = ((& git -C $fullPath branch --show-current) -join '').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve checkout branch' }
    $topLevel = (& git -C $fullPath rev-parse --show-toplevel).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve checkout top level' }
    # Git may canonicalize a worktree path differently from the caller (for
    # example, when TEMP uses a short 8.3 path). Use Git's checkout root for
    # both identity fields so clean source manifests have one stable path.
    $fullPath = Get-DispatchM3PathIdentity $topLevel
    $gitDirOutput = (& git -C $fullPath rev-parse --git-dir).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve checkout git directory' }
    $commonDirOutput = (& git -C $fullPath rev-parse --git-common-dir).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve checkout common git directory' }
    $gitDir = if ([System.IO.Path]::IsPathRooted($gitDirOutput)) { Get-DispatchM3PathIdentity $gitDirOutput } else { Get-DispatchM3PathIdentity (Join-Path $fullPath $gitDirOutput) }
    $commonDir = if ([System.IO.Path]::IsPathRooted($commonDirOutput)) { Get-DispatchM3PathIdentity $commonDirOutput } else { Get-DispatchM3PathIdentity (Join-Path $fullPath $commonDirOutput) }
    $changes = (& git -C $fullPath status --porcelain --untracked-files=all) -join ''
    if ($LASTEXITCODE -ne 0) { throw 'could not verify checkout cleanliness' }
    [pscustomobject]@{
        root = $fullPath
        revision = $revision
        tree = $tree
        branch = $branch
        clean = [string]::IsNullOrWhiteSpace($changes)
        checkout = [ordered]@{
            top_level = Get-DispatchM3PathIdentity $topLevel
            git_dir = $gitDir
            git_common_dir = $commonDir
        }
    }
}

function Assert-DispatchCleanupM3SourceCheckout {
    param(
        [Parameter(Mandatory)] [string]$SourceRoot,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$SourceRole,
        [Parameter(Mandatory)] [string]$RunnerRoot
    )
    $profile = Get-DispatchCleanupM3Profile
    $identity = Get-DispatchM3GitCheckoutIdentity $SourceRoot
    $expectedRevision = if ($SourceRole -eq 'Baseline') { $profile.BaselineRevision } else { $profile.CandidateRevision }
    $expectedTree = if ($SourceRole -eq 'Baseline') { $profile.BaselineTree } else { $profile.CandidateTree }
    $runnerIdentity = Get-DispatchM3GitCheckoutIdentity $RunnerRoot
    if (-not $identity.clean -or $identity.revision -ne $expectedRevision -or $identity.tree -ne $expectedTree -or
        -not [string]::IsNullOrWhiteSpace($identity.branch) -or $identity.checkout.top_level -ne $identity.root -or
        $identity.checkout.git_common_dir -ne $runnerIdentity.checkout.git_common_dir -or
        -not (Test-DispatchM3PathOutside -Path $identity.root -RepositoryRoot $RunnerRoot)) {
        throw ('DispatchCleanupM3 ' + $SourceRole + ' must be a clean detached worktree at its exact pinned SHA/tree outside the runner checkout')
    }
    if (-not (Test-Path -LiteralPath (Join-Path $identity.root 'rust\Cargo.toml') -PathType Leaf)) {
        throw ('DispatchCleanupM3 ' + $SourceRole + ' source root is missing the Rust workspace')
    }
    $identity
}

function Get-DispatchCleanupM3RunnerIdentity {
    param([Parameter(Mandatory)] [string]$RunnerRoot)
    $profile = Get-DispatchCleanupM3Profile
    $identity = Get-DispatchM3GitCheckoutIdentity $RunnerRoot
    if (-not $identity.clean -or $identity.checkout.top_level -ne $identity.root -or
        $identity.branch -ne $profile.CandidateLineageBranch) {
        throw 'DispatchCleanupM3 runner must be a clean checkout on refactor/dispatch-runtime-cleanup'
    }
    $remoteLine = @(& git -C $identity.root ls-remote origin ('refs/heads/' + $profile.CandidateLineageBranch) 2>&1)
    $remoteExit = $LASTEXITCODE
    if ($remoteExit -ne 0) { throw ('could not verify DispatchCleanupM3 runner lineage against origin: ' + ($remoteLine -join ' ')) }
    $remoteRows = @($remoteLine | ForEach-Object { [string]$_ } | Where-Object { $_ -match '^([0-9a-f]{40})\s+refs/heads/refactor/dispatch-runtime-cleanup$' })
    if ($remoteRows.Count -ne 1) { throw 'origin does not advertise exactly one refactor/dispatch-runtime-cleanup tip' }
    $lineageTip = [regex]::Match($remoteRows[0], '^([0-9a-f]{40})').Groups[1].Value
    if ($identity.revision -ne $lineageTip) { throw 'runner checkout must match the pushed branch tip before M3 manifest build or qualification' }
    & git -C $identity.root cat-file -e ($profile.CandidateRevision + '^{commit}')
    if ($LASTEXITCODE -ne 0) { throw 'accepted M2 candidate commit is unavailable in the runner repository' }
    $candidateTree = (& git -C $identity.root rev-parse ($profile.CandidateRevision + '^{tree}')).Trim()
    if ($LASTEXITCODE -ne 0 -or $candidateTree -ne $profile.CandidateTree) { throw 'accepted M2 candidate tree does not match its pinned identity' }
    & git -C $identity.root merge-base --is-ancestor $profile.CandidateRevision $lineageTip
    if ($LASTEXITCODE -ne 0) { throw 'accepted M2 candidate is not an ancestor of the verified remote branch tip' }
    [pscustomobject]@{
        root = $identity.root
        revision = $identity.revision
        tree = $identity.tree
        branch = $identity.branch
        clean = $identity.clean
        checkout = $identity.checkout
        candidate_lineage_branch = $profile.CandidateLineageBranch
        candidate_lineage_tip_revision = $lineageTip
        candidate_lineage_verified = $true
    }
}

function Get-DispatchSessionRedesignM3RunnerIdentity {
    param([Parameter(Mandatory)] [string]$RunnerRoot)
    $profile = Get-DispatchSessionRedesignM3Profile
    $identity = Get-DispatchM3GitCheckoutIdentity $RunnerRoot
    if (-not $identity.clean -or $identity.checkout.top_level -ne $identity.root -or
        $identity.branch -ne $profile.CandidateLineageBranch) {
        throw 'DispatchSessionRedesignM3 runner must be a clean checkout on refactor/dispatch-session-redesign'
    }
    $remoteLine = @(& git -C $identity.root ls-remote origin ('refs/heads/' + $profile.CandidateLineageBranch) 2>&1)
    $remoteExit = $LASTEXITCODE
    if ($remoteExit -ne 0) { throw ('could not verify DispatchSessionRedesignM3 runner lineage against origin: ' + ($remoteLine -join ' ')) }
    $remoteRows = @($remoteLine | ForEach-Object { [string]$_ } | Where-Object { $_ -match '^([0-9a-f]{40})\s+refs/heads/refactor/dispatch-session-redesign$' })
    if ($remoteRows.Count -ne 1) { throw 'origin does not advertise exactly one refactor/dispatch-session-redesign tip' }
    $lineageTip = [regex]::Match($remoteRows[0], '^([0-9a-f]{40})').Groups[1].Value
    if ($identity.revision -ne $lineageTip) { throw 'runner checkout must match the pushed refactor/dispatch-session-redesign tip' }
    & git -C $identity.root merge-base --is-ancestor $profile.BaselineRevision $lineageTip
    if ($LASTEXITCODE -ne 0) { throw 'runner candidate is not descended from the pinned M3 baseline' }
    [pscustomobject]@{
        root = $identity.root
        revision = $identity.revision
        tree = $identity.tree
        branch = $identity.branch
        clean = $identity.clean
        checkout = $identity.checkout
        baseline_revision = $profile.BaselineRevision
        baseline_tree = $profile.BaselineTree
        candidate_lineage_branch = $profile.CandidateLineageBranch
        candidate_lineage_tip_revision = $lineageTip
        candidate_lineage_verified = $true
    }
}

function Assert-DispatchSessionRedesignM3SourceCheckout {
    param(
        [Parameter(Mandatory)] [string]$SourceRoot,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$SourceRole,
        [Parameter(Mandatory)] [string]$RunnerRoot
    )
    $profile = Get-DispatchSessionRedesignM3Profile
    $identity = Get-DispatchM3GitCheckoutIdentity $SourceRoot
    $runnerIdentity = Get-DispatchSessionRedesignM3RunnerIdentity -RunnerRoot $RunnerRoot
    $expectedRevision = if ($SourceRole -eq 'Baseline') { $profile.BaselineRevision } else { $runnerIdentity.revision }
    $expectedTree = if ($SourceRole -eq 'Baseline') { $profile.BaselineTree } else { $runnerIdentity.tree }
    if (-not $identity.clean -or $identity.revision -ne $expectedRevision -or $identity.tree -ne $expectedTree -or
        -not [string]::IsNullOrWhiteSpace($identity.branch) -or $identity.checkout.top_level -ne $identity.root -or
        $identity.checkout.git_common_dir -ne $runnerIdentity.checkout.git_common_dir -or
        -not (Test-DispatchM3PathOutside -Path $identity.root -RepositoryRoot $RunnerRoot)) {
        throw ('DispatchSessionRedesignM3 ' + $SourceRole + ' must be a clean detached worktree at its exact source SHA/tree outside the runner checkout')
    }
    if (-not (Test-Path -LiteralPath (Join-Path $identity.root 'rust\Cargo.toml') -PathType Leaf)) {
        throw ('DispatchSessionRedesignM3 ' + $SourceRole + ' source root is missing the Rust workspace')
    }
    $identity
}

function Assert-PlaybackCoreLifecycleV3BaselineSuccessor {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [string]$Revision,
        [string]$ExpectedTree,
        [string]$ExpectedLockSha256
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version 3
    if (-not $Revision) { $Revision = $profile.BaselineSuccessorRevision }
    if (-not $ExpectedTree) { $ExpectedTree = $profile.BaselineSuccessorTree }
    if (-not $ExpectedLockSha256) { $ExpectedLockSha256 = $profile.BaselineSuccessorLockSha256 }
    if ($Revision -ne $profile.BaselineSuccessorRevision) {
        throw 'PlaybackCoreLifecycleV3 baseline successor revision differs from its immutable pin'
    }
    if ($ExpectedTree -ne $profile.BaselineSuccessorTree) {
        throw 'PlaybackCoreLifecycleV3 baseline successor expected tree differs from its immutable pin'
    }
    if ($ExpectedLockSha256 -ne $profile.BaselineSuccessorLockSha256) {
        throw 'PlaybackCoreLifecycleV3 baseline successor expected lock hash differs from its immutable pin'
    }
    & git -C $RepositoryRoot cat-file -e ($revision + '^{commit}')
    if ($LASTEXITCODE -ne 0) { throw 'PlaybackCoreLifecycleV3 baseline successor commit is unavailable' }
    $parent = (& git -C $RepositoryRoot rev-parse ($revision + '^')).Trim()
    if ($LASTEXITCODE -ne 0 -or $parent -ne $profile.BaselineSuccessorParentRevision) {
        throw 'PlaybackCoreLifecycleV3 baseline successor must have the exact V2 anchor as its parent'
    }
    $tree = (& git -C $RepositoryRoot rev-parse ($revision + '^{tree}')).Trim()
    if ($LASTEXITCODE -ne 0 -or $tree -ne $profile.BaselineSuccessorTree) {
        throw 'PlaybackCoreLifecycleV3 baseline successor tree differs from its pinned identity'
    }
    $changedPaths = @(& git -C $RepositoryRoot diff-tree --no-commit-id --name-only -r $revision)
    if ($LASTEXITCODE -ne 0 -or $changedPaths.Count -ne 1 -or $changedPaths[0] -ne 'rust/Cargo.lock') {
        throw 'PlaybackCoreLifecycleV3 baseline successor must change rust/Cargo.lock only'
    }
    $patch = (& git -C $RepositoryRoot diff ($revision + '^') $revision -- rust/Cargo.lock) -join [Environment]::NewLine
    $changedLines = @($patch -split '\r?\n' | Where-Object { $_ -match '^[+-](?![+-])' })
    $expectedChangedLines = @(
        '-version = "0.18.2"',
        '+version = "0.18.3"',
        '-checksum = "1a373e3602691c3cdea496d2f0ee5935151e6168fe87739483c463db1b2f2f87"',
        '+checksum = "d55f2d6c30eeec7d5c6f98bd4395d4abb1895629eab97ee6522d340623615f8a"'
    )
    if ($LASTEXITCODE -ne 0 -or
        $changedLines.Count -ne $expectedChangedLines.Count -or
        (@(Compare-Object -ReferenceObject $expectedChangedLines -DifferenceObject $changedLines -SyncWindow 0)).Count -ne 0) {
        throw 'PlaybackCoreLifecycleV3 baseline successor lock delta is not the exact cookie 0.18.2 to 0.18.3 patch'
    }
    [pscustomobject]@{
        revision = $revision
        tree = $tree
        parent_revision = $parent
        lock_sha256 = $ExpectedLockSha256
        changed_paths = $changedPaths
    }
}

function Test-DispatchGitAncestor {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [string]$AncestorRevision,
        [Parameter(Mandatory)] [string]$DescendantRevision
    )
    $gitOutput = @(& git -C $RepositoryRoot merge-base --is-ancestor $AncestorRevision $DescendantRevision 2>&1)
    $exitCode = $LASTEXITCODE
    if ($exitCode -eq 0) { return $true }
    if ($exitCode -eq 1) { return $false }
    throw ('git merge-base --is-ancestor failed with exit ' + $exitCode + ': ' + ($gitOutput -join ' '))
}

function Assert-PlaybackCoreLifecycleV3CandidateLineage {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [string]$CandidateRevision,
        [Parameter(Mandatory)] [string]$RemoteTipRevision
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version 3
    if ($CandidateRevision -notmatch '^[0-9a-f]{40}$' -or
        $CandidateRevision -ne $RemoteTipRevision) {
        throw 'PlaybackCoreLifecycleV3 candidate must match the exact remote PR branch tip'
    }
    if (-not (Test-DispatchGitAncestor -RepositoryRoot $RepositoryRoot -AncestorRevision $profile.LineageAnchorRevision -DescendantRevision $CandidateRevision)) {
        throw 'PlaybackCoreLifecycleV3 candidate is not descended from the immutable V2 lineage anchor'
    }
    if (Test-DispatchGitAncestor -RepositoryRoot $RepositoryRoot -AncestorRevision $profile.BaselineSuccessorRevision -DescendantRevision $CandidateRevision) {
        throw 'PlaybackCoreLifecycleV3 baseline successor must remain a sibling of the candidate lineage'
    }
    $true
}

function Assert-PlaybackCoreLifecyclePairLineage {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [ValidateSet(1, 2, 3)] [int]$ProfileVersion,
        [Parameter(Mandatory)] [string]$CandidateRevision,
        [Parameter(Mandatory)] [string]$RemoteTipRevision,
        [Parameter(Mandatory)] [string]$ControlRevision,
        [Parameter(Mandatory)] [string]$ControlTree,
        [string]$ControlLockSha256
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version $ProfileVersion
    if ($CandidateRevision -notmatch '^[0-9a-f]{40}$' -or $CandidateRevision -ne $RemoteTipRevision) {
        throw ('PlaybackCoreLifecycle ' + $profile.Name + ' candidate must match the exact remote PR branch tip')
    }
    if ($ControlRevision -ne $profile.BaselineRevision -or $ControlTree -ne $profile.BaselineTree) {
        throw ('PlaybackCoreLifecycle ' + $profile.Name + ' control must use its exact pinned baseline revision and tree')
    }

    if ($ProfileVersion -eq 3) {
        Assert-PlaybackCoreLifecycleV3BaselineSuccessor -RepositoryRoot $RepositoryRoot `
            -Revision $ControlRevision -ExpectedTree $ControlTree -ExpectedLockSha256 $ControlLockSha256 | Out-Null
        Assert-PlaybackCoreLifecycleV3CandidateLineage -RepositoryRoot $RepositoryRoot `
            -CandidateRevision $CandidateRevision -RemoteTipRevision $RemoteTipRevision | Out-Null
    } elseif (-not (Test-DispatchGitAncestor -RepositoryRoot $RepositoryRoot `
            -AncestorRevision $ControlRevision -DescendantRevision $CandidateRevision)) {
        throw ('PlaybackCoreLifecycle ' + $profile.Name + ' control source revision is not an ancestor of the candidate checkout')
    }
    $true
}

function Assert-PlaybackCoreLifecycleV3LockfileBytes {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$SourceRole
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version 3
    $root = Get-DispatchM3PathIdentity $RepositoryRoot
    $lockPath = Join-Path $root 'rust\Cargo.lock'
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        throw ('PlaybackCoreLifecycleV3 ' + $SourceRole + ' checkout is missing rust/Cargo.lock')
    }

    $bytes = [System.IO.File]::ReadAllBytes($lockPath)
    if ([Array]::IndexOf($bytes, [byte]13) -ge 0) {
        throw ('PlaybackCoreLifecycleV3 ' + $SourceRole + ' rust/Cargo.lock contains CR bytes; create both Windows worktrees with core.autocrlf=false and core.eol=lf. To restore only this working-tree file, run: git -c core.autocrlf=false -c core.eol=lf -C "' + $root + '" checkout-index --force -- rust/Cargo.lock; then verify SHA-256 ' + $profile.BaselineSuccessorLockSha256)
    }

    $actualHash = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $profile.BaselineSuccessorLockSha256) {
        throw ('PlaybackCoreLifecycleV3 ' + $SourceRole + ' rust/Cargo.lock SHA-256 must match the pinned LF bytes; expected ' + $profile.BaselineSuccessorLockSha256 + ', got ' + $actualHash)
    }
    $actualHash
}

function Get-PlaybackCoreLifecycleRunnerIdentity {
    param([Parameter(Mandatory)] [string]$RunnerRoot, [ValidateSet(1, 2, 3)] [int]$ProfileVersion = 1)
    $profile = Get-PlaybackCoreLifecycleProfile -Version $ProfileVersion
    if ($ProfileVersion -eq 3) {
        Assert-PlaybackCoreLifecycleV3LockfileBytes -RepositoryRoot $RunnerRoot -SourceRole Candidate | Out-Null
        $baselineSuccessor = Assert-PlaybackCoreLifecycleV3BaselineSuccessor -RepositoryRoot $RunnerRoot
    }
    $identity = Get-DispatchM3GitCheckoutIdentity $RunnerRoot
    if (-not $identity.clean -or $identity.checkout.top_level -ne $identity.root -or
        $identity.branch -ne $profile.CandidateLineageBranch) {
        throw ('PlaybackCoreLifecycle runner must be a clean checkout on ' + $profile.CandidateLineageBranch)
    }
    $remoteLine = @(& git -C $identity.root ls-remote origin ('refs/heads/' + $profile.CandidateLineageBranch) 2>&1)
    $remoteExit = $LASTEXITCODE
    if ($remoteExit -ne 0) { throw ('could not verify PlaybackCoreLifecycle candidate against origin: ' + ($remoteLine -join ' ')) }
    $remoteRows = @($remoteLine | ForEach-Object { [string]$_ } | Where-Object { $_ -match ('^([0-9a-f]{40})\s+refs/heads/' + [regex]::Escape($profile.CandidateLineageBranch) + '$') })
    if ($remoteRows.Count -ne 1) { throw 'origin does not advertise exactly one PlaybackCoreLifecycle candidate branch tip' }
    $lineageTip = [regex]::Match($remoteRows[0], '^([0-9a-f]{40})').Groups[1].Value
    if ($ProfileVersion -eq 3) {
        Assert-PlaybackCoreLifecycleV3CandidateLineage -RepositoryRoot $identity.root -CandidateRevision $identity.revision -RemoteTipRevision $lineageTip | Out-Null
        $baselineRefOutput = @(& git -C $identity.root ls-remote origin $profile.BaselineSuccessorRef 2>&1)
        $baselineRefExit = $LASTEXITCODE
        if ($baselineRefExit -ne 0) { throw ('could not verify PlaybackCoreLifecycleV3 baseline successor ref: ' + ($baselineRefOutput -join ' ')) }
        $baselineRefRows = @($baselineRefOutput | ForEach-Object { [string]$_ } | Where-Object { $_ -match ('^([0-9a-f]{40})\s+' + [regex]::Escape($profile.BaselineSuccessorRef) + '$') })
        if ($baselineRefRows.Count -ne 1 -or [regex]::Match($baselineRefRows[0], '^([0-9a-f]{40})').Groups[1].Value -ne $profile.BaselineSuccessorRevision) {
            throw 'PlaybackCoreLifecycleV3 pinned baseline successor ref is missing or does not point to the exact B1 commit'
        }
    } else {
        if ($identity.revision -ne $lineageTip) { throw 'PlaybackCoreLifecycle runner must match the pushed candidate branch tip' }
        & git -C $identity.root merge-base --is-ancestor $profile.BaselineRevision $lineageTip
        if ($LASTEXITCODE -ne 0) { throw 'PlaybackCoreLifecycle candidate is not descended from the pinned redesign baseline' }
    }
    [pscustomobject]@{
        root = $identity.root
        revision = $identity.revision
        tree = $identity.tree
        branch = $identity.branch
        clean = $identity.clean
        checkout = $identity.checkout
        baseline_revision = $profile.BaselineRevision
        baseline_tree = $profile.BaselineTree
        candidate_lineage_branch = $profile.CandidateLineageBranch
        candidate_lineage_tip_revision = $lineageTip
        candidate_lineage_verified = $true
        lineage_anchor_revision = if ($ProfileVersion -eq 3) { $profile.LineageAnchorRevision } else { $null }
        baseline_successor_revision = if ($ProfileVersion -eq 3) { $baselineSuccessor.revision } else { $null }
        baseline_successor_tree_sha = if ($ProfileVersion -eq 3) { $baselineSuccessor.tree } else { $null }
        baseline_successor_parent_revision = if ($ProfileVersion -eq 3) { $baselineSuccessor.parent_revision } else { $null }
        baseline_successor_ref = if ($ProfileVersion -eq 3) { $profile.BaselineSuccessorRef } else { $null }
        baseline_successor_lock_sha256 = if ($ProfileVersion -eq 3) { $baselineSuccessor.lock_sha256 } else { $null }
    }
}

function Assert-PlaybackCoreLifecycleSourceCheckout {
    param(
        [Parameter(Mandatory)] [string]$SourceRoot,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$SourceRole,
        [Parameter(Mandatory)] [string]$RunnerRoot,
        [ValidateSet(1, 2, 3)] [int]$ProfileVersion = 1
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version $ProfileVersion
    if ($ProfileVersion -eq 3) {
        Assert-PlaybackCoreLifecycleV3LockfileBytes -RepositoryRoot $SourceRoot -SourceRole $SourceRole | Out-Null
    }
    $identity = Get-DispatchM3GitCheckoutIdentity $SourceRoot
    $runnerIdentity = Get-PlaybackCoreLifecycleRunnerIdentity -RunnerRoot $RunnerRoot -ProfileVersion $ProfileVersion
    $expectedRevision = if ($SourceRole -eq 'Baseline') { $profile.BaselineRevision } else { $runnerIdentity.revision }
    $expectedTree = if ($SourceRole -eq 'Baseline') { $profile.BaselineTree } else { $runnerIdentity.tree }
    $expectedBranch = if ($SourceRole -eq 'Baseline') { '' } else { $profile.CandidateLineageBranch }
    if (-not $identity.clean -or $identity.revision -ne $expectedRevision -or $identity.tree -ne $expectedTree -or
        $identity.branch -ne $expectedBranch -or $identity.checkout.top_level -ne $identity.root -or
        $identity.checkout.git_common_dir -ne $runnerIdentity.checkout.git_common_dir -or
        ($SourceRole -eq 'Baseline' -and -not (Test-DispatchM3PathOutside -Path $identity.root -RepositoryRoot $RunnerRoot)) -or
        ($SourceRole -eq 'Candidate' -and $identity.root -ne $runnerIdentity.root)) {
        throw ('PlaybackCoreLifecycle ' + $SourceRole + ' must match its exact clean source identity and role')
    }
    if (-not (Test-Path -LiteralPath (Join-Path $identity.root 'rust\Cargo.toml') -PathType Leaf)) {
        throw ('PlaybackCoreLifecycle ' + $SourceRole + ' source root is missing the Rust workspace')
    }
    $identity
}

Export-ModuleMember -Function Get-DispatchSessionRedesignM3Profile, Get-DispatchCleanupM3Profile, Get-PlaybackCoreLifecycleProfile, Get-PlaybackCoreLifecycleProfileForName, Test-DispatchGitAncestor, Assert-PlaybackCoreLifecycleV3BaselineSuccessor, Assert-PlaybackCoreLifecycleV3CandidateLineage, Assert-PlaybackCoreLifecyclePairLineage, Assert-PlaybackCoreLifecycleV3LockfileBytes, Get-DispatchM3PathIdentity, Test-DispatchM3PathOutside, Get-DispatchM3GitCheckoutIdentity, Assert-DispatchCleanupM3SourceCheckout, Get-DispatchCleanupM3RunnerIdentity, Get-DispatchSessionRedesignM3RunnerIdentity, Assert-DispatchSessionRedesignM3SourceCheckout, Get-PlaybackCoreLifecycleRunnerIdentity, Assert-PlaybackCoreLifecycleSourceCheckout
