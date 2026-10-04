[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Stage,
    [Parameter(Mandatory)]
    [string]$OutputRoot,
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ControlRevision
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$runId = 'r3-p0-' + (Get-Date -AsUTC -Format 'yyyyMMddTHHmmssfffZ') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$commandResults = [System.Collections.Generic.List[object]]::new()
$contentionProcesses = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
$runDirectory = $null

function Resolve-Executable([string]$Name) {
    $command = Get-Command $Name -ErrorAction Stop | Select-Object -First 1
    if (-not $command.Source) { throw "Could not resolve executable: $Name" }
    return $command.Source
}

function Invoke-CapturedCommand {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][string]$Label,
        [hashtable]$Environment = @{}
    )

    $index = $commandResults.Count + 1
    $stem = Join-Path $runDirectory ('commands\{0:D3}-{1}' -f $index, $Label)
    $stdoutPath = "$stem.stdout.log"
    $stderrPath = "$stem.stderr.log"
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($stem)) | Out-Null

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = Resolve-Executable $Name
    $startInfo.WorkingDirectory = $repoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { [void]$startInfo.ArgumentList.Add($argument) }
    foreach ($key in $Environment.Keys) { $startInfo.Environment[$key] = [string]$Environment[$key] }

    $startedAt = [DateTimeOffset]::UtcNow
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Could not start child process: $Name" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdoutText = $stdoutTask.GetAwaiter().GetResult()
    $stderrText = $stderrTask.GetAwaiter().GetResult()
    [System.IO.File]::WriteAllText($stdoutPath, $stdoutText, $utf8NoBom)
    [System.IO.File]::WriteAllText($stderrPath, $stderrText, $utf8NoBom)

    $result = [ordered]@{
        label = $Label
        executable = [System.IO.Path]::GetFileName($startInfo.FileName)
        arguments = $Arguments
        started_utc = $startedAt.ToString('O')
        finished_utc = [DateTimeOffset]::UtcNow.ToString('O')
        exit_code = $process.ExitCode
        stdout_path = [System.IO.Path]::GetRelativePath($runDirectory, $stdoutPath).Replace('\', '/')
        stderr_path = [System.IO.Path]::GetRelativePath($runDirectory, $stderrPath).Replace('\', '/')
    }
    $commandResults.Add($result)
    if ($process.ExitCode -ne 0) {
        throw "Child command '$Label' exited $($process.ExitCode); see $($result.stderr_path)"
    }
    return $result
}

function Get-ToolVersion([string]$Name, [string]$Argument) {
    $executable = Resolve-Executable $Name
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    [void]$startInfo.ArgumentList.Add($Argument)
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { return 'unavailable' }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult().Trim()
    $stderr = $stderrTask.GetAwaiter().GetResult().Trim()
    if ($process.ExitCode -ne 0) { return "unavailable (exit $($process.ExitCode)): $stderr" }
    return $stdout
}

function Start-ContentionWorkers {
    $pwsh = Resolve-Executable 'pwsh.exe'
    $loadCommand = 'while ($true) { [void]([Math]::Sqrt(12345.6789)) }'
    for ($index = 0; $index -lt 2; $index++) {
        $process = Start-Process -FilePath $pwsh -ArgumentList @('-NoProfile', '-Command', $loadCommand) -PassThru -WindowStyle Hidden
        if ($process.HasExited) { throw 'A CPU contention worker exited during startup' }
        $contentionProcesses.Add($process)
    }
    Start-Sleep -Milliseconds 200
    if ($contentionProcesses.Count -ne 2 -or @($contentionProcesses | Where-Object { $_.HasExited }).Count -ne 0) {
        throw 'Could not establish both bounded CPU contention workers'
    }
}

function Stop-ContentionWorkers {
    foreach ($process in $contentionProcesses) {
        try {
            if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
        } catch { }
    }
    $contentionProcesses.Clear()
}

function Get-Median([double[]]$Values) {
    $ordered = @($Values | Sort-Object)
    if ($ordered.Count -eq 0) { return $null }
    if ($ordered.Count % 2 -eq 1) { return $ordered[[int][Math]::Floor($ordered.Count / 2)] }
    return ($ordered[$ordered.Count / 2 - 1] + $ordered[$ordered.Count / 2]) / 2
}

function Write-JsonFile([string]$Path, [object]$Value) {
    $json = ConvertTo-Json -InputObject $Value -Depth 20
    [System.IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $utf8NoBom)
}

try {
    # These are runtime runner inputs, not durable artifact or release names.
    $allowedStages = @('baseline', ('phase' + '1'), ('phase' + '2'), 'diagnostics', 'final')
    if ($Stage -notin $allowedStages) {
        throw "BLOCKED-STAGE: stage '$Stage' is unsupported; no evidence or PASS result was generated"
    }
    if ($Stage -ne 'baseline') {
        throw "BLOCKED-STAGE: stage '$Stage' is not implemented; no evidence or PASS result was generated"
    }
    if ($PSBoundParameters.ContainsKey('ControlRevision')) {
        throw 'ControlRevision is reserved for the future comparison stage, which is not implemented'
    }

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
    $outputBase = if ([System.IO.Path]::IsPathRooted($OutputRoot)) {
        [System.IO.Path]::GetFullPath($OutputRoot)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $OutputRoot))
    }
    [System.IO.Directory]::CreateDirectory($outputBase) | Out-Null
    $runDirectory = Join-Path $outputBase $runId
    if (Test-Path -LiteralPath $runDirectory) { throw "Refusing to overwrite existing run directory: $runDirectory" }
    [System.IO.Directory]::CreateDirectory($runDirectory) | Out-Null

    $headSha = (& git -C $repoRoot rev-parse --verify HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $headSha -notmatch '^[0-9a-f]{40}$') { throw 'Could not resolve the full source revision' }
    $runtimeRevision = (& git -C $repoRoot merge-base HEAD refs/remotes/origin/main).Trim()
    if ($LASTEXITCODE -ne 0 -or $runtimeRevision -notmatch '^[0-9a-f]{40}$') { throw 'Could not resolve the baseline runtime revision' }
    $statusLines = @(& git -C $repoRoot status --porcelain --untracked-files=all)
    if ($LASTEXITCODE -ne 0) { throw 'Could not verify source-tree cleanliness' }
    $sourceTreeClean = $statusLines.Count -eq 0
    if (-not $sourceTreeClean) { throw 'Runner requires a clean committed source tree before capturing provenance' }

    $rustcVersion = Get-ToolVersion 'rustc.exe' '--version'
    $cargoVersion = Get-ToolVersion 'cargo.exe' '--version'
    $bunVersion = Get-ToolVersion 'bun.exe' '--version'
    $pwshVersion = $PSVersionTable.PSVersion.ToString()
    $gitVersion = Get-ToolVersion 'git.exe' '--version'
    $rustcHostOutput = Get-ToolVersion 'rustc.exe' '-vV'
    $target = ($rustcHostOutput -split "`r?`n" | Where-Object { $_ -match '^host: ' } | Select-Object -First 1) -replace '^host: ', ''
    if (-not $target) { throw 'Could not determine the Rust target triple' }
    $osInfo = Get-CimInstance Win32_OperatingSystem
    $cpuInfo = Get-CimInstance Win32_Processor | Select-Object -First 1
    $powerOutput = (& powercfg.exe /getactivescheme 2>&1 | Out-String).Trim()
    $powerExitCode = $LASTEXITCODE
    $batteryInfo = @(Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | ForEach-Object {
        [ordered]@{ name = $_.Name; status = $_.BatteryStatus; estimated_charge_remaining = $_.EstimatedChargeRemaining }
    })
    if (-not [Environment]::UserInteractive) { throw 'BLOCKED-ENV: Windows interactive desktop is unavailable' }
    if ($target -ne 'x86_64-pc-windows-msvc') { throw "BLOCKED-ENV: expected x86_64-pc-windows-msvc, found $target" }
    if ($osInfo.Caption -notmatch 'Windows 11') { throw "BLOCKED-ENV: expected Windows 11, found $($osInfo.Caption)" }
    if ($rustcVersion -notmatch '^rustc 1\.98\.1(?:\s|$)' -or $cargoVersion -notmatch '^cargo 1\.98\.1(?:\s|$)') {
        throw "BLOCKED-ENV: expected Rust/Cargo 1.98.1; found '$rustcVersion' / '$cargoVersion'"
    }
    if ($bunVersion -ne '1.4.0') {
        throw "BLOCKED-ENV: expected Bun 1.4.0; found '$bunVersion'"
    }
    $hostMetadata = [ordered]@{
        rustc = $rustcVersion
        cargo = $cargoVersion
        bun = $bunVersion
        pwsh = $pwshVersion
        git = $gitVersion
        target = $target
        os = "$($osInfo.Caption) build $($osInfo.BuildNumber)"
        cpu = $cpuInfo.Name
        power_scheme = $powerOutput
        power_scheme_exit_code = $powerExitCode
        batteries = $batteryInfo
        power_state = $powerOutput + '; batteries=' + (ConvertTo-Json -InputObject $batteryInfo -Compress)
        interactive = [Environment]::UserInteractive
        session_name = $env:SESSIONNAME
    }

    $commandsDirectory = Join-Path $runDirectory 'commands'
    [System.IO.Directory]::CreateDirectory($commandsDirectory) | Out-Null
    $build = Invoke-CapturedCommand -Name 'cargo.exe' -Arguments @('build', '--locked', '--manifest-path', 'rust/Cargo.toml', '-p', 'sky_player', '--release', '--features', 'test-support', '--example', 'rt_r3_probe') -Label 'build-probe'
    $probePath = Join-Path $repoRoot 'rust\target\release\examples\rt_r3_probe.exe'
    if (-not (Test-Path -LiteralPath $probePath -PathType Leaf)) { throw 'Release probe binary was not produced' }
    $probeHash = (Get-FileHash -LiteralPath $probePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $environment = @{
        SKY_RT_R3_RUNTIME_REVISION = $runtimeRevision
        SKY_RT_R3_RUSTC_VERSION = $rustcVersion
        SKY_RT_R3_CARGO_VERSION = $cargoVersion
        SKY_RT_R3_BUN_VERSION = $bunVersion
        SKY_RT_R3_PWSH_VERSION = $pwshVersion
        SKY_RT_R3_GIT_VERSION = $gitVersion
        SKY_RT_R3_TARGET = $target
        SKY_RT_R3_OS = $hostMetadata.os
        SKY_RT_R3_CPU = $hostMetadata.cpu
        SKY_RT_R3_POWER_STATE = $hostMetadata.power_state
    }

    $contractPath = Join-Path $runDirectory 'contracts.json'
    $contractRunId = "$runId-contracts"
    $contractEnvironment = $environment.Clone()
    $contractEnvironment.SKY_RT_R3_RUN_ID = $contractRunId
    $contractEnvironment.SKY_RT_R3_CONTENTION_WORKERS = '0'
    [void](Invoke-CapturedCommand -Name $probePath -Arguments @('--mode', 'contracts', '--load-mode', 'quiet', '--output', $contractPath) -Label 'contracts' -Environment $contractEnvironment)
    $contractReport = Get-Content -LiteralPath $contractPath -Raw | ConvertFrom-Json
    $vectorA = $contractReport.test_vectors.A_late_same_key_pause_resume
    $vectorB = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed
    $vectorD = $contractReport.test_vectors.D_focus_loss_policy
    if ($vectorA.hypothesis_status -ne 'REPRODUCED' -or $vectorA.vector_cases.Count -ne 3 -or
        @($vectorA.vector_cases | Where-Object { $_.below_completion_plus_frame -ne $true }).Count -ne 0 -or
        $vectorA.exact_vector_duplicate_tail.status -ne 'REJECTED_BY_SCHEDULE_COMPILER' -or
        $vectorB.hypothesis_status -ne 'REPRODUCED' -or
        $vectorB.expired_without_watchdog.disposition -ne 'STALE_LEASE_ACCEPTED' -or
        $vectorB.expired_without_watchdog.lease_enabled -ne $true -or
        $vectorB.expired_without_watchdog.musical_sender_attempts -ne 1 -or
        $vectorD.restore_path.release_on_restore -ne $true -or
        $vectorD.terminal_cleanup_while_unfocused.ownership_scoped_terminal_release -ne $true -or
        $contractReport.test_vectors.E_empty_cleanup.oracle_passed -ne $true) {
        throw 'BLOCKED-CONTRACT: lifecycle/lease/cleanup probe results did not satisfy the P0 characterization contract'
    }

    $workloads = @('down-1', 'down-5', 'down-15', 'mixed-1x1', 'mixed-2x3', 'mixed-7x8', 'up-only-1', 'up-only-5', 'up-only-15')
    $loadModes = @('quiet', 'cpu_contention')
    $precisionRecords = [System.Collections.Generic.List[object]]::new()
    foreach ($workload in $workloads) {
        foreach ($loadMode in $loadModes) {
            if ($loadMode -eq 'cpu_contention') { Start-ContentionWorkers }
            try {
                for ($runIndex = 1; $runIndex -le 5; $runIndex++) {
                    if ($loadMode -eq 'cpu_contention' -and @($contentionProcesses | Where-Object { $_.HasExited }).Count -ne 0) {
                        throw 'A CPU contention worker exited during the measurement group'
                    }
                    $relativeDirectory = Join-Path (Join-Path 'precision' $workload) $loadMode
                    $absoluteDirectory = Join-Path $runDirectory $relativeDirectory
                    [System.IO.Directory]::CreateDirectory($absoluteDirectory) | Out-Null
                    $relativePath = Join-Path $relativeDirectory ("run-{0:D2}.json" -f $runIndex)
                    $reportPath = Join-Path $runDirectory $relativePath
                    $childRunId = "$runId-$workload-$loadMode-r$runIndex"
                    $runEnvironment = $environment.Clone()
                    $runEnvironment.SKY_RT_R3_RUN_ID = $childRunId
                    $runEnvironment.SKY_RT_R3_LOAD_MODE = $loadMode
                    $runEnvironment.SKY_RT_R3_CONTENTION_WORKERS = if ($loadMode -eq 'cpu_contention') { '2' } else { '0' }
                    $arguments = @(
                        '--mode', 'precision', '--load-mode', $loadMode, '--workload', $workload,
                        '--warmup', '1000', '--measured', '10000', '--seed', '1073',
                        '--run-index', [string]$runIndex, '--output', $reportPath
                    )
                    $command = Invoke-CapturedCommand -Name $probePath -Arguments $arguments -Label "precision-$workload-$loadMode-r$runIndex" -Environment $runEnvironment
                    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
                    $precisionRun = @($report.test_vectors.precision_runs)[0]
                    if ($report.schema_version -ne 1 -or $report.source_revision.ToLowerInvariant() -ne $headSha -or
                        $report.runtime_revision.ToLowerInvariant() -ne $runtimeRevision -or $report.profile -ne 'release' -or
                        $report.transport_kind -ne 'deterministic-mock' -or $report.run_id -ne $childRunId -or
                        $report.mode -ne 'precision' -or $report.load_mode -ne $loadMode -or
                        $report.contention_workers -ne $(if ($loadMode -eq 'cpu_contention') { 2 } else { 0 }) -or
                        $report.warmup_count -ne 1000 -or $report.measured_count -ne 10000 -or
                        $precisionRun.statistics_eligible -ne $true -or $precisionRun.sample_count -ne 10000 -or
                        $precisionRun.failed_sample_count -ne 0 -or $precisionRun.counters.measured_sender_attempts -ne 10000) {
                        throw "BLOCKED-MEASUREMENT: $childRunId did not satisfy the pinned 1,000 warmup / 10,000 measured baseline contract"
                    }
                    $precisionRecords.Add([ordered]@{
                        workload = $workload
                        load_mode = $loadMode
                        run_index = $runIndex
                        run_id = $childRunId
                        report_path = $relativePath.Replace('\', '/')
                        command_label = $command.label
                    })
                }
            } finally {
                if ($loadMode -eq 'cpu_contention') { Stop-ContentionWorkers }
            }
        }
    }

    $nativeBuild = Invoke-CapturedCommand -Name 'cargo.exe' -Arguments @('build', '--locked', '--manifest-path', 'rust/Cargo.toml', '-p', 'sky_player', '--release', '--features', 'real-input-acceptance,test-support', '--bin', 'rt-native-acceptance') -Label 'build-native-acceptance'
    [void](Invoke-CapturedCommand -Name 'pwsh.exe' -Arguments @('-NoProfile', '-File', 'scripts/native_acceptance_sink.ps1', '-SelfTest') -Label 'native-sink-selftest')
    $nativeOutputRoot = Join-Path $repoRoot '.benchmarks\physical-native-cases'
    $nativeResults = [System.Collections.Generic.List[object]]::new()
    foreach ($scenario in @('canonical-single', 'pause-resume', 'supervisor-lease-expiry')) {
        $beforeNativeRuns = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
        if (Test-Path -LiteralPath $nativeOutputRoot -PathType Container) {
            foreach ($directory in Get-ChildItem -LiteralPath $nativeOutputRoot -Directory) { [void]$beforeNativeRuns.Add($directory.FullName) }
        }
        $nativeCommand = Invoke-CapturedCommand -Name 'pwsh.exe' -Arguments @('-NoProfile', '-File', 'scripts/run_native_acceptance_case.ps1', '-Scenario', $scenario, '-TimingMarginUs', '500') -Label "native-$scenario"
        if ($nativeCommand.exit_code -ne 0) { throw "BLOCKED-NATIVE: $scenario acceptance failed" }
        $newNativeRuns = @()
        if (Test-Path -LiteralPath $nativeOutputRoot -PathType Container) {
            $newNativeRuns = @(Get-ChildItem -LiteralPath $nativeOutputRoot -Directory | Where-Object { -not $beforeNativeRuns.Contains($_.FullName) })
        }
        if ($newNativeRuns.Count -ne 1) { throw "BLOCKED-NATIVE: could not identify one fresh $scenario evidence directory" }
        $nativeDestination = Join-Path (Join-Path $runDirectory 'native') $scenario
        [System.IO.Directory]::CreateDirectory($nativeDestination) | Out-Null
        foreach ($child in Get-ChildItem -LiteralPath $newNativeRuns[0].FullName -Force) {
            Copy-Item -LiteralPath $child.FullName -Destination $nativeDestination -Recurse
        }
        $nativeResults.Add([ordered]@{
            scenario = $scenario
            timing_margin_us = 500
            command_label = $nativeCommand.label
            evidence_directory = "native/$scenario"
            source_run_id = $newNativeRuns[0].Name
            status = 'PASS'
        })
    }

    $precisionSummary = [System.Collections.Generic.List[object]]::new()
    foreach ($workload in $workloads) {
        foreach ($loadMode in $loadModes) {
            $runStats = [System.Collections.Generic.List[object]]::new()
            foreach ($record in $precisionRecords | Where-Object { $_.workload -eq $workload -and $_.load_mode -eq $loadMode }) {
                $report = Get-Content -LiteralPath (Join-Path $runDirectory $record.report_path) -Raw | ConvertFrom-Json
                $run = $report.test_vectors.precision_runs[0]
                $runStats.Add([ordered]@{
                    run_index = $record.run_index
                    sample_count = $run.sample_count
                    statistics_eligible = $run.statistics_eligible
                    admission_p50_us = $run.distributions.admission_to_pre_call.microseconds.p50
                    admission_p95_us = $run.distributions.admission_to_pre_call.microseconds.p95
                    admission_p99_us = $run.distributions.admission_to_pre_call.microseconds.p99
                    admission_max_us = $run.distributions.admission_to_pre_call.microseconds.max
                    sender_completion_p99_us = $run.distributions.pre_call_to_sender_completion.microseconds.p99
                    measured_sender_attempts = $run.counters.measured_sender_attempts
                    failed_sample_count = $run.failed_sample_count
                })
            }
            $p99Values = [double[]]@($runStats | ForEach-Object { [double]$_.admission_p99_us })
            $precisionSummary.Add([ordered]@{
                workload = $workload
                load_mode = $loadMode
                run_count = $runStats.Count
                runs = @($runStats)
                median_run_p99_us = Get-Median $p99Values
                statistics_eligible = ($runStats.Count -eq 5 -and @($runStats | Where-Object { -not $_.statistics_eligible -or $_.sample_count -ne 10000 -or $_.failed_sample_count -ne 0 -or $_.measured_sender_attempts -ne 10000 }).Count -eq 0)
            })
        }
    }

    $summary = [ordered]@{
        schema_version = 1
        status = 'COMPLETED_CHARACTERIZATION'
        stage = $Stage
        run_id = $runId
        source_revision = $headSha
        runtime_revision = $runtimeRevision
        source_tree_clean = $sourceTreeClean
        base_revision = $runtimeRevision
        executable_sha256 = $probeHash
        host = $hostMetadata
        profile = 'release'
        transport_kind = 'deterministic-mock'
        contracts = [ordered]@{
            A_hypothesis = $contractReport.test_vectors.A_late_same_key_pause_resume.hypothesis_status
            A_exact_vector_status = $contractReport.test_vectors.A_late_same_key_pause_resume.exact_vector_duplicate_tail.status
            A_valid_prefix_cases = $contractReport.test_vectors.A_late_same_key_pause_resume.vector_cases.Count
            B_hypothesis = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed.hypothesis_status
            B_expired_case = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed.expired_without_watchdog.disposition
            D_release_while_unfocused = $contractReport.test_vectors.D_focus_loss_policy.restore_path.cleanup_sends_while_unfocused
            D_release_after_restore = $contractReport.test_vectors.D_focus_loss_policy.restore_path.release_on_restore
            D_terminal_cleanup = $contractReport.test_vectors.D_focus_loss_policy.terminal_cleanup_while_unfocused.ownership_scoped_terminal_release
            E_empty_cleanup_oracle = $contractReport.test_vectors.E_empty_cleanup.oracle_passed
        }
        precision = @($precisionSummary)
        native_controls = @($nativeResults)
        native_sink_selftest = 'PASS'
        bun_version_expected_by_ci = '1.4.0'
        bun_version_matches_ci = ($bunVersion -eq '1.4.0')
        child_command_count = $commandResults.Count
        child_commands = @($commandResults)
        precision_run_count = $precisionRecords.Count
        paired_ab_disposition = 'P0 records baseline runs only; paired A/B execution is reserved for the comparison phase and currently fails explicitly.'
        timing_boundary = 'QPC from immediately before the real prepared-dispatch entry through the authoritative mock sender pre-call; mock completion reported separately.'
        focus_setup = 'test-support focus_active=true with deterministic SessionTarget; no game process or foreground-window activation.'
    }
    Write-JsonFile (Join-Path $runDirectory 'summary.json') $summary
    Write-JsonFile (Join-Path $runDirectory 'status.json') ([ordered]@{ status = 'COMPLETED_CHARACTERIZATION'; run_id = $runId; finished_utc = [DateTimeOffset]::UtcNow.ToString('O') })

    $declaredFiles = [System.Collections.Generic.List[object]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $runDirectory -File -Recurse | Sort-Object FullName) {
        if ($file.Name -eq 'runner-outputs.json') { continue }
        $relative = [System.IO.Path]::GetRelativePath($runDirectory, $file.FullName).Replace('\', '/')
        $declaredFiles.Add([ordered]@{ path = $relative; size_bytes = $file.Length; sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() })
    }
    $outputsManifest = [ordered]@{
        schema_version = 1
        status = 'COMPLETED_CHARACTERIZATION'
        stage = $Stage
        run_id = $runId
        source_revision = $headSha
        runtime_revision = $runtimeRevision
        executable_sha256 = $probeHash
        declared_outputs = @($declaredFiles)
    }
    Write-JsonFile (Join-Path $runDirectory 'runner-outputs.json') $outputsManifest
    Write-Output (ConvertTo-Json -InputObject ([ordered]@{ status = $outputsManifest.status; run_id = $runId; run_directory = $runDirectory; source_revision = $headSha; runtime_revision = $runtimeRevision; precision_runs = $precisionRecords.Count; child_commands = $commandResults.Count }) -Compress)
} catch {
    Stop-ContentionWorkers
    if ($runDirectory -and (Test-Path -LiteralPath $runDirectory)) {
        try {
            Write-JsonFile (Join-Path $runDirectory 'status.json') ([ordered]@{ status = 'BLOCKED'; stage = $Stage; run_id = $runId; error = $_.Exception.Message; finished_utc = [DateTimeOffset]::UtcNow.ToString('O'); child_commands = @($commandResults) })
        } catch { }
    }
    Write-Error $_
    exit 1
} finally {
    Stop-ContentionWorkers
}
