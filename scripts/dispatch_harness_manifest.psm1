Set-StrictMode -Version Latest

$script:c1SourceRevision = 'b52106b47e4163d0906abbaa59d343935f4fba42'
$script:c1SourceTree = '0e1933cc565ad8f9b42239312c996facf2a5126a'
$script:c1AllowedSourcePath = 'rust/crates/sky_player/src/bin/rt_native_acceptance.rs'
$script:c1ArchiveSha256 = '423231b0de048bd5ac86de855e184159f7d45df5b6401738842d6af5e4935536'
$script:c1OverlaySha256 = '62490e0d3664b3434c3402e96099206336f255c15d006657fcd6b43f43de32bf'
$script:c1DerivedSourceSha256 = '527ea2358e9babe1307e8e996891633fa229dbbbdc20c147825bb0185127b897'
$script:c1ChangesSha256 = '92afb7eef1c7011d4e61382aef9f89086bbecef198190ef7f84d49a2c6f8f329'
$script:c1InventorySha256 = '7722ba7ff3a32a0560a176249e64160c8a9c2188a5747c7bed96793017218bdc'

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

Export-ModuleMember -Function Get-ManifestProperty, Assert-C1TimedControlProvenance
