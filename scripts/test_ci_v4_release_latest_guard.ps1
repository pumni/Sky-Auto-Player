[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "v4_release_latest_policy.ps1")

function Fail([string]$Message) { throw "FAILED: $Message" }
function Assert-Throws([scriptblock]$Action, [string]$Name) {
    try { & $Action; Fail "$Name unexpectedly passed" } catch {
        if ($_.Exception.Message -like "FAILED:*") { throw }
    }
}

$sha = "a" * 40
$oldSha = "b" * 40
$published = [pscustomobject]@{
    id = 42; tag_name = "v4.1.2"; target_commitish = $sha; draft = $false
    prerelease = $false; published_at = "2026-09-19T00:00:00Z"; immutable = $true
}
$stableLatest = [pscustomobject]@{
    id = 42; tag_name = "v4.1.2"; target_commitish = $sha; draft = $false
    prerelease = $false; published_at = $published.published_at
}
Assert-V4GitHubLatestPolicy -Channel stable -PublishedRelease $published -PostPublicationLatest $stableLatest `
    -ExpectedTag "v4.1.2" -ExpectedSourceSha $sha

Assert-Throws {
    $wrong = $stableLatest.PSObject.Copy(); $wrong.id = 41
    Assert-V4GitHubLatestPolicy -Channel stable -PublishedRelease $published -PostPublicationLatest $wrong `
        -ExpectedTag "v4.1.2" -ExpectedSourceSha $sha
} "wrong Latest"
Assert-Throws {
    $wrong = $published.PSObject.Copy(); $wrong.target_commitish = $oldSha
    Assert-V4GitHubLatestPolicy -Channel stable -PublishedRelease $wrong -PostPublicationLatest $stableLatest `
        -ExpectedTag "v4.1.2" -ExpectedSourceSha $sha
} "source mismatch"
Assert-Throws {
    $wrong = $published.PSObject.Copy(); $wrong.immutable = $false
    Assert-V4GitHubLatestPolicy -Channel stable -PublishedRelease $wrong -PostPublicationLatest $stableLatest `
        -ExpectedTag "v4.1.2" -ExpectedSourceSha $sha
} "immutable false"
Assert-Throws {
    $wrong = [pscustomobject]@{ id = 42; tag_name = "v4.1.2"; target_commitish = $sha; draft = $false; prerelease = $false; published_at = $published.published_at }
    Assert-V4GitHubLatestPolicy -Channel stable -PublishedRelease $wrong -PostPublicationLatest $stableLatest `
        -ExpectedTag "v4.1.2" -ExpectedSourceSha $sha
} "immutable missing"

$betaPublished = [pscustomobject]@{
    id = 43; tag_name = "v4.1.3-beta.1"; target_commitish = $sha; draft = $false
    prerelease = $true; published_at = $published.published_at; immutable = $true
}
$betaBefore = [pscustomobject]@{
    id = 41; tag_name = "v4.1.1"; target_commitish = $oldSha; draft = $false
    prerelease = $false; published_at = "2026-09-18T00:00:00Z"
}
Assert-V4GitHubLatestPolicy -Channel beta -PublishedRelease $betaPublished `
    -PrePublicationLatest $betaBefore -PostPublicationLatest $betaBefore `
    -ExpectedTag "v4.1.3-beta.1" -ExpectedSourceSha $sha
Assert-Throws {
    Assert-V4GitHubLatestPolicy -Channel beta -PublishedRelease $betaPublished `
        -PrePublicationLatest $betaBefore -PostPublicationLatest $betaPublished `
        -ExpectedTag "v4.1.3-beta.1" -ExpectedSourceSha $sha
} "beta displaces Latest"

Write-Host "V4 Latest policy guard behavioral matrix: PASS (zero network, zero mutation)"
