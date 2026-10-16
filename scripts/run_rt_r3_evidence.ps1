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
$runPrefix = if ($Stage -eq 'baseline') { 'r3-p0-' } elseif ($Stage -eq ('phase' + '1')) { 'r3-p1-' } elseif ($Stage -eq ('phase' + '2')) { 'r3-p2-' } elseif ($Stage -eq 'diagnostics') { 'r3-p4-' } else { 'r3-blocked-' }
$runId = $runPrefix + (Get-Date -AsUTC -Format 'yyyyMMddTHHmmssfffZ') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$commandResults = [System.Collections.Generic.List[object]]::new()
$contentionProcesses = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
$runDirectory = $null
$controlWorktree = $null
$controlWorktreeCreated = $false
$script:blockedPerf = $false
$script:blockedDefect = $false

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
        [string]$WorkingDirectory = $repoRoot,
        [hashtable]$Environment = @{},
        [switch]$CaptureWorkingSet
    )

    $index = $commandResults.Count + 1
    $stem = Join-Path $runDirectory ('commands\{0:D3}-{1}' -f $index, $Label)
    $stdoutPath = "$stem.stdout.log"
    $stderrPath = "$stem.stderr.log"
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($stem)) | Out-Null

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = Resolve-Executable $Name
    $startInfo.WorkingDirectory = $WorkingDirectory
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
    $workingSetStartBytes = $null
    $peakWorkingSetBytes = $null
    $workingSetSampleCount = 0
    $workingSetSampleIntervalMs = $null
    if ($CaptureWorkingSet) {
        $workingSetSampleIntervalMs = 5
        $process.Refresh()
        try {
            $workingSetStartBytes = [long]$process.WorkingSet64
            $peakWorkingSetBytes = $workingSetStartBytes
            $workingSetSampleCount++
        } catch { }
        while (-not $process.WaitForExit($workingSetSampleIntervalMs)) {
            $process.Refresh()
            try {
                $sampleBytes = [long]$process.WorkingSet64
                if ($null -eq $workingSetStartBytes) { $workingSetStartBytes = $sampleBytes }
                if ($null -eq $peakWorkingSetBytes -or $sampleBytes -gt $peakWorkingSetBytes) { $peakWorkingSetBytes = $sampleBytes }
                $workingSetSampleCount++
            } catch { }
        }
        $process.Refresh()
        try {
            $finalWorkingSetBytes = [long]$process.WorkingSet64
            if ($null -eq $workingSetStartBytes) { $workingSetStartBytes = $finalWorkingSetBytes }
            if ($null -eq $peakWorkingSetBytes -or $finalWorkingSetBytes -gt $peakWorkingSetBytes) { $peakWorkingSetBytes = $finalWorkingSetBytes }
            $workingSetSampleCount++
        } catch { }
    } else {
        $process.WaitForExit()
    }
    $stdoutText = $stdoutTask.GetAwaiter().GetResult()
    $stderrText = $stderrTask.GetAwaiter().GetResult()
    [System.IO.File]::WriteAllText($stdoutPath, $stdoutText, $utf8NoBom)
    [System.IO.File]::WriteAllText($stderrPath, $stderrText, $utf8NoBom)

    $result = [ordered]@{
        label = $Label
        executable = [System.IO.Path]::GetFileName($startInfo.FileName)
        working_directory = $WorkingDirectory
        arguments = $Arguments
        started_utc = $startedAt.ToString('O')
        finished_utc = [DateTimeOffset]::UtcNow.ToString('O')
        exit_code = $process.ExitCode
        working_set_capture = [bool]$CaptureWorkingSet
        working_set_start_bytes = $workingSetStartBytes
        working_set_peak_bytes = $peakWorkingSetBytes
        working_set_sample_count = $workingSetSampleCount
        working_set_sample_interval_ms = $workingSetSampleIntervalMs
        working_set_scope = if ($CaptureWorkingSet) { 'child-process working set sampled by the PowerShell parent; includes shared resident pages and probe/runtime memory; not allocator-requested bytes' } else { 'not captured' }
        stdout_path = [System.IO.Path]::GetRelativePath($runDirectory, $stdoutPath).Replace('\', '/')
        stderr_path = [System.IO.Path]::GetRelativePath($runDirectory, $stderrPath).Replace('\', '/')
    }
    $commandResults.Add($result)
    if ($process.ExitCode -ne 0) {
        if ($stderrText -match 'BLOCKED-DEFECT') {
            $script:blockedDefect = $true
            throw "BLOCKED-DEFECT: child command '$Label' reported a physical invariant defect; see $($result.stderr_path)"
        }
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

function Get-PrecisionRunStats([string]$Root, [object]$Record) {
    $report = Get-Content -LiteralPath (Join-Path $Root $Record.report_path) -Raw | ConvertFrom-Json
    $run = $report.test_vectors.precision_runs[0]
    return [ordered]@{
        pair_index = $Record.pair_index
        pair_order = $Record.pair_order
        arm = $Record.arm
        run_id = $Record.run_id
        sample_count = $run.sample_count
        statistics_eligible = $run.statistics_eligible
        failed_sample_count = $run.failed_sample_count
        measured_sender_attempts = $run.counters.measured_sender_attempts
        admission_p50_us = $run.distributions.admission_to_pre_call.microseconds.p50
        admission_p95_us = $run.distributions.admission_to_pre_call.microseconds.p95
        admission_p99_us = $run.distributions.admission_to_pre_call.microseconds.p99
        admission_max_us = $run.distributions.admission_to_pre_call.microseconds.max
        sender_completion_p50_us = $run.distributions.pre_call_to_sender_completion.microseconds.p50
        sender_completion_p95_us = $run.distributions.pre_call_to_sender_completion.microseconds.p95
        sender_completion_p99_us = $run.distributions.pre_call_to_sender_completion.microseconds.p99
        sender_completion_max_us = $run.distributions.pre_call_to_sender_completion.microseconds.max
        source_revision = $report.source_revision
        runtime_revision = $report.runtime_revision
        executable_sha256 = $report.executable_sha256
        report_path = $Record.report_path
    }
}

try {
    # These are runtime runner inputs, not durable artifact or release names.
    $allowedStages = @('baseline', ('phase' + '1'), ('phase' + '2'), 'diagnostics', 'final')
    if ($Stage -notin $allowedStages) {
        throw "BLOCKED-STAGE: stage '$Stage' is unsupported; no evidence or PASS result was generated"
    }
    if ($Stage -notin @('baseline', ('phase' + '1'), ('phase' + '2'), 'diagnostics')) {
        throw "BLOCKED-STAGE: stage '$Stage' is not implemented; no evidence or PASS result was generated"
    }
    $isP2Comparison = $Stage -eq ('phase' + '2')
    $isP4Diagnostics = $Stage -eq 'diagnostics'
    if ($isP2Comparison -and -not $PSBoundParameters.ContainsKey('ControlRevision')) {
        throw 'BLOCKED-STAGE: P2 requires the exact accepted-P1 ControlRevision'
    }
    if (-not $isP2Comparison -and $PSBoundParameters.ContainsKey('ControlRevision')) {
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
    if ($isP2Comparison) {
        $controlRevision = $ControlRevision.ToLowerInvariant()
        $originMain = (& git -C $repoRoot rev-parse --verify refs/remotes/origin/main).Trim()
        if ($LASTEXITCODE -ne 0 -or $originMain.ToLowerInvariant() -ne $controlRevision -or $runtimeRevision -ne $controlRevision) {
            throw "BLOCKED-CONTROL: ControlRevision $controlRevision must exactly match both origin/main ($originMain) and the P2 merge base ($runtimeRevision)"
        }
    }
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
    if ($rustcVersion -notmatch '^rustc 1\.99\.1(?:\s|$)' -or $cargoVersion -notmatch '^cargo 1\.99\.1(?:\s|$)') {
        throw "BLOCKED-ENV: expected Rust/Cargo 1.99.1; found '$rustcVersion' / '$cargoVersion'"
    }
    if ($bunVersion -ne '1.4.3') {
        throw "BLOCKED-ENV: expected Bun 1.4.3; found '$bunVersion'"
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

    if ($isP4Diagnostics) {
        $helpEnvironment = $environment.Clone()
        $helpEnvironment.SKY_RT_R3_RUN_ID = "$runId-help"
        [void](Invoke-CapturedCommand -Name $probePath -Arguments @('--help') -Label 'diagnostics-probe-help' -Environment $helpEnvironment)
        $helpPath = Join-Path $runDirectory $commandResults[$commandResults.Count - 1].stdout_path
        $helpText = Get-Content -LiteralPath $helpPath -Raw
        foreach ($requiredText in @('contracts|precision|hol|startup|polling', '--generation-count', '17167us', 'no real keyboard input')) {
            if ($helpText -notmatch [regex]::Escape($requiredText)) { throw "BLOCKED-STAGE: diagnostics probe help does not declare '$requiredText'" }
        }

        $holPath = Join-Path $runDirectory 'hol.json'
        $holEnvironment = $environment.Clone()
        $holEnvironment.SKY_RT_R3_RUN_ID = "$runId-hol"
        $holCommand = Invoke-CapturedCommand -Name $probePath -Arguments @('--mode', 'hol', '--output', $holPath) -Label 'diagnostics-hol' -Environment $holEnvironment -CaptureWorkingSet
        $holReport = Get-Content -LiteralPath $holPath -Raw | ConvertFrom-Json
        $hol = $holReport.test_vectors.hol_scenarios
        if ($holReport.source_revision.ToLowerInvariant() -ne $headSha -or
            $holReport.runtime_revision.ToLowerInvariant() -ne $runtimeRevision -or
            $holReport.mode -ne 'hol' -or
            $hol.scenario_id -ne 'r3-p4-hol-fixed-v1' -or
            $hol.scheduler_duplicated -ne $false -or
            $hol.base_scenario_count -ne 4 -or
            $hol.maximum_chord_case_count -ne 36 -or
            $hol.physical_packet_row_count -ne 91) {
            throw 'BLOCKED-DIAGNOSTICS: HOL scenario identity, count, provenance, or prepared-path declaration did not match the fixed dataset'
        }
        if ($hol.physical_floor_violation_count -ne 0 -or $hol.down_replay_case_count -ne 0) {
            $script:blockedDefect = $true
            throw "BLOCKED-DEFECT: P4 HOL diagnostics observed $($hol.physical_floor_violation_count) physical-floor violation(s) and $($hol.down_replay_case_count) Down replay case(s); production remains unchanged for coordinator review"
        }
        foreach ($case in $hol.cases) {
            if ($case.authored_offsets_unchanged -ne $true -or
                $case.single_attempt_per_packet -ne $true -or
                $case.active_generations -ne 0 -or
                $case.release_obligation_mask -ne 0) {
                throw "BLOCKED-DIAGNOSTICS: HOL accounting/offset oracle failed for $($case.case_id)"
            }
        }

        $startupRecords = [System.Collections.Generic.List[object]]::new()
        foreach ($generationCount in @(100, 1000, 10000, 100000)) {
            $startupPath = Join-Path $runDirectory ("startup-{0}.json" -f $generationCount)
            $startupEnvironment = $environment.Clone()
            $startupEnvironment.SKY_RT_R3_RUN_ID = "$runId-startup-$generationCount"
            $startupCommand = Invoke-CapturedCommand -Name $probePath -Arguments @(
                '--mode', 'startup',
                '--generation-count', [string]$generationCount,
                '--seed', '1379160116',
                '--output', $startupPath
            ) -Label "diagnostics-startup-$generationCount" -Environment $startupEnvironment -CaptureWorkingSet
            $startupReport = Get-Content -LiteralPath $startupPath -Raw | ConvertFrom-Json
            $startupCase = $startupReport.test_vectors.startup_case
            $startupBudget = $startupCase.startup_budget
            $expectedWouldAdmit = [long]$startupBudget.measured_worker_path_us -le [long]$startupBudget.captured_available_startup_budget_us
            $expectedAdmissionOutcome = if ($expectedWouldAdmit) { 'WOULD_ADMIT_WITHIN_SCOPED_MIN_PREROLL_BUDGET' } else { 'WOULD_REJECT_SCOPED_MIN_PREROLL_BUDGET' }
            if ($startupReport.source_revision.ToLowerInvariant() -ne $headSha -or
                $startupReport.runtime_revision.ToLowerInvariant() -ne $runtimeRevision -or
                $startupReport.mode -ne 'startup' -or
                $startupCase.dataset_id -ne 'r3-p4-startup-synthetic-v1' -or
                $startupCase.generation_count -ne $generationCount -or
                $startupCase.action_count -ne (2 * $generationCount) -or
                $startupCase.below_existing_action_limit -ne $true -or
                $startupBudget.would_admit_prepared_path -ne $expectedWouldAdmit -or
                $startupBudget.prepared_path_admission_outcome -ne $expectedAdmissionOutcome -or
                $startupBudget.production_startup_attempted -ne $false -or
                $startupBudget.production_startup_rejection_observed -ne $false -or
                $startupCase.prepared_storage.prepared_entry_count -lt 1) {
                throw "BLOCKED-DIAGNOSTICS: startup dataset or provenance mismatch at $generationCount generations"
            }
            $startupRecords.Add([ordered]@{
                run_id = $startupReport.run_id
                case = $startupCase
                process_working_set = [ordered]@{
                    start_bytes = $startupCommand.working_set_start_bytes
                    peak_bytes = $startupCommand.working_set_peak_bytes
                    sample_count = $startupCommand.working_set_sample_count
                    sample_interval_ms = $startupCommand.working_set_sample_interval_ms
                    scope = $startupCommand.working_set_scope
                }
                command_label = $startupCommand.label
            })
        }

        $pollingPath = Join-Path $runDirectory 'polling.json'
        $pollingEnvironment = $environment.Clone()
        $pollingEnvironment.SKY_RT_R3_RUN_ID = "$runId-polling"
        $pollingCommand = Invoke-CapturedCommand -Name $probePath -Arguments @('--mode', 'polling', '--output', $pollingPath) -Label 'diagnostics-polling' -Environment $pollingEnvironment -CaptureWorkingSet
        $pollingReport = Get-Content -LiteralPath $pollingPath -Raw | ConvertFrom-Json
        $polling = $pollingReport.test_vectors.frame_polling
        if ($pollingReport.source_revision.ToLowerInvariant() -ne $headSha -or
            $pollingReport.runtime_revision.ToLowerInvariant() -ne $runtimeRevision -or
            $pollingReport.mode -ne 'polling' -or
            $polling.scenario_id -ne 'r3-p4-frame-polling-fixed-v1' -or
            $polling.matrix_case_count -ne 1620 -or
            $polling.supplemental_17_167us_reference_case_count -ne 6 -or
            $polling.rows.Count -ne 1626 -or
            $polling.short_hold_missed_down_count -lt 1) {
            throw 'BLOCKED-DIAGNOSTICS: receiver polling fixed dataset, provenance, or missed-key visibility oracle failed'
        }
        if (@($polling.rows | Where-Object { $_.evidence_label -eq 'MODELED_POLLING_ONLY' -and ($_.sky_consumption -ne 'UNKNOWN') }).Count -ne 0) {
            throw 'BLOCKED-DIAGNOSTICS: polling model mislabeled unknown Sky consumption'
        }

        $diagnosticsJson = [ordered]@{
            schema_version = 1
            status = 'COMPLETED_CHARACTERIZATION'
            stage = 'diagnostics'
            run_id = $runId
            source_revision = $headSha
            runtime_revision = $runtimeRevision
            source_tree_clean = $sourceTreeClean
            base_revision = $runtimeRevision
            executable_sha256 = $probeHash
            host = $hostMetadata
            toolchain = [ordered]@{ rustc = $rustcVersion; cargo = $cargoVersion; bun = $bunVersion }
            dataset = [ordered]@{
                hol_id = $hol.scenario_id
                startup_id = 'r3-p4-startup-synthetic-v1'
                startup_seed = 1379160116
                startup_generation_counts = @(100, 1000, 10000, 100000)
                polling_id = $polling.scenario_id
                polling_matrix_cases = $polling.matrix_case_count
                short_hold_reference_cases = $polling.supplemental_17_167us_reference_case_count
            }
            hol = $hol
            startup = @($startupRecords)
            polling = $polling
            process_working_set = [ordered]@{
                hol = [ordered]@{ start_bytes = $holCommand.working_set_start_bytes; peak_bytes = $holCommand.working_set_peak_bytes; sample_count = $holCommand.working_set_sample_count; interval_ms = $holCommand.working_set_sample_interval_ms; scope = $holCommand.working_set_scope }
                polling = [ordered]@{ start_bytes = $pollingCommand.working_set_start_bytes; peak_bytes = $pollingCommand.working_set_peak_bytes; sample_count = $pollingCommand.working_set_sample_count; interval_ms = $pollingCommand.working_set_sample_interval_ms; scope = $pollingCommand.working_set_scope }
            }
            native_full_rerun = $false
            paired_ab_rerun = $false
            command_count = $commandResults.Count
            commands = @($commandResults)
            disposition = 'HOL, startup cost, and receiver polling limits are reported as characterization findings; no scheduler/runtime policy change is authorized.'
        }
        $diagnosticsJsonPath = Join-Path $runDirectory 'diagnostics.json'
        Write-JsonFile $diagnosticsJsonPath $diagnosticsJson

        $csvRows = [System.Collections.Generic.List[object]]::new()
        foreach ($case in $hol.cases) {
            foreach ($row in $case.rows) {
                $csvRows.Add([ordered]@{
                    section = 'hol'
                    case_id = $row.case_id
                    scenario = $case.scenario
                    packet_index = $row.packet_index
                    authored_target_us = $row.authored_target_us
                    ready_floor_us = $row.ready_floor_us
                    actual_pre_us = $row.actual_pre_us
                    actual_complete_us = $row.actual_complete_us
                    blocking_packet = $row.blocking_packet
                    blocking_keymask = $row.blocking_keymask
                    blocking_reason = $row.blocking_reason
                    delay_due_to_earlier_packet_us = $row.delay_due_to_earlier_packet_us
                    sender_duration_us = $row.actual_sender_duration_us
                    physical_floor_violation = $row.physical_floor_violation
                    down_replay = $case.down_replay
                    transport_attempts = $row.transport_attempts
                    activated_generations = $row.accounting.activated
                    released_generations = $row.accounting.released
                    active_generations = $row.accounting.active
                    release_obligation_mask = $case.release_obligation_mask
                    generation_count = $null
                    action_count = $null
                    prepare_wall_us = $null
                    allocation_calls = $null
                    requested_allocation_bytes = $null
                    compile_wall_us = $null
                    compile_allocation_calls = $null
                    compile_allocation_bytes = $null
                    schedule_clone_wall_us = $null
                    schedule_clone_allocation_calls = $null
                    schedule_clone_allocation_bytes = $null
                    coordinator_init_wall_us = $null
                    coordinator_allocation_calls = $null
                    coordinator_allocation_bytes = $null
                    coordinator_clone_wall_us = $null
                    coordinator_clone_allocation_calls = $null
                    coordinator_clone_allocation_bytes = $null
                    prepared_stream_wall_us = $null
                    prepared_stream_allocation_calls = $null
                    prepared_stream_allocation_bytes = $null
                    prepared_entry_count = $null
                    prepared_entry_array_bytes = $null
                    rebuilt_coordinator_heap_payload_bytes = $null
                    process_working_set_start_bytes = $null
                    process_working_set_peak_bytes = $null
                    startup_budget_us = $null
                    admission_outcome = $null
                    fps = $null
                    margin_us = $null
                    poll_phase = $null
                    receiver_stall_frames = $null
                    delivery_skew_down_us = $null
                    delivery_skew_up_us = $null
                    down_state_observed = $null
                    separating_up_state_observed = $null
                    evidence_label = 'DETERMINISTIC_PREPARED_DISPATCH_MOCK'
                })
            }
        }
        foreach ($record in $startupRecords) {
            $case = $record.case
            $csvRows.Add([ordered]@{
                section = 'startup'
                case_id = $case.dataset_id
                scenario = 'fixed feasible generation schedule'
                packet_index = $null
                authored_target_us = $null
                ready_floor_us = $null
                actual_pre_us = $null
                actual_complete_us = $null
                blocking_packet = $null
                blocking_keymask = $null
                blocking_reason = $null
                delay_due_to_earlier_packet_us = $null
                sender_duration_us = $null
                physical_floor_violation = $null
                down_replay = $null
                transport_attempts = $null
                activated_generations = $null
                released_generations = $null
                active_generations = $null
                release_obligation_mask = $null
                generation_count = $case.generation_count
                action_count = $case.action_count
                prepare_wall_us = $case.prepared_stream_build_and_coordinator_reset.wall_us
                allocation_calls = $case.prepared_stream_build_and_coordinator_reset.allocation_calls
                requested_allocation_bytes = $case.prepared_stream_build_and_coordinator_reset.requested_allocation_bytes
                compile_wall_us = $case.schedule_compile_and_materialize.wall_us
                compile_allocation_calls = $case.schedule_compile_and_materialize.allocation_calls
                compile_allocation_bytes = $case.schedule_compile_and_materialize.requested_allocation_bytes
                schedule_clone_wall_us = $case.coordinator_input_schedule_clone.wall_us
                schedule_clone_allocation_calls = $case.coordinator_input_schedule_clone.allocation_calls
                schedule_clone_allocation_bytes = $case.coordinator_input_schedule_clone.requested_allocation_bytes
                coordinator_init_wall_us = $case.coordinator_initialization.wall_us
                coordinator_allocation_calls = $case.coordinator_initialization.allocation_calls
                coordinator_allocation_bytes = $case.coordinator_initialization.requested_allocation_bytes
                coordinator_clone_wall_us = $case.coordinator_clone_for_characterization.wall_us
                coordinator_clone_allocation_calls = $case.coordinator_clone_for_characterization.allocation_calls
                coordinator_clone_allocation_bytes = $case.coordinator_clone_for_characterization.requested_allocation_bytes
                prepared_stream_wall_us = $case.prepared_stream_build_and_coordinator_reset.wall_us
                prepared_stream_allocation_calls = $case.prepared_stream_build_and_coordinator_reset.allocation_calls
                prepared_stream_allocation_bytes = $case.prepared_stream_build_and_coordinator_reset.requested_allocation_bytes
                prepared_entry_count = $case.prepared_storage.prepared_entry_count
                prepared_entry_array_bytes = $case.prepared_storage.prepared_entry_array_bytes
                rebuilt_coordinator_heap_payload_bytes = $case.prepared_storage.rebuilt_coordinator_heap_payload_bytes
                process_working_set_start_bytes = $record.process_working_set.start_bytes
                process_working_set_peak_bytes = $record.process_working_set.peak_bytes
                startup_budget_us = $case.startup_budget.captured_available_startup_budget_us
                admission_outcome = $case.startup_budget.prepared_path_admission_outcome
                fps = $null
                margin_us = $null
                poll_phase = $null
                receiver_stall_frames = $null
                delivery_skew_down_us = $null
                delivery_skew_up_us = $null
                down_state_observed = $null
                separating_up_state_observed = $null
                evidence_label = 'PROBE_ALLOCATOR_AND_RUNNER_WORKING_SET'
            })
        }
        foreach ($row in $polling.rows) {
            $csvRows.Add([ordered]@{
                section = 'polling'
                case_id = $row.case_id
                scenario = $row.hold_policy
                packet_index = $null
                authored_target_us = $null
                ready_floor_us = $null
                actual_pre_us = $null
                actual_complete_us = $null
                blocking_packet = $null
                blocking_keymask = $null
                blocking_reason = $null
                delay_due_to_earlier_packet_us = $null
                sender_duration_us = $null
                physical_floor_violation = $null
                down_replay = $null
                transport_attempts = $null
                activated_generations = $null
                released_generations = $null
                active_generations = $null
                release_obligation_mask = $null
                generation_count = $null
                action_count = $null
                prepare_wall_us = $null
                allocation_calls = $null
                requested_allocation_bytes = $null
                compile_wall_us = $null
                compile_allocation_calls = $null
                compile_allocation_bytes = $null
                schedule_clone_wall_us = $null
                schedule_clone_allocation_calls = $null
                schedule_clone_allocation_bytes = $null
                coordinator_init_wall_us = $null
                coordinator_allocation_calls = $null
                coordinator_allocation_bytes = $null
                coordinator_clone_wall_us = $null
                coordinator_clone_allocation_calls = $null
                coordinator_clone_allocation_bytes = $null
                prepared_stream_wall_us = $null
                prepared_stream_allocation_calls = $null
                prepared_stream_allocation_bytes = $null
                prepared_entry_count = $null
                prepared_entry_array_bytes = $null
                rebuilt_coordinator_heap_payload_bytes = $null
                process_working_set_start_bytes = $null
                process_working_set_peak_bytes = $null
                startup_budget_us = $null
                admission_outcome = $null
                fps = $row.fps
                margin_us = $row.margin_us
                poll_phase = $row.poll_phase
                receiver_stall_frames = $row.receiver_stall_frames
                delivery_skew_down_us = $row.down_delivery_skew_us
                delivery_skew_up_us = $row.up_delivery_skew_us
                down_state_observed = $row.down_state_observed
                separating_up_state_observed = $row.separating_up_state_observed
                evidence_label = $row.evidence_label
            })
        }
        $csvPath = Join-Path $runDirectory 'diagnostics.csv'
        $csvText = @($csvRows | ForEach-Object { [pscustomobject]$_ } | ConvertTo-Csv -NoTypeInformation)
        [System.IO.File]::WriteAllText($csvPath, [string]::Join([Environment]::NewLine, [string[]]$csvText) + [Environment]::NewLine, $utf8NoBom)

        $markdownLines = [System.Collections.Generic.List[string]]::new()
        $markdownLines.Add('# R3 P4 diagnostics')
        $markdownLines.Add('')
        $markdownLines.Add("Run: $runId")
        $markdownLines.Add("Source revision: $headSha")
        $markdownLines.Add("Runtime/base revision: $runtimeRevision")
        $markdownLines.Add("Toolchain: $rustcVersion; $cargoVersion; Bun $bunVersion")
        $markdownLines.Add('Transport: deterministic test-support mock for HOL; no SendInput or game process.')
        $markdownLines.Add('')
        $markdownLines.Add('## Head-of-line and prepared packet scenarios')
        $markdownLines.Add('')
        $markdownLines.Add('| Scenario | Packet | Authored target (us) | Ready floor (us) | Actual pre-call (us) | Completion (us) | Sender duration (us) | Blocking packet/keymask | Delay due to earlier packet (us) | Floor violation | Attempts |')
        $markdownLines.Add('| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | ---: | --- | ---: |')
        foreach ($case in $hol.cases) {
            foreach ($row in $case.rows) {
                $blockerText = if ($null -eq $row.blocking_packet) { 'none' } else { "$($row.blocking_packet)/$($row.blocking_keymask)" }
                $markdownLines.Add("| $($case.case_id) | $($row.packet_index) | $($row.authored_target_us) | $($row.ready_floor_us) | $($row.actual_pre_us) | $($row.actual_complete_us) | $($row.actual_sender_duration_us) | $blockerText | $($row.delay_due_to_earlier_packet_us) | $($row.physical_floor_violation) | $($row.transport_attempts) |")
            }
        }
        $markdownLines.Add('')
        $markdownLines.Add('## Startup and memory')
        $markdownLines.Add('')
        $markdownLines.Add('| Generations | Actions | Compile/materialize (us) | Schedule clone (us) | Coordinator init (us) | Coordinator clone (us; test-support only) | Prepared build/reset (us) | Prepared entries | Build allocations (count / requested bytes) | Peak process working set (bytes) | Available budget (us) | Prepared-path budget outcome |')
        $markdownLines.Add('| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | ---: | ---: | --- |')
        foreach ($record in $startupRecords) {
            $case = $record.case
            $markdownLines.Add("| $($case.generation_count) | $($case.action_count) | $($case.schedule_compile_and_materialize.wall_us) | $($case.coordinator_input_schedule_clone.wall_us) | $($case.coordinator_initialization.wall_us) | $($case.coordinator_clone_for_characterization.wall_us) | $($case.prepared_stream_build_and_coordinator_reset.wall_us) | $($case.prepared_storage.prepared_entry_count) | $($case.prepared_stream_build_and_coordinator_reset.allocation_calls) / $($case.prepared_stream_build_and_coordinator_reset.requested_allocation_bytes) | $($record.process_working_set.peak_bytes) | $($case.startup_budget.captured_available_startup_budget_us) | $($case.startup_budget.prepared_path_admission_outcome) |")
        }
        $markdownLines.Add('')
        $markdownLines.Add('Allocation counts/bytes are successful allocation and realloc requests in the probe process during each named operation; they exclude deallocation and allocator metadata. Process working set is sampled separately by PowerShell every 5 ms and includes shared resident pages. Neither metric is inferred from the other. Schedule compile/materialize is separate from coordinator initialization and prepared-stream build. The scoped budget comparison includes coordinator initialization plus prepared build/reset, and excludes other worker startup work.')
        $markdownLines.Add('')
        $markdownLines.Add('The RuntimeSchedule clone that supplies the coordinator its owned schedule is measured separately. RuntimeDispatchCoordinator cloning exists only in test-support for characterization; the deep clone column is not part of production startup. Actual coordinator initialization and post-preparation reconstruction are measured in their own columns.')
        $markdownLines.Add('The prepared-path budget outcome is a scoped characterization result: an out-of-budget measurement is labeled WOULD_REJECT. No production boot was attempted, so this does not claim an observed product rejection or model excluded startup work.')
        $markdownLines.Add('')
        $markdownLines.Add('## Offline receiver frame-polling model')
        $markdownLines.Add('')
        $markdownLines.Add('| FPS | Margin (us) | Cases | Missed Down states | Missed separating Up states |')
        $markdownLines.Add('| ---: | ---: | ---: | ---: | ---: |')
        foreach ($fps in @(30, 60, 120, 240)) {
            foreach ($margin in @(0, 500, 3000)) {
                $group = @($polling.rows | Where-Object { $_.evidence_label -eq 'MODELED_POLLING_ONLY' -and $_.fps -eq $fps -and $_.margin_us -eq $margin })
                $missedDown = @($group | Where-Object { -not $_.down_state_observed }).Count
                $missedUp = @($group | Where-Object { -not $_.separating_up_state_observed }).Count
                $markdownLines.Add("| $fps | $margin | $($group.Count) | $missedDown | $missedUp |")
            }
        }
        $markdownLines.Add('')
        $markdownLines.Add('### 17,167 us short-hold reference cases')
        $markdownLines.Add('')
        $markdownLines.Add('| Poll phase | Receiver stall (frames) | Down observed | Separating Up observed |')
        $markdownLines.Add('| --- | ---: | --- | --- |')
        foreach ($row in @($polling.rows | Where-Object { $_.evidence_label -eq 'MODELED_SHORT_HOLD_REFERENCE' })) {
            $markdownLines.Add("| $($row.poll_phase) | $($row.receiver_stall_frames) | $($row.down_state_observed) | $($row.separating_up_state_observed) |")
        }
        $markdownLines.Add('')
        $markdownLines.Add('The polling model is offline. It reports state samples for all fixed phases, stalls, and independent Down/Up delivery skews. Actual ReceiveOnly sink observations are a separate evidence layer; Sky consumption remains unknown. DWM or monitor refresh rate is not treated as game polling.')
        $markdownLines.Add('')
        $markdownLines.Add('## Fixed disposition')
        $markdownLines.Add('')
        $markdownLines.Add('Head-of-line blocking, overdue bursts, startup cost, and long-hold receiver limitations remain documented characterization findings. No scheduler rewrite, storage optimization, timing/policy change, or new receiver guarantee is proposed.')
        $markdownPath = Join-Path $runDirectory 'diagnostics.md'
        [System.IO.File]::WriteAllText($markdownPath, [string]::Join([Environment]::NewLine, [string[]]$markdownLines) + [Environment]::NewLine, $utf8NoBom)

        $summary = [ordered]@{
            schema_version = 1
            status = 'COMPLETED_CHARACTERIZATION'
            stage = 'diagnostics'
            run_id = $runId
            source_revision = $headSha
            runtime_revision = $runtimeRevision
            base_revision = $runtimeRevision
            source_tree_clean = $sourceTreeClean
            executable_sha256 = $probeHash
            host = $hostMetadata
            hol_case_count = $hol.cases.Count
            hol_physical_packet_rows = $hol.rows.Count
            hol_physical_floor_violation_count = $hol.physical_floor_violation_count
            hol_down_replay_case_count = $hol.down_replay_case_count
            startup_generation_counts = @($startupRecords | ForEach-Object { $_.case.generation_count })
            startup_admission_outcomes = @($startupRecords | ForEach-Object { $_.case.startup_budget.prepared_path_admission_outcome })
            polling_matrix_cases = $polling.matrix_case_count
            short_hold_reference_cases = $polling.supplemental_17_167us_reference_case_count
            short_hold_missed_down_count = $polling.short_hold_missed_down_count
            native_full_rerun = $false
            paired_ab_rerun = $false
            output_files = @('diagnostics.json', 'diagnostics.csv', 'diagnostics.md', 'hol.json', 'polling.json', 'startup-100.json', 'startup-1000.json', 'startup-10000.json', 'startup-100000.json')
            child_command_count = $commandResults.Count
            child_commands = @($commandResults)
            disposition = 'READY FOR COORDINATOR REVIEW; NEXT PHASE NOT STARTED'
        }
        Write-JsonFile (Join-Path $runDirectory 'summary.json') $summary
        Write-JsonFile (Join-Path $runDirectory 'status.json') ([ordered]@{ status = 'COMPLETED_CHARACTERIZATION'; stage = 'diagnostics'; run_id = $runId; finished_utc = [DateTimeOffset]::UtcNow.ToString('O') })
        $declaredFiles = [System.Collections.Generic.List[object]]::new()
        foreach ($file in Get-ChildItem -LiteralPath $runDirectory -File -Recurse | Sort-Object FullName) {
            if ($file.Name -eq 'runner-outputs.json') { continue }
            $relative = [System.IO.Path]::GetRelativePath($runDirectory, $file.FullName).Replace('\', '/')
            $declaredFiles.Add([ordered]@{ path = $relative; size_bytes = $file.Length; sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() })
        }
        $outputsManifest = [ordered]@{
            schema_version = 1
            status = 'COMPLETED_CHARACTERIZATION'
            stage = 'diagnostics'
            run_id = $runId
            source_revision = $headSha
            runtime_revision = $runtimeRevision
            executable_sha256 = $probeHash
            declared_outputs = @($declaredFiles)
        }
        Write-JsonFile (Join-Path $runDirectory 'runner-outputs.json') $outputsManifest
        Write-Output (ConvertTo-Json -InputObject ([ordered]@{ status = $outputsManifest.status; stage = 'diagnostics'; run_id = $runId; run_directory = $runDirectory; source_revision = $headSha; runtime_revision = $runtimeRevision; hol_rows = $hol.rows.Count; startup_cases = $startupRecords.Count; polling_cases = $polling.rows.Count; child_commands = $commandResults.Count }) -Compress)
        return
    }

    $controlProbePath = $null
    $controlProbeHash = $null
    if ($isP2Comparison) {
        $controlWorktree = Join-Path $env:TEMP ('rt-r3-p2-control-' + [guid]::NewGuid().ToString('N'))
        [void](Invoke-CapturedCommand -Name 'git.exe' -Arguments @('-C', $repoRoot, 'worktree', 'add', '--detach', $controlWorktree, $controlRevision) -Label 'create-control-worktree')
        $controlWorktreeCreated = $true
        $controlTarget = Join-Path $controlWorktree 'rust\target'
        $controlBuildEnvironment = @{
            CARGO_TARGET_DIR = $controlTarget
            SKY_RT_R3_RUNTIME_REVISION = $controlRevision
        }
        [void](Invoke-CapturedCommand -Name 'cargo.exe' -Arguments @('build', '--locked', '--manifest-path', 'rust/Cargo.toml', '-p', 'sky_player', '--release', '--features', 'test-support', '--example', 'rt_r3_probe') -Label 'build-p1-control-probe' -WorkingDirectory $controlWorktree -Environment $controlBuildEnvironment)
        $controlStatus = @(& git -C $controlWorktree status --porcelain --untracked-files=all)
        if ($LASTEXITCODE -ne 0 -or $controlStatus.Count -ne 0) { throw 'BLOCKED-CONTROL: accepted P1 control worktree was not clean after its build' }
        $controlProbePath = Join-Path $controlTarget 'release\examples\rt_r3_probe.exe'
        if (-not (Test-Path -LiteralPath $controlProbePath -PathType Leaf)) { throw 'BLOCKED-CONTROL: accepted P1 release probe binary was not produced' }
        $controlProbeHash = (Get-FileHash -LiteralPath $controlProbePath -Algorithm SHA256).Hash.ToLowerInvariant()
    }

    if ($Stage -eq ('phase' + '1')) {
        $contractPath = Join-Path $runDirectory 'contracts.json'
        $contractRunId = "$runId-contracts"
        $contractEnvironment = $environment.Clone()
        $contractEnvironment.SKY_RT_R3_RUN_ID = $contractRunId
        $contractEnvironment.SKY_RT_R3_CONTENTION_WORKERS = '0'
        [void](Invoke-CapturedCommand -Name $probePath -Arguments @('--mode', 'contracts', '--load-mode', 'quiet', '--output', $contractPath) -Label 'contracts' -Environment $contractEnvironment)
        $contractReport = Get-Content -LiteralPath $contractPath -Raw | ConvertFrom-Json
        $vectorA = $contractReport.test_vectors.A_same_key_physical_floor_pause_resume
        $vectorCases = @($vectorA.vector_cases)
        if ($contractReport.source_revision.ToLowerInvariant() -ne $headSha -or
            $contractReport.runtime_revision.ToLowerInvariant() -ne $runtimeRevision -or
            $contractReport.profile -ne 'release' -or
            $contractReport.transport_kind -ne 'deterministic-mock' -or
            $contractReport.run_id -ne $contractRunId -or
            $vectorA.hypothesis_status -ne 'NO-REPRODUCTION' -or
            $vectorA.characterization_integrity_passed -ne $true -or
            $vectorA.normal_gap_contract_passed -ne $true -or
            $vectorCases.Count -ne 3 -or
            $contractReport.test_vectors.E_empty_cleanup.oracle_passed -ne $true -or
            $contractReport.test_vectors.E_empty_cleanup.keyboard_transport_calls -ne 0) {
            throw 'BLOCKED-CONTRACT: fixed lifecycle vector or empty-cleanup contract failed qualification'
        }
        $expectedFrequencies = @(1000000, 10000000, 10000003)
        for ($caseIndex = 0; $caseIndex -lt $vectorCases.Count; $caseIndex++) {
            $case = $vectorCases[$caseIndex]
            $frequency = [long]$case.qpc_frequency_hz
            $frameTicks = [long][Math]::Ceiling([double]$frequency * 16667 / 1000000)
            $upCompletion = [long]$case.events.musical_up.completion.qpc_ticks
            $pauseQpc = [long]$case.events.pause_qpc.qpc_ticks
            $requiredDown = $upCompletion + $frameTicks
            $postCleanupBarrier = $pauseQpc + $frameTicks
            $downFloor = [long]$case.production_down_floor.qpc_ticks
            if ($frequency -ne $expectedFrequencies[$caseIndex] -or
                $case.normal_gap_contract_passed -ne $true -or
                $case.below_completion_plus_frame -ne $false -or
                $case.due_seam_integrity_passed -ne $true -or
                $case.cleanup_transport_sends -ne 0 -or
                $downFloor -lt $requiredDown -or
                $downFloor -lt $postCleanupBarrier -or
                [long]$case.lifecycle_snapshots.before_next_down.post_cleanup_recomputed_floor.qpc_ticks -ne $downFloor -or
                [long]$case.events.redown.pre_call.qpc_ticks -ne $downFloor -or
                $case.lifecycle_snapshots.before_next_down.dispatch_step_at_resume -ne 'None' -or
                $case.lifecycle_snapshots.before_next_down.current_floor_was_due_at_resume -ne $false -or
                $case.lifecycle_snapshots.before_next_down.not_due_without_send_or_advance -ne $true -or
                $case.conversion.effective_hold_h_us -ne 17167 -or
                $case.conversion.min_release_gap_us -ne 17167 -or
                $case.conversion.authored_offsets_us.Count -ne 4 -or
                $case.final_generation_accounting.activated -ne 2 -or
                $case.final_generation_accounting.released -ne 2 -or
                $case.final_release_obligation_mask -ne 0) {
                throw "BLOCKED-CONTRACT: lifecycle vector failed at frequency $frequency"
            }
        }
        if ($contractReport.test_vectors.C_serial_HOL.timing_config.native_admission -ne 'PASS' -or
            $contractReport.test_vectors.C_serial_HOL.timing_config.frame_gap_g_us -ne 16667 -or
            $contractReport.test_vectors.C_serial_HOL.timing_config.effective_hold_h_us -ne 17167 -or
            $contractReport.test_vectors.C_serial_HOL.timing_config.min_release_gap_us -ne 17167 -or
            $contractReport.test_vectors.D_focus_loss_policy.restore_path.release_on_restore -ne $true) {
            throw 'BLOCKED-CONTRACT: retained H/G or focus-restoration controls failed qualification'
        }

        [void](Invoke-CapturedCommand -Name 'cargo.exe' -Arguments @('build', '--locked', '--manifest-path', 'rust/Cargo.toml', '-p', 'sky_player', '--release', '--features', 'real-input-acceptance,test-support', '--bin', 'rt-native-acceptance') -Label 'build-native-acceptance')
        [void](Invoke-CapturedCommand -Name 'pwsh.exe' -Arguments @('-NoProfile', '-File', 'scripts/native_acceptance_sink.ps1', '-SelfTest') -Label 'native-sink-selftest')
        $nativeOutputRoot = Join-Path $repoRoot '.benchmarks\physical-native-cases'
        $nativeResults = [System.Collections.Generic.List[object]]::new()
        $nativeObservedEventCount = [long]0
        foreach ($scenario in @('pause-resume', 'suspend-resume', 'focus-loss', 'near-minimum-retrigger', 'mixed-up-down', 'stop-cleanup')) {
            $beforeNativeRuns = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
            if (Test-Path -LiteralPath $nativeOutputRoot -PathType Container) {
                foreach ($directory in Get-ChildItem -LiteralPath $nativeOutputRoot -Directory) { [void]$beforeNativeRuns.Add($directory.FullName) }
            }
            $nativeCommand = Invoke-CapturedCommand -Name 'pwsh.exe' -Arguments @('-NoProfile', '-File', 'scripts/run_native_acceptance_case.ps1', '-Scenario', $scenario, '-TimingMarginUs', '500') -Label "native-$scenario"
            $newNativeRuns = @()
            if (Test-Path -LiteralPath $nativeOutputRoot -PathType Container) {
                $newNativeRuns = @(Get-ChildItem -LiteralPath $nativeOutputRoot -Directory | Where-Object { -not $beforeNativeRuns.Contains($_.FullName) })
            }
            if ($newNativeRuns.Count -ne 1) { throw "BLOCKED-NATIVE: could not identify one fresh $scenario evidence directory" }
            $nativeRunDirectory = $newNativeRuns[0].FullName
            $nativeConfig = Get-Content -LiteralPath (Join-Path $nativeRunDirectory 'run-config.json') -Raw | ConvertFrom-Json
            $nativeInvocation = Get-Content -LiteralPath (Join-Path $nativeRunDirectory 'invocation.json') -Raw | ConvertFrom-Json
            $nativeReportPath = Join-Path $nativeRunDirectory 'rt-native-acceptance.jsonl'
            $nativeReportLines = @(Get-Content -LiteralPath $nativeReportPath)
            if ($nativeReportLines.Count -ne 1) { throw "BLOCKED-NATIVE: $scenario did not emit one native report" }
            $nativeReport = $nativeReportLines[0] | ConvertFrom-Json
            if ($nativeCommand.exit_code -ne 0 -or
                $nativeConfig.source_revision.ToLowerInvariant() -ne $headSha -or
                $nativeConfig.source_tree_clean -ne $true -or
                $nativeConfig.scenario -ne $scenario -or
                $nativeConfig.timing_margin_us -ne 500 -or
                $nativeInvocation.exit_code -ne 0 -or
                $nativeInvocation.status -ne 'PASS' -or
                $nativeReport.run_id -ne $nativeConfig.run_id -or
                $nativeReport.scenario -ne $scenario -or
                $nativeReport.timing_margin_us -ne 500 -or
                $nativeReport.status -ne 'PASS') {
                throw "BLOCKED-NATIVE: $scenario failed source, margin, or ReceiveOnly acceptance checks"
            }
            $nativeDestination = Join-Path (Join-Path $runDirectory 'native') $scenario
            [System.IO.Directory]::CreateDirectory($nativeDestination) | Out-Null
            foreach ($child in Get-ChildItem -LiteralPath $nativeRunDirectory -Force) {
                Copy-Item -LiteralPath $child.FullName -Destination $nativeDestination -Recurse
            }
            $nativeObservedEventCount += [long]$nativeInvocation.observed_event_count
            $nativeResults.Add([ordered]@{
                scenario = $scenario
                timing_margin_us = 500
                status = $nativeReport.status
                run_id = $nativeConfig.run_id
                command_label = $nativeCommand.label
                evidence_directory = "native/$scenario"
                source_run_id = $newNativeRuns[0].Name
                executable_sha256 = $nativeConfig.harness_sha256
                observed_event_count = $nativeInvocation.observed_event_count
                expected_event_count = $nativeInvocation.expected_event_count
            })
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
            transport_kind = 'deterministic-mock contract plus real SendInput to the project-owned ReceiveOnly sink'
            contracts = [ordered]@{
                fixed_lifecycle_vector_status = $vectorA.hypothesis_status
                fixed_lifecycle_vector_integrity = $vectorA.characterization_integrity_passed
                fixed_lifecycle_normal_gap = $vectorA.normal_gap_contract_passed
                qpc_frequencies_hz = $expectedFrequencies
                empty_cleanup_oracle = $contractReport.test_vectors.E_empty_cleanup.oracle_passed
                retained_H_us = 17167
                retained_G_us = 16667
            }
            native_controls = @($nativeResults)
            native_control_count = $nativeResults.Count
            native_observed_event_count = $nativeObservedEventCount
            native_sink_selftest = 'PASS'
            child_command_count = $commandResults.Count
            child_commands = @($commandResults)
            precision_run_count = 0
            statistical_sample_count = 0
            measurement_boundary = 'Six native scenario controls use the project-owned ReceiveOnly sink. Their event counts qualify scenario outcomes and are not timing-distribution samples. Deterministic contracts use the release test-support probe and mock transport.'
            receipt_boundary = 'Sink event observation does not prove Sky/game/audio receipt.'
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
        Write-Output (ConvertTo-Json -InputObject ([ordered]@{ status = $outputsManifest.status; run_id = $runId; run_directory = $runDirectory; source_revision = $headSha; runtime_revision = $runtimeRevision; native_controls = $nativeResults.Count; native_events = $summary.native_observed_event_count; child_commands = $commandResults.Count }) -Compress)
        return
    }

    if ($isP2Comparison) {
        $controlContractPath = Join-Path $runDirectory 'control-contracts.json'
        $candidateContractPath = Join-Path $runDirectory 'candidate-contracts.json'
        $controlContractEnvironment = $environment.Clone()
        $controlContractEnvironment.SKY_RT_R3_RUNTIME_REVISION = $controlRevision
        $controlContractEnvironment.SKY_RT_R3_RUN_ID = "$runId-control-contracts"
        $controlContractEnvironment.SKY_RT_R3_CONTENTION_WORKERS = '0'
        [void](Invoke-CapturedCommand -Name $controlProbePath -Arguments @('--mode', 'contracts', '--load-mode', 'quiet', '--output', $controlContractPath) -Label 'control-contracts' -WorkingDirectory $controlWorktree -Environment $controlContractEnvironment)
        $controlContractReport = Get-Content -LiteralPath $controlContractPath -Raw | ConvertFrom-Json
        $candidateContractEnvironment = $environment.Clone()
        $candidateContractEnvironment.SKY_RT_R3_RUNTIME_REVISION = $headSha
        $candidateContractEnvironment.SKY_RT_R3_RUN_ID = "$runId-candidate-contracts"
        $candidateContractEnvironment.SKY_RT_R3_CONTENTION_WORKERS = '0'
        [void](Invoke-CapturedCommand -Name $probePath -Arguments @('--mode', 'contracts', '--load-mode', 'quiet', '--output', $candidateContractPath) -Label 'candidate-contracts' -Environment $candidateContractEnvironment)
        $candidateContractReport = Get-Content -LiteralPath $candidateContractPath -Raw | ConvertFrom-Json
        $controlLifecycle = $controlContractReport.test_vectors.A_same_key_physical_floor_pause_resume
        $controlLeaseBaseline = $controlContractReport.test_vectors.B_enabled_lease_watchdog_delayed
        $candidateLifecycle = $candidateContractReport.test_vectors.A_same_key_physical_floor_pause_resume
        $candidateLeaseAge = $candidateContractReport.test_vectors.F_final_lease_age_proof
        if ($controlContractReport.source_revision.ToLowerInvariant() -ne $controlRevision -or
            $controlContractReport.runtime_revision.ToLowerInvariant() -ne $controlRevision -or
            $controlContractReport.run_id -ne "$runId-control-contracts" -or
            $controlContractReport.executable_sha256.ToLowerInvariant() -ne $controlProbeHash -or
            $controlContractReport.profile -ne 'release' -or
            $controlLeaseBaseline.hypothesis_status -ne 'REPRODUCED' -or
            $controlLeaseBaseline.expired_without_watchdog.disposition -ne 'STALE_LEASE_ACCEPTED' -or
            $controlLeaseBaseline.expired_without_watchdog.musical_sender_attempts -ne 1 -or
            $controlLeaseBaseline.timeout_equality.disposition -ne 'AT_TIMEOUT_ALLOWED' -or
            $controlLeaseBaseline.prior_fresh_progress.disposition -ne 'FRESH_PROGRESS_ALLOWED' -or
            $controlLifecycle.hypothesis_status -ne 'NO-REPRODUCTION' -or
            $controlLifecycle.characterization_integrity_passed -ne $true -or
            $controlLifecycle.normal_gap_contract_passed -ne $true) {
            throw 'BLOCKED-CONTROL: accepted P1 control did not reproduce the unlatched overdue lease or preserve the merged P1 lifecycle floor'
        }
        if ($candidateContractReport.source_revision.ToLowerInvariant() -ne $headSha -or
            $candidateContractReport.runtime_revision.ToLowerInvariant() -ne $headSha -or
            $candidateContractReport.run_id -ne "$runId-candidate-contracts" -or
            $candidateContractReport.executable_sha256.ToLowerInvariant() -ne $probeHash -or
            $candidateContractReport.profile -ne 'release' -or
            $candidateLifecycle.hypothesis_status -ne 'NO-REPRODUCTION' -or
            $candidateLifecycle.characterization_integrity_passed -ne $true -or
            $candidateLifecycle.normal_gap_contract_passed -ne $true -or
            $candidateLeaseAge.status -ne 'PASS' -or
            $candidateLeaseAge.overdue_passed -ne $true -or
            [long]$candidateLeaseAge.overdue_unlatched.elapsed_ticks -le [long]$candidateLeaseAge.overdue_unlatched.timeout_ticks -or
            $candidateLeaseAge.unproven_passed -ne $true -or
            $candidateLeaseAge.fresh_passed -ne $true -or
            $candidateContractReport.test_vectors.E_empty_cleanup.oracle_passed -ne $true) {
            throw 'BLOCKED-CONTRACT: candidate final lease-age vector or retained P1 lifecycle contract failed qualification'
        }

        [void](Invoke-CapturedCommand -Name 'cargo.exe' -Arguments @('build', '--locked', '--manifest-path', 'rust/Cargo.toml', '-p', 'sky_player', '--release', '--features', 'real-input-acceptance,test-support', '--bin', 'rt-native-acceptance') -Label 'build-native-acceptance')
        [void](Invoke-CapturedCommand -Name 'pwsh.exe' -Arguments @('-NoProfile', '-File', 'scripts/native_acceptance_sink.ps1', '-SelfTest') -Label 'native-sink-selftest')
        $nativeOutputRoot = Join-Path $repoRoot '.benchmarks\physical-native-cases'
        $nativeResults = [System.Collections.Generic.List[object]]::new()
        foreach ($scenario in @('supervisor-lease-expiry', 'hold', 'canonical-max-chord', 'mixed-up-down', 'modifier-held-after-owned')) {
            $beforeNativeRuns = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
            if (Test-Path -LiteralPath $nativeOutputRoot -PathType Container) {
                foreach ($directory in Get-ChildItem -LiteralPath $nativeOutputRoot -Directory) { [void]$beforeNativeRuns.Add($directory.FullName) }
            }
            $nativeCommand = Invoke-CapturedCommand -Name 'pwsh.exe' -Arguments @('-NoProfile', '-File', 'scripts/run_native_acceptance_case.ps1', '-Scenario', $scenario, '-TimingMarginUs', '500') -Label "native-$scenario"
            $newNativeRuns = @()
            if (Test-Path -LiteralPath $nativeOutputRoot -PathType Container) {
                $newNativeRuns = @(Get-ChildItem -LiteralPath $nativeOutputRoot -Directory | Where-Object { -not $beforeNativeRuns.Contains($_.FullName) })
            }
            if ($newNativeRuns.Count -ne 1) { throw "BLOCKED-NATIVE: could not identify one fresh $scenario evidence directory" }
            $nativeRunDirectory = $newNativeRuns[0].FullName
            $nativeConfig = Get-Content -LiteralPath (Join-Path $nativeRunDirectory 'run-config.json') -Raw | ConvertFrom-Json
            $nativeInvocation = Get-Content -LiteralPath (Join-Path $nativeRunDirectory 'invocation.json') -Raw | ConvertFrom-Json
            $nativeReportPath = Join-Path $nativeRunDirectory 'rt-native-acceptance.jsonl'
            $nativeReportLines = @(Get-Content -LiteralPath $nativeReportPath)
            if ($nativeReportLines.Count -ne 1) { throw "BLOCKED-NATIVE: $scenario did not emit one native report" }
            $nativeReport = $nativeReportLines[0] | ConvertFrom-Json
            if ($nativeCommand.exit_code -ne 0 -or
                $nativeConfig.source_revision.ToLowerInvariant() -ne $headSha -or
                $nativeConfig.source_tree_clean -ne $true -or
                $nativeConfig.scenario -ne $scenario -or
                $nativeConfig.timing_margin_us -ne 500 -or
                $nativeInvocation.exit_code -ne 0 -or
                $nativeInvocation.status -ne 'PASS' -or
                $nativeReport.run_id -ne $nativeConfig.run_id -or
                $nativeReport.scenario -ne $scenario -or
                $nativeReport.timing_margin_us -ne 500 -or
                $nativeReport.status -ne 'PASS') {
                throw "BLOCKED-NATIVE: $scenario failed source, margin, or ReceiveOnly acceptance checks"
            }
            $nativeDestination = Join-Path (Join-Path $runDirectory 'native') $scenario
            [System.IO.Directory]::CreateDirectory($nativeDestination) | Out-Null
            foreach ($child in Get-ChildItem -LiteralPath $nativeRunDirectory -Force) {
                Copy-Item -LiteralPath $child.FullName -Destination $nativeDestination -Recurse
            }
            $nativeResults.Add([ordered]@{
                scenario = $scenario
                timing_margin_us = 500
                status = $nativeReport.status
                run_id = $nativeConfig.run_id
                command_label = $nativeCommand.label
                evidence_directory = "native/$scenario"
                source_run_id = $newNativeRuns[0].Name
                executable_sha256 = $nativeConfig.harness_sha256
                observed_event_count = $nativeInvocation.observed_event_count
                expected_event_count = $nativeInvocation.expected_event_count
            })
        }

        $workloads = @('down-1', 'down-5', 'down-15', 'mixed-1x1', 'mixed-2x3', 'mixed-7x8', 'up-only-1', 'up-only-5', 'up-only-15')
        $loadModes = @('quiet', 'cpu_contention')
        $precisionRecords = [System.Collections.Generic.List[object]]::new()
        $precisionRounds = [System.Collections.Generic.List[object]]::new()
        $maximumRounds = 1
        for ($roundIndex = 1; $roundIndex -le $maximumRounds; $roundIndex++) {
            foreach ($workload in $workloads) {
                foreach ($loadMode in $loadModes) {
                    if ($loadMode -eq 'cpu_contention') { Start-ContentionWorkers }
                    try {
                        for ($pairIndex = 1; $pairIndex -le 5; $pairIndex++) {
                            if ($loadMode -eq 'cpu_contention' -and @($contentionProcesses | Where-Object { $_.HasExited }).Count -ne 0) {
                                throw 'A CPU contention worker exited during a paired performance group'
                            }
                            $order = if ($pairIndex % 2 -eq 1) { @('control', 'candidate') } else { @('candidate', 'control') }
                            $pairOrder = ($order | ForEach-Object { if ($_ -eq 'control') { 'A' } else { 'B' } }) -join ''
                            foreach ($arm in $order) {
                                $armCode = if ($arm -eq 'control') { 'A' } else { 'B' }
                                $armProbe = if ($arm -eq 'control') { $controlProbePath } else { $probePath }
                                $armWorkingDirectory = if ($arm -eq 'control') { $controlWorktree } else { $repoRoot }
                                $armRevision = if ($arm -eq 'control') { $controlRevision } else { $headSha }
                                $armHash = if ($arm -eq 'control') { $controlProbeHash } else { $probeHash }
                                $relativeDirectory = Join-Path (Join-Path (Join-Path 'precision' ("round-{0:D2}" -f $roundIndex)) $workload) $loadMode
                                $absoluteDirectory = Join-Path $runDirectory $relativeDirectory
                                [System.IO.Directory]::CreateDirectory($absoluteDirectory) | Out-Null
                                $relativePath = Join-Path $relativeDirectory ("pair-{0:D2}-{1}.json" -f $pairIndex, $armCode)
                                $reportPath = Join-Path $runDirectory $relativePath
                                $childRunId = "$runId-round$roundIndex-$workload-$loadMode-p$pairIndex-$armCode"
                                $runEnvironment = $environment.Clone()
                                $runEnvironment.SKY_RT_R3_RUNTIME_REVISION = $armRevision
                                $runEnvironment.SKY_RT_R3_RUN_ID = $childRunId
                                $runEnvironment.SKY_RT_R3_LOAD_MODE = $loadMode
                                $runEnvironment.SKY_RT_R3_CONTENTION_WORKERS = if ($loadMode -eq 'cpu_contention') { '2' } else { '0' }
                                $arguments = @('--mode', 'precision', '--load-mode', $loadMode, '--workload', $workload, '--warmup', '1000', '--measured', '10000', '--seed', '1073', '--run-index', [string]$pairIndex, '--output', $reportPath)
                                $command = Invoke-CapturedCommand -Name $armProbe -Arguments $arguments -Label "r$roundIndex-$workload-$loadMode-p$pairIndex-$armCode" -WorkingDirectory $armWorkingDirectory -Environment $runEnvironment
                                $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
                                $precisionRun = @($report.test_vectors.precision_runs)[0]
                                $expectedSetupRefreshes = if ($workload.StartsWith('mixed-') -or $workload.StartsWith('up-only-')) { 11000 } else { 0 }
                                $expectedFrameTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 16667 / 1000000)
                                $expectedMarginTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 500 / 1000000)
                                $expectedHoldTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 17167 / 1000000)
                                $expectedLeaseTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 3000000 / 1000000)
                                if ($report.schema_version -ne 1 -or
                                    $report.source_revision.ToLowerInvariant() -ne $armRevision -or
                                    $report.runtime_revision.ToLowerInvariant() -ne $armRevision -or
                                    $report.executable_sha256.ToLowerInvariant() -ne $armHash -or
                                    $report.profile -ne 'release' -or $report.transport_kind -ne 'deterministic-mock' -or
                                    $report.run_id -ne $childRunId -or $report.mode -ne 'precision' -or
                                    $report.load_mode -ne $loadMode -or
                                    $report.contention_workers -ne $(if ($loadMode -eq 'cpu_contention') { 2 } else { 0 }) -or
                                    $report.warmup_count -ne 1000 -or $report.measured_count -ne 10000 -or
                                    $precisionRun.r3_timing_config.frame_gap_g_us -ne 16667 -or
                                    $precisionRun.r3_timing_config.frame_gap_g_ticks -ne $expectedFrameTicks -or
                                    $precisionRun.r3_timing_config.frame_base_hold_us -ne 16667 -or
                                    $precisionRun.r3_timing_config.timing_margin_us -ne 500 -or
                                    $precisionRun.r3_timing_config.timing_margin_ticks -ne $expectedMarginTicks -or
                                    $precisionRun.r3_timing_config.effective_hold_h_us -ne 17167 -or
                                    $precisionRun.r3_timing_config.effective_hold_h_ticks -ne $expectedHoldTicks -or
                                    $precisionRun.r3_timing_config.min_release_gap_us -ne 17167 -or
                                    $precisionRun.r3_timing_config.min_release_gap_ticks -ne $expectedHoldTicks -or
                                    $precisionRun.r3_timing_config.native_admission -ne 'PASS' -or
                                    $precisionRun.r3_timing_config.tick_domain_admission -ne 'PASS' -or
                                    $precisionRun.lease.enabled -ne $true -or $precisionRun.lease.timeout_us -ne 3000000 -or
                                    $precisionRun.lease.timeout_ticks -ne $expectedLeaseTicks -or
                                    $precisionRun.lease.progress_state -ne 'fresh_progress_before_each_sample' -or
                                    $precisionRun.lease.sample_progress_publication_count -ne 11000 -or
                                    $precisionRun.lease.expected_sample_progress_publications -ne 11000 -or
                                    [long]$precisionRun.lease.first_sample_progress_qpc_ticks -le 0 -or
                                    [long]$precisionRun.lease.last_sample_progress_qpc_ticks -lt [long]$precisionRun.lease.first_sample_progress_qpc_ticks -or
                                    $precisionRun.lease.setup_progress_publication_count -ne $expectedSetupRefreshes -or
                                    $precisionRun.lease.setup_progress_published_before_setup_dispatch -ne $true -or
                                    $precisionRun.lease.watchdog_scheduler_started -ne $false -or
                                    [string]::IsNullOrWhiteSpace($precisionRun.completion_model) -or
                                    $precisionRun.statistics_eligible -ne $true -or $precisionRun.sample_count -ne 10000 -or
                                    $precisionRun.failed_sample_count -ne 0 -or $precisionRun.counters.measured_sender_attempts -ne 10000) {
                                    throw "BLOCKED-MEASUREMENT: $childRunId failed the paired 1,000 warmup / 10,000 measured contract"
                                }
                                $precisionRecords.Add([ordered]@{
                                    round_index = $roundIndex
                                    workload = $workload
                                    load_mode = $loadMode
                                    pair_index = $pairIndex
                                    pair_order = $pairOrder
                                    arm = $arm
                                    run_id = $childRunId
                                    report_path = $relativePath.Replace('\', '/')
                                    command_label = $command.label
                                })
                            }
                        }
                    } finally {
                        if ($loadMode -eq 'cpu_contention') { Stop-ContentionWorkers }
                    }
                }
            }

            $roundComparisons = [System.Collections.Generic.List[object]]::new()
            foreach ($workload in $workloads) {
                foreach ($loadMode in $loadModes) {
                    $group = @($precisionRecords | Where-Object { $_.round_index -eq $roundIndex -and $_.workload -eq $workload -and $_.load_mode -eq $loadMode })
                    $aStats = @($group | Where-Object { $_.arm -eq 'control' } | Sort-Object pair_index | ForEach-Object { Get-PrecisionRunStats $runDirectory $_ })
                    $bStats = @($group | Where-Object { $_.arm -eq 'candidate' } | Sort-Object pair_index | ForEach-Object { Get-PrecisionRunStats $runDirectory $_ })
                    $pairOrders = @($group | Where-Object { $_.arm -eq 'control' } | Sort-Object pair_index | ForEach-Object { $_.pair_order })
                    $allEligible = $aStats.Count -eq 5 -and $bStats.Count -eq 5 -and
                        @($aStats + $bStats | Where-Object { -not $_.statistics_eligible -or $_.sample_count -ne 10000 -or $_.failed_sample_count -ne 0 -or $_.measured_sender_attempts -ne 10000 }).Count -eq 0 -and
                        ($pairOrders -join ',') -eq 'AB,BA,AB,BA,AB'
                    $baselineP99 = if ($aStats.Count -eq 5) { Get-Median ([double[]]@($aStats | ForEach-Object { [double]$_.admission_p99_us })) } else { $null }
                    $candidateP99 = if ($bStats.Count -eq 5) { Get-Median ([double[]]@($bStats | ForEach-Object { [double]$_.admission_p99_us })) } else { $null }
                    $allowance = if ($null -eq $baselineP99) { $null } elseif ($workload.StartsWith('up-only-')) { [Math]::Max(2.0, 0.10 * $baselineP99) } else { [Math]::Max(5.0, 0.15 * $baselineP99) }
                    $budgetPassed = $allEligible -and $candidateP99 -le ($baselineP99 + $allowance)
                    $roundComparisons.Add([ordered]@{
                        workload = $workload
                        load_mode = $loadMode
                        pair_order = $pairOrders
                        pair_count = [Math]::Min($aStats.Count, $bStats.Count)
                        sample_count_per_run = 10000
                        baseline_median_run_p99_us = $baselineP99
                        candidate_median_run_p99_us = $candidateP99
                        candidate_minus_baseline_p99_us = if ($null -ne $candidateP99 -and $null -ne $baselineP99) { $candidateP99 - $baselineP99 } else { $null }
                        allowed_increase_us = $allowance
                        budget = if ($workload.StartsWith('up-only-')) { 'candidate p99 <= baseline p99 + max(2 us, 10% baseline)' } else { 'candidate p99 <= baseline p99 + max(5 us, 15% baseline)' }
                        baseline_runs = $aStats
                        candidate_runs = $bStats
                        statistics_eligible = $allEligible
                        budget_passed = $budgetPassed
                    })
                }
            }
            $roundPassed = @($roundComparisons | Where-Object { -not $_.budget_passed }).Count -eq 0
            $precisionRounds.Add([ordered]@{
                round_index = $roundIndex
                paired_order = @('AB', 'BA', 'AB', 'BA', 'AB')
                budget_passed = $roundPassed
                comparisons = @($roundComparisons)
            })
            if (-not $roundPassed -and $roundIndex -eq 1) {
                $maximumRounds = 2
                Write-Output 'BLOCKED-PERF candidate budget failed initial matrix; retaining all samples and running one complete paired matrix repeat'
            }
        }
        $finalRound = $precisionRounds[$precisionRounds.Count - 1]
        if (-not $finalRound.budget_passed) {
            $script:blockedPerf = $true
            $blockedSummary = [ordered]@{
                schema_version = 1
                status = 'BLOCKED-PERF'
                stage = $Stage
                run_id = $runId
                source_revision = $headSha
                runtime_revision = $controlRevision
                base_revision = $controlRevision
                candidate_revision = $headSha
                source_tree_clean = $sourceTreeClean
                control_executable_sha256 = $controlProbeHash
                candidate_executable_sha256 = $probeHash
                host = $hostMetadata
                paired_precision_rounds = @($precisionRounds)
                native_controls = @($nativeResults)
                child_command_count = $commandResults.Count
                child_commands = @($commandResults)
                failure = 'Second complete paired matrix did not satisfy every per-workload and per-mode candidate p99 budget. No tuning was applied.'
            }
            Write-JsonFile (Join-Path $runDirectory 'summary.json') $blockedSummary
            throw 'BLOCKED-PERF: second complete paired matrix failed one or more coordinator p99 budgets'
        }

        $summary = [ordered]@{
            schema_version = 1
            status = 'COMPLETED_CHARACTERIZATION'
            stage = $Stage
            run_id = $runId
            source_revision = $headSha
            runtime_revision = $controlRevision
            base_revision = $controlRevision
            candidate_revision = $headSha
            source_tree_clean = $sourceTreeClean
            control_executable_sha256 = $controlProbeHash
            candidate_executable_sha256 = $probeHash
            host = $hostMetadata
            profile = 'release'
            transport_kind = 'deterministic-mock precision comparison plus real SendInput to the project-owned ReceiveOnly sink'
            control_contracts = [ordered]@{
                source_revision = $controlContractReport.source_revision
                old_lease_vector = $controlLeaseBaseline.hypothesis_status
                overdue_disposition = $controlLeaseBaseline.expired_without_watchdog.disposition
                overdue_sender_attempts = $controlLeaseBaseline.expired_without_watchdog.musical_sender_attempts
                lifecycle_vector = $controlLifecycle.hypothesis_status
            }
            candidate_contracts = [ordered]@{
                source_revision = $candidateContractReport.source_revision
                final_lease_age_status = $candidateLeaseAge.status
                overdue_rejected = $candidateLeaseAge.overdue_passed
                unproven_rejected = $candidateLeaseAge.unproven_passed
                fresh_allowed = $candidateLeaseAge.fresh_passed
                lifecycle_vector = $candidateLifecycle.hypothesis_status
                empty_cleanup = $candidateContractReport.test_vectors.E_empty_cleanup.oracle_passed
            }
            native_controls = @($nativeResults)
            native_control_count = $nativeResults.Count
            native_sink_selftest = 'PASS'
            paired_precision_rounds = @($precisionRounds)
            paired_ab_disposition = if ($precisionRounds.Count -eq 1) { 'PASS on first five-pair matrix per workload/mode' } else { 'PASS after one complete paired matrix repeat; initial matrix retained' }
            precision_run_count = $precisionRecords.Count
            paired_pair_count = $precisionRecords.Count / 2
            statistical_sample_count = [long]($precisionRecords.Count * 10000)
            measurement_boundary = 'Each sample starts at QPC immediately before prepared-dispatch entry and ends at the mock sender authoritative pre-call QPC; this includes modifier observation and the candidate final lease-age proof. Mock completion samples do not measure SendInput, device, game, or audio receipt.'
            comparison = [ordered]@{
                control_revision = $controlRevision
                candidate_revision = $headSha
                order_by_pair = @('AB', 'BA', 'AB', 'BA', 'AB')
                warmup_samples_per_run = 1000
                measured_samples_per_run = 10000
                contention_workers = 2
                down_mixed_budget = 'candidate median of five run p99 values <= baseline median of five run p99 values + max(5 us, 15% baseline)'
                up_only_budget = 'candidate median of five run p99 values <= baseline median of five run p99 values + max(2 us, 10% baseline)'
            }
            child_command_count = $commandResults.Count
            child_commands = @($commandResults)
            receipt_boundary = 'ReceiveOnly sink events qualify native acceptance scenarios; they do not prove Sky/game/audio receipt.'
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
            runtime_revision = $controlRevision
            executable_sha256 = $probeHash
            control_executable_sha256 = $controlProbeHash
            declared_outputs = @($declaredFiles)
        }
        Write-JsonFile (Join-Path $runDirectory 'runner-outputs.json') $outputsManifest
        Write-Output (ConvertTo-Json -InputObject ([ordered]@{ status = $outputsManifest.status; run_id = $runId; run_directory = $runDirectory; source_revision = $headSha; control_revision = $controlRevision; precision_runs = $precisionRecords.Count; native_controls = $nativeResults.Count; child_commands = $commandResults.Count }) -Compress)
        return
    }

    $contractPath = Join-Path $runDirectory 'contracts.json'
    $contractRunId = "$runId-contracts"
    $contractEnvironment = $environment.Clone()
    $contractEnvironment.SKY_RT_R3_RUN_ID = $contractRunId
    $contractEnvironment.SKY_RT_R3_CONTENTION_WORKERS = '0'
    [void](Invoke-CapturedCommand -Name $probePath -Arguments @('--mode', 'contracts', '--load-mode', 'quiet', '--output', $contractPath) -Label 'contracts' -Environment $contractEnvironment)
    $contractReport = Get-Content -LiteralPath $contractPath -Raw | ConvertFrom-Json
    $vectorA = $contractReport.test_vectors.A_same_key_physical_floor_pause_resume
    $vectorB = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed
    $vectorC = $contractReport.test_vectors.C_serial_HOL
    $vectorD = $contractReport.test_vectors.D_focus_loss_policy
    if ($vectorA.hypothesis_status -ne 'REPRODUCED' -or
        $vectorA.characterization_integrity_passed -ne $true -or
        $vectorA.normal_gap_contract_passed -ne $false -or
        $vectorA.vector_cases.Count -ne 3 -or
        @($vectorA.vector_cases | Where-Object {
            $resumeSnapshot = $_.lifecycle_snapshots.before_next_down
            $expectedResumeStep = if ($resumeSnapshot.current_floor_was_due_at_resume) { 'Some(Dispatched)' } else { 'None' }
            $_.due_seam_integrity_passed -ne $true -or
            $_.normal_gap_contract_passed -ne $false -or
            $_.below_completion_plus_frame -ne $true -or
            $_.disposition -ne 'REPRODUCED' -or
            $_.conversion.authored_offsets_us.Count -ne 4 -or
            $_.conversion.native_admission -ne 'PASS' -or
            $_.conversion.effective_hold_h_us -ne 17167 -or
            $_.conversion.min_release_gap_us -ne 17167 -or
            $_.no_pause_negative_control.qpc.microseconds_from_zero -ne 118300 -or
            $_.no_pause_negative_control.floor.microseconds_from_zero -ne 133884 -or
            $_.lifecycle_snapshots.before_next_down.earliest_dispatch_qpc.qpc_ticks -ne $_.events.redown.pre_call.qpc_ticks -or
            $_.lifecycle_snapshots.before_next_down.post_cleanup_recomputed_floor.qpc_ticks -ne $_.production_down_floor.qpc_ticks -or
            $resumeSnapshot.dispatch_step_at_resume -ne $expectedResumeStep -or
            $_.steps.final_up -ne 'Dispatched' -or
            $_.final_generation_accounting.activated -ne 2 -or
            $_.final_generation_accounting.released -ne 2 -or
            $_.final_release_obligation_mask -ne 0
        }).Count -ne 0 -or
        @($vectorA.vector_cases | Where-Object {
            $resumeSnapshot = $_.lifecycle_snapshots.before_next_down
            $_.no_pause_negative_control.not_due_without_send_or_advance -ne $true -or
            ($resumeSnapshot.current_floor_was_due_at_resume -eq $false -and $resumeSnapshot.not_due_without_send_or_advance -ne $true) -or
            ($resumeSnapshot.current_floor_was_due_at_resume -eq $true -and $null -ne $resumeSnapshot.not_due_without_send_or_advance)
        }).Count -ne 0 -or
        $vectorA.malformed_duplicate_up_control.status -ne 'REJECTED_BY_SCHEDULE_COMPILER' -or
        $vectorC.timing_config.native_admission -ne 'PASS' -or
        $vectorC.timing_config.tick_domain_admission -ne 'PASS' -or
        $vectorC.timing_config.frame_gap_g_us -ne 16667 -or
        $vectorC.timing_config.effective_hold_h_us -ne 17167 -or
        $vectorC.timing_config.min_release_gap_us -ne 17167 -or
        $vectorB.hypothesis_status -ne 'REPRODUCED' -or
        $vectorB.expired_without_watchdog.disposition -ne 'STALE_LEASE_ACCEPTED' -or
        $vectorB.expired_without_watchdog.lease_enabled -ne $true -or
        $vectorB.expired_without_watchdog.musical_sender_attempts -ne 1 -or
        $vectorB.timeout_equality.lease_enabled -ne $true -or
        $vectorB.timeout_equality.disposition -ne 'AT_TIMEOUT_ALLOWED' -or
        $vectorB.timeout_equality.musical_sender_attempts -ne 1 -or
        $vectorB.timeout_equality.dispatch_was_allowed -ne $true -or
        $vectorB.prior_fresh_progress.disposition -ne 'FRESH_PROGRESS_ALLOWED' -or
        $vectorB.prior_fresh_progress.lease_enabled -ne $true -or
        $vectorB.prior_fresh_progress.musical_sender_attempts -ne 1 -or
        $vectorB.prior_fresh_progress.dispatch_was_allowed -ne $true -or
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
                    $expectedSetupRefreshes = if ($workload.StartsWith('mixed-') -or $workload.StartsWith('up-only-')) { 11000 } else { 0 }
                    $expectedFrameTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 16667 / 1000000)
                    $expectedMarginTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 500 / 1000000)
                    $expectedHoldTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 17167 / 1000000)
                    $expectedLeaseTicks = [long][Math]::Ceiling([double]$report.qpc_frequency_hz * 3000000 / 1000000)
                    if ($report.schema_version -ne 1 -or $report.source_revision.ToLowerInvariant() -ne $headSha -or
                        $report.runtime_revision.ToLowerInvariant() -ne $runtimeRevision -or $report.profile -ne 'release' -or
                        $report.transport_kind -ne 'deterministic-mock' -or $report.run_id -ne $childRunId -or
                        $report.mode -ne 'precision' -or $report.load_mode -ne $loadMode -or
                        $report.contention_workers -ne $(if ($loadMode -eq 'cpu_contention') { 2 } else { 0 }) -or
                        $report.warmup_count -ne 1000 -or $report.measured_count -ne 10000 -or
                        $precisionRun.r3_timing_config.frame_gap_g_us -ne 16667 -or
                        $precisionRun.r3_timing_config.frame_gap_g_ticks -ne $expectedFrameTicks -or
                        $precisionRun.r3_timing_config.frame_base_hold_us -ne 16667 -or
                        $precisionRun.r3_timing_config.timing_margin_us -ne 500 -or
                        $precisionRun.r3_timing_config.timing_margin_ticks -ne $expectedMarginTicks -or
                        $precisionRun.r3_timing_config.effective_hold_h_us -ne 17167 -or
                        $precisionRun.r3_timing_config.effective_hold_h_ticks -ne $expectedHoldTicks -or
                        $precisionRun.r3_timing_config.min_release_gap_us -ne 17167 -or
                        $precisionRun.r3_timing_config.min_release_gap_ticks -ne $expectedHoldTicks -or
                        $precisionRun.r3_timing_config.native_admission -ne 'PASS' -or
                        $precisionRun.r3_timing_config.tick_domain_admission -ne 'PASS' -or
                        $precisionRun.lease.enabled -ne $true -or $precisionRun.lease.timeout_us -ne 3000000 -or
                        $precisionRun.lease.timeout_ticks -ne $expectedLeaseTicks -or
                        $precisionRun.lease.progress_state -ne 'fresh_progress_before_each_sample' -or
                        $precisionRun.lease.sample_progress_publication_count -ne 11000 -or
                        $precisionRun.lease.expected_sample_progress_publications -ne 11000 -or
                        [long]$precisionRun.lease.first_sample_progress_qpc_ticks -le 0 -or
                        [long]$precisionRun.lease.last_sample_progress_qpc_ticks -lt [long]$precisionRun.lease.first_sample_progress_qpc_ticks -or
                        $precisionRun.lease.setup_progress_publication_count -ne $expectedSetupRefreshes -or
                        $precisionRun.lease.setup_progress_published_before_setup_dispatch -ne $true -or
                        $precisionRun.lease.watchdog_scheduler_started -ne $false -or
                        [string]::IsNullOrWhiteSpace($precisionRun.completion_model) -or
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
            A_contract = $contractReport.test_vectors.A_same_key_physical_floor_pause_resume.hypothesis_status
            A_characterization_integrity = $contractReport.test_vectors.A_same_key_physical_floor_pause_resume.characterization_integrity_passed
            A_normal_gap_contract = $contractReport.test_vectors.A_same_key_physical_floor_pause_resume.normal_gap_contract_passed
            A_malformed_duplicate_control = $contractReport.test_vectors.A_same_key_physical_floor_pause_resume.malformed_duplicate_up_control.status
            A_four_event_cases = $contractReport.test_vectors.A_same_key_physical_floor_pause_resume.vector_cases.Count
            B_hypothesis = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed.hypothesis_status
            B_expired_case = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed.expired_without_watchdog.disposition
            B_equality_case = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed.timeout_equality.disposition
            B_fresh_case = $contractReport.test_vectors.B_enabled_lease_watchdog_delayed.prior_fresh_progress.disposition
            C_native_admission = $contractReport.test_vectors.C_serial_HOL.timing_config.native_admission
            D_release_while_unfocused = $contractReport.test_vectors.D_focus_loss_policy.restore_path.cleanup_sends_while_unfocused
            D_release_after_restore = $contractReport.test_vectors.D_focus_loss_policy.restore_path.release_on_restore
            D_terminal_cleanup = $contractReport.test_vectors.D_focus_loss_policy.terminal_cleanup_while_unfocused.ownership_scoped_terminal_release
            E_empty_cleanup_oracle = $contractReport.test_vectors.E_empty_cleanup.oracle_passed
        }
        precision = @($precisionSummary)
        native_controls = @($nativeResults)
        native_sink_selftest = 'PASS'
        bun_version_expected_by_ci = '1.4.3'
        bun_version_matches_ci = ($bunVersion -eq '1.4.3')
        child_command_count = $commandResults.Count
        child_commands = @($commandResults)
        precision_run_count = $precisionRecords.Count
        paired_ab_disposition = 'P0 records baseline runs only; paired A/B execution is reserved for the comparison phase and currently fails explicitly.'
        timing_boundary = 'Sample-start QPC immediately before prepared-dispatch entry through the mock sender authoritative pre-call QPC. Mock emitter completion is sampled inside the deterministic mock; any delta includes test-seam/QPC overhead and does not measure SendInput cost or device/game receipt.'
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
            $blockedStatus = if ($script:blockedDefect) { 'BLOCKED-DEFECT' } elseif ($script:blockedPerf) { 'BLOCKED-PERF' } else { 'BLOCKED' }
            Write-JsonFile (Join-Path $runDirectory 'status.json') ([ordered]@{ status = $blockedStatus; stage = $Stage; run_id = $runId; error = $_.Exception.Message; finished_utc = [DateTimeOffset]::UtcNow.ToString('O'); child_commands = @($commandResults) })
        } catch { }
    }
    Write-Error $_
    exit 1
} finally {
    Stop-ContentionWorkers
    if ($controlWorktreeCreated -and $controlWorktree) {
        $controlTempRoot = [System.IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
        $controlFullPath = [System.IO.Path]::GetFullPath($controlWorktree)
        if ($controlFullPath.StartsWith($controlTempRoot, [System.StringComparison]::OrdinalIgnoreCase) -and
            [System.IO.Path]::GetFileName($controlFullPath).StartsWith('rt-r3-p2-control-', [System.StringComparison]::Ordinal)) {
            $removeOutput = & git -C $repoRoot worktree remove --force $controlFullPath 2>&1
            if ($LASTEXITCODE -ne 0) { Write-Warning ('Could not remove temporary P1 control worktree: ' + ($removeOutput -join ' ')) }
        }
    }
}
