param(
    [Parameter(Mandatory)]
    [string]$OutputRoot,
    [ValidateSet('frozen')]
    [string]$GuardMode = 'frozen',
    [switch]$QpcReadCounting,
    [ValidateSet('DispatchCore', 'Scheduler', 'FrameSnapshot', 'DispatchCleanupM3')]
    [string]$QualificationProfile = 'DispatchCore',
    [ValidateSet('Baseline', 'Candidate')]
    [string]$SourceRole,
    [string]$SourceRoot
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Import-Module (Join-Path $PSScriptRoot 'dispatch_harness_manifest.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'dispatch_qualification_profiles.psm1') -Force
$targetTriple = 'x86_64-pc-windows-msvc'
$profile = 'dist'
$isDispatchCleanupM3 = $QualificationProfile -eq 'DispatchCleanupM3'
$dispatchCleanupM3 = if ($isDispatchCleanupM3) { Get-DispatchCleanupM3Profile } else { $null }
$sourceRootPath = if ($SourceRoot) { [System.IO.Path]::GetFullPath($SourceRoot) } else { $root }
$rustRoot = Join-Path $sourceRootPath 'rust'
$features = [System.Collections.Generic.List[string]]::new()
$features.Add('real-input-acceptance')
if ($QpcReadCounting) { $features.Add('qpc-read-counting') }
$featureArgument = $features -join ','
$qualifiedProfile = $QualificationProfile -in @('Scheduler', 'FrameSnapshot', 'DispatchCleanupM3')
$workloadVersion = if ($isDispatchCleanupM3) { $dispatchCleanupM3.WorkloadVersion } elseif ($QualificationProfile -eq 'FrameSnapshot') { 'frame-snapshot-v1' } elseif ($QualificationProfile -eq 'Scheduler') { 'scheduler-single-owner-v1' } else { 'dispatch-core-646-a1' }
$baseRevision = if ($isDispatchCleanupM3) { $dispatchCleanupM3.BaselineRevision } elseif ($QualificationProfile -eq 'FrameSnapshot') { '8a800ff07a5dc362a5ee3aaa59204ff40b6ae327' } elseif ($QualificationProfile -eq 'Scheduler') { 'b9ed666e3ddddc4952831626f1654e1cc9071a10' } else { '843cc0c6a1392dfff0ca4b017551bd1ffb180579' }
$baseTree = if ($isDispatchCleanupM3) { $dispatchCleanupM3.BaselineTree } elseif ($QualificationProfile -eq 'FrameSnapshot') { '48eec424c155e98fa1a81ad018bc832b6e6b964f' } elseif ($QualificationProfile -eq 'Scheduler') { 'f554c41b23a3969d02ed29842d4178f6143c9d86' } else { $null }
$expectedBranch = if ($isDispatchCleanupM3) { $dispatchCleanupM3.CandidateLineageBranch } elseif ($QualificationProfile -eq 'FrameSnapshot') { 'refactor/frame-snapshot-dispatch' } elseif ($QualificationProfile -eq 'Scheduler') { 'refactor/scheduler-single-owner-20261112' } else { 'perf/dispatch-core-20261109' }
$performanceTimingMarginUs = if ($isDispatchCleanupM3 -or $QualificationProfile -eq 'FrameSnapshot') { 0 } else { 500 }
$safetyTimingMarginUs = 500
$receiveOnlyObserverMode = if ($qualifiedProfile) { 'SchedulerQueuedV3Compat3' } else { 'LegacyInline' }
$receiveOnlyObserverVersion = if ($qualifiedProfile) { 'scheduler-receiveonly-queue-v3-compat3' } else { $null }
$destination = [System.IO.Path]::GetFullPath($OutputRoot)
$cargoTargetRoot = if ($isDispatchCleanupM3) { Join-Path $destination 'cargo-target' } else { $null }
$runnerIdentity = $null

if ($isDispatchCleanupM3 -and $QpcReadCounting) {
    throw 'DispatchCleanupM3 does not permit qpc-read-counting outside a separately approved evidence protocol'
}

if ($qualifiedProfile) {
    if (-not $SourceRoot -or -not $SourceRole) {
        throw ($QualificationProfile + ' builds require -SourceRoot and -SourceRole Baseline|Candidate')
    }
} elseif ($SourceRoot -or $SourceRole) {
    throw '-SourceRoot and -SourceRole are available only with a qualified profile'
}

function Get-Sha256 {
    param([string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-SourceIdentity {
    if ($isDispatchCleanupM3) {
        $identity = Assert-DispatchCleanupM3SourceCheckout -SourceRoot $sourceRootPath -SourceRole $SourceRole -RunnerRoot $root
        return [ordered]@{
            revision = $identity.revision; tree = $identity.tree; branch = $identity.branch; root = $identity.root
            checkout = $identity.checkout; clean = $identity.clean
        }
    }
    $revision = (& git -C $sourceRootPath rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source revision' }
    $tree = (& git -C $sourceRootPath rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source tree' }
    $branch = ((& git -C $sourceRootPath branch --show-current) -join '').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source branch' }
    $changes = (& git -C $sourceRootPath status --porcelain --untracked-files=all) -join ''
    if ($LASTEXITCODE -ne 0) { throw 'could not verify source tree cleanliness' }
    if (-not [string]::IsNullOrWhiteSpace($changes)) { throw 'immutable harness builds require a clean source checkout' }
    if ($qualifiedProfile -and $SourceRole -eq 'Baseline') {
        if ($revision -ne $baseRevision -or $tree -ne $baseTree -or -not [string]::IsNullOrWhiteSpace($branch)) {
            throw ($QualificationProfile + ' baseline must be a detached checkout at the pinned base revision')
        }
    } else {
        if ($branch -ne $expectedBranch) { throw ('expected branch ' + $expectedBranch + ', got ' + $branch) }
        & git -C $sourceRootPath merge-base --is-ancestor $baseRevision HEAD
        if ($LASTEXITCODE -ne 0) { throw ('source revision is not descended from ' + $baseRevision) }
    }
    [ordered]@{ revision = $revision; tree = $tree; branch = $branch; root = $sourceRootPath }
}

function ConvertTo-DispatchCleanupM3CheckoutManifest {
    param($Identity)
    [ordered]@{
        root = $Identity.root
        top_level = $Identity.checkout.top_level
        git_dir = $Identity.checkout.git_dir
        git_common_dir = $Identity.checkout.git_common_dir
        revision = $Identity.revision
        tree = $Identity.tree
        branch = $Identity.branch
        clean = $Identity.clean
    }
}

function Assert-PathOutsideRepository {
    param([string]$Path)
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    foreach ($repositoryRoot in @($root, $sourceRootPath) | Select-Object -Unique) {
        $fullRoot = [System.IO.Path]::GetFullPath($repositoryRoot).TrimEnd('\')
        $rootPrefix = $fullRoot + '\'
        if ([string]::Equals($fullPath.TrimEnd('\'), $fullRoot, [System.StringComparison]::OrdinalIgnoreCase) -or
            $fullPath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw 'output root must be outside the source checkout'
        }
    }
}

Assert-PathOutsideRepository $destination
if (Test-Path -LiteralPath $destination) {
    if ((Get-ChildItem -LiteralPath $destination -Force | Measure-Object).Count -ne 0) {
        throw 'output root must be absent or empty'
    }
} else {
    New-Item -ItemType Directory -Path $destination | Out-Null
}

$runnerIdentity = if ($isDispatchCleanupM3) { Get-DispatchCleanupM3RunnerIdentity -RunnerRoot $root } else { $null }
$priorCargoTargetDir = $env:CARGO_TARGET_DIR
$source = $null
$rustcVersion = $null
$cargoVersion = $null
$cleanExit = 1
$buildExit = 1
$cleanOutput = @()
$buildOutput = @()
$sourceManifest = Join-Path $rustRoot 'Cargo.toml'
$cargoCleanCommand = 'cargo clean --manifest-path "' + $sourceManifest + '" --profile ' + $profile + ' --target ' + $targetTriple
$cargoBuildCommand = 'cargo build --locked --manifest-path "' + $sourceManifest + '" -p sky_player --profile ' + $profile + ' --target ' + $targetTriple + ' --features ' + $featureArgument + ' --bin rt-native-acceptance'
$buildLog = Join-Path $destination 'build.log'

if ($isDispatchCleanupM3) {
    $env:CARGO_TARGET_DIR = $cargoTargetRoot
    Push-Location -LiteralPath $rustRoot
}
try {
    $source = Get-SourceIdentity
    $rustcVersion = (& rustc -Vv) -join [Environment]::NewLine
    if ($LASTEXITCODE -ne 0) { throw 'could not identify active rustc version' }
    $cargoVersion = (& cargo --version).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not identify active cargo version' }
    if ($isDispatchCleanupM3 -and
        ($rustcVersion -notmatch '(?m)^release: 1\.99\.1\r?$' -or
            $rustcVersion -notmatch '(?m)^host: x86_64-pc-windows-msvc\r?$' -or
            $cargoVersion -notmatch '^cargo 1\.99\.1(?:\s|$)')) {
        throw 'DispatchCleanupM3 requires Rust/Cargo 1.99.1 for the Windows MSVC host'
    }

    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $cleanOutput = @(& cargo clean --manifest-path (Join-Path $rustRoot 'Cargo.toml') --profile $profile --target $targetTriple 2>&1)
    $cleanExit = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    $cleanOutput | Set-Content -LiteralPath (Join-Path $destination 'clean.log') -Encoding UTF8
    if ($cleanExit -ne 0) { throw ('cargo clean failed with exit code ' + $cleanExit) }

    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $buildOutput = @(& cargo build --locked --manifest-path (Join-Path $rustRoot 'Cargo.toml') -p sky_player --profile $profile --target $targetTriple --features $featureArgument --bin rt-native-acceptance 2>&1)
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    $buildOutput | Set-Content -LiteralPath $buildLog -Encoding UTF8
    if ($buildExit -ne 0) { throw ('cargo build failed with exit code ' + $buildExit) }
} finally {
    if ($isDispatchCleanupM3) {
        Pop-Location
        $env:CARGO_TARGET_DIR = $priorCargoTargetDir
    }
}

$sourceAfterBuild = Get-SourceIdentity
if ($sourceAfterBuild.revision -ne $source.revision -or $sourceAfterBuild.tree -ne $source.tree) {
    throw 'source identity changed during immutable harness build'
}
$builtBinary = if ($isDispatchCleanupM3) { Join-Path $cargoTargetRoot ($targetTriple + '\dist\rt-native-acceptance.exe') } else { Join-Path $rustRoot ('target\' + $targetTriple + '\dist\rt-native-acceptance.exe') }
if (-not (Test-Path -LiteralPath $builtBinary -PathType Leaf)) { throw 'cargo build did not produce the expected Windows dist harness' }
$binaryPath = Join-Path $destination 'rt-native-acceptance.exe'
Copy-Item -LiteralPath $builtBinary -Destination $binaryPath
$binaryHash = Get-Sha256 $binaryPath
$builderHash = Get-Sha256 $PSCommandPath
$builderCopy = Join-Path $destination 'build_dispatch_core_harness.ps1'
Copy-Item -LiteralPath $PSCommandPath -Destination $builderCopy
$runnerScriptProvenance = @()
if ($isDispatchCleanupM3) {
    $runnerIdentityAfterBuild = Get-DispatchCleanupM3RunnerIdentity -RunnerRoot $root
    if ($runnerIdentityAfterBuild.revision -ne $runnerIdentity.revision -or $runnerIdentityAfterBuild.tree -ne $runnerIdentity.tree) {
        throw 'runner source identity changed during immutable harness build'
    }
    $provenanceRoot = Join-Path $destination 'runner-provenance'
    $runnerPaths = @($dispatchCleanupM3.RunnerProvenancePaths)
    foreach ($relativePath in $runnerPaths) {
        $runnerFile = Join-Path $root ($relativePath.Replace('/', '\'))
        if (-not (Test-Path -LiteralPath $runnerFile -PathType Leaf)) { throw ('runner provenance source is missing: ' + $relativePath) }
        $evidenceRelativePath = 'runner-provenance/' + $relativePath.Replace('/', '\')
        $evidenceFile = Join-Path $destination ($evidenceRelativePath.Replace('/', '\'))
        $evidenceDirectory = Split-Path -Parent $evidenceFile
        New-Item -ItemType Directory -Force -Path $evidenceDirectory | Out-Null
        Copy-Item -LiteralPath $runnerFile -Destination $evidenceFile
        $sourceHash = Get-Sha256 $runnerFile
        if ((Get-Sha256 $evidenceFile) -ne $sourceHash) { throw ('runner provenance copy changed during build: ' + $relativePath) }
        $runnerScriptProvenance += [ordered]@{
            source_path = $relativePath
            evidence_path = $evidenceRelativePath
            sha256 = $sourceHash
        }
    }
}
$manifest = [ordered]@{
    schema_version = 1
    binary_name = 'rt-native-acceptance.exe'
    binary_path = $binaryPath
    binary_sha256 = $binaryHash
    source_revision = $source.revision
    source_tree_sha = $source.tree
    source_branch = $source.branch
    qualification_profile = $QualificationProfile
    source_role = if ($qualifiedProfile) { $SourceRole } else { $null }
    source_root = $source.root
    base_source_revision = if ($qualifiedProfile) { $baseRevision } else { $null }
    base_source_tree_sha = if ($qualifiedProfile) { $baseTree } else { $null }
    source_tree_clean = $true
    clean_build = $true
    cargo_profile = $profile
    cargo_features = $features.ToArray()
    qpc_read_counting = [bool]$QpcReadCounting
    observer_mode = 'qualification-on'
    receiveonly_observer_mode = $receiveOnlyObserverMode
    receiveonly_observer_version = $receiveOnlyObserverVersion
    guard_mode = $GuardMode
    workload_version = $workloadVersion
    performance_timing_margin_us = $performanceTimingMarginUs
    safety_timing_margin_us = $safetyTimingMarginUs
    target = $targetTriple
    rustc_version = $rustcVersion.Trim()
    cargo_version = $cargoVersion
    cargo_clean_command = $cargoCleanCommand
    cargo_clean_exit_code = $cleanExit
    cargo_build_command = $cargoBuildCommand
    cargo_build_exit_code = $buildExit
    builder_script_path = $PSCommandPath
    builder_script_sha256 = $builderHash
    built_utc = [DateTimeOffset]::UtcNow.ToString('O')
}
$manifestPath = Join-Path $destination 'manifest.json'
if ($isDispatchCleanupM3) {
    $manifest['baseline_source_revision'] = $dispatchCleanupM3.BaselineRevision
    $manifest['baseline_source_tree_sha'] = $dispatchCleanupM3.BaselineTree
    $manifest['candidate_source_revision'] = $dispatchCleanupM3.CandidateRevision
    $manifest['candidate_source_tree_sha'] = $dispatchCleanupM3.CandidateTree
    $manifest['source_checkout_identity'] = ConvertTo-DispatchCleanupM3CheckoutManifest $source
    $manifest['output_root'] = $destination
    $manifest['cargo_target_dir'] = $cargoTargetRoot
    $manifest['target_identity'] = $targetTriple
    $manifest['receiveonly_observer_schema_version'] = $dispatchCleanupM3.ObserverSchemaVersion
    $manifest['receiveonly_observer_event_schema_version'] = $dispatchCleanupM3.ObserverEventSchemaVersion
    $manifest['runner_source_revision'] = $runnerIdentity.revision
    $manifest['runner_source_tree_sha'] = $runnerIdentity.tree
    $manifest['runner_source_branch'] = $runnerIdentity.branch
    $manifest['runner_source_root'] = $runnerIdentity.root
    $manifest['runner_source_clean'] = $runnerIdentity.clean
    $manifest['runner_source_checkout_identity'] = ConvertTo-DispatchCleanupM3CheckoutManifest $runnerIdentity
    $manifest['candidate_lineage_branch'] = $runnerIdentity.candidate_lineage_branch
    $manifest['candidate_lineage_tip_revision'] = $runnerIdentity.candidate_lineage_tip_revision
    $manifest['candidate_lineage_verified'] = $runnerIdentity.candidate_lineage_verified
    $manifest['runner_script_provenance'] = $runnerScriptProvenance
    $manifest['manifest_path'] = $manifestPath
    Assert-DispatchCleanupM3ManifestIdentity -Manifest $manifest -ExpectedRole $SourceRole -RepositoryRoot $root | Out-Null
}
$manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
$manifestHash = Get-Sha256 $manifestPath
$checksumLines = [System.Collections.Generic.List[string]]::new()
$checksumLines.Add($binaryHash + '  rt-native-acceptance.exe')
$checksumLines.Add($manifestHash + '  manifest.json')
$checksumLines.Add($builderHash + '  build_dispatch_core_harness.ps1')
$checksumLines.Add((Get-Sha256 $buildLog) + '  build.log')
$checksumLines.Add((Get-Sha256 (Join-Path $destination 'clean.log')) + '  clean.log')
foreach ($entry in $runnerScriptProvenance) {
    $evidenceFile = Join-Path $destination ($entry.evidence_path.Replace('/', '\'))
    $checksumLines.Add(([string]$entry.sha256 + '  ' + [string]$entry.evidence_path.Replace('\', '/')))
}
Set-Content -LiteralPath (Join-Path $destination 'SHA256SUMS.txt') -Value $checksumLines.ToArray() -Encoding UTF8
$null = Assert-ImmutableHarnessManifestIntegrity -Manifest ($manifest | ConvertTo-Json -Depth 8 | ConvertFrom-Json) -ManifestPath $manifestPath
Write-Output ('SOURCE ' + $source.revision + ' tree=' + $source.tree)
if ($isDispatchCleanupM3) {
    Write-Output ('RUNNER ' + $runnerIdentity.revision + ' tree=' + $runnerIdentity.tree + ' branch=' + $runnerIdentity.branch + ' candidate-lineage=' + $runnerIdentity.candidate_lineage_tip_revision)
    Write-Output ('ROLE ' + $SourceRole + ' profile=' + $QualificationProfile + ' workload=' + $workloadVersion + ' performance-margin-us=' + $performanceTimingMarginUs + ' safety-margin-us=' + $safetyTimingMarginUs)
    Write-Output ('CARGO_TARGET_DIR ' + $cargoTargetRoot)
}
Write-Output ('BINARY ' + $binaryPath + ' sha256=' + $binaryHash)
Write-Output ('MANIFEST ' + $manifestPath + ' sha256=' + $manifestHash)
