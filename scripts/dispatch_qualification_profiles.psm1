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
        [Parameter(Mandatory)] [string]$CandidateLockSha256,
        [Parameter(Mandatory)] [string]$ControlLockSha256
    )
    $profile = Get-PlaybackCoreLifecycleProfile -Version $ProfileVersion
    if ($CandidateRevision -notmatch '^[0-9a-f]{40}$' -or $CandidateRevision -ne $RemoteTipRevision) {
        throw ('PlaybackCoreLifecycle ' + $profile.Name + ' candidate must match the exact remote PR branch tip')
    }
    if ($ControlRevision -ne $profile.BaselineRevision -or $ControlTree -ne $profile.BaselineTree) {
        throw ('PlaybackCoreLifecycle ' + $profile.Name + ' control must use its exact pinned baseline revision and tree')
    }
    if ($CandidateLockSha256 -notmatch '^[0-9a-f]{64}$' -or
        $ControlLockSha256 -notmatch '^[0-9a-f]{64}$' -or
        $CandidateLockSha256 -ne $ControlLockSha256) {
        throw ('PlaybackCoreLifecycle ' + $profile.Name + ' candidate and control Cargo.lock SHA-256 values must match')
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

function New-CompletionAnchoredPulseV1SafetyRow {
    param(
        [Parameter(Mandatory)] [string]$Id,
        [Parameter(Mandatory)] [int]$EffectiveHoldUs,
        [Parameter(Mandatory)] [ValidateSet('acceptance-floor', 'acceptance-floor-unused-no-down', 'isolated-long-hold')] [string]$HoldSource,
        [Parameter(Mandatory)] [ValidateSet('planned native', 'controlled mock', 'conditional')] [string]$EvidenceClass,
        [switch]$PreDownFailClosed,
        [Parameter(Mandatory)] [string]$Trigger,
        [Parameter(Mandatory)] [string]$ObligationAtTrigger,
        [Parameter(Mandatory)] [string]$Mask,
        [Parameter(Mandatory)] [string]$Verdict,
        [Parameter(Mandatory)] [string]$Proof,
        [Parameter(Mandatory)] [string]$RustRef
    )
    [pscustomobject][ordered]@{
        id = $Id
        scenario = $Id
        workload = 'safety-smoke'
        effective_hold_us = $EffectiveHoldUs
        hold_source = $HoldSource
        evidence_class = $EvidenceClass
        pre_down_fail_closed = [bool]$PreDownFailClosed
        control_trigger = $Trigger
        obligation_at_trigger = $ObligationAtTrigger
        expected_active_obligation_mask = $Mask
        proof_criteria = $Proof
        harness_classification = $Verdict
        rust_source_ref = $RustRef
    }
}

function Get-CompletionAnchoredPulseV1Profile {
    [pscustomobject]@{
        Name = 'CompletionAnchoredPulseV1'
        WorkloadVersion = 'completion-anchored-pulse-v1'
        HarnessManifestSchemaVersion = 4
        BaselineRevision = '1901cff6e2db3617b57145c7eae8d44a6c877fa2'
        BaselineTree = 'c15bcacb45245f1a29f9a4a7043dbdf33839a468'
        BaselineLockSha256 = '9b9ba6c03131d96029421bfa9f6f69f9ce331e003e32e095eac54a714e73021e'
        CandidateLineageBranch = 'refactor/completion-anchored-pulse-engine'
        Target = 'x86_64-pc-windows-msvc'
        CargoProfile = 'dist'
        RequiredFeatures = @('real-input-acceptance')
        RustVersion = '1.99.1'
        CargoVersion = '1.99.1'
        ObserverMode = 'SchedulerQueuedV3Compat3'
        ObserverVersion = 'scheduler-receiveonly-queue-v3-compat3'
        ObserverSchemaVersion = 3
        ObserverEventSchemaVersion = 3
        GameFps = 60
        ConfiguredHUs = 17167
        PerformanceTimingMarginUs = 500
        SafetyTimingMarginUs = 500
        PairCount = 6
        SampleSeconds = 15
        PairOrder = 'AB/BA alternating'
        LoadCells = @('Quiet', 'Contention')
        PairedPerformanceWorkloads = @('dense-alternating', 'near-minimum-retrigger', 'long-idle')
        ExcludedFromPairedPerformance = @('mixed-up-down', 'canonical-max-chord')
        CandidateOnlyGeometryWorkloads = @('mixed-up-down', 'canonical-max-chord')
        ExpectedAuthoredStreams = @{
            'dense-alternating' = [pscustomobject]@{ event_count = 896; group_count = 896; first_us = 50000; last_us = 15414465; sha256 = '802e84e0d9fa26cec3b293ed20af9a03202b5eb49b5f0cf66531d5b81f41fe73' }
            'near-minimum-retrigger' = [pscustomobject]@{ event_count = 880; group_count = 880; first_us = 50000; last_us = 15139793; sha256 = '2a01c69071ecfc28deafcfe719e3c35be882c15bcd41b291ba99d0a63306247c' }
            'long-idle' = [pscustomobject]@{ event_count = 32; group_count = 32; first_us = 50000; last_us = 16324672; sha256 = '7da0dce05f83dc76ee9429b3d5a2b820a43dea2274c12b18ce1e00774459f672' }
        }
        SafetyScenarios = @(
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'focus-loss' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor' -EvidenceClass 'planned native' `
                -Trigger 'After the first Down/Up pair is observed, a project-owned probe window takes foreground and a foreground hint reports target focus loss; the sink is then restored and a focused hint is published' `
                -ObligationAtTrigger 'zero: first note pair already released; the session is idle until the 3 s second onset' `
                -Mask 'zero_at_trigger_idle_between_notes' `
                -Verdict 'Pass when focus_evidence_clean holds (gate observed, focus-loss and target-change counters, probe and sink windows clean) and the common cleanup checks are clean; Inconclusive when a focus transition cannot be established or does not resume within the bounded proof deadline; Fail otherwise' `
                -Proof 'Session pauses on focus loss, resumes after restoration, no probe-window key events, no residue' `
                -RustRef 'scenarios.rs FocusLoss; rt_native_acceptance.rs needs_focus_probe branch and focus_evidence_clean'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'owner-mismatch' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor-unused-no-down' -EvidenceClass 'controlled mock' -PreDownFailClosed `
                -Trigger 'Test-only owner identity override makes the prepared Down fail window-owner verification before the sender' `
                -ObligationAtTrigger 'zero: no Down is sent' `
                -Mask 'zero_pre_down_fail_closed' `
                -Verdict 'Pass when terminal error is prepared_down_owner_mismatch, ReceiveOnly sink events are empty and the release obligation mask is zero; Fail otherwise' `
                -Proof 'Prepared-Down owner authority failure stops before gameplay delivery with zero sink events' `
                -RustRef 'scenarios.rs OwnerMismatch (no Down slots); owner_identity.rs failure_result'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'owner-process-termination' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor-unused-no-down' -EvidenceClass 'conditional' -PreDownFailClosed `
                -Trigger 'At first Down admission the harness terminates the identity-bound project-owned ReceiveOnly owner process' `
                -ObligationAtTrigger 'zero: first Down is not yet sent; zero Down sender attempts required' `
                -Mask 'zero_pre_down_fail_closed' `
                -Verdict 'Pass only when OS process termination is verified, terminal error is prepared_down_owner_process_terminated or focus_lost_during_preroll, sink events are empty, mask is zero and Down sender attempts are zero; Inconclusive when the checkpoint stops before the sender but termination is not verified; Fail otherwise' `
                -Proof 'Owner process termination fails closed before the sender; no cleanup transport is required' `
                -RustRef 'scenarios.rs OwnerProcessTermination (no Down slots); owner_identity.rs failure_result'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'pause-resume' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor' -EvidenceClass 'planned native' `
                -Trigger 'Pause after the first Down/Up pair is observed (keys idle), 50 ms later Resume; second onset at 500 ms' `
                -ObligationAtTrigger 'zero: ordinary Pause between notes; this is not the active-key cleanup case' `
                -Mask 'zero_at_pause_idle_between_notes' `
                -Verdict 'Pass when the pause commits, Resume is requested and common cleanup, sink-sequence and residue checks are clean; Fail when pause/resume control evidence is incomplete' `
                -Proof 'Pause observed, Resume requested, both notes delivered, no residue' `
                -RustRef 'scenarios.rs PauseResume; rt_native_acceptance.rs PauseResume branch'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'pause-resume-active-cleanup' -EffectiveHoldUs 450000 -HoldSource 'isolated-long-hold' -EvidenceClass 'planned native' `
                -Trigger 'Pause while the first Down (onset 50 ms, H 450 ms) is still held; Resume; second same-key onset at 600 ms' `
                -ObligationAtTrigger 'nonzero: slot 0 held, bit 0 of the pause cleanup obligation mask must be set' `
                -Mask 'nonzero_bit0_at_pause_cleared_while_paused' `
                -Verdict 'Pass when pause cleanup is verified with bit 0 set, QPC ordering of pause, cleanup-verified, Resume request/call/worker-commit and resume Down sender pre-call holds, and the sink sequence is press/release/press/release; Fail otherwise; Inconclusive on bounded-wait or ordering-evidence unavailability' `
                -Proof 'Active key released and verified at Pause; worker-committed Resume follows; fresh Down after Resume' `
                -RustRef 'scenarios.rs isolated_long_hold_us PauseResumeActiveCleanup=450000; rt_native_acceptance.rs PauseResumeActiveCleanup branch and active_cleanup_resume_evidence_is_ordered'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'suspend-resume' -EffectiveHoldUs 450000 -HoldSource 'isolated-long-hold' -EvidenceClass 'controlled mock' `
                -Trigger 'After the first Down is committed the harness injects a synthetic system-suspend notification (no real OS suspend); no resume notification is injected' `
                -ObligationAtTrigger 'nonzero: slot 0 held; cleanup attempted_mask bit 0 and at least one cleanup attempt are required' `
                -Mask 'nonzero_bit0_at_suspend_cleared' `
                -Verdict 'Pass when the session terminates with outcome system_suspend within the bounded deadline, suspend notifications are nonzero, resume notifications are zero, Down remains blocked, timeline rebase count is zero and the sink sequence is press/release; Fail on incomplete evidence; Inconclusive when suspend is not accepted or does not terminate in time. Does not prove cleanup before a real OS suspend completes, and no resume phase is exercised' `
                -Proof 'Synthetic suspend terminal cleanup of the held key; Down blocked afterward' `
                -RustRef 'scenarios.rs isolated_long_hold_us SuspendResume=450000; rt_native_acceptance.rs SuspendResume branch (notify_system_power(true) only)'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'stop-cleanup' -EffectiveHoldUs 9950000 -HoldSource 'isolated-long-hold' -EvidenceClass 'planned native' `
                -Trigger 'Stop (session quit) requested after the first Down is committed and while H 9.95 s keeps the key held' `
                -ObligationAtTrigger 'nonzero: slot 0 held; cleanup attempted_mask bit 0 and at least one attempt required' `
                -Mask 'nonzero_bit0_at_stop_cleared' `
                -Verdict 'Pass when outcome is quit, sink events are nonempty, cleanup attempted_mask bit 0 is set with nonzero attempts and common residue checks are clean; Fail otherwise; stop-to-release latency is recorded, not gated by this plan' `
                -Proof 'Terminal cleanup releases the active physical key; raw terminal Up calls recorded where available' `
                -RustRef 'scenarios.rs isolated_long_hold_us StopCleanup=9950000; rt_native_acceptance.rs StopCleanup branch'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'skip-cleanup' -EffectiveHoldUs 9950000 -HoldSource 'isolated-long-hold' -EvidenceClass 'planned native' `
                -Trigger 'Skip requested after the first Down is committed and while H 9.95 s keeps the key held' `
                -ObligationAtTrigger 'nonzero: slot 0 held; cleanup attempted_mask bit 0 and at least one attempt required' `
                -Mask 'nonzero_bit0_at_skip_cleared' `
                -Verdict 'Pass when outcome is skipped, sink events are nonempty, cleanup attempted_mask bit 0 is set with nonzero attempts and common residue checks are clean; Fail otherwise' `
                -Proof 'Owned key released by skip cleanup; no residue' `
                -RustRef 'scenarios.rs isolated_long_hold_us SkipCleanup=9950000; rt_native_acceptance.rs SkipCleanup branch'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'cleanup-full-release' -EffectiveHoldUs 9950000 -HoldSource 'isolated-long-hold' -EvidenceClass 'planned native' `
                -Trigger 'All 15 keys are Down in one chord (H 9.95 s), the harness waits for all Down events, then requests quit' `
                -ObligationAtTrigger 'nonzero: all 15 key slots held (mask 0x7fff)' `
                -Mask 'nonzero_all_15_at_quit_cleared' `
                -Verdict 'Pass when the full-release cleanup evidence is clean (cleanup_evidence_clean with full release requested) and common residue checks are clean; Fail otherwise; Inconclusive when the Down events are not observed before the proof deadline' `
                -Proof 'Full 15-key terminal release verified with zero stuck keys' `
                -RustRef 'scenarios.rs CleanupFullRelease (MAX_KEYS=15) and isolated_long_hold_us=9950000; rt_native_acceptance.rs CleanupFullRelease branch'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'supervisor-lease-expiry' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor-unused-no-down' -EvidenceClass 'planned native' -PreDownFailClosed `
                -Trigger 'A session supervisor progress point is seeded once and then not renewed; the only onset is at 5 s, so the lease watchdog expires before any Down' `
                -ObligationAtTrigger 'zero: no Down is sent before expiry' `
                -Mask 'zero_pre_down_fail_closed' `
                -Verdict 'Pass when the supervisor-expiry cleanup exception holds, the wait was interrupted, timeline rebase count is zero, sink events are empty and the independent helper liveness probe advanced; Fail otherwise' `
                -Proof 'Session watchdog fails closed while an unrelated helper stays alive' `
                -RustRef 'scenarios.rs SupervisorLeaseExpiry (no Down slots); rt_native_acceptance.rs SupervisorLeaseExpiry branch'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'short-up-gap-stress' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor' -EvidenceClass 'planned native' `
                -Trigger 'Authored same-key Down at exactly onset + H for 513 consecutive cycles; the executor must send Up first, commit it, then re-evaluate Down' `
                -ObligationAtTrigger 'alternating: key held for each hold and released at each boundary' `
                -Mask 'alternating_held_and_released' `
                -Verdict 'Pass when physical hold-floor violations are zero and at least 512 hold samples are observed with clean residue checks; Fail on any hold-floor violation or sample shortfall' `
                -Proof 'Zero hold-floor violations; every observed physical hold meets H' `
                -RustRef 'generation_stress.rs SHORT_UP_GAP_STRESS_CYCLES=513, SHORT_UP_GAP_STRESS_MIN_HOLD_SAMPLES=512, production_visibility_qualification'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'target-hwnd-change' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor-unused-no-down' -EvidenceClass 'controlled mock' -PreDownFailClosed `
                -Trigger 'After Armed publication the harness invalidates the pinned target through the test seam; the only onset is at 500 ms' `
                -ObligationAtTrigger 'zero: no Down is sent' `
                -Mask 'zero_pre_down_fail_closed' `
                -Verdict 'Pass when the target-change preflight error and cleanup exception hold with zero sink transport events and zero release obligation; Fail otherwise' `
                -Proof 'Target change fails closed before gameplay delivery with zero transport events' `
                -RustRef 'scenarios.rs TargetHwndChange (no Down slots); rt_native_acceptance.rs invalidate_target_for_test and TargetHwndChange assertion'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'modifier-held-final-boundary' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor-unused-no-down' -EvidenceClass 'controlled mock' -PreDownFailClosed `
                -Trigger 'Test-support GetAsyncKeyState seam reports Shift (VK 0x10) held before the first Down; this is not physical user input' `
                -ObligationAtTrigger 'zero: first-Down modifier rejection; no gameplay and no manufactured cleanup' `
                -Mask 'zero_pre_down_fail_closed' `
                -Verdict 'Pass when terminal error contains physical_modifier_held, sink events are empty and cleanup attempted_mask and attempts are zero; Fail otherwise' `
                -Proof 'Modifier guard rejects the first Down with zero events and zero cleanup' `
                -RustRef 'modifier_guard.rs query_for_scenario and validate_rejection (ModifierHeldFinalBoundary)'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'modifier-held-after-owned' -EffectiveHoldUs 100000 -HoldSource 'isolated-long-hold' -EvidenceClass 'controlled mock' `
                -Trigger 'After the first owned Down is committed (H 100 ms) the test-support seam asserts Shift held; the second onset at 100 ms is rejected. The modifier is a seam value, not journal-owned' `
                -ObligationAtTrigger 'nonzero journal-owned: slot 0 only; the modifier is externally held and never owned' `
                -Mask 'owned_bit0_cleared_modifier_not_owned' `
                -Verdict 'Pass when terminal error contains physical_modifier_held, the sink sequence is press/release of the first key, and cleanup attempted_mask is 1 with exactly 1 attempt; Fail otherwise; Inconclusive when the first committed Down cannot be confirmed' `
                -Proof 'Cleanup is limited to the prior owned key; no modifier release is attempted' `
                -RustRef 'scenarios.rs isolated_long_hold_us ModifierHeldAfterOwned=100000; modifier_guard.rs arm_after_owned_down and validate_rejection'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'ambiguous-packet' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor' -EvidenceClass 'conditional' `
                -Trigger 'Injected partial SendInput outcome (mask 0b11) on a mixed Up+Down packet, only if timely split dispatch actually produces a mixed packet' `
                -ObligationAtTrigger 'conditional: nonzero cleanup mask 0b11 only when the injection is reached' `
                -Mask 'conditional_cleanup_mask_0b11' `
                -Verdict 'Inconclusive when sendinput_partial_events is zero (mixed packet never reached); Pass when the cleanup mask is exactly 0b11 with release verified and the exact bounded event sequence; Fail otherwise' `
                -Proof 'If a mixed packet is reached, exactly the bounded active keys are released and verified' `
                -RustRef 'scenarios.rs AmbiguousPacket; rt_native_acceptance.rs AmbiguousPacket verdicts; snapshot_report.rs ambiguous_packet_cleanup_qualified'),
            (New-CompletionAnchoredPulseV1SafetyRow -Id 'preflight-user-held' -EffectiveHoldUs 17167 -HoldSource 'acceptance-floor-unused-no-down' -EvidenceClass 'controlled mock' -PreDownFailClosed `
                -Trigger 'Preflight user-held injection reports a playback key held before session start' `
                -ObligationAtTrigger 'zero: the session is rejected before arming' `
                -Mask 'zero_pre_start_aborted' `
                -Verdict 'Pass when the pre-arm UserHeld preflight rejection holds with zero keyboard transport events; Fail otherwise' `
                -Proof 'Preflight refuses dispatch before any key event or cleanup transport' `
                -RustRef 'snapshot_report.rs preflight_user_held_result')
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
            'scripts/test_qualify_dispatch_core_contracts.ps1',
            'docs/playback-verification.md'
        )
    }
}

function Assert-CompletionAnchoredPulseV1LockfileBytes {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$SourceRole
    )
    $profile = Get-CompletionAnchoredPulseV1Profile
    $root = Get-DispatchM3PathIdentity $RepositoryRoot
    $lockPath = Join-Path $root 'rust\Cargo.lock'
    if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        throw ('CompletionAnchoredPulseV1 ' + $SourceRole + ' checkout is missing rust/Cargo.lock')
    }
    $bytes = [System.IO.File]::ReadAllBytes($lockPath)
    if ([Array]::IndexOf($bytes, [byte]13) -ge 0) {
        throw ('CompletionAnchoredPulseV1 ' + $SourceRole + ' rust/Cargo.lock contains CR bytes; LF bytes required')
    }
    $actualHash = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $profile.BaselineLockSha256) {
        throw ('CompletionAnchoredPulseV1 ' + $SourceRole + ' rust/Cargo.lock SHA-256 confounder: expected ' + $profile.BaselineLockSha256 + ', got ' + $actualHash)
    }
    $actualHash
}

function Assert-CompletionAnchoredPulseV1Lineage {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [string]$CandidateRevision,
        [Parameter(Mandatory)] [string]$RemoteTipRevision,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$SourceRole = 'Candidate',
        [string]$SourceTreeSha = ''
    )
    $profile = Get-CompletionAnchoredPulseV1Profile
    if ($SourceRole -eq 'Baseline') {
        if ($CandidateRevision -ne $profile.BaselineRevision) {
            throw 'CompletionAnchoredPulseV1 baseline revision differs from pinned 1901cff6'
        }
        if ([string]::IsNullOrWhiteSpace($SourceTreeSha) -or $SourceTreeSha -ne $profile.BaselineTree) {
            throw 'CompletionAnchoredPulseV1 baseline tree is missing or differs from pinned 1901cff6 tree'
        }
        return $true
    }
    if ([string]::IsNullOrWhiteSpace($RemoteTipRevision) -or $CandidateRevision -ne $RemoteTipRevision) {
        throw 'CompletionAnchoredPulseV1 candidate must match the exact pushed branch tip'
    }
    if (-not (Test-DispatchGitAncestor -RepositoryRoot $RepositoryRoot -AncestorRevision $profile.BaselineRevision -DescendantRevision $CandidateRevision)) {
        throw 'CompletionAnchoredPulseV1 candidate does not descend from pinned baseline 1901cff6'
    }
    $true
}

function Get-GitObjectRawSha256 {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [string]$ObjectSpec
    )
    $psi = [System.Diagnostics.ProcessStartInfo]::new('git', ('-C "' + $RepositoryRoot + '" cat-file blob ' + $ObjectSpec))
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $proc = [System.Diagnostics.Process]::Start($psi)
    $ms = [System.IO.MemoryStream]::new()
    $proc.StandardOutput.BaseStream.CopyTo($ms)
    $errText = $proc.StandardError.ReadToEnd()
    $proc.WaitForExit()
    if ($proc.ExitCode -ne 0) {
        throw ('git cat-file blob failed for ' + $ObjectSpec + ': ' + $errText.Trim())
    }
    $bytes = $ms.ToArray()
    if ([Array]::IndexOf($bytes, [byte]13) -ge 0) {
        throw ('Git blob ' + $ObjectSpec + ' contains CR bytes; LF bytes required')
    }
    [System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::HashData($bytes)).Replace('-', '').ToLowerInvariant()
}

function Test-CompletionAnchoredPulseV1ParityInt {
    param($Value, [long]$Minimum)
    ($Value -is [int] -or $Value -is [long]) -and ([long]$Value -ge $Minimum)
}

function Get-CompletionAnchoredPulseV1ParityRejection {
    param(
        [Parameter(Mandatory)] $Entry,
        [Parameter(Mandatory)] $Profile,
        [Parameter(Mandatory)] [hashtable]$Identities
    )
    $expected = $Profile.ExpectedAuthoredStreams[[string]$Entry.workload]
    $names = @($Entry.PSObject.Properties.Name)
    $settings = [ordered]@{ game_fps = $Profile.GameFps; timing_margin_us = $Profile.PerformanceTimingMarginUs; configured_H_us = $Profile.ConfiguredHUs; sample_seconds = $Profile.SampleSeconds }
    if ($names -notcontains 'evidence_origin' -or [string]::IsNullOrWhiteSpace([string]$Entry.evidence_origin)) { return 'missing evidence_origin' }
    foreach ($key in $settings.Keys) {
        if ($names -notcontains $key -or -not (Test-CompletionAnchoredPulseV1ParityInt $Entry.$key 0)) { return ('missing or non-integer workload setting ' + $key) }
        if ([long]$Entry.$key -ne [long]$settings[$key]) { return ('workload setting ' + $key + ' is ' + $Entry.$key + ', expected ' + $settings[$key]) }
    }
    foreach ($role in @('baseline', 'candidate')) {
        foreach ($field in @('revision', 'tree', 'lock_sha256')) {
            $name = $role + '_' + $field
            $pattern = if ($field -eq 'lock_sha256') { '^[0-9a-f]{64}$' } else { '^[0-9a-f]{40}$' }
            if ($names -notcontains $name -or [string]$Entry.$name -cnotmatch $pattern) { return ('missing or malformed ' + $name) }
            if ([string]$Entry.$name -ne [string]$Identities[$name]) { return ('stale or mismatched ' + $name + ': evidence ' + $Entry.$name + ', verified ' + $Identities[$name]) }
        }
        if ($names -notcontains ($role + '_verified') -or $Entry.($role + '_verified') -isnot [bool] -or $Entry.($role + '_verified') -ne $true) { return ($role + '_verified must be boolean true') }
        $sha = $role + '_sha256'
        if ($names -notcontains $sha -or [string]$Entry.$sha -cnotmatch '^[0-9a-f]{64}$') { return ('missing or invalid ' + $sha + ' (64 lowercase hex required)') }
        foreach ($field in @('event_count', 'group_count')) {
            $name = $role + '_' + $field
            if ($names -notcontains $name -or -not (Test-CompletionAnchoredPulseV1ParityInt $Entry.$name 1)) { return ('missing or non-positive ' + $name) }
        }
        $first = $role + '_first_us'; $last = $role + '_last_us'
        if ($names -notcontains $first -or -not (Test-CompletionAnchoredPulseV1ParityInt $Entry.$first 0)) { return ('missing or negative ' + $first) }
        if ($names -notcontains $last -or -not (Test-CompletionAnchoredPulseV1ParityInt $Entry.$last 0)) { return ('missing or negative ' + $last) }
        if ([long]$Entry.$last -lt [long]$Entry.$first) { return ($role + ' last_us precedes first_us') }
        if ([long]$Entry.($role + '_group_count') -gt [long]$Entry.($role + '_event_count')) { return ($role + ' group_count exceeds event_count') }
        if ([string]$Entry.$sha -ne $expected.sha256 -or
            [long]$Entry.($role + '_event_count') -ne $expected.event_count -or
            [long]$Entry.($role + '_group_count') -ne $expected.group_count -or
            [long]$Entry.$first -ne $expected.first_us -or
            [long]$Entry.$last -ne $expected.last_us) { return ($role + ' stream digest or geometry differs from the immutable expected authored stream') }
    }
    $null
}

function New-CompletionAnchoredPulseV1NonphysicalPlan {
    param(
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [switch]$PlanOnly,
        [switch]$OwnerApprovedReceiveOnlyRun,
        [object[]]$ParityEvidence
    )
    if ($PlanOnly -and $OwnerApprovedReceiveOnlyRun) {
        throw '-OwnerApprovedReceiveOnlyRun cannot be combined with -PlanOnly'
    }
    if (-not $PlanOnly) {
        throw 'CompletionAnchoredPulseV1 physical execution requires explicit owner authorization (use -PlanOnly for nonphysical plan)'
    }
    $profile = Get-CompletionAnchoredPulseV1Profile
    $identity = Get-DispatchM3GitCheckoutIdentity $RepositoryRoot
    if (-not $identity.clean) {
        throw 'CompletionAnchoredPulseV1 PlanOnly requires a clean checkout'
    }
    if ($identity.branch -ne $profile.CandidateLineageBranch) {
        throw ('CompletionAnchoredPulseV1 PlanOnly requires branch ' + $profile.CandidateLineageBranch)
    }

    # Verify actual pushed remote tip on origin via git ls-remote
    $remoteRef = 'refs/heads/' + $profile.CandidateLineageBranch
    $remoteLines = @(& git -C $identity.root ls-remote origin $remoteRef 2>&1)
    $remoteExit = $LASTEXITCODE
    if ($remoteExit -ne 0) {
        throw ('CompletionAnchoredPulseV1 could not verify remote branch tip against origin: ' + ($remoteLines -join ' '))
    }
    $remoteRows = @($remoteLines | ForEach-Object { [string]$_ } | Where-Object { $_ -match ('^([0-9a-f]{40})\s+' + [regex]::Escape($remoteRef) + '$') })
    if ($remoteRows.Count -ne 1) {
        throw ('origin does not advertise exactly one remote tip for ' + $remoteRef)
    }
    $remoteTipRevision = [regex]::Match($remoteRows[0], '^([0-9a-f]{40})').Groups[1].Value

    # Independently validate baseline commit, tree, and lockfile from immutable Git objects
    $actualBaselineCommit = (& git -C $identity.root rev-parse --verify ($profile.BaselineRevision + '^{commit}') 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $actualBaselineCommit -ne $profile.BaselineRevision) {
        throw ('CompletionAnchoredPulseV1 baseline commit ' + $profile.BaselineRevision + ' is missing or unresolvable in repository')
    }
    $actualBaselineTree = (& git -C $identity.root rev-parse --verify ($profile.BaselineRevision + '^{tree}') 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $actualBaselineTree -ne $profile.BaselineTree) {
        throw ('CompletionAnchoredPulseV1 baseline tree mismatch: expected ' + $profile.BaselineTree + ', got ' + $actualBaselineTree)
    }
    [void](Assert-CompletionAnchoredPulseV1Lineage -RepositoryRoot $identity.root -CandidateRevision $actualBaselineCommit -RemoteTipRevision $actualBaselineCommit -SourceRole Baseline -SourceTreeSha $actualBaselineTree)
    $actualBaselineLockSha = Get-GitObjectRawSha256 -RepositoryRoot $identity.root -ObjectSpec ($profile.BaselineRevision + ':rust/Cargo.lock')
    if ($actualBaselineLockSha -ne $profile.BaselineLockSha256) {
        throw ('CompletionAnchoredPulseV1 baseline rust/Cargo.lock SHA-256 mismatch: expected ' + $profile.BaselineLockSha256 + ', got ' + $actualBaselineLockSha)
    }

    $candidateLock = Assert-CompletionAnchoredPulseV1LockfileBytes -RepositoryRoot $identity.root -SourceRole Candidate
    [void](Assert-CompletionAnchoredPulseV1Lineage -RepositoryRoot $identity.root -CandidateRevision $identity.revision -RemoteTipRevision $remoteTipRevision -SourceRole Candidate)

    # Bind caller-supplied authored-stream parity evidence to the exact verified identities, settings and
    # immutable expected streams. The plan never executes the parity test itself.
    $identities = @{
        baseline_revision = $actualBaselineCommit
        baseline_tree = $actualBaselineTree
        baseline_lock_sha256 = $actualBaselineLockSha
        candidate_revision = $identity.revision
        candidate_tree = $identity.tree
        candidate_lock_sha256 = $candidateLock
    }
    $parityByWorkload = @{}
    $parityRejections = @()
    $evidenceIndex = 0
    if ($null -ne $ParityEvidence) {
        foreach ($item in @($ParityEvidence)) {
            $evidenceIndex++
            if ($null -eq $item -or $null -eq $item.PSObject.Properties['workload'] -or [string]::IsNullOrWhiteSpace([string]$item.workload)) {
                $parityRejections += [pscustomobject][ordered]@{ index = $evidenceIndex; workload = $null; reason = 'entry is null or has no workload' }
                continue
            }
            $name = [string]$item.workload
            if ($profile.PairedPerformanceWorkloads -cnotcontains $name) {
                $parityRejections += [pscustomobject][ordered]@{ index = $evidenceIndex; workload = $name; reason = 'unknown workload is not a paired performance workload' }
                continue
            }
            if (-not $parityByWorkload.ContainsKey($name)) { $parityByWorkload[$name] = @() }
            $parityByWorkload[$name] += ,$item
        }
    }
    $eligiblePairedWorkloads = @()
    $excludedByParity = @()
    $paritySummary = @()
    foreach ($workload in $profile.PairedPerformanceWorkloads) {
        $entries = @()
        if ($parityByWorkload.ContainsKey($workload)) { $entries = @($parityByWorkload[$workload]) }
        $entry = $null
        $bound = $false
        $reason = 'no parity evidence supplied'
        if ($entries.Count -gt 1) {
            $reason = 'duplicate parity evidence entries (' + $entries.Count + '); workload excluded'
        } elseif ($entries.Count -eq 1) {
            $entry = $entries[0]
            $rejection = Get-CompletionAnchoredPulseV1ParityRejection -Entry $entry -Profile $profile -Identities $identities
            if ($null -eq $rejection) {
                $bound = $true
                $reason = 'caller-supplied evidence bound to exact baseline/candidate identities, settings and immutable expected authored stream; not executed by this plan'
            } else {
                $reason = 'evidence rejected: ' + $rejection
            }
        }
        $summary = [ordered]@{
            workload = $workload
            evidence_bound = $bound
            reason = $reason
            evidence_origin = if ($bound) { [string]$entry.evidence_origin } else { $null }
            baseline_sha256 = if ($bound) { [string]$entry.baseline_sha256 } else { $null }
            candidate_sha256 = if ($bound) { [string]$entry.candidate_sha256 } else { $null }
            baseline_revision = if ($bound) { [string]$entry.baseline_revision } else { $null }
            baseline_tree = if ($bound) { [string]$entry.baseline_tree } else { $null }
            candidate_revision = if ($bound) { [string]$entry.candidate_revision } else { $null }
            candidate_tree = if ($bound) { [string]$entry.candidate_tree } else { $null }
            event_count = if ($bound) { [long]$entry.candidate_event_count } else { $null }
            group_count = if ($bound) { [long]$entry.candidate_group_count } else { $null }
            first_us = if ($bound) { [long]$entry.candidate_first_us } else { $null }
            last_us = if ($bound) { [long]$entry.candidate_last_us } else { $null }
            game_fps = if ($bound) { [long]$entry.game_fps } else { $null }
            timing_margin_us = if ($bound) { [long]$entry.timing_margin_us } else { $null }
            configured_H_us = if ($bound) { [long]$entry.configured_H_us } else { $null }
            sample_seconds = if ($bound) { [long]$entry.sample_seconds } else { $null }
        }
        $paritySummary += [pscustomobject]$summary
        if ($bound) {
            $eligiblePairedWorkloads += $workload
        } else {
            $excludedByParity += $workload
        }
    }
    $pairedEligibilityStatus = if ($eligiblePairedWorkloads.Count -eq $profile.PairedPerformanceWorkloads.Count) {
        'EVIDENCE_BOUND_ALL_PAIRED_WORKLOADS'
    } elseif ($eligiblePairedWorkloads.Count -gt 0) {
        'PARTIAL_EVIDENCE_BOUND'
    } else {
        'UNVERIFIED_NO_PARITY_EVIDENCE_SUPPLIED'
    }

    $plannedPairedSessions = @()
    foreach ($workload in $eligiblePairedWorkloads) {
        foreach ($load in $profile.LoadCells) {
            for ($pair = 1; $pair -le $profile.PairCount; $pair++) {
                $firstRole = if (($pair % 2) -eq 1) { 'Baseline' } else { 'Candidate' }
                $secondRole = if ($firstRole -eq 'Baseline') { 'Candidate' } else { 'Baseline' }
                foreach ($role in @($firstRole, $secondRole)) {
                    $plannedPairedSessions += [pscustomobject][ordered]@{
                        workload = $workload
                        load = $load
                        pair_index = $pair
                        role = $role
                        game_fps = $profile.GameFps
                        timing_margin_us = $profile.PerformanceTimingMarginUs
                        configured_H_us = $profile.ConfiguredHUs
                        sample_seconds = $profile.SampleSeconds
                        observer_mode = $profile.ObserverMode
                    }
                }
            }
        }
    }
    [pscustomobject][ordered]@{
        qualification_profile = $profile.Name
        workload_version = $profile.WorkloadVersion
        schema_version = $profile.HarnessManifestSchemaVersion
        plan_only = $true
        receiver_started = $false
        harness_started = $false
        no_native_invocation = $true
        native_build_status = 'NOT BUILT'
        native_manifest_status = 'NOT VERIFIED'
        binary_sha256 = 'NOT_BUILT'
        sendinput_calls = 0
        baseline_revision = $actualBaselineCommit
        baseline_tree = $actualBaselineTree
        baseline_lock_sha256 = $actualBaselineLockSha
        baseline_identity_verified_from_git_objects = $true
        candidate_revision = $identity.revision
        remote_tip_revision = $remoteTipRevision
        candidate_tree = $identity.tree
        candidate_branch = $identity.branch
        candidate_lock_sha256 = $candidateLock
        locks_identical = ($candidateLock -eq $actualBaselineLockSha)
        paired_eligibility_status = $pairedEligibilityStatus
        parity_evidence_origin = 'caller_supplied_test_output'
        parity_executed_by_plan = $false
        parity_evidence_rejections = @($parityRejections)
        paired_performance_workloads = @($eligiblePairedWorkloads)
        excluded_due_to_unbound_parity = @($excludedByParity)
        parity_evidence_summary = @($paritySummary)
        excluded_from_paired_performance = @($profile.ExcludedFromPairedPerformance)
        planned_paired_session_count = $plannedPairedSessions.Count
        planned_paired_session_count_if_all_verified = ($profile.PairedPerformanceWorkloads.Count * $profile.LoadCells.Count * $profile.PairCount * 2)
        planned_paired_sessions = $plannedPairedSessions
        candidate_only_geometry_workloads = @($profile.CandidateOnlyGeometryWorkloads)
        safety_scenarios = @($profile.SafetyScenarios)
        target_identity = $profile.TargetIdentity
    }
}

Export-ModuleMember -Function Get-DispatchSessionRedesignM3Profile, Get-DispatchCleanupM3Profile, Get-PlaybackCoreLifecycleProfile, Get-PlaybackCoreLifecycleProfileForName, Test-DispatchGitAncestor, Assert-PlaybackCoreLifecycleV3BaselineSuccessor, Assert-PlaybackCoreLifecycleV3CandidateLineage, Assert-PlaybackCoreLifecyclePairLineage, Assert-PlaybackCoreLifecycleV3LockfileBytes, Get-DispatchM3PathIdentity, Test-DispatchM3PathOutside, Get-DispatchM3GitCheckoutIdentity, Assert-DispatchCleanupM3SourceCheckout, Get-DispatchCleanupM3RunnerIdentity, Get-DispatchSessionRedesignM3RunnerIdentity, Assert-DispatchSessionRedesignM3SourceCheckout, Get-PlaybackCoreLifecycleRunnerIdentity, Assert-PlaybackCoreLifecycleSourceCheckout, Get-CompletionAnchoredPulseV1Profile, Assert-CompletionAnchoredPulseV1LockfileBytes, Assert-CompletionAnchoredPulseV1Lineage, New-CompletionAnchoredPulseV1NonphysicalPlan
