Set-StrictMode -Version Latest

$script:c1SourceRevision = 'b52106b47e4163d0906abbaa59d343935f4fba42'
$script:c1SourceTree = '0e1933cc565ad8f9b42239312c996facf2a5126a'
$script:c1AllowedSourcePath = 'rust/crates/sky_player/src/bin/rt_native_acceptance.rs'
$script:c1ArchiveSha256 = '423231b0de048bd5ac86de855e184159f7d45df5b6401738842d6af5e4935536'
$script:c1OverlaySha256 = '7029fd6889ce1c0955f7cb8e911c7350a2a05ccfb13ad34b397ab0cf829287b6'
$script:c1DerivedSourceSha256 = '82580ef5c4d5b8089e15511f6f8f4663a1ac5d8cd1a42553cadc63d7f5160cbd'
$script:c1ChangesSha256 = '92afb7eef1c7011d4e61382aef9f89086bbecef198190ef7f84d49a2c6f8f329'
$script:c1InventorySha256 = 'de73297399584506a65c9be91c3c488d37fde0759b40baf1d4121e912a20d6f5'

function Get-ManifestProperty {
    param($Manifest, [string]$Name)
    $property = $Manifest.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    $property.Value
}

function Get-FileSha256 {
    param([string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-FrameSnapshotManifestIdentity {
    param(
        [Parameter(Mandatory)] $Manifest,
        [Parameter(Mandatory)] [ValidateSet('Baseline', 'Candidate')] [string]$ExpectedRole
    )

    $expectedBranch = if ($ExpectedRole -eq 'Baseline') { '' } else { 'refactor/frame-snapshot-dispatch' }
    $sourceRevision = [string](Get-ManifestProperty $Manifest 'source_revision')
    $sourceTree = [string](Get-ManifestProperty $Manifest 'source_tree_sha')
    $binarySha = [string](Get-ManifestProperty $Manifest 'binary_sha256')
    if ((Get-ManifestProperty $Manifest 'qualification_profile') -ne 'FrameSnapshot' -or
        (Get-ManifestProperty $Manifest 'source_role') -ne $ExpectedRole -or
        (Get-ManifestProperty $Manifest 'source_branch') -ne $expectedBranch -or
        -not [bool](Get-ManifestProperty $Manifest 'source_tree_clean') -or
        -not [bool](Get-ManifestProperty $Manifest 'clean_build') -or
        (Get-ManifestProperty $Manifest 'base_source_revision') -ne '8a800ff07a5dc362a5ee3aaa59204ff40b6ae327' -or
        (Get-ManifestProperty $Manifest 'base_source_tree_sha') -ne '48eec424c155e98fa1a81ad018bc832b6e6b964f' -or
        (Get-ManifestProperty $Manifest 'workload_version') -ne 'frame-snapshot-v1' -or
        (Get-ManifestProperty $Manifest 'receiveonly_observer_mode') -ne 'SchedulerQueuedV3Compat3' -or
        (Get-ManifestProperty $Manifest 'receiveonly_observer_version') -ne 'scheduler-receiveonly-queue-v3-compat3' -or
        (Get-ManifestProperty $Manifest 'performance_timing_margin_us') -ne 0 -or
        (Get-ManifestProperty $Manifest 'safety_timing_margin_us') -ne 500 -or
        (Get-ManifestProperty $Manifest 'target') -ne 'x86_64-pc-windows-msvc' -or
        $sourceRevision -notmatch '^[0-9a-f]{40}$' -or
        $sourceTree -notmatch '^[0-9a-f]{40}$' -or
        $binarySha -notmatch '^[0-9a-f]{64}$') {
        throw ('FrameSnapshot ' + $ExpectedRole + ' manifest identity, role, branch, base, hashes, workload, observer, or timing margins are invalid')
    }

    if ($ExpectedRole -eq 'Baseline' -and
        ($sourceRevision -ne '8a800ff07a5dc362a5ee3aaa59204ff40b6ae327' -or
            $sourceTree -ne '48eec424c155e98fa1a81ad018bc832b6e6b964f')) {
        throw 'FrameSnapshot baseline must be the exact pinned source SHA and tree'
    }
}

function Assert-ImmutableHarnessManifestIntegrity {
    param(
        [Parameter(Mandatory)] $Manifest,
        [Parameter(Mandatory)] [string]$ManifestPath
    )

    $manifestFullPath = [System.IO.Path]::GetFullPath($ManifestPath)
    $manifestRoot = [System.IO.Path]::GetDirectoryName($manifestFullPath)
    $expectedManifestPath = [System.IO.Path]::GetFullPath((Join-Path $manifestRoot 'manifest.json'))
    if (-not [string]::Equals($manifestFullPath, $expectedManifestPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'immutable harness manifest must be named manifest.json in its evidence directory'
    }
    $expectedBinaryPath = [System.IO.Path]::GetFullPath((Join-Path $manifestRoot ([string](Get-ManifestProperty $Manifest 'binary_name'))))
    $actualBinaryPath = [System.IO.Path]::GetFullPath([string](Get-ManifestProperty $Manifest 'binary_path'))
    if (-not [string]::Equals($actualBinaryPath, $expectedBinaryPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'immutable harness binary path must be the manifest sibling named by binary_name'
    }

    $checksumPath = Join-Path $manifestRoot 'SHA256SUMS.txt'
    if (-not (Test-Path -LiteralPath $checksumPath -PathType Leaf)) { throw 'immutable harness SHA256SUMS.txt is missing' }
    $expected = @{
        'manifest.json' = Get-FileSha256 $manifestFullPath
        ([string](Get-ManifestProperty $Manifest 'binary_name')) = Get-FileSha256 $actualBinaryPath
        'build_dispatch_core_harness.ps1' = Get-FileSha256 (Join-Path $manifestRoot 'build_dispatch_core_harness.ps1')
        'build.log' = Get-FileSha256 (Join-Path $manifestRoot 'build.log')
        'clean.log' = Get-FileSha256 (Join-Path $manifestRoot 'clean.log')
    }
    if ((Get-FileSha256 (Join-Path $manifestRoot 'build_dispatch_core_harness.ps1')) -ne
        [string](Get-ManifestProperty $Manifest 'builder_script_sha256')) {
        throw 'immutable harness builder copy does not match its manifest source hash'
    }
    $actual = @{}
    foreach ($line in Get-Content -LiteralPath $checksumPath) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw 'immutable harness checksum file contains an invalid row' }
        $name = $Matches[2]
        if ($actual.ContainsKey($name)) { throw ('immutable harness checksum file repeats ' + $name) }
        $actual[$name] = $Matches[1]
    }
    if ($actual.Count -ne $expected.Count) { throw 'immutable harness checksum inventory is incomplete or contains unexpected files' }
    foreach ($name in $expected.Keys) {
        if (-not $actual.ContainsKey($name) -or $actual[$name] -ne $expected[$name]) {
            throw ('immutable harness checksum mismatch for ' + $name)
        }
    }
    if ((Get-FileSha256 $actualBinaryPath) -ne [string](Get-ManifestProperty $Manifest 'binary_sha256')) {
        throw 'immutable harness binary hash differs from manifest binary_sha256'
    }
    $true
}

function Resolve-OutsideRepositoryFile {
    param([string]$Path, [string]$RepositoryRoot, [string]$Label)
    if ([string]::IsNullOrWhiteSpace($Path) -or -not [System.IO.Path]::IsPathRooted($Path)) {
        throw ($Label + ' must be an absolute path')
    }
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $rootPrefix = [System.IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\') + '\'
    if ($fullPath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase) -or
        [string]::Equals($fullPath.TrimEnd('\'), $rootPrefix.TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw ($Label + ' must remain outside the source checkout')
    }
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
        throw ($Label + ' is missing')
    }
    $fullPath
}

function Assert-C1TimedControlProvenance {
    param(
        [Parameter(Mandatory)] $Manifest,
        [Parameter(Mandatory)] [string]$RepositoryRoot,
        [Parameter(Mandatory)] [ValidateSet('candidate', 'control', 'case-runner')] [string]$Role,
        [switch]$AllowHistoricalControl
    )

    $controlKind = Get-ManifestProperty $Manifest 'control_kind'
    if ($null -eq $controlKind) { return $false }
    if ($controlKind -ne 'c1-timed-harness-overlay') {
        throw 'unsupported immutable harness control_kind'
    }
    if ($Role -eq 'candidate') { throw 'C1-TIMED overlay manifests are accepted only as controls' }
    if ($Role -eq 'case-runner' -and -not $AllowHistoricalControl) {
        throw 'C1-TIMED requires the explicit historical-control invocation'
    }

    $requiredIdentity = @{
        control_label = 'C1-TIMED'
        source_revision = $script:c1SourceRevision
        source_tree_sha = $script:c1SourceTree
        source_branch = 'perf/dispatch-core-20261109'
        base_source_revision = $script:c1SourceRevision
        base_source_tree_sha = $script:c1SourceTree
        source_path_allowlist = @($script:c1AllowedSourcePath)
    }
    foreach ($entry in $requiredIdentity.GetEnumerator()) {
        $actual = Get-ManifestProperty $Manifest $entry.Key
        if ($entry.Key -eq 'source_path_allowlist') {
            if (@($actual).Count -ne 1 -or @($actual)[0] -ne $script:c1AllowedSourcePath) {
                throw 'C1-TIMED source path allowlist must contain only the acceptance harness'
            }
        } elseif ($actual -ne $entry.Value) {
            throw ('C1-TIMED manifest has invalid ' + $entry.Key)
        }
    }
    if ([bool](Get-ManifestProperty $Manifest 'source_tree_clean') -or
        -not [bool](Get-ManifestProperty $Manifest 'derived_source_tree_clean') -or
        -not [bool](Get-ManifestProperty $Manifest 'clean_build') -or
        -not [bool](Get-ManifestProperty $Manifest 'control_timing_eligible') -or
        [bool](Get-ManifestProperty $Manifest 'control_os_termination_qualified') -or
        [bool](Get-ManifestProperty $Manifest 'c0_timing_eligible')) {
        throw 'C1-TIMED source cleanliness, build, or limited-coverage identity is invalid'
    }

    $archivePath = Resolve-OutsideRepositoryFile (Get-ManifestProperty $Manifest 'base_source_archive_path') $RepositoryRoot 'C1-TIMED base source archive'
    $patchPath = Resolve-OutsideRepositoryFile (Get-ManifestProperty $Manifest 'overlay_patch_path') $RepositoryRoot 'C1-TIMED overlay patch'
    $baseFilePath = Resolve-OutsideRepositoryFile (Get-ManifestProperty $Manifest 'base_source_file_path') $RepositoryRoot 'C1-TIMED base harness source'
    $derivedFilePath = Resolve-OutsideRepositoryFile (Get-ManifestProperty $Manifest 'derived_source_file_path') $RepositoryRoot 'C1-TIMED derived harness source'
    $changesPath = Resolve-OutsideRepositoryFile (Get-ManifestProperty $Manifest 'source_changes_path') $RepositoryRoot 'C1-TIMED source change list'
    $inventoryPath = Resolve-OutsideRepositoryFile (Get-ManifestProperty $Manifest 'derived_source_inventory_path') $RepositoryRoot 'C1-TIMED derived source inventory'

    $expectedHashes = @(
        @{ Name = 'base_source_archive_sha256'; Path = $archivePath },
        @{ Name = 'overlay_sha256'; Path = $patchPath },
        @{ Name = 'base_source_file_sha256'; Path = $baseFilePath },
        @{ Name = 'derived_source_file_sha256'; Path = $derivedFilePath },
        @{ Name = 'source_changes_sha256'; Path = $changesPath },
        @{ Name = 'derived_source_inventory_sha256'; Path = $inventoryPath }
    )
    foreach ($entry in $expectedHashes) {
        $expected = [string](Get-ManifestProperty $Manifest $entry.Name)
        if ($expected -notmatch '^[0-9a-f]{64}$' -or (Get-FileSha256 $entry.Path) -ne $expected) {
            throw ('C1-TIMED ' + $entry.Name + ' is missing or mismatched')
        }
    }

    if ((Get-ManifestProperty $Manifest 'base_source_archive_sha256') -ne $script:c1ArchiveSha256 -or
        (Get-ManifestProperty $Manifest 'overlay_sha256') -ne $script:c1OverlaySha256 -or
        (Get-ManifestProperty $Manifest 'derived_source_file_sha256') -ne $script:c1DerivedSourceSha256 -or
        (Get-ManifestProperty $Manifest 'source_changes_sha256') -ne $script:c1ChangesSha256 -or
        (Get-ManifestProperty $Manifest 'derived_source_inventory_sha256') -ne $script:c1InventorySha256) {
        throw 'C1-TIMED archive, overlay, derived source, or source inventory differs from the approved control'
    }
    $baseBlob = (& git -C $RepositoryRoot rev-parse ($script:c1SourceRevision + ':' + $script:c1AllowedSourcePath)).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve the frozen C0 harness source blob' }
    $providedBaseBlob = (& git -C $RepositoryRoot hash-object -- $baseFilePath).Trim()
    if ($LASTEXITCODE -ne 0 -or $providedBaseBlob -ne $baseBlob) {
        throw 'C1-TIMED base harness source does not match the frozen C0 A1 blob'
    }

    $patchText = [System.IO.File]::ReadAllText($patchPath)
    $patchHeaders = [regex]::Matches($patchText, '(?m)^diff --git a/(.+?) b/(.+?)$')
    if ($patchHeaders.Count -ne 1 -or
        $patchHeaders[0].Groups[1].Value -ne $script:c1AllowedSourcePath -or
        $patchHeaders[0].Groups[2].Value -ne $script:c1AllowedSourcePath -or
        -not $patchText.Contains('needs_supervisor_progress_heartbeat(args.scenario, args.sample_seconds)')) {
        throw 'C1-TIMED overlay must be a single-file timed-heartbeat patch'
    }
    $changedPaths = @(Get-Content -LiteralPath $changesPath | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($changedPaths.Count -ne 1 -or $changedPaths[0] -ne $script:c1AllowedSourcePath) {
        throw 'C1-TIMED derived source changed paths exceed the allowlist'
    }
    $derivedText = [System.IO.File]::ReadAllText($derivedFilePath)
    if (-not $derivedText.Contains('needs_supervisor_progress_heartbeat(args.scenario, args.sample_seconds)')) {
        throw 'C1-TIMED derived harness source does not contain the declared heartbeat patch'
    }

    $true
}

Export-ModuleMember -Function Get-ManifestProperty, Assert-C1TimedControlProvenance, Assert-FrameSnapshotManifestIdentity, Assert-ImmutableHarnessManifestIntegrity
