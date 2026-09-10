[CmdletBinding()]
param(
    [ValidateSet("Baseline", "Capture", "Verify")]
    [string]$Mode = "Baseline",

    [ValidateSet("stable", "beta")]
    [string]$Channel,

    [string]$ExpectedTag,
    [string]$ExpectedSourceSha,
    [string]$StateRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$canonicalRepository = "pumni/Sky-Auto-Player"

function Fail([string]$Message) {
    throw "V4 GitHub Latest policy guard failed closed: $Message"
}

if ([string]::IsNullOrWhiteSpace($env:GITHUB_REPOSITORY) -or
    $env:GITHUB_REPOSITORY -ne $canonicalRepository) {
    Fail "guard requires the canonical repository identity"
}

function Get-GitHubJson([string]$Path) {
    $payload = & gh api $Path --header "Accept: application/vnd.github+json"
    if ($LASTEXITCODE -ne 0) {
        Fail "GitHub read failed for $Path"
    }
    return ($payload -join "`n") | ConvertFrom-Json
}

function Get-LatestRelease {
    $latest = Get-GitHubJson "repos/$canonicalRepository/releases/latest"
    if ([string]$latest.url -notlike "https://api.github.com/repos/$canonicalRepository/releases/*") {
        Fail "unexpected repository in GitHub Latest response: $($latest.url)"
    }
    if ([bool]$latest.draft -or [bool]$latest.prerelease -or
        [string]::IsNullOrWhiteSpace([string]$latest.published_at)) {
        Fail "GitHub Latest must be published and non-prerelease: $($latest.tag_name)"
    }
    if ([string]$latest.tag_name -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') {
        Fail "GitHub Latest is outside the supported stable transition namespace: $($latest.tag_name)"
    }
    return $latest
}

function Get-LatestIdentity([object]$Release) {
    return [ordered]@{
        id = [int64]$Release.id
        tag_name = [string]$Release.tag_name
        target_commitish = [string]$Release.target_commitish
        draft = [bool]$Release.draft
        prerelease = [bool]$Release.prerelease
        published_at = [string]$Release.published_at
    }
}

function Get-GuardStateRoot {
    if ([string]::IsNullOrWhiteSpace($StateRoot)) { Fail "StateRoot is required for $Mode mode" }
    $full = [IO.Path]::GetFullPath($StateRoot)
    New-Item -ItemType Directory -Path $full -Force | Out-Null
    return $full
}

function Write-GuardJson([string]$Path, [object]$Value) {
    $Value | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $Path -Encoding utf8NoBOM
}

if ($Mode -eq "Baseline") {
    $latest = Get-LatestRelease
    @"
V4 GitHub Latest baseline guard: PASS
repository=$canonicalRepository
latest_tag=$($latest.tag_name)
latest_release=$($latest.html_url)
transition_compatible=true
read_only=true
"@ | Write-Host
    exit 0
}

$guardRoot = Get-GuardStateRoot
$beforePath = Join-Path $guardRoot "github-latest-before.json"

if ($Mode -eq "Capture") {
    $latest = Get-LatestRelease
    Write-GuardJson $beforePath (Get-LatestIdentity $latest)
    @"
V4 GitHub Latest policy capture: PASS
repository=$canonicalRepository
captured_tag=$($latest.tag_name)
read_only=true
"@ | Write-Host
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Channel) -or
    [string]::IsNullOrWhiteSpace($ExpectedTag) -or
    [string]::IsNullOrWhiteSpace($ExpectedSourceSha)) {
    Fail "Verify mode requires channel, expected tag, and expected source SHA"
}
if ($ExpectedTag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
    Fail "expected tag is not canonical"
}
if ($ExpectedSourceSha -notmatch '^[0-9a-fA-F]{40}$') {
    Fail "expected source SHA is not an exact commit SHA"
}
$statePath = Join-Path $guardRoot "release-state.json"
if (-not (Test-Path -LiteralPath $statePath -PathType Leaf)) {
    Fail "release-state.json is required for Verify mode"
}
$state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
if ($null -eq $state.release_id) { Fail "release-state.json has no release_id" }

$latest = Get-LatestRelease
$latestIdentity = Get-LatestIdentity $latest
$published = Get-GitHubJson "repos/$canonicalRepository/releases/tags/$ExpectedTag"
if ([int64]$published.id -ne [int64]$state.release_id -or
    [string]$published.tag_name -ne $ExpectedTag -or
    [string]$published.target_commitish -ne $ExpectedSourceSha.ToLowerInvariant()) {
    Fail "published release identity does not match the exact requested source/tag"
}
if ([bool]$published.draft -or
    [string]::IsNullOrWhiteSpace([string]$published.published_at)) {
    Fail "published release is still draft or unpublished"
}
if (($Channel -eq "stable" -and [bool]$published.prerelease) -or
    ($Channel -eq "beta" -and -not [bool]$published.prerelease)) {
    Fail "published release prerelease state does not match channel $Channel"
}
if ($null -eq $published.immutable -or -not [bool]$published.immutable) {
    Fail "published release is not marked immutable before Latest policy verification"
}

if ($Channel -eq "stable") {
    if ([int64]$latestIdentity.id -ne [int64]$published.id -or
        $latestIdentity.tag_name -ne $ExpectedTag -or
        $latestIdentity.target_commitish -ne $ExpectedSourceSha.ToLowerInvariant()) {
        Fail "stable publication did not become the exact GitHub Latest release"
    }
} else {
    if (-not (Test-Path -LiteralPath $beforePath -PathType Leaf)) {
        Fail "beta verification requires the pre-publication GitHub Latest snapshot"
    }
    $before = Get-Content -LiteralPath $beforePath -Raw | ConvertFrom-Json
    foreach ($property in @("id", "tag_name", "target_commitish", "draft", "prerelease", "published_at")) {
        if ([string]$latestIdentity[$property] -ne [string]$before.$property) {
            Fail "beta publication changed GitHub Latest identity: $property"
        }
    }
    if ($latestIdentity.tag_name -eq $ExpectedTag -or [bool]$latestIdentity.prerelease) {
        Fail "beta publication displaced GitHub Latest"
    }
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
