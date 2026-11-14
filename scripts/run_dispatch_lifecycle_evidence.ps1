[CmdletBinding()]
param(
    [string]$OutputPath = (Join-Path ([System.IO.Path]::GetTempPath()) ("dispatch-lifecycle-{0}.json" -f [guid]::NewGuid().ToString('N')))
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib/dispatch_lifecycle_report_contract.ps1')

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$resolvedOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$rootPrefix = $repositoryRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
if ($resolvedOutputPath.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'lifecycle evidence output must be outside the repository checkout'
}

Push-Location -LiteralPath $repositoryRoot
try {
    $branch = (& git branch --show-current).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'git branch --show-current failed' }
    $dirty = @(& git status --porcelain=v1)
    if ($LASTEXITCODE -ne 0) { throw 'git status failed' }
    if ($dirty.Count -ne 0) { throw 'lifecycle evidence requires a clean source checkout' }
    $sourceSha = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'git rev-parse HEAD failed' }
    $sourceTree = (& git rev-parse 'HEAD^{tree}').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'git rev-parse HEAD^{tree} failed' }

    $cargoArguments = @(
        'run', '--quiet', '--locked', '--manifest-path', 'rust/Cargo.toml',
        '-p', 'sky_player', '--features', 'test-support',
        '--example', 'dispatch_lifecycle_evidence', '--',
        '--source-sha', $sourceSha, '--source-tree', $sourceTree
    )
    $runOutput = @(& cargo @cargoArguments 2>&1)
    $cargoExitCode = $LASTEXITCODE
    if ($cargoExitCode -ne 0) {
        $details = ($runOutput | ForEach-Object { $_.ToString() }) -join "`n"
        throw "offline lifecycle probe failed (exit $cargoExitCode):`n$details"
    }
    $jsonText = ($runOutput | ForEach-Object { $_.ToString() }) -join "`n"
    $report = $jsonText | ConvertFrom-Json -AsHashtable
    $contract = Test-DispatchLifecycleReport -Report $report
    if ($contract.EvidenceClass -ne 'live_scheduler') { throw 'runner expected schema v2 live scheduler evidence' }

    $parent = [System.IO.Path]::GetDirectoryName($resolvedOutputPath)
    if (-not [System.IO.Directory]::Exists($parent)) {
        [System.IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    Set-Content -LiteralPath $resolvedOutputPath -Value $jsonText -Encoding utf8NoBOM
    $hash = (Get-FileHash -LiteralPath $resolvedOutputPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Output "source_branch=$branch"
    Write-Output "source_sha=$sourceSha"
    Write-Output "source_tree_sha=$sourceTree"
    Write-Output "schema_version=$($contract.SchemaVersion) evidence_class=$($contract.EvidenceClass)"
    Write-Output "report_path=$resolvedOutputPath"
    Write-Output "report_sha256=$hash"
    Write-Output 'native_sendinput_calls=0'
}
finally {
    Pop-Location
}
