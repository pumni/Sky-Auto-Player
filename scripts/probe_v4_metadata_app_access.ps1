[CmdletBinding()]
param(
    [string]$Repository = "pumni/Sky-Auto-Player"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($Repository -ne "pumni/Sky-Auto-Player") {
    throw "release-metadata App access probe requires the canonical repository"
}
if ([string]::IsNullOrWhiteSpace($env:GH_TOKEN)) {
    throw "release-metadata App access probe requires GH_TOKEN"
}

function Invoke-ReadOnlyMetadataGet([string]$Path, [switch]$AllowNotFound) {
    $output = & gh api $Path --header "Accept: application/vnd.github+json" 2>&1
    if ($LASTEXITCODE -ne 0) {
        $text = ($output -join "`n")
        if ($AllowNotFound -and $text -match '(?i)(404|not found)') { return $null }
        throw "release-metadata App read-only access probe failed for $Path"
    }
    return (($output -join "`n") | ConvertFrom-Json)
}

$branch = Invoke-ReadOnlyMetadataGet "repos/$Repository/git/ref/heads/release-metadata"
if ([string]$branch.ref -ne "refs/heads/release-metadata" -or
    [string]::IsNullOrWhiteSpace([string]$branch.object.sha)) {
    throw "release-metadata App access probe received an invalid branch ref"
}
$bootstrap = Invoke-ReadOnlyMetadataGet "repos/$Repository/contents/.release-metadata/README.md?ref=release-metadata"
if ($null -eq $bootstrap -or [string]::IsNullOrWhiteSpace([string]$bootstrap.content)) {
    throw "release-metadata App access probe could not read the bootstrap contract"
}
foreach ($channel in @("stable", "beta")) {
    $existing = Invoke-ReadOnlyMetadataGet "repos/$Repository/contents/channels/$channel/latest.json?ref=release-metadata" -AllowNotFound
    if ($null -ne $existing -and [string]::IsNullOrWhiteSpace([string]$existing.content)) {
        throw "release-metadata App access probe received empty $channel metadata"
    }
}
Write-Host "V4 release-metadata App access probe: PASS (read-only branch, bootstrap, and channel reads)"
