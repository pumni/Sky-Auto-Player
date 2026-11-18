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
    [pscustomobject]@{
        Name = 'PlaybackCoreLifecycle'
        WorkloadVersion = 'playback-core-lifecycle-v1'
        HarnessManifestSchemaVersion = 1
        BaselineRevision = '2544daa13eb23fdf03bd922e65b96fcffaba421c'
        BaselineTree = '155882714df131ddc9b9985133a1f0611736b7cd'
        CandidateLineageBranch = 'refactor/playback-core-lifecycle'
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
            'stop-to-up-to-verified-release-us',
            'worker-cpu-us',
            'worker-spin-us',
            'raw-up-call-count',
            'worker-joined-to-next-start-us'
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

function Get-PlaybackCoreLifecycleRunnerIdentity {
    param([Parameter(Mandatory)] [string]$RunnerRoot)
    $profile = Get-PlaybackCoreLifecycleProfile
    $identity = Get-DispatchM3GitCheckoutIdentity $RunnerRoot
    if (-not $identity.clean -or $identity.checkout.top_level -ne $identity.root -or
        $identity.branch -ne $profile.CandidateLineageBranch) {
        throw 'PlaybackCoreLifecycle runner must be a clean checkout on refactor/playback-core-lifecycle'
    }
    $remoteLine = @(& git -C $identity.root ls-remote origin ('refs/heads/' + $profile.CandidateLineageBranch) 2>&1)
    $remoteExit = $LASTEXITCODE
    if ($remoteExit -ne 0) { throw ('could not verify PlaybackCoreLifecycle candidate against origin: ' + ($remoteLine -join ' ')) }
    $remoteRows = @($remoteLine | ForEach-Object { [string]$_ } | Where-Object { $_ -match ('^([0-9a-f]{40})\s+refs/heads/' + [regex]::Escape($profile.CandidateLineageBranch) + '$') })
    if ($remoteRows.Count -ne 1) { throw 'origin does not advertise exactly one PlaybackCoreLifecycle candidate branch tip' }
    $lineageTip = [regex]::Match($remoteRows[0], '^([0-9a-f]{40})').Groups[1].Value
    if ($identity.revision -ne $lineageTip) { throw 'PlaybackCoreLifecycle runner must match the pushed candidate branch tip' }
    & git -C $identity.root merge-base --is-ancestor $profile.BaselineRevision $lineageTip
    if ($LASTEXITCODE -ne 0) { throw 'PlaybackCoreLifecycle candidate is not descended from the pinned redesign baseline' }
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

function Assert-PlaybackCoreLifecycleSourceCheckout {
    param(
        [Parameter(Mandatory)] [string]$SourceRoot,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$SourceRole,
        [Parameter(Mandatory)] [string]$RunnerRoot
    )
    $profile = Get-PlaybackCoreLifecycleProfile
    $identity = Get-DispatchM3GitCheckoutIdentity $SourceRoot
    $runnerIdentity = Get-PlaybackCoreLifecycleRunnerIdentity -RunnerRoot $RunnerRoot
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

Export-ModuleMember -Function Get-DispatchSessionRedesignM3Profile, Get-DispatchCleanupM3Profile, Get-PlaybackCoreLifecycleProfile, Get-DispatchM3PathIdentity, Test-DispatchM3PathOutside, Get-DispatchM3GitCheckoutIdentity, Assert-DispatchCleanupM3SourceCheckout, Get-DispatchCleanupM3RunnerIdentity, Get-DispatchSessionRedesignM3RunnerIdentity, Assert-DispatchSessionRedesignM3SourceCheckout, Get-PlaybackCoreLifecycleRunnerIdentity, Assert-PlaybackCoreLifecycleSourceCheckout
