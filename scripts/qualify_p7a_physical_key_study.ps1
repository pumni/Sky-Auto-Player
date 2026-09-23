param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\.benchmarks\p7a-physical-key-study'),
    [ValidateRange(100, 100000)]
    [int]$Iterations = 300,
    [ValidateRange(1, 1000000)]
    [int]$DueUs = 5000
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$burnerScript = Join-Path $PSScriptRoot 'p7a_cpu_contention.ps1'
$pwshPath = (Get-Command pwsh).Source

$envNames = @(
    'RT_HANDOFF_BENCH_REQUIRE_FOCUS',
    'RT_HANDOFF_BENCH_REAL_FOREGROUND',
    'RT_HANDOFF_BENCH_ITERATIONS',
    'RT_HANDOFF_BENCH_DUE_US',
    'RT_HANDOFF_BENCH_SCOPE',
    'RT_HANDOFF_BENCH_MODE',
    'RT_HANDOFF_P7A_CANDIDATE',
    'RT_HANDOFF_P7A_LOAD',
    'RT_HANDOFF_P7A_RETRIGGER_ITERATIONS'
)
$savedEnv = @{}
foreach ($name in $envNames) {
    $savedEnv[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

function Invoke-P7aCandidate([string]$Candidate, [string]$Load, [int]$Round) {
    $fileName = "round${Round}_${Load}_${Candidate}.json"
    $outputPath = Join-Path $outputRoot $fileName
    $env:RT_HANDOFF_BENCH_REQUIRE_FOCUS = '1'
    $env:RT_HANDOFF_BENCH_REAL_FOREGROUND = '1'
    $env:RT_HANDOFF_BENCH_ITERATIONS = "$Iterations"
    $env:RT_HANDOFF_BENCH_DUE_US = "$DueUs"
    $env:RT_HANDOFF_BENCH_SCOPE = 'p7a_physical_study'
    $env:RT_HANDOFF_BENCH_MODE = 'real_wait'
    $env:RT_HANDOFF_P7A_CANDIDATE = $Candidate
    $env:RT_HANDOFF_P7A_LOAD = $Load
    $env:RT_HANDOFF_P7A_RETRIGGER_ITERATIONS = '30'
    & $benchPath $outputPath '--scope' 'p7a_physical_study' '--mode' 'real_wait' > $null
    if ($LASTEXITCODE -ne 0) {
        throw "P7a candidate $Candidate failed with exit code $LASTEXITCODE"
    }
    Write-Host "saved $outputPath"
}

try {
    Push-Location (Join-Path $repoRoot 'rust')
    & cargo build --locked --release -p sky_player --features test-support --example rt_handoff_bench
    if ($LASTEXITCODE -ne 0) {
        throw "P7a benchmark build failed with exit code $LASTEXITCODE"
    }
    $benchPath = Join-Path $repoRoot 'rust\target\release\examples\rt_handoff_bench.exe'
    if (-not (Test-Path -LiteralPath $benchPath)) {
        throw 'P7a benchmark executable is missing after the build'
    }
    $orders = @(
        ,([string[]]@('A', 'B', 'C', 'D'))
        ,([string[]]@('C', 'D', 'A', 'B'))
    )
    foreach ($round in 1..$orders.Count) {
        foreach ($candidate in $orders[$round - 1]) {
            Invoke-P7aCandidate $candidate 'quiet' $round
        }
    }

    foreach ($round in 1..$orders.Count) {
        $burner = Start-Process -FilePath $pwshPath `
            -ArgumentList @('-NoProfile', '-File', $burnerScript) `
            -PassThru -WindowStyle Hidden
        try {
            $burner.PriorityClass = 'BelowNormal'
            Start-Sleep -Milliseconds 400
            foreach ($candidate in $orders[$round - 1]) {
                Invoke-P7aCandidate $candidate 'cpu_contention' $round
            }
        }
        finally {
            if ($burner -and -not $burner.HasExited) {
                Stop-Process -Id $burner.Id -Force
            }
        }
    }

    $manifest = [ordered]@{
        phase = 'P7a evidence-only'
        head = (& git rev-parse HEAD).Trim()
        iterations_per_workload_per_run = $Iterations
        due_us = $DueUs
        candidate_order = $orders
        loads = @('quiet', 'cpu_contention')
        generated_utc = [DateTime]::UtcNow.ToString('o')
        os_caption = (Get-CimInstance Win32_OperatingSystem).Caption
        os_version = (Get-CimInstance Win32_OperatingSystem).Version
        processor = (Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name)
        logical_processors = [Environment]::ProcessorCount
        transport = 'deterministic mock; physical-state queries only'
    }
    $manifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $outputRoot 'manifest.json')
}
finally {
    Pop-Location
    foreach ($name in $envNames) {
        [Environment]::SetEnvironmentVariable($name, $savedEnv[$name], 'Process')
    }
}
