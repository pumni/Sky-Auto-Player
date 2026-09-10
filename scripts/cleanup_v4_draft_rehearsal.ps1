[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$StateRoot,

    [Parameter(Mandatory = $true)]
    [string]$Tag,

    [Parameter(Mandatory = $true)]
    [string]$SourceSha
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$canonicalRepository = "pumni/Sky-Auto-Player"
. (Join-Path $PSScriptRoot "v4_release_draft_lookup.ps1")

function Fail([string]$Message) {
    throw "V4 draft rehearsal cleanup failed closed: $Message"
}

function Get-PropertyValue([object]$Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Get-IsolatedStateRoot {
    if ([string]::IsNullOrWhiteSpace($StateRoot)) { Fail "StateRoot is required" }
    if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { Fail "RUNNER_TEMP is unavailable" }

    $full = [IO.Path]::GetFullPath($StateRoot)
    $runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
    $runnerTempPrefix = $runnerTemp.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $workspace = if ([string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE)) {
        $repoRoot
    } else {
        [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
    }
    $workspacePrefix = $workspace.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
    $repoPrefix = $repoRoot.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar

    if ($full.Equals($runnerTemp, [StringComparison]::OrdinalIgnoreCase) -or
        -not $full.StartsWith($runnerTempPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "StateRoot must be a child of RUNNER_TEMP"
    }
    if ($full.Equals($workspace, [StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $full.Equals($repoRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "refusing to mutate a path inside the repository workspace"
    }
    New-Item -ItemType Directory -Path $full -Force | Out-Null
    return $full
}

function Invoke-GitHubApi {
    param(
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [switch]$AllowNotFound
    )

    $errorPath = Join-Path $script:isolatedStateRoot ("gh-cleanup-error-" + [guid]::NewGuid().ToString("N") + ".log")
    try {
        $response = & gh @Arguments 2>$errorPath
        $exitCode = $LASTEXITCODE
        if ($exitCode -ne 0) {
            $errorText = if (Test-Path -LiteralPath $errorPath) { Get-Content -LiteralPath $errorPath -Raw } else { "" }
            if ($AllowNotFound -and $errorText -match '(?i)(404|not found)') { return $null }
            Fail "GitHub API request failed"
        }
        $responseText = ($response -join "`n")
        if ([string]::IsNullOrWhiteSpace($responseText)) { return $null }
        return ($responseText | ConvertFrom-Json)
    } finally {
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}

function Assert-CanonicalRepository {
    if ($env:GITHUB_REPOSITORY -ne $canonicalRepository) {
        Fail "cleanup is permitted only for the canonical repository"
    }
}

function Assert-SourceIdentity {
    if ([string]::IsNullOrWhiteSpace($Tag) -or $Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
        Fail "Tag is not a canonical v4 release tag"
    }
    if ([string]::IsNullOrWhiteSpace($SourceSha) -or $SourceSha -notmatch '^[0-9a-fA-F]{40}$') {
        Fail "SourceSha must be an exact 40-character commit SHA"
    }
}

function Read-CleanupAuthorization {
    $path = Join-Path $script:isolatedStateRoot "draft-cleanup-authorized.json"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }
    try {
        return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    } catch {
        Fail "cleanup authorization marker is not valid JSON"
    }
}

function Assert-CleanupAuthorization([object]$Authorization) {
    if ($null -eq $Authorization -or [string](Get-PropertyValue $Authorization "status") -ne "READY") {
        Fail "cleanup authorization marker is missing; refusing to mutate pre-existing draft/tag"
    }
    if ([string](Get-PropertyValue $Authorization "tag") -ne $Tag -or
        [string](Get-PropertyValue $Authorization "source_sha") -ne $SourceSha.ToLowerInvariant()) {
        Fail "cleanup authorization marker does not match the requested source identity"
    }
}

function Read-CleanupState {
    $path = Join-Path $script:isolatedStateRoot "release-state.json"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }
    try {
        $state = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    } catch {
        Fail "release state is not valid JSON"
    }
    if ([string](Get-PropertyValue $state "tag") -ne $Tag -or
        [string](Get-PropertyValue $state "source_sha") -ne $SourceSha.ToLowerInvariant()) {
        Fail "release state identity does not match the requested cleanup"
    }
    $releaseId = Get-PropertyValue $state "release_id"
    if ($null -eq $releaseId -or [int64]$releaseId -le 0) {
        Fail "release state does not contain a valid release id"
    }
    return $state
}

function Get-ReleaseCollection([string]$Repository) {
    return @(Invoke-GitHubApi -Arguments @(
        "api", "--paginate", "--slurp", "repos/$Repository/releases?per_page=100"
    ))
}

function Get-ReleaseForTag([string]$Repository, [string]$RequestedTag) {
    $direct = Invoke-GitHubApi -Arguments @(
        "api", "repos/$Repository/releases/tags/$RequestedTag"
    ) -AllowNotFound
    $collection = if ($null -eq $direct) { Get-ReleaseCollection $Repository } else { @() }
    return Select-V4ReleaseByTag -DirectRelease $direct -ReleaseCollection $collection -Tag $RequestedTag
}

function Get-ReleaseForCleanup([string]$Repository, [object]$State) {
    if ($null -ne $State) {
        $releaseId = [int64](Get-PropertyValue $State "release_id")
        return Invoke-GitHubApi -Arguments @(
            "api", "repos/$Repository/releases/$releaseId"
        ) -AllowNotFound
    }
    return Get-ReleaseForTag $Repository $Tag
}

function Assert-DraftMatchesSource([object]$Release) {
    $releaseId = Get-PropertyValue $Release "id"
    if ($null -eq $releaseId -or [int64]$releaseId -le 0) {
        Fail "draft release id is missing"
    }
    if ([string](Get-PropertyValue $Release "tag_name") -ne $Tag) {
        Fail "draft release tag does not match the requested tag"
    }
    if (-not [bool](Get-PropertyValue $Release "draft") -or
        -not [string]::IsNullOrWhiteSpace([string](Get-PropertyValue $Release "published_at"))) {
        Fail "refusing to delete a published release"
    }
    $targetCommitish = [string](Get-PropertyValue $Release "target_commitish")
    if ($targetCommitish -notmatch '^[0-9a-fA-F]{40}$' -or
        $targetCommitish.ToLowerInvariant() -ne $SourceSha.ToLowerInvariant()) {
        Fail "refusing to delete a draft with a mismatched source"
    }
    $body = [string](Get-PropertyValue $Release "body")
    $source = $SourceSha.ToLowerInvariant()
    if ($body -notmatch "(?m)^source_sha:\s*$([regex]::Escape($source))\s*$") {
        Fail "refusing to delete a draft with a mismatched source_sha"
    }
}

function Assert-TagMatchesSource([object]$Ref) {
    $object = Get-PropertyValue $Ref "object"
    $sha = [string](Get-PropertyValue $object "sha")
    if ($sha -notmatch '^[0-9a-fA-F]{40}$' -or
        $sha.ToLowerInvariant() -ne $SourceSha.ToLowerInvariant()) {
        Fail "refusing to delete a tag with a mismatched source"
    }
}

$script:isolatedStateRoot = Get-IsolatedStateRoot
Assert-CanonicalRepository
Assert-SourceIdentity
$authorization = Read-CleanupAuthorization
$state = Read-CleanupState
$repository = $env:GITHUB_REPOSITORY
$release = Get-ReleaseForCleanup $repository $state
$tagRef = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/ref/tags/$Tag") -AllowNotFound

if ($null -eq $authorization -and ($null -ne $release -or $null -ne $tagRef)) {
    Fail "cleanup authorization marker is missing; refusing to mutate pre-existing draft/tag"
}
if ($null -ne $authorization) { Assert-CleanupAuthorization $authorization }

$draftDeleted = $false
$tagDeleted = $false
if ($null -ne $release) {
    Assert-DraftMatchesSource $release
    $releaseId = Get-PropertyValue $release "id"
    if ($null -eq $releaseId) { Fail "draft release id is missing" }
    Invoke-GitHubApi -Arguments @(
        "api", "--method", "DELETE", "repos/$repository/releases/$releaseId"
    ) -AllowNotFound | Out-Null
    $remainingReleaseById = Invoke-GitHubApi -Arguments @(
        "api", "repos/$repository/releases/$releaseId"
    ) -AllowNotFound
    if ($null -ne $remainingReleaseById) { Fail "draft release could not be removed by release id" }
    $remainingRelease = Get-ReleaseForTag $repository $Tag
    if ($null -ne $remainingRelease) { Fail "draft release could not be removed" }
    $draftDeleted = $true
}

if ($null -ne $tagRef) {
    if ($null -eq $release) {
        Fail "refusing to delete an orphan tag without its matching draft release"
    }
    Assert-TagMatchesSource $tagRef
    Invoke-GitHubApi -Arguments @(
        "api", "--method", "DELETE", "repos/$repository/git/refs/tags/$Tag"
    ) -AllowNotFound | Out-Null
    $remainingTag = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/ref/tags/$Tag") -AllowNotFound
    if ($null -ne $remainingTag) { Fail "draft tag could not be removed" }
    $tagDeleted = $true
}

$evidence = [ordered]@{
    schema_version = 1
    status = "PASS"
    repository = $repository
    tag = $Tag
    source_sha = $SourceSha.ToLowerInvariant()
    draft_deleted = $draftDeleted
    tag_deleted = $tagDeleted
    published_release_refused = $false
    mismatched_source_refused = $false
}
$evidencePath = Join-Path $script:isolatedStateRoot "draft-cleanup.json"
$evidence | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $evidencePath -Encoding utf8
Write-Host "V4 draft rehearsal cleanup: PASS (draft_deleted=$draftDeleted; tag_deleted=$tagDeleted)"
