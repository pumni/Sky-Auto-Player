param(
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$outputPath = [System.IO.Path]::GetFullPath($OutputRoot)
$rootPrefix = $repositoryRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
if ($outputPath.Equals($repositoryRoot, [System.StringComparison]::OrdinalIgnoreCase) -or
    $outputPath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'OutputRoot must be outside the repository.'
}
if (Test-Path -LiteralPath $outputPath) {
    throw 'OutputRoot must be a new path that does not already exist.'
}

$branch = (& git -C $repositoryRoot branch --show-current).Trim()
if ($LASTEXITCODE -ne 0 -or $branch -ne 'refactor/prepared-playback-runtime') {
    throw "Expected branch refactor/prepared-playback-runtime, found '$branch'."
}
$sourceRevision = (& git -C $repositoryRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Could not resolve HEAD.' }
$sourceTree = (& git -C $repositoryRoot rev-parse 'HEAD^{tree}').Trim()
if ($LASTEXITCODE -ne 0) { throw 'Could not resolve HEAD tree.' }
$dirty = @(& git -C $repositoryRoot status --porcelain)
if ($LASTEXITCODE -ne 0 -or $dirty.Count -ne 0) {
    throw 'Candidate checkout must be clean so both guard arms share an exact source revision.'
}

$cargoVersion = (& cargo --version).Trim()
if ($LASTEXITCODE -ne 0) { throw 'cargo --version failed.' }
$rustcVersion = (& rustc --version).Trim()
if ($LASTEXITCODE -ne 0) { throw 'rustc --version failed.' }
$rustcVerbose = (& rustc -vV) -join "`n"
if ($LASTEXITCODE -ne 0) { throw 'rustc -vV failed.' }
$hostTarget = [regex]::Match($rustcVerbose, '(?m)^host: (.+)$').Groups[1].Value
$cargoLock = Join-Path $repositoryRoot 'rust/Cargo.lock'
$lockHash = (Get-FileHash -LiteralPath $cargoLock -Algorithm SHA256).Hash.ToLowerInvariant()
$benchmarkSource = Join-Path $repositoryRoot 'rust/crates/sky_player/src/engine/worker/timing_guard_ab_benchmark.rs'
$benchmarkHash = (Get-FileHash -LiteralPath $benchmarkSource -Algorithm SHA256).Hash.ToLowerInvariant()

New-Item -ItemType Directory -Path $outputPath | Out-Null
$testLog = Join-Path $outputPath 'cargo-test.log'
$manifestPath = Join-Path $outputPath 'run-manifest.json'
$priorOutputDir = [Environment]::GetEnvironmentVariable('ISSUE_666_M4_OUTPUT_DIR', 'Process')
$command = 'cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib offline_waiter_guard_ab_benchmark -- --ignored --nocapture'
$exitCode = $null
$testSummary = $null
$runnerBinary = $null
$runnerHash = $null
$resultJson = $null

try {
    [Environment]::SetEnvironmentVariable('ISSUE_666_M4_OUTPUT_DIR', $outputPath, 'Process')
    Push-Location $repositoryRoot
    try {
        & cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --lib offline_waiter_guard_ab_benchmark -- --ignored --nocapture *> $testLog
        $exitCode = $LASTEXITCODE
    }
    finally {
        Pop-Location
    }

    $logText = [System.IO.File]::ReadAllText($testLog)
    $resultMarkers = [regex]::Matches($logText, '(?m)^M4_GUARD_AB_RESULT_JSON=(\{.*\})\s*$')
    if ($resultMarkers.Count -eq 1) {
        $resultJson = $resultMarkers[0].Groups[1].Value | ConvertFrom-Json -AsHashtable
        [System.IO.File]::WriteAllText(
            (Join-Path $outputPath 'result.json'),
            ($resultJson | ConvertTo-Json -Depth 20),
            [System.Text.UTF8Encoding]::new($false))
    }
    $summaryMatches = [regex]::Matches($logText, '(?m)test result: (.+)$')
    if ($summaryMatches.Count -gt 0) { $testSummary = $summaryMatches[$summaryMatches.Count - 1].Groups[1].Value }
    $binaryMatches = [regex]::Matches($logText, '(?m)^\s*Running unittests .+\((.+\.exe)\)\s*$')
    if ($binaryMatches.Count -eq 1) {
        $runnerBinary = $binaryMatches[0].Groups[1].Value.Trim()
        if (-not [System.IO.Path]::IsPathRooted($runnerBinary)) {
            $runnerBinary = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $runnerBinary))
        }
        if (Test-Path -LiteralPath $runnerBinary) {
            $runnerHash = (Get-FileHash -LiteralPath $runnerBinary -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
}
finally {
    [Environment]::SetEnvironmentVariable('ISSUE_666_M4_OUTPUT_DIR', $priorOutputDir, 'Process')
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $cpu = Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1
    $manifest = [ordered]@{
        schema_version = 1
        evidence_kind = 'offline_waiter_guard_ab_not_playback_qualification'
        status = if ($exitCode -eq 0 -and $resultJson) { 'completed' } else { 'failed_or_incomplete' }
        source_revision = $sourceRevision
        source_tree_sha = $sourceTree
        branch = $branch
        source_tree_clean_before_run = $true
        host_os = [System.Environment]::OSVersion.VersionString
        windows_caption = $os.Caption
        windows_version = $os.Version
        processor = $cpu.Name
        logical_processor_count = [System.Environment]::ProcessorCount
        rustc = $rustcVersion
        cargo = $cargoVersion
        host_target = $hostTarget
        cargo_lock_sha256 = $lockHash
        benchmark_source_sha256 = $benchmarkHash
        command = $command
        exit_code = $exitCode
        test_summary = $testSummary
        test_runner_binary = $runnerBinary
        test_runner_binary_sha256 = $runnerHash
        raw_sample_artifact_sha256 = if (Test-Path -LiteralPath (Join-Path $outputPath 'wait-samples.csv')) { (Get-FileHash -LiteralPath (Join-Path $outputPath 'wait-samples.csv') -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
        run_summary_artifact_sha256 = if (Test-Path -LiteralPath (Join-Path $outputPath 'wait-runs.csv')) { (Get-FileHash -LiteralPath (Join-Path $outputPath 'wait-runs.csv') -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
        result_json_present = [bool]$resultJson
        sendinput_called = $false
    }
    [System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
}

if ($exitCode -ne 0) { throw "Offline waiter A/B failed with exit code $exitCode. See $testLog and $manifestPath" }
if (-not $resultJson -or -not $runnerHash) { throw "A/B output or test-runner binary evidence is incomplete. See $testLog and $manifestPath" }
Write-Output "source_revision=$sourceRevision"
Write-Output "source_tree_sha=$sourceTree"
Write-Output "test_summary=$testSummary"
Write-Output "result=$((Join-Path $outputPath 'result.json'))"
Write-Output "manifest=$manifestPath"
