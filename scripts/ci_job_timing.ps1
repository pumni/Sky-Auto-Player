[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("start", "finish")]
    [string] $Mode,

    [Parameter(Mandatory = $true)]
    [ValidatePattern("^[A-Za-z0-9_-]+$")]
    [string] $Job,

    [string] $CacheHit
)

Set-StrictMode -Version Latest
$stateKey = "SKY_CI_JOB_STARTED_$($Job.ToUpperInvariant().Replace('-', '_'))"

if ($Mode -eq "start") {
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
        throw "GITHUB_ENV is required when starting CI timing."
    }

    $startedAt = [DateTimeOffset]::UtcNow.ToString("o")
    "$stateKey=$startedAt" | Add-Content -LiteralPath $env:GITHUB_ENV -Encoding ASCII
    exit 0
}

if ([string]::IsNullOrWhiteSpace($env:GITHUB_STEP_SUMMARY)) {
    throw "GITHUB_STEP_SUMMARY is required when finishing CI timing."
}

$startedText = [Environment]::GetEnvironmentVariable($stateKey)
if ([string]::IsNullOrWhiteSpace($startedText)) {
    throw "CI timing start value '$stateKey' was not found."
}

$startedAt = [DateTimeOffset]::Parse($startedText)
$elapsedSeconds = [math]::Round(([DateTimeOffset]::UtcNow - $startedAt).TotalSeconds, 2)
$cacheLine = if ([string]::IsNullOrWhiteSpace($CacheHit)) {
    ""
} else {
    "- Rust cache hit: ``$CacheHit``"
}

@"
## CI job timing
- Job: ``$Job``
- Elapsed seconds: ``$elapsedSeconds``
$cacheLine
"@ | Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding UTF8
