[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Capture", "Verify")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$StateRoot,

    [Parameter(Mandatory = $true)]
    [string]$Tag
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$canonicalRepository = "pumni/Sky-Auto-Player"
$metadataEndpoints = [ordered]@{
    stable = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json"
    beta = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json"
}
. (Join-Path $PSScriptRoot "v4_release_draft_lookup.ps1")

function Fail([string]$Message) {
    throw "V4 draft rehearsal external-state check failed closed: $Message"
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

function Invoke-ReadOnlyGitHubApi {
    param(
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [switch]$AllowNotFound
    )

    $errorPath = Join-Path $script:isolatedStateRoot ("gh-state-error-" + [guid]::NewGuid().ToString("N") + ".log")
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

function Get-ReleaseCollection([string]$Repository) {
    return @(Invoke-ReadOnlyGitHubApi -Arguments @(
        "api", "--paginate", "--slurp", "repos/$Repository/releases?per_page=100"
    ))
}

function Get-ReleaseForTag([string]$Repository, [string]$RequestedTag) {
    $direct = Invoke-ReadOnlyGitHubApi -Arguments @(
        "api", "repos/$Repository/releases/tags/$RequestedTag"
    ) -AllowNotFound
    $collection = if ($null -eq $direct) { Get-ReleaseCollection $Repository } else { @() }
    return Select-V4ReleaseByTag -DirectRelease $direct -ReleaseCollection $collection -Tag $RequestedTag
}

function Assert-CanonicalRepository {
    if ($env:GITHUB_REPOSITORY -ne $canonicalRepository) {
        Fail "external-state inspection is permitted only for the canonical repository"
    }
}

function Assert-TagIdentity {
    if ([string]::IsNullOrWhiteSpace($Tag) -or $Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
        Fail "Tag is not a canonical v4 release tag"
    }
}

function Get-LegacyLatestSnapshot {
    $release = Invoke-ReadOnlyGitHubApi -Arguments @(
        "api", "repos/$env:GITHUB_REPOSITORY/releases/latest"
    )
    if ($null -eq $release) { Fail "legacy GitHub Latest release is unavailable" }
    $tagName = [string](Get-PropertyValue $release "tag_name")
    if ($tagName -notmatch '^v3\.') { Fail "GitHub Latest is not the expected immutable legacy v3 release" }
    if ([bool](Get-PropertyValue $release "draft") -or
        [string]::IsNullOrWhiteSpace([string](Get-PropertyValue $release "published_at"))) {
        Fail "GitHub Latest is not a published non-draft release"
    }

    $assetRecords = @(
        @(Get-PropertyValue $release "assets") | ForEach-Object {
            $digest = Get-PropertyValue $_ "digest"
            [ordered]@{
                name = [string](Get-PropertyValue $_ "name")
                size = [int64](Get-PropertyValue $_ "size")
                digest = if ($null -eq $digest) { $null } else { [string]$digest }
                browser_download_url = [string](Get-PropertyValue $_ "browser_download_url")
            }
        } | Sort-Object -Property name
    )
    return [ordered]@{
        id = [int64](Get-PropertyValue $release "id")
        tag_name = $tagName
        draft = [bool](Get-PropertyValue $release "draft")
        prerelease = [bool](Get-PropertyValue $release "prerelease")
        published_at = [string](Get-PropertyValue $release "published_at")
        assets = @($assetRecords)
    }
}

function Get-RawMetadataSnapshot([string]$Channel) {
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $request = $null
    $response = $null
    try {
        $url = [string]$metadataEndpoints[$Channel]
        $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Get, $url)
        if ($null -ne $request.Headers.Authorization) {
            Fail "raw metadata request must not carry an Authorization header"
        }
        $request.Headers.UserAgent.ParseAdd("Sky-Auto-Player-v4-draft-rehearsal/1.0")
        $response = $client.SendAsync($request).GetAwaiter().GetResult()
        if ($response.StatusCode -eq [System.Net.HttpStatusCode]::NotFound) {
            return [ordered]@{
                channel = $Channel
                endpoint = $url
                status = 404
                byte_length = 0
                sha256 = $null
            }
        }
        if ($response.StatusCode -ne [System.Net.HttpStatusCode]::OK) {
            Fail "raw $Channel metadata endpoint returned unexpected status $([int]$response.StatusCode)"
        }
        $bytes = $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $digest = [Convert]::ToHexString($sha.ComputeHash($bytes)).ToLowerInvariant()
        } finally {
            $sha.Dispose()
        }
        return [ordered]@{
            channel = $Channel
            endpoint = $url
            status = 200
            byte_length = [int64]$bytes.Length
            sha256 = $digest
        }
    } finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
        $client.Dispose()
        $handler.Dispose()
    }
}

function Get-TargetResidue {
    $release = Get-ReleaseForTag $env:GITHUB_REPOSITORY $Tag
    $tagRef = Invoke-ReadOnlyGitHubApi -Arguments @(
        "api", "repos/$env:GITHUB_REPOSITORY/git/ref/tags/$Tag"
    ) -AllowNotFound
    return [ordered]@{
        release_present = ($null -ne $release)
        tag_present = ($null -ne $tagRef)
    }
}

function Get-ExternalSnapshot {
    $target = Get-TargetResidue
    return [ordered]@{
        schema_version = 1
        repository = $env:GITHUB_REPOSITORY
        legacy_latest = Get-LegacyLatestSnapshot
        metadata = [ordered]@{
            stable = Get-RawMetadataSnapshot "stable"
            beta = Get-RawMetadataSnapshot "beta"
        }
        target = $target
    }
}

function Convert-SnapshotToJson([object]$Snapshot) {
    return $Snapshot | ConvertTo-Json -Compress -Depth 30
}

function Get-JsonSha256([string]$Json) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return [Convert]::ToHexString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Json))).ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Write-JsonFile([string]$Path, [object]$Value) {
    $json = $Value | ConvertTo-Json -Depth 30
    [IO.File]::WriteAllText($Path, $json + "`n", [Text.UTF8Encoding]::new($false))
}

$script:isolatedStateRoot = Get-IsolatedStateRoot
Assert-CanonicalRepository
Assert-TagIdentity
$beforePath = Join-Path $script:isolatedStateRoot "external-state-before.json"
$afterPath = Join-Path $script:isolatedStateRoot "external-state-after.json"
$verificationPath = Join-Path $script:isolatedStateRoot "external-state-verification.json"

if ($Mode -eq "Capture") {
    $snapshot = Get-ExternalSnapshot
    if ([bool]$snapshot.target.release_present -or [bool]$snapshot.target.tag_present) {
        Fail "target release/tag already exists before controlled draft rehearsal"
    }
    Write-JsonFile $beforePath $snapshot
    Write-JsonFile (Join-Path $script:isolatedStateRoot "draft-cleanup-authorized.json") ([ordered]@{
        schema_version = 1
        status = "READY"
        repository = $env:GITHUB_REPOSITORY
        tag = $Tag
        source_sha = [string]$env:V4_DRAFT_REHEARSAL_SOURCE_SHA
        captured_utc = [DateTime]::UtcNow.ToString("o")
    })
    Write-Host "V4 draft rehearsal external-state capture: PASS (legacy Latest and stable/beta metadata snapshotted)"
    exit 0
}

if (-not (Test-Path -LiteralPath $beforePath -PathType Leaf)) {
    Fail "external-state-before.json is missing"
}
$before = Get-Content -LiteralPath $beforePath -Raw | ConvertFrom-Json
$after = Get-ExternalSnapshot
if ([bool]$after.target.release_present -or [bool]$after.target.tag_present) {
    Fail "controlled draft rehearsal left a release or tag residue"
}
Write-JsonFile $afterPath $after
$beforeJson = Convert-SnapshotToJson $before
$afterJson = Convert-SnapshotToJson $after
$beforeHash = Get-JsonSha256 $beforeJson
$afterHash = Get-JsonSha256 $afterJson
if ($beforeJson -ne $afterJson) {
    Fail "legacy Latest or stable/beta metadata changed during controlled draft rehearsal"
}
Write-JsonFile $verificationPath ([ordered]@{
    schema_version = 1
    status = "PASS"
    repository = $env:GITHUB_REPOSITORY
    tag = $Tag
    target_release_absent = $true
    target_tag_absent = $true
    legacy_latest_unchanged = $true
    stable_metadata_unchanged = $true
    beta_metadata_unchanged = $true
    before_sha256 = $beforeHash
    after_sha256 = $afterHash
})
Write-Host "V4 draft rehearsal external-state verification: PASS (draft/tag absent; legacy Latest and stable/beta metadata unchanged)"
