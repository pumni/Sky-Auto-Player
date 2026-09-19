[CmdletBinding()]
param(
    [ValidateSet("Baseline", "Verify")]
    [string]$Mode = "Baseline",

    [ValidateSet("stable", "beta")]
    [string]$Channel,

    [string]$ExpectedTag,
    [string]$ExpectedSourceSha,
    [int64]$BeforeLatestId,
    [string]$BeforeLatestTag,
    [string]$BeforeLatestSourceSha,
    [string]$BeforeLatestPublishedAt
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$canonicalRepository = "pumni/Sky-Auto-Player"
. (Join-Path $PSScriptRoot "v4_release_latest_policy.ps1")

function Fail([string]$Message) {
    throw "V4 GitHub Latest policy guard failed closed: $Message"
}

if ([string]::IsNullOrWhiteSpace($env:GITHUB_REPOSITORY) -or
    $env:GITHUB_REPOSITORY -ne $canonicalRepository) {
    Fail "guard requires the canonical repository identity"
}

function Get-GitHubJson([string]$Path) {
    $payload = & gh api $Path --header "Accept: application/vnd.github+json"
    if ($LASTEXITCODE -ne 0) { Fail "GitHub read failed for $Path" }
    return ($payload -join "`n") | ConvertFrom-Json
}

function Get-LatestRelease {
    $latest = Get-GitHubJson "repos/$canonicalRepository/releases/latest"
    try { Assert-V4StableLatestRelease $latest } catch { Fail $_.Exception.Message }
    return $latest
}

if ($Mode -eq "Baseline") {
    $latest = Get-LatestRelease
    @"
V4 GitHub Latest baseline guard: PASS
repository=$canonicalRepository
latest_tag=$($latest.tag_name)
latest_release=$($latest.html_url)
read_only=true
"@ | Write-Host
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Channel) -or
    [string]::IsNullOrWhiteSpace($ExpectedTag) -or
    [string]::IsNullOrWhiteSpace($ExpectedSourceSha) -or
    $ExpectedSourceSha -notmatch '^[0-9a-fA-F]{40}$') {
    Fail "Verify mode requires channel, expected tag, and exact source SHA"
}
$published = Get-GitHubJson "repos/$canonicalRepository/releases/tags/$ExpectedTag"
$latest = Get-LatestRelease
$before = $null
if ($Channel -eq "beta") {
    if ($BeforeLatestId -le 0 -or [string]::IsNullOrWhiteSpace($BeforeLatestTag) -or
        [string]::IsNullOrWhiteSpace($BeforeLatestSourceSha) -or
        [string]::IsNullOrWhiteSpace($BeforeLatestPublishedAt)) {
        Fail "beta Verify requires the in-memory pre-publication Latest identity"
    }
    $before = [pscustomobject]@{
        id = $BeforeLatestId
        tag_name = $BeforeLatestTag
        target_commitish = $BeforeLatestSourceSha
        draft = $false
        prerelease = $false
        published_at = $BeforeLatestPublishedAt
    }
}
try {
    Assert-V4GitHubLatestPolicy `
        -Channel $Channel `
        -PublishedRelease $published `
        -PrePublicationLatest $before `
        -PostPublicationLatest $latest `
        -ExpectedTag $ExpectedTag `
        -ExpectedSourceSha $ExpectedSourceSha.ToLowerInvariant()
} catch {
    Fail $_.Exception.Message
}
@"
V4 GitHub Latest policy guard: PASS
repository=$canonicalRepository
channel=$Channel
latest_tag=$($latest.tag_name)
published_tag=$ExpectedTag
published_release=$($published.html_url)
make_latest=$(if ($Channel -eq "stable") { "true" } else { "false" })
read_only=true
"@ | Write-Host
