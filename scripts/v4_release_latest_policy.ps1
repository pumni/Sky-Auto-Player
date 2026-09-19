Set-StrictMode -Version Latest

function Get-V4LatestIdentity([object]$Release) {
    if ($null -eq $Release) { throw "GitHub Latest response is missing" }
    return [ordered]@{
        id = [int64]$Release.id
        tag_name = [string]$Release.tag_name
        target_commitish = [string]$Release.target_commitish
        draft = [bool]$Release.draft
        prerelease = [bool]$Release.prerelease
        published_at = [string]$Release.published_at
    }
}

function Assert-V4StableLatestRelease([object]$Release) {
    if ($null -eq $Release -or [bool]$Release.draft -or [bool]$Release.prerelease -or
        [string]::IsNullOrWhiteSpace([string]$Release.published_at)) {
        throw "GitHub Latest must be published and non-prerelease"
    }
    if ([string]$Release.tag_name -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') {
        throw "GitHub Latest is outside the supported stable namespace: $($Release.tag_name)"
    }
}

function Assert-V4GitHubLatestPolicy {
    param(
        [Parameter(Mandatory = $true)] [ValidateSet("stable", "beta")] [string]$Channel,
        [Parameter(Mandatory = $true)] [object]$PublishedRelease,
        [Parameter(Mandatory = $true)] [object]$PostPublicationLatest,
        [object]$PrePublicationLatest,
        [Parameter(Mandatory = $true)] [string]$ExpectedTag,
        [Parameter(Mandatory = $true)] [string]$ExpectedSourceSha
    )

    if ($null -eq $PublishedRelease -or [int64]$PublishedRelease.id -le 0) {
        throw "published release identity is missing"
    }
    if ([string]$PublishedRelease.tag_name -ne $ExpectedTag) {
        throw "published release tag does not match the exact requested tag"
    }
    $publishedSource = [string]$PublishedRelease.target_commitish
    if ($publishedSource -notmatch '^[0-9a-fA-F]{40}$' -or
        $publishedSource.ToLowerInvariant() -ne $ExpectedSourceSha.ToLowerInvariant()) {
        throw "published release source does not match the exact requested source SHA"
    }
    if ([bool]$PublishedRelease.draft -or
        [string]::IsNullOrWhiteSpace([string]$PublishedRelease.published_at)) {
        throw "published release is still draft or unpublished"
    }
    if ($null -eq $PublishedRelease.immutable -or -not [bool]$PublishedRelease.immutable) {
        throw "published release is not immutable"
    }

    Assert-V4StableLatestRelease $PostPublicationLatest
    $postIdentity = Get-V4LatestIdentity $PostPublicationLatest
    if ($Channel -eq "stable") {
        if ([int64]$postIdentity.id -ne [int64]$PublishedRelease.id -or
            [string]$postIdentity.tag_name -ne $ExpectedTag -or
            [string]$postIdentity.target_commitish.ToLowerInvariant() -ne $ExpectedSourceSha.ToLowerInvariant()) {
            throw "stable publication did not become the exact GitHub Latest release"
        }
        return
    }

    if ($null -eq $PrePublicationLatest) {
        throw "beta Latest policy requires an in-memory pre-publication Latest identity"
    }
    $beforeIdentity = Get-V4LatestIdentity $PrePublicationLatest
    foreach ($property in @("id", "tag_name", "target_commitish", "draft", "prerelease", "published_at")) {
        if ([string]$postIdentity[$property] -ne [string]$beforeIdentity[$property]) {
            throw "beta publication changed GitHub Latest identity: $property"
        }
    }
    if ([int64]$postIdentity.id -eq [int64]$PublishedRelease.id -or
        [string]$postIdentity.tag_name -eq $ExpectedTag -or
        [bool]$postIdentity.prerelease) {
        throw "beta publication displaced GitHub Latest"
    }
}
