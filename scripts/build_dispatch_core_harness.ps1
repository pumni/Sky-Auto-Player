param(
    [Parameter(Mandatory)]
    [string]$OutputRoot,
    [ValidateSet('frozen', 'adaptive')]
    [string]$GuardMode = 'frozen',
    [switch]$QpcReadCounting
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$rustRoot = Join-Path $root 'rust'
$targetTriple = 'x86_64-pc-windows-msvc'
$profile = 'dist'
$features = [System.Collections.Generic.List[string]]::new()
$features.Add('real-input-acceptance')
if ($QpcReadCounting) { $features.Add('qpc-read-counting') }
$featureArgument = $features -join ','
$workloadVersion = 'dispatch-core-646-a1'
$baseRevision = '843cc0c6a1392dfff0ca4b017551bd1ffb180579'
$expectedBranch = 'perf/dispatch-core-20261109'
$destination = [System.IO.Path]::GetFullPath($OutputRoot)

function Get-Sha256 {
    param([string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-SourceIdentity {
    $revision = (& git -C $root rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source revision' }
    $tree = (& git -C $root rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source tree' }
    $branch = (& git -C $root branch --show-current).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'could not resolve source branch' }
    $changes = (& git -C $root status --porcelain --untracked-files=all) -join ''
    if ($LASTEXITCODE -ne 0) { throw 'could not verify source tree cleanliness' }
    if (-not [string]::IsNullOrWhiteSpace($changes)) { throw 'immutable harness builds require a clean source checkout' }
    if ($branch -ne $expectedBranch) { throw ('expected branch ' + $expectedBranch + ', got ' + $branch) }
    & git -C $root merge-base --is-ancestor $baseRevision HEAD
    if ($LASTEXITCODE -ne 0) { throw ('source revision is not descended from ' + $baseRevision) }
    [ordered]@{ revision = $revision; tree = $tree; branch = $branch }
}

function Assert-PathOutsideRepository {
    param([string]$Path)
    $rootPrefix = $root.TrimEnd('\') + '\'
    if ([string]::Equals($Path.TrimEnd('\'), $root.TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase) -or
        $Path.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'output root must be outside the source checkout'
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

$source = Get-SourceIdentity
$rustcVersion = (& rustc -Vv) -join [Environment]::NewLine
if ($LASTEXITCODE -ne 0) { throw 'could not identify active rustc version' }
$cargoVersion = (& cargo --version).Trim()
if ($LASTEXITCODE -ne 0) { throw 'could not identify active cargo version' }
$cargoCleanCommand = 'cargo clean --manifest-path rust/Cargo.toml --profile dist --target x86_64-pc-windows-msvc'
$cargoBuildCommand = 'cargo build --locked --manifest-path rust/Cargo.toml -p sky_player --profile dist --target x86_64-pc-windows-msvc --features ' + $featureArgument + ' --bin rt-native-acceptance'
$buildLog = Join-Path $destination 'build.log'

$cleanOutput = @(& cargo clean --manifest-path (Join-Path $rustRoot 'Cargo.toml') --profile $profile --target $targetTriple 2>&1)
$cleanExit = $LASTEXITCODE
$cleanOutput | Set-Content -LiteralPath (Join-Path $destination 'clean.log') -Encoding UTF8
if ($cleanExit -ne 0) { throw ('cargo clean failed with exit code ' + $cleanExit) }

$buildOutput = @(& cargo build --locked --manifest-path (Join-Path $rustRoot 'Cargo.toml') -p sky_player --profile $profile --target $targetTriple --features $featureArgument --bin rt-native-acceptance 2>&1)
$buildExit = $LASTEXITCODE
$buildOutput | Set-Content -LiteralPath $buildLog -Encoding UTF8
if ($buildExit -ne 0) { throw ('cargo build failed with exit code ' + $buildExit) }

$sourceAfterBuild = Get-SourceIdentity
if ($sourceAfterBuild.revision -ne $source.revision -or $sourceAfterBuild.tree -ne $source.tree) {
    throw 'source identity changed during immutable harness build'
}
$builtBinary = Join-Path $rustRoot ('target\' + $targetTriple + '\dist\rt-native-acceptance.exe')
if (-not (Test-Path -LiteralPath $builtBinary -PathType Leaf)) { throw 'cargo build did not produce the expected Windows dist harness' }
$binaryPath = Join-Path $destination 'rt-native-acceptance.exe'
Copy-Item -LiteralPath $builtBinary -Destination $binaryPath
$binaryHash = Get-Sha256 $binaryPath
$builderHash = Get-Sha256 $PSCommandPath
$manifest = [ordered]@{
    schema_version = 1
    binary_name = 'rt-native-acceptance.exe'
    binary_path = $binaryPath
    binary_sha256 = $binaryHash
    source_revision = $source.revision
    source_tree_sha = $source.tree
    source_branch = $source.branch
    source_tree_clean = $true
    clean_build = $true
    cargo_profile = $profile
    cargo_features = $features.ToArray()
    qpc_read_counting = [bool]$QpcReadCounting
    observer_mode = 'qualification-on'
    guard_mode = $GuardMode
    workload_version = $workloadVersion
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
$manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
$manifestHash = Get-Sha256 $manifestPath
$checksumLines = @(
    ($binaryHash + '  rt-native-acceptance.exe'),
    ($manifestHash + '  manifest.json'),
    ($builderHash + '  build_dispatch_core_harness.ps1'),
    ((Get-Sha256 $buildLog) + '  build.log'),
    ((Get-Sha256 (Join-Path $destination 'clean.log')) + '  clean.log')
)
Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $destination 'build_dispatch_core_harness.ps1')
Set-Content -LiteralPath (Join-Path $destination 'SHA256SUMS.txt') -Value $checksumLines -Encoding UTF8
Write-Output ('SOURCE ' + $source.revision + ' tree=' + $source.tree)
Write-Output ('BINARY ' + $binaryPath + ' sha256=' + $binaryHash)
Write-Output ('MANIFEST ' + $manifestPath + ' sha256=' + $manifestHash)
