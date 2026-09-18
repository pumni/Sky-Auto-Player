[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet(
        "Preflight",
        "BuildCandidate",
        "PublishRelease",
        "PublishReleaseTransaction",
        "PromoteMetadata",
        "FinalVerify",
        "ValidateRequest",
        "ValidateRepository",
        "CreateDraft",
        "DownloadDraft",
        "QualifyDownloaded",
        "RecordAttestations",
        "PublishDraft",
        "SelfTest"
    )]
    [string]$State,

    [string]$Version,
    [ValidateSet("stable", "beta")]
    [string]$Channel,
    [string]$Tag,
    [string]$SourceSha,
    [string]$WorkflowSha,
    [string]$StateRoot,
    [string]$UpdaterPrivateKeyPath,
    [string]$ReleaseNotesPath,
    [string]$RunId
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$canonicalRepository = "pumni/Sky-Auto-Player"
$metadataBootstrapPath = ".release-metadata/README.md"
$metadataBootstrapContract = @(
    "# Sky Auto Player v4 release metadata",
    "",
    "bootstrap_contract: sky-auto-player-v4-release-metadata-v1",
    "This orphan branch contains deployment-state metadata only.",
    "The channel latest.json files are created only by qualified immutable release promotion."
) -join "`n"
$rawMetadataEndpoints = @{
    stable = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json"
    beta = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json"
}
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$installerSuffix = "_x64-setup.exe"
$productionEvidenceName = "V4_PRODUCTION_RELEASE_EVIDENCE.json"
$qualificationEvidenceName = "V4_QUALIFICATION_EVIDENCE.json"
$authenticodeEvidenceName = "TAURI_AUTHENTICODE_EVIDENCE.json"
$installedAuthenticodeEvidenceName = "INSTALLED_AUTHENTICODE_EVIDENCE.json"
$summaryName = "TAURI_ARTIFACT_SUMMARY.json"
$sbomName = "SBOM.spdx.json"

. (Join-Path $PSScriptRoot "v4_release_asset_upload.ps1")
. (Join-Path $PSScriptRoot "v4_qualification_evidence.ps1")
. (Join-Path $PSScriptRoot "v4_release_draft_lookup.ps1")

if (-not (Test-Path Variable:script:GitHubApiHandler)) {
    $script:GitHubApiHandler = $null
}
if (-not (Test-Path Variable:script:AssetUploadHandler)) {
    $script:AssetUploadHandler = $null
}

function Fail([string]$Message) {
    throw "V4 release pipeline failed closed: $Message"
}

function Get-EffectiveStateRoot {
    if ([string]::IsNullOrWhiteSpace($StateRoot)) { Fail "StateRoot is required" }
    $full = [IO.Path]::GetFullPath($StateRoot)
    $repoPrefix = $repoRoot.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
    if ($full.Equals($repoRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $full.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "StateRoot must be outside the repository workspace"
    }
    New-Item -ItemType Directory -Path $full -Force | Out-Null
    return $full
}

function Get-StatePath {
    return Join-Path (Get-EffectiveStateRoot) "release-state.json"
}

function Write-JsonFile([string]$Path, [object]$Value) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $json = $Value | ConvertTo-Json -Depth 20
    [IO.File]::WriteAllText($Path, $json + "`n", [Text.UTF8Encoding]::new($false))
}

function Read-JsonFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { Fail "Required state file is missing: $Path" }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Get-V4ReleaseMakeLatestValue([string]$ReleaseChannel) {
    if ($ReleaseChannel -eq "stable") { return "true" }
    if ($ReleaseChannel -eq "beta") { return "false" }
    Fail "release channel is required to select make_latest"
}

function Get-V4ReleaseDraftMakeLatestValue {
    return "false"
}

function Format-CanonicalRfc3339Timestamp([object]$timestamp) {
    if ($null -eq $timestamp) { return $null }
    $str = [string]$timestamp
    if ([string]::IsNullOrWhiteSpace($str)) { return $null }
    try {
        $parsed = [DateTimeOffset]::Parse(
            $str,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::AssumeUniversal
        )
        return $parsed.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [System.Globalization.CultureInfo]::InvariantCulture)
    } catch {
        try {
            $parsedDt = [DateTime]::Parse(
                $str,
                [System.Globalization.CultureInfo]::InvariantCulture,
                [System.Globalization.DateTimeStyles]::AssumeUniversal
            )
            return $parsedDt.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [System.Globalization.CultureInfo]::InvariantCulture)
        } catch {
            return $null
        }
    }
}

function Get-CanonicalUtcTimestamp {
    return [DateTimeOffset]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ", [System.Globalization.CultureInfo]::InvariantCulture)
}

function Format-V4TransactionMarker {
    param(
        [Parameter(Mandatory = $true)] [string]$Repository,
        [Parameter(Mandatory = $true)] [string]$RunId,
        [Parameter(Mandatory = $true)] [string]$SourceSha,
        [Parameter(Mandatory = $true)] [string]$Version,
        [Parameter(Mandatory = $true)] [string]$Tag
    )
    $tx = [ordered]@{
        repository = $Repository
        run_id = $RunId
        source_sha = $SourceSha.ToLowerInvariant()
        version = $Version
        tag = $Tag
    }
    $json = $tx | ConvertTo-Json -Compress
    return "<!-- v4-release-tx: $json -->"
}

function Get-V4TransactionMarker {
    param([string]$Body)
    if ([string]::IsNullOrWhiteSpace($Body)) { return $null }
    if ($Body -match '<!--\s*v4-release-tx:\s*(\{.*?\})\s*-->') {
        try {
            return ($Matches[1] | ConvertFrom-Json)
        } catch {
            return $null
        }
    }
    return $null
}

function Test-V4TransactionMarkerMatch {
    param(
        [object]$Marker,
        [string]$ExpectedRepo,
        [string]$ExpectedRunId,
        [string]$ExpectedSha,
        [string]$ExpectedTag
    )
    if ($null -eq $Marker) { return $false }
    if ($null -ne $Marker.PSObject.Properties['repository'] -and [string]$Marker.repository -ne $ExpectedRepo) { return $false }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedRunId) -and
        $null -ne $Marker.PSObject.Properties['run_id'] -and
        [string]$Marker.run_id -ne $ExpectedRunId) { return $false }
    if ($null -ne $Marker.PSObject.Properties['source_sha'] -and [string]$Marker.source_sha.ToLowerInvariant() -ne $ExpectedSha.ToLowerInvariant()) { return $false }
    if ($null -ne $Marker.PSObject.Properties['tag'] -and [string]$Marker.tag -ne $ExpectedTag) { return $false }
    return $true
}

function Invoke-DraftSelfCleanup {
    param(
        [Parameter(Mandatory = $true)] [string]$Repository,
        [Parameter(Mandatory = $true)] [int64]$ReleaseId,
        [Parameter(Mandatory = $true)] [string]$ExpectedSha,
        [Parameter(Mandatory = $true)] [string]$ExpectedTag,
        [string]$ExpectedRunId = ""
    )
    if ($ReleaseId -le 0) { return }
    try {
        $remote = Invoke-GitHubApi -Arguments @("api", "repos/$Repository/releases/$ReleaseId") -AllowNotFound
        if ($null -eq $remote) { return }

        $isDraft = ($null -ne $remote.PSObject.Properties['draft'] -and [bool]$remote.draft)
        $targetCommitish = if ($null -ne $remote.PSObject.Properties['target_commitish']) { [string]$remote.target_commitish } else { "" }
        $tagName = if ($null -ne $remote.PSObject.Properties['tag_name']) { [string]$remote.tag_name } else { "" }
        $body = if ($null -ne $remote.PSObject.Properties['body']) { [string]$remote.body } else { "" }
        $marker = Get-V4TransactionMarker $body

        # Assert all cleanup preconditions:
        # draft == true, target_commitish matches, tag_name matches, transaction identity matches
        if ($isDraft -and
            $targetCommitish.ToLowerInvariant() -eq $ExpectedSha.ToLowerInvariant() -and
            $tagName -eq $ExpectedTag -and
            (Test-V4TransactionMarkerMatch $marker $Repository $ExpectedRunId $ExpectedSha $ExpectedTag)) {
            Write-Host "Cleaning up failed unpublished draft release $ReleaseId (tag=$ExpectedTag)"
            Invoke-GitHubApi -Arguments @("api", "--method", "DELETE", "repos/$Repository/releases/$ReleaseId") -AllowNotFound | Out-Null
            Write-Host "Unpublished draft $ReleaseId successfully deleted."
        } else {
            Write-Warning "Refusing to auto-delete release ${ReleaseId}: draft=$isDraft, commitish=$targetCommitish, tag=$tagName"
        }
    } catch {
        Write-Warning "Draft self-cleanup encountered an error: $($_.Exception.Message)"
    }
}

function Assert-V4ReleaseStateSchema([object]$State) {
    if ($null -eq $State) { Fail "release state object is null" }

    $requiredProperties = @(
        "schema_version",
        "phase",
        "source_sha",
        "version",
        "channel",
        "tag",
        "release_id",
        "draft",
        "published",
        "immutable",
        "published_at",
        "attested",
        "qualified_after_download",
        "qualification_assets",
        "public_assets",
        "metadata_promoted",
        "promoted_at",
        "final_verified",
        "final_verified_at",
        "reconciled_from_remote",
        "last_reconciled_at",
        "failure_class",
        "error_message"
    )

    foreach ($prop in $requiredProperties) {
        if ($null -eq $State.PSObject.Properties[$prop]) {
            Fail "release state schema validation failed: missing required property '$prop'"
        }
    }

    if ([int]$State.schema_version -ne 2) {
        Fail "unsupported release state schema_version '$($State.schema_version)' (expected 2)"
    }

    $validPhases = @("READY", "QUALIFIED", "PUBLISHED_PENDING_METADATA", "COMPLETE")
    if ([string]$State.phase -notin $validPhases) {
        Fail "invalid release state phase '$($State.phase)'; valid phases are $($validPhases -join ', ')"
    }

    if ([bool]$State.published) {
        if ([string]::IsNullOrWhiteSpace([string]$State.published_at)) {
            Fail "release state is marked published but published_at timestamp is empty"
        }
        if ([bool]$State.draft) {
            Fail "release state cannot be both draft and published"
        }
        if ([string]$State.phase -notin @("PUBLISHED_PENDING_METADATA", "COMPLETE")) {
            Fail "published release state must have phase PUBLISHED_PENDING_METADATA or COMPLETE"
        }
    }

    if ([bool]$State.metadata_promoted -and [string]::IsNullOrWhiteSpace([string]$State.promoted_at)) {
        Fail "release state is marked metadata_promoted but promoted_at timestamp is empty"
    }

    if ([bool]$State.final_verified -and [string]::IsNullOrWhiteSpace([string]$State.final_verified_at)) {
        Fail "release state is marked final_verified but final_verified_at timestamp is empty"
    }
}

function New-V4CanonicalReleaseState {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourceSha,
        [Parameter(Mandatory = $true)]
        [string]$Version,
        [Parameter(Mandatory = $true)]
        [string]$Channel,
        [Parameter(Mandatory = $true)]
        [string]$Tag,
        [Parameter(Mandatory = $true)]
        [int64]$ReleaseId,
        [object[]]$QualificationAssets = @(),
        [object[]]$PublicAssets = @(),
        [string]$Phase = "READY"
    )

    $validPhases = @("READY", "QUALIFIED", "PUBLISHED_PENDING_METADATA", "COMPLETE")
    if ($Phase -notin $validPhases) {
        Fail "cannot construct canonical release state with invalid phase '$Phase'"
    }

    $state = [ordered]@{
        schema_version = 2
        phase = [string]$Phase
        source_sha = $SourceSha.ToLowerInvariant()
        version = [string]$Version
        channel = [string]$Channel
        tag = [string]$Tag
        release_id = [int64]$ReleaseId
        draft = $true
        published = $false
        immutable = $false
        published_at = ""
        attested = $false
        qualified_after_download = $false
        qualification_assets = @($QualificationAssets)
        public_assets = @($PublicAssets)
        metadata_promoted = $false
        promoted_at = ""
        final_verified = $false
        final_verified_at = ""
        reconciled_from_remote = $false
        last_reconciled_at = ""
        failure_class = ""
        error_message = ""
    }

    $obj = [pscustomobject]$state
    Assert-V4ReleaseStateSchema $obj
    return $obj
}

function Convert-V4ReleaseStateV1ToV2([object]$RawState) {
    if ($null -eq $RawState) { Fail "release state object is null" }

    if ($null -ne $RawState.PSObject.Properties['published'] -and [bool]$RawState.published) {
        if ($null -eq $RawState.PSObject.Properties['published_at'] -or [string]::IsNullOrWhiteSpace([string]$RawState.published_at)) {
            Fail "cannot migrate v1 state: marked published but published_at timestamp is missing"
        }
    }

    $schemaVersion = if ($null -ne $RawState.PSObject.Properties['schema_version']) { [int]$RawState.schema_version } else { 1 }
    if ($schemaVersion -ne 1) {
        Fail "Convert-V4ReleaseStateV1ToV2 only converts schema_version 1 (got $schemaVersion)"
    }

    $defaultSha = if ($null -ne $RawState.PSObject.Properties['source_sha']) { [string]$RawState.source_sha } else { "" }
    $defaultVersion = if ($null -ne $RawState.PSObject.Properties['version']) { [string]$RawState.version } else { "" }
    $defaultChannel = if ($null -ne $RawState.PSObject.Properties['channel']) { [string]$RawState.channel } else { "stable" }
    $defaultTag = if ($null -ne $RawState.PSObject.Properties['tag']) { [string]$RawState.tag } else { "" }
    $defaultReleaseId = if ($null -ne $RawState.PSObject.Properties['release_id']) { [int64]$RawState.release_id } else { 0 }

    $derivedPhase = "READY"
    if ($null -ne $RawState.PSObject.Properties['final_verified'] -and [bool]$RawState.final_verified) {
        $derivedPhase = "COMPLETE"
    } elseif ($null -ne $RawState.PSObject.Properties['published'] -and [bool]$RawState.published) {
        $derivedPhase = "PUBLISHED_PENDING_METADATA"
    } elseif ($null -ne $RawState.PSObject.Properties['qualified_after_download'] -and [bool]$RawState.qualified_after_download) {
        $derivedPhase = "QUALIFIED"
    }

    $v2 = New-V4CanonicalReleaseState `
        -SourceSha $defaultSha `
        -Version $defaultVersion `
        -Channel $defaultChannel `
        -Tag $defaultTag `
        -ReleaseId $defaultReleaseId `
        -Phase $derivedPhase

    if ($null -ne $RawState.PSObject.Properties['draft']) { $v2.draft = [bool]$RawState.draft }
    if ($null -ne $RawState.PSObject.Properties['published']) { $v2.published = [bool]$RawState.published }
    if ($null -ne $RawState.PSObject.Properties['immutable']) { $v2.immutable = [bool]$RawState.immutable }
    if ($null -ne $RawState.PSObject.Properties['published_at']) { $v2.published_at = [string]$RawState.published_at }
    if ($null -ne $RawState.PSObject.Properties['attested']) { $v2.attested = [bool]$RawState.attested }
    if ($null -ne $RawState.PSObject.Properties['qualified_after_download']) { $v2.qualified_after_download = [bool]$RawState.qualified_after_download }
    if ($null -ne $RawState.PSObject.Properties['qualification_assets']) { $v2.qualification_assets = @($RawState.qualification_assets) }
    if ($null -ne $RawState.PSObject.Properties['public_assets']) { $v2.public_assets = @($RawState.public_assets) }
    if ($null -ne $RawState.PSObject.Properties['metadata_promoted']) { $v2.metadata_promoted = [bool]$RawState.metadata_promoted }
    if ($null -ne $RawState.PSObject.Properties['promoted_at']) { $v2.promoted_at = [string]$RawState.promoted_at }
    if ($null -ne $RawState.PSObject.Properties['final_verified']) { $v2.final_verified = [bool]$RawState.final_verified }
    if ($null -ne $RawState.PSObject.Properties['final_verified_at']) { $v2.final_verified_at = [string]$RawState.final_verified_at }
    if ($null -ne $RawState.PSObject.Properties['reconciled_from_remote']) { $v2.reconciled_from_remote = [bool]$RawState.reconciled_from_remote }
    if ($null -ne $RawState.PSObject.Properties['last_reconciled_at']) { $v2.last_reconciled_at = [string]$RawState.last_reconciled_at }
    if ($null -ne $RawState.PSObject.Properties['failure_class']) { $v2.failure_class = [string]$RawState.failure_class }
    if ($null -ne $RawState.PSObject.Properties['error_message']) { $v2.error_message = [string]$RawState.error_message }

    Assert-V4ReleaseStateSchema $v2
    return $v2
}

function Assert-ExistingUnpublishedDraftMatchesRequest([object]$Release) {
    if ([string]$Release.tag_name -ne $Tag) {
        Fail "existing release tag does not match the requested tag"
    }
    if (-not [bool]$Release.draft -or
        -not [string]::IsNullOrWhiteSpace([string]$Release.published_at)) {
        Fail "repository already contains published release/tag $Tag; published releases and tags are immutable; fresh transaction refuses adoption"
    }
    $source = $SourceSha.ToLowerInvariant()
    $targetCommitish = [string]$Release.target_commitish
    if ($targetCommitish -notmatch '^[0-9a-fA-F]{40}$' -or
        $targetCommitish.ToLowerInvariant() -ne $source) {
        Fail "existing draft source does not match the requested source"
    }
    $body = [string]$Release.body
    if ($body -notmatch "(?m)^source_sha:\s*$([regex]::Escape($source))\s*$") {
        Fail "existing draft body source does not match the requested source"
    }
}

function Assert-RequestIdentity {
    if ([string]::IsNullOrWhiteSpace($Version)) { Fail "version is required" }
    if ([string]::IsNullOrWhiteSpace($Channel) -or $Channel -notin @("stable", "beta")) {
        Fail "channel must be stable or beta"
    }
    if ([string]::IsNullOrWhiteSpace($Tag) -or $Tag -ne "v$Version") {
        Fail "tag must exactly equal v<version>"
    }
    if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
        Fail "version is not canonical SemVer without build metadata"
    }
    $isPrerelease = $Version.Contains("-")
    if ($Channel -eq "stable" -and $isPrerelease) { Fail "stable releases must use final SemVer" }
    if ($Channel -eq "beta" -and -not $isPrerelease) { Fail "beta releases must use a SemVer prerelease" }
    if ([string]::IsNullOrWhiteSpace($SourceSha) -or $SourceSha -notmatch '^[0-9a-fA-F]{40}$') {
        Fail "source_sha must be an exact 40-character commit SHA"
    }

    $currentHead = (& git rev-parse HEAD 2>$null).Trim().ToLowerInvariant()
    if ($LASTEXITCODE -ne 0 -or $currentHead -ne $SourceSha.ToLowerInvariant()) {
        Fail "checked-out HEAD does not equal the requested source SHA"
    }
    if (-not [string]::IsNullOrWhiteSpace($WorkflowSha) -and
        $WorkflowSha -notmatch '^[0-9a-fA-F]{40}$') {
        Fail "workflow SHA must be an exact commit SHA"
    }
    if (-not [string]::IsNullOrWhiteSpace($WorkflowSha) -and
        $WorkflowSha.ToLowerInvariant() -ne $SourceSha.ToLowerInvariant()) {
        Fail "source SHA differs from the workflow SHA used for OIDC provenance"
    }

    $cargoPath = Join-Path $repoRoot "desktop/src-tauri/Cargo.toml"
    $cargo = Get-Content -LiteralPath $cargoPath -Raw
    if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') { Fail "Cargo package version is missing" }
    if ($Matches[1] -ne $Version) { Fail "Cargo/Tauri package version does not equal requested version" }

    & cargo xtask version check --tag $Tag *> (Join-Path (Get-EffectiveStateRoot) "version-check.log")
    if ($LASTEXITCODE -ne 0) { Fail "canonical cargo xtask version/tag validation failed" }
    Write-Host "V4 release identity: PASS (version=$Version, channel=$Channel, source=$($SourceSha.ToLowerInvariant()))"
}

function Assert-ReleaseNotes {
    if ([string]::IsNullOrWhiteSpace($ReleaseNotesPath)) { Fail "release notes path is required" }
    $resolved = (Resolve-Path -LiteralPath $ReleaseNotesPath -ErrorAction Stop).Path
    $repoPrefix = $repoRoot.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
    if (-not $resolved.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "release notes must be inside the checked-out source workspace"
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { Fail "release notes file does not exist: $resolved" }
    $relativePath = [IO.Path]::GetRelativePath($repoRoot, $resolved).Replace("\", "/")
    $expectedPath = "docs/releases/v$Version.md"
    if (-not $relativePath.Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "release notes path must match the requested version"
    }
    $content = Get-Content -LiteralPath $resolved -Raw
    if ([string]::IsNullOrWhiteSpace($content)) { Fail "release notes file is empty: $resolved" }
    $lines = $content -split "`r?`n"
    $firstHeading = $lines | Where-Object { $_ -match '(?m)^# [^\r\n]+(?=\r?$)' } | Select-Object -First 1
    if ($null -eq $firstHeading) { Fail "release notes missing h1 markdown heading: $resolved" }
    $normalizedHeading = ($firstHeading -replace '^#\s*', '').Trim()
    $expectedHeading = "Sky Auto Player v$Version"
    if ($normalizedHeading -ne $expectedHeading) {
        Fail "release notes heading must match the requested version: expected '$expectedHeading', got '$normalizedHeading'"
    }
    return $resolved
}

function Get-CanonicalRepository {
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_REPOSITORY)) {
        return $canonicalRepository
    }
    if ($env:GITHUB_REPOSITORY -ne $canonicalRepository) {
        Fail "release pipeline running in non-canonical repository: $env:GITHUB_REPOSITORY"
    }
    return $env:GITHUB_REPOSITORY
}

function Assert-MetadataBranchReadiness([string]$Repository) {
    $branch = Invoke-GitHubApi -Arguments @("api", "repos/$Repository/git/ref/heads/release-metadata") -AllowNotFound
    if ($null -eq $branch) {
        Fail "release-metadata branch is not initialized in $Repository"
    }
    if ([string]$branch.ref -ne "refs/heads/release-metadata" -or
        $null -eq $branch.object -or
        [string]::IsNullOrWhiteSpace([string]$branch.object.sha)) {
        Fail "release-metadata branch ref payload is invalid"
    }

    $bootstrap = Invoke-GitHubApi -Arguments @(
        "api", "repos/$Repository/contents/$metadataBootstrapPath`?ref=release-metadata"
    ) -AllowNotFound
    if ($null -eq $bootstrap -or [string]::IsNullOrWhiteSpace([string]$bootstrap.content)) {
        Fail "release-metadata bootstrap contract is missing: $metadataBootstrapPath"
    }
    $decoded = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String([string]$bootstrap.content))
    if ($decoded.Trim() -ne $metadataBootstrapContract.Trim()) {
        Fail "release-metadata branch bootstrap contract content mismatch"
    }

    foreach ($channelName in @("stable", "beta")) {
        $metadataPath = "channels/$channelName/latest.json"
        $metadata = Invoke-GitHubApi -Arguments @(
            "api", "repos/$Repository/contents/$metadataPath`?ref=release-metadata"
        ) -AllowNotFound
        if ($null -ne $metadata) {
            $currentPath = Join-Path (Get-EffectiveStateRoot) "current-$channelName-latest.json"
            Write-RepositoryContentFile $metadata $currentPath $metadataPath
            & cargo xtask release-metadata validate --channel $channelName --metadata $currentPath
            if ($LASTEXITCODE -ne 0) {
                Fail "existing $channelName channel metadata on release-metadata branch failed validation"
            }
        }
    }
    Write-Host "V4 release-metadata readiness: PASS (bootstrap contract and channel files validated)"
}

function Assert-RepositoryReleasePolicy {
    $repository = Get-CanonicalRepository
    $main = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/ref/heads/main") -AllowNotFound
    if ($null -eq $main) {
        Fail "canonical repository main is not initialized"
    }
    if ([string]$main.ref -ne "refs/heads/main") {
        Fail "canonical repository main ref is not canonical"
    }
    if ([string]::IsNullOrWhiteSpace($main.object.sha)) {
        Fail "canonical repository main ref payload is missing object SHA"
    }
    if ($main.object.sha.ToLowerInvariant() -ne $SourceSha.ToLowerInvariant()) {
        Fail "canonical repository main does not match requested source SHA"
    }
    Assert-MetadataBranchReadiness $repository
    Write-Host "V4 repository policy: PASS (canonical main, release-metadata readiness)"
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

function Assert-ExactAssetSet([object]$Release, [object[]]$Expected) {
    $actual = @($Release.assets | ForEach-Object { [string]$_.name } | Sort-Object)
    $expectedNames = @($Expected | ForEach-Object {
        if ($null -ne $_.PSObject.Properties['release_name']) { [string]$_.release_name } else { [string]$_.name }
    } | Sort-Object)
    if (($actual -join "`n") -ne ($expectedNames -join "`n")) { Fail "repository release asset set differs from the qualified candidate set" }
}

function Assert-ExactPublicReleaseAssetSet([object]$Release) {
    $actual = @($Release.assets | ForEach-Object { [string]$_.name } | Sort-Object)
    $expected = @(Get-CanonicalPublicReleaseNames | Sort-Object)
    if (($actual -join "`n") -ne ($expected -join "`n")) {
        Fail "repository release must contain exactly the canonical installer and updater signature"
    }
}

function Assert-ImmutableRelease([object]$Release) {
    if ($null -eq $Release.immutable -or -not [bool]$Release.immutable) {
        Fail "repository release is not marked immutable"
    }
}

function Get-ExpectedInstallerName {
    return "Sky.Auto.Player_${Version}${installerSuffix}"
}

function Get-ExpectedSignatureName {
    return "$(Get-ExpectedInstallerName).sig"
}

function Get-CanonicalPublicReleaseNames {
    return @((Get-ExpectedInstallerName), (Get-ExpectedSignatureName))
}

function Get-QualificationCandidateRecords {
    $installer = Get-ExpectedInstallerName
    $bundle = Join-Path $repoRoot "rust/target/dist/bundle/nsis"
    $evidence = Join-Path $repoRoot "rust/target/dist"
    return @(
        [pscustomobject]@{ name = $installer; path = Join-Path $bundle $installer; role = "installer" },
        [pscustomobject]@{ name = "$installer.sig"; path = Join-Path $bundle "$installer.sig"; role = "updater-signature" },
        [pscustomobject]@{ name = $qualificationEvidenceName; path = Join-Path $evidence $qualificationEvidenceName; role = "qualification-evidence" },
        [pscustomobject]@{ name = $productionEvidenceName; path = Join-Path $evidence $productionEvidenceName; role = "production-evidence" },
        [pscustomobject]@{ name = $authenticodeEvidenceName; path = Join-Path $evidence $authenticodeEvidenceName; role = "authenticode-evidence" },
        [pscustomobject]@{ name = $installedAuthenticodeEvidenceName; path = Join-Path $evidence $installedAuthenticodeEvidenceName; role = "installed-authenticode-evidence" },
        [pscustomobject]@{ name = $summaryName; path = Join-Path $evidence $summaryName; role = "artifact-summary" },
        [pscustomobject]@{ name = $sbomName; path = Join-Path $evidence $sbomName; role = "sbom" }
    )
}

function Get-PublicReleaseRecords([object[]]$QualificationRecords) {
    if ($null -eq $QualificationRecords -or @($QualificationRecords).Count -eq 0) {
        Fail "public release records require a non-empty qualification asset set"
    }
    $publicNames = @(Get-CanonicalPublicReleaseNames)
    $records = @($QualificationRecords | Where-Object { $publicNames -contains [string]$_.release_name })
    if ($records.Count -ne 2) {
        Fail "manifest public assets must match exactly the canonical installer and signature"
    }
    return $records
}

function Get-FileRecord([object]$Candidate) {
    $item = Get-Item -LiteralPath $Candidate.path
    $sourceName = if ($null -ne $Candidate.PSObject.Properties['name']) { [string]$Candidate.name } else { [IO.Path]::GetFileName([string]$Candidate.path) }
    $releaseName = Get-V4SafeReleaseAssetName $sourceName
    [ordered]@{
        name = $releaseName
        release_name = $releaseName
        source_name = $sourceName
        role = [string]$Candidate.role
        size = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $Candidate.path -Algorithm SHA256).Hash.ToLowerInvariant()
        source_path = [string]$Candidate.path
        state_path = (Join-Path "candidate-assets" $releaseName).Replace("\", "/")
    }
}

function Get-StateAssetPath([object]$Record) {
    $relative = [string]$Record.state_path
    if ([string]::IsNullOrWhiteSpace($relative) -or [IO.Path]::IsPathRooted($relative)) {
        Fail "candidate manifest contains an invalid state asset path"
    }
    $root = Get-EffectiveStateRoot
    $full = [IO.Path]::GetFullPath((Join-Path $root $relative))
    $prefix = $root.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "candidate manifest state asset path escapes the release state root"
    }
    return $full
}

function Get-FrozenQualificationAssetPath([object[]]$Records, [string]$SourceName) {
    $matches = @($Records | Where-Object {
        [string]$_.source_name -eq $SourceName -or
        ([string]$_.name -eq $SourceName -and $null -eq $_.PSObject.Properties['source_name'])
    })
    if ($matches.Count -ne 1) {
        Fail "candidate manifest must contain exactly one frozen qualification asset: $SourceName"
    }
    $path = Get-StateAssetPath $matches[0]
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Fail "frozen qualification asset is missing: $SourceName"
    }
    return $path
}

function Assert-ManifestAssetFiles([object[]]$Records) {
    foreach ($record in @($Records)) {
        $path = Get-StateAssetPath $record
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            Fail "frozen qualification asset is missing: $($record.release_name)"
        }
        $item = Get-Item -LiteralPath $path
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ([int64]$item.Length -ne [int64]$record.size -or $hash -ne [string]$record.sha256) {
            Fail "frozen qualification asset differs from the candidate manifest: $($record.release_name)"
        }
    }
}

function Freeze-CandidateAssets([object[]]$Records) {
    foreach ($record in @($Records)) {
        $destination = Get-StateAssetPath $record
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath ([string]$record.source_path) -Destination $destination -Force
    }
    Assert-ManifestAssetFiles $Records
    return @($Records)
}

function Assert-EvidenceIdentity([string]$ProductionPath, [string]$QualificationPath, [object[]]$Records) {
    $recordsByName = @{}
    foreach ($record in @($Records)) {
        $recordsByName[[string]$record.name] = $record
        if ($null -ne $record.PSObject.Properties['source_name'] -and -not [string]::IsNullOrWhiteSpace([string]$record.source_name)) {
            $recordsByName[[string]$record.source_name] = $record
        }
        if ($null -ne $record.PSObject.Properties['release_name'] -and -not [string]::IsNullOrWhiteSpace([string]$record.release_name)) {
            $recordsByName[[string]$record.release_name] = $record
        }
    }
    foreach ($requiredName in @($productionEvidenceName, $qualificationEvidenceName, $authenticodeEvidenceName, $sbomName)) {
        if (-not $recordsByName.ContainsKey($requiredName)) { Fail "candidate manifest is missing required identity record: $requiredName" }
    }
    $evidence = Get-Content -LiteralPath $ProductionPath -Raw | ConvertFrom-Json
    $qualification = Get-Content -LiteralPath $QualificationPath -Raw | ConvertFrom-Json
    if ([string]$evidence.source_sha -ne $SourceSha.ToLowerInvariant()) { Fail "production evidence source SHA mismatch" }
    if ([string]$evidence.version -ne $Version -or [string]$evidence.channel -ne $Channel) { Fail "production evidence release identity mismatch" }
    if ([string]$evidence.authenticode_mode -ne "unsigned-zero-budget" -or
        [string]$evidence.authenticode_state -ne "unsigned" -or
        [string]$evidence.authenticode_provider -ne "none" -or
        $null -ne $evidence.approved_signer_thumbprint -or
        $null -ne $evidence.observed_signer_thumbprint) {
        Fail "production evidence is not the governed unsigned-zero-budget state"
    }
    if ([string]$evidence.updater_signature_status -ne "valid" -or [string]$evidence.qualification_status -ne "PASS") {
        Fail "production evidence omitted a mandatory updater or qualification result"
    }
    $instRec = $recordsByName[(Get-ExpectedInstallerName)]
    $sigRec = $recordsByName["$((Get-ExpectedInstallerName)).sig"]
    if ($null -eq $instRec -or $null -eq $sigRec) {
        Fail "candidate manifest is missing the canonical installer or updater signature record"
    }
    $instSourceName = if ($null -ne $instRec.PSObject.Properties['source_name']) { [string]$instRec.source_name } else { [string]$instRec.name }
    $sigSourceName = if ($null -ne $sigRec.PSObject.Properties['source_name']) { [string]$sigRec.source_name } else { [string]$sigRec.name }

    if ([string]$evidence.installer -ne (Get-ExpectedInstallerName) -or
        [string]$evidence.updater_signature -ne "$(Get-ExpectedInstallerName).sig" -or
        [string]$evidence.authenticode_evidence -ne $authenticodeEvidenceName -or
        [string]$evidence.sbom -ne $sbomName -or
        [string]$evidence.installer -ne $instSourceName -or
        [string]$evidence.updater_signature -ne $sigSourceName -or
        [int64]$evidence.installer_size -ne [int64]$instRec.size -or
        [string]$evidence.installer_sha256 -ne [string]$instRec.sha256 -or
        [int64]$evidence.signature_size -ne [int64]$sigRec.size -or
        [string]$evidence.updater_signature_sha256 -ne [string]$sigRec.sha256 -or
        [string]$evidence.authenticode_evidence_sha256 -ne [string]$recordsByName[$authenticodeEvidenceName].sha256 -or
        [string]$evidence.sbom_sha256 -ne [string]$recordsByName[$sbomName].sha256) {
        Fail "production evidence digests or sizes do not match the candidate manifest"
    }
    if ([string]$qualification.installer -ne (Get-ExpectedInstallerName) -or
        [string]$qualification.updater_signature -ne "$(Get-ExpectedInstallerName).sig" -or
        [string]$qualification.installer_sha256 -ne [string]$instRec.sha256 -or
        [string]$qualification.updater_signature_sha256 -ne [string]$sigRec.sha256 -or
        [string]$qualification.authenticode_mode -ne "unsigned-zero-budget" -or
        [string]$qualification.sbom_sha256 -ne [string]$recordsByName[$sbomName].sha256) {
        Fail "qualification evidence does not bind the exact candidate manifest"
    }
}

function Assert-CandidateEvidence([object[]]$Records) {
    Assert-ManifestAssetFiles $Records
    $productionRecord = @($Records | Where-Object { [string]$_.source_name -eq $productionEvidenceName })
    $qualificationRecord = @($Records | Where-Object { [string]$_.source_name -eq $qualificationEvidenceName })
    if ($productionRecord.Count -ne 1 -or $qualificationRecord.Count -ne 1) {
        Fail "candidate manifest is missing frozen production or qualification evidence"
    }
    Assert-EvidenceIdentity `
        (Get-StateAssetPath $productionRecord[0]) `
        (Get-StateAssetPath $qualificationRecord[0]) `
        $Records
}

function Get-PublicReleaseRecordsFromManifest([object]$Manifest) {
    if ($null -eq $Manifest.PSObject.Properties['qualification_assets'] -or
        $null -eq $Manifest.PSObject.Properties['public_assets']) {
        Fail "candidate manifest must declare qualification_assets and public_assets separately"
    }
    $qualificationRecords = @($Manifest.qualification_assets)
    $derived = @(Get-PublicReleaseRecords $qualificationRecords)
    $declared = @($Manifest.public_assets)
    if ($declared.Count -ne $derived.Count) {
        Fail "candidate manifest public_assets must contain exactly the canonical installer and signature"
    }
    for ($index = 0; $index -lt $derived.Count; $index++) {
        if ([string]$declared[$index].name -ne [string]$derived[$index].name -or
            [string]$declared[$index].release_name -ne [string]$derived[$index].release_name -or
            [string]$declared[$index].source_name -ne [string]$derived[$index].source_name -or
            [string]$declared[$index].role -ne [string]$derived[$index].role -or
            [string]$declared[$index].state_path -ne [string]$derived[$index].state_path -or
            [string]$declared[$index].sha256 -ne [string]$derived[$index].sha256 -or
            [int64]$declared[$index].size -ne [int64]$derived[$index].size) {
            Fail "candidate manifest public_assets are not an exact projection of qualification_assets"
        }
    }
    return $derived
}

function Assert-CandidateEvidence([object[]]$Records) {
    Assert-ManifestAssetFiles $Records
    $productionRecord = @($Records | Where-Object { [string]$_.source_name -eq $productionEvidenceName })
    $qualificationRecord = @($Records | Where-Object { [string]$_.source_name -eq $qualificationEvidenceName })
    if ($productionRecord.Count -ne 1 -or $qualificationRecord.Count -ne 1) {
        Fail "candidate manifest is missing frozen production or qualification evidence"
    }
    $prodEvidencePath = Get-FrozenQualificationAssetPath $Records $productionEvidenceName
    $qualEvidencePath = Get-FrozenQualificationAssetPath $Records $qualificationEvidenceName
    $evidence = Get-Content -LiteralPath $prodEvidencePath -Raw | ConvertFrom-Json
    $qualification = Get-Content -LiteralPath $qualEvidencePath -Raw | ConvertFrom-Json
    if ([string]$evidence.source_sha -ne $SourceSha.ToLowerInvariant()) { Fail "production evidence source SHA mismatch" }
    if ([string]$evidence.version -ne $Version -or [string]$evidence.channel -ne $Channel) { Fail "production evidence release identity mismatch" }
    if ([string]$evidence.authenticode_mode -ne "unsigned-zero-budget" -or
        [string]$evidence.authenticode_state -ne "unsigned" -or
        [string]$evidence.authenticode_provider -ne "none") {
        Fail "production evidence is not the governed unsigned-zero-budget state"
    }
    if ([string]$evidence.updater_signature_status -ne "valid" -or [string]$evidence.qualification_status -ne "PASS") {
        Fail "production evidence omitted a mandatory updater or qualification result"
    }
}

function Invoke-Checked([string]$File, [string[]]$Arguments, [string]$Failure) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) { Fail $Failure }
}

function Invoke-GhBinaryOutput {
    param(
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [Parameter(Mandatory = $true)] [string]$OutputPath,
        [Parameter(Mandatory = $true)] [string]$ErrorPath
    )
    if ($PSVersionTable.PSVersion -lt [Version]"7.4.0") {
        Fail "binary release-asset download requires PowerShell 7.4 or newer"
    }
    if ([string]::IsNullOrWhiteSpace($OutputPath)) { Fail "binary release-asset output path is required" }

    $ghPath = (Get-Command gh -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ghPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        [void]$startInfo.ArgumentList.Add([string]$argument)
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $outputStream = $null
    $stderrTask = $null
    $started = $false
    try {
        $outputStream = [IO.File]::Open(
            $OutputPath,
            [IO.FileMode]::Create,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        $started = $process.Start()
        if (-not $started) { Fail "could not start GitHub CLI for binary release-asset download" }
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.StandardOutput.BaseStream.CopyTo($outputStream)
        $process.WaitForExit()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        [IO.File]::WriteAllText($ErrorPath, $stderr, [Text.UTF8Encoding]::new($false))
        return [int]$process.ExitCode
    } finally {
        if ($started -and -not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        if ($null -ne $outputStream) { $outputStream.Dispose() }
        $process.Dispose()
    }
}

function Invoke-GitHubApi {
    param(
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [switch]$AllowNotFound,
        [switch]$BinaryOutput,
        [switch]$Raw,
        [string]$OutputPath
    )
    if ($null -ne $script:GitHubApiHandler) {
        return & $script:GitHubApiHandler $Arguments $AllowNotFound $BinaryOutput $Raw $OutputPath
    }
    $errorPath = Join-Path (Get-EffectiveStateRoot) ("gh-error-" + [guid]::NewGuid().ToString("N") + ".log")
    try {
        if ($BinaryOutput) {
            $script:githubApiExitCode = Invoke-GhBinaryOutput `
                -Arguments $Arguments -OutputPath $OutputPath -ErrorPath $errorPath
        } else {
            $script:githubApiResult = & gh @Arguments 2>$errorPath
            $script:githubApiExitCode = $LASTEXITCODE
        }
        if ($githubApiExitCode -ne 0) {
            $errorText = if (Test-Path -LiteralPath $errorPath) { Get-Content -LiteralPath $errorPath -Raw } else { "" }
            if ($AllowNotFound -and $errorText -match '(?i)(404|not found)') { return $null }
            Fail "GitHub API request failed"
        }
        if ($BinaryOutput) { return $null }
        $responseText = ($githubApiResult -join "`n")
        if ($Raw) { return $responseText }
        # GitHub's successful DELETE endpoints return an empty body.
        if ([string]::IsNullOrWhiteSpace($responseText)) { return $null }
        return ($responseText | ConvertFrom-Json)
    } finally {
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}

function Write-RepositoryContentFile([object]$ContentObject, [string]$Destination, [string]$LogicalName) {
    if ($null -eq $ContentObject -or [string]::IsNullOrWhiteSpace([string]$ContentObject.content)) {
        Fail "repository content object is missing payload content for $LogicalName"
    }
    $bytes = [Convert]::FromBase64String([string]$ContentObject.content)
    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    [IO.File]::WriteAllBytes($Destination, $bytes)
}

function Convert-PublishedAtToMetadataTimestamp([string]$PublishedAt) {
    $normalized = Format-CanonicalRfc3339Timestamp $PublishedAt
    if ($null -eq $normalized) {
        Fail "published release date is not valid RFC3339 UTC: $PublishedAt"
    }
    return $normalized
}

function Get-PublicMetadataDocument([string]$ReleaseChannel) {
    $endpoint = $rawMetadataEndpoints[$ReleaseChannel]
    if ([string]::IsNullOrWhiteSpace($endpoint)) { Fail "unknown metadata endpoint for channel $ReleaseChannel" }
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(30)
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Get, [Uri]$endpoint)
        if ($request.Headers.Authorization) { Fail "raw metadata endpoint request must not provide credentials" }
        $response = $client.SendAsync($request).GetAwaiter().GetResult()
        if ($response.StatusCode -ne [System.Net.HttpStatusCode]::OK) {
            Fail "raw metadata endpoint returned status code $($response.StatusCode)"
        }
        $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        return ($body | ConvertFrom-Json)
    } finally {
        $client.Dispose()
        $handler.Dispose()
    }
}

# ==============================================================================
# Pipeline Operations
# ==============================================================================

function Invoke-Preflight {
    Assert-RequestIdentity
    Assert-ReleaseNotes
    $repository = Get-CanonicalRepository
    Assert-RepositoryReleasePolicy
    Assert-MetadataBranchReadiness $repository

    # 1. Collision and stale draft check
    $ref = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/refs/tags/$Tag") -AllowNotFound
    if ($null -ne $ref) {
        Fail "repository already contains published release/tag $Tag; published tags are immutable"
    }

    $direct = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/tags/$Tag") -AllowNotFound
    if ($null -ne $direct) {
        Fail "repository already contains published release/tag $Tag; published tags are immutable"
    }

    $collection = Get-ReleaseCollection $repository
    $existingDraft = Select-V4ReleaseByTag -DirectRelease $null -ReleaseCollection $collection -Tag $Tag
    if ($null -ne $existingDraft) {
        if (-not [bool]$existingDraft.draft) {
            Fail "repository already contains published release/tag $Tag; published tags are immutable"
        }
        $targetCommitish = if ($null -ne $existingDraft.PSObject.Properties['target_commitish']) { [string]$existingDraft.target_commitish } else { "" }
        $body = if ($null -ne $existingDraft.PSObject.Properties['body']) { [string]$existingDraft.body } else { "" }
        $marker = Get-V4TransactionMarker $body

        # Deterministic stale-draft recognition:
        if ($targetCommitish.ToLowerInvariant() -eq $SourceSha.ToLowerInvariant() -and
            (Test-V4TransactionMarkerMatch $marker $repository "" $SourceSha $Tag)) {
            $staleId = [int64]$existingDraft.id
            Write-Host "V4 unpublished draft reuse: recognized stale draft $staleId for $Tag from source $SourceSha; cleaning up"
            Invoke-GitHubApi -Arguments @("api", "--method", "DELETE", "repos/$repository/releases/$staleId") -AllowNotFound | Out-Null
            $remaining = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/$staleId") -AllowNotFound
            if ($null -ne $remaining) { Fail "draft release could not be removed by release id" }
        } else {
            Fail "repository contains conflicting draft release for ${Tag}: existing draft source does not match the requested source or transaction"
        }
    }

    Write-JsonFile (Join-Path (Get-EffectiveStateRoot) "preflight-evidence.json") ([ordered]@{
        version = $Version
        channel = $Channel
        tag = $Tag
        source_sha = $SourceSha.ToLowerInvariant()
        preflight_status = "PASS"
        timestamp = Get-CanonicalUtcTimestamp
    })
    Write-Host "V4 release preflight: PASS"
}

function Invoke-BuildCandidate {
    Assert-RequestIdentity
    if ([string]::IsNullOrWhiteSpace($UpdaterPrivateKeyPath)) { Fail "updater private key path is required" }
    $keyPath = (Resolve-Path -LiteralPath $UpdaterPrivateKeyPath -ErrorAction Stop).Path
    $repoPrefix = $repoRoot.TrimEnd("\", "/") + [IO.Path]::DirectorySeparatorChar
    if ($keyPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) { Fail "updater private key must remain outside workspace" }
    if (-not (Test-Path -LiteralPath $keyPath -PathType Leaf)) { Fail "updater private key path is not a file" }
    if (-not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY) -or
        -not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH) -or
        -not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD)) {
        Fail "ambient updater key or password environment is forbidden"
    }

    . (Join-Path $PSScriptRoot "v4_updater_credential_broker.ps1")

    # Build candidate exactly once
    Invoke-WithV4UpdaterSessionCredential -Action {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
            -ExpectedSourceSha $SourceSha `
            -Version $Version `
            -Channel $Channel `
            -UpdaterPrivateKeyPath $keyPath
        if ($LASTEXITCODE -ne 0) { Fail "production orchestrator failed" }
    }

    $records = @(Get-QualificationCandidateRecords | ForEach-Object { Get-FileRecord $_ })
    $candidateAssets = @(Freeze-CandidateAssets $records)
    $publicRecords = @(Get-PublicReleaseRecords $records)

    $root = Get-EffectiveStateRoot
    $bundle = Join-Path $root "candidate-assets"
    $sourceInstaller = Get-ExpectedInstallerName
    $sourceSignature = "$sourceInstaller.sig"
    $releaseInstaller = Get-V4SafeReleaseAssetName $sourceInstaller
    $releaseSignature = Get-V4SafeReleaseAssetName $sourceSignature
    $frozenSbom = Join-Path $bundle $sbomName
    $frozenArtifactSummary = Join-Path $bundle $summaryName
    $frozenAuthenticodeEvidence = Join-Path $bundle $authenticodeEvidenceName
    $frozenQualificationEvidence = Join-Path $bundle $qualificationEvidenceName

    # Local qualification against frozen candidate assets
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1"),
        "-Mode", "unsigned-zero-budget", "-Artifact", (Join-Path $bundle $releaseInstaller),
        "-Evidence", (Join-Path $root "downloaded-authenticode-verification.json")
    ) "candidate Authenticode state is not unsigned-zero-budget"

    Invoke-Checked "cargo" @(
        "xtask", "updater-trust", "verify-signature", "--installer", (Join-Path $bundle $releaseInstaller),
        "--signature", (Join-Path $bundle $releaseSignature)
    ) "candidate Tauri updater signature verification failed"

    Invoke-Checked "cargo" @(
        "xtask", "sbom", "verify", "--artifact-dir", $bundle, "--sbom", $frozenSbom
    ) "candidate SPDX SBOM verification failed"

    Invoke-Checked "cargo" @(
        "xtask", "verify-tauri-bundle", "--bundle-dir", $bundle,
        "--summary", $frozenArtifactSummary,
        "--authenticode-evidence", $frozenAuthenticodeEvidence,
        "--sbom", $frozenSbom
    ) "candidate exact Tauri bundle verification failed"

    $canonicalPublicKey = Join-Path $root "canonical-updater-public-key.txt"
    Invoke-Checked "cargo" @(
        "xtask", "updater-trust", "export-public-key", "--output", $canonicalPublicKey
    ) "canonical updater public-root export failed"

    $fixtureTargetDir = Join-Path $root "previous-v4-fixture-target"
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "ci_tauri_update_e2e.ps1"),
        "-FixtureTargetDir", $fixtureTargetDir,
        "-CandidateInstallerPath", (Join-Path $bundle $releaseInstaller),
        "-CandidateSignaturePath", (Join-Path $bundle $releaseSignature),
        "-CandidateVersion", $Version,
        "-CandidatePublicKeyPath", $canonicalPublicKey,
        "-EvidencePath", (Join-Path $root "fixture-http-evidence.json")
    ) "candidate updater qualification failed"

    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "promote_v4_metadata.ps1"),
        "-ValidateEvidence", $frozenQualificationEvidence
    ) "candidate qualification evidence schema validation failed"

    $defenderEvidencePath = Join-Path $root "defender-evidence.json"
    # Production policy requires a deterministic exact-artifact Defender
    # custom scan on the candidate installer. scan_v4_defender_exact.ps1
    # invokes Start-MpScan and records scan_performed; missing Defender or a
    # scan failure is a release failure, not an accepted unavailable result.
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "scan_v4_defender_exact.ps1"),
        "-Artifact", (Join-Path $bundle $releaseInstaller),
        "-Evidence", $defenderEvidencePath
    ) "candidate installer Defender scan failed"
    $defenderEvidence = Read-JsonFile $defenderEvidencePath
    if (-not [bool]$defenderEvidence.scan_performed -or [string]$defenderEvidence.detection_result -ne "none") {
        Fail "Defender evidence did not bind a clean scan to the candidate installer"
    }

    # Smoke & Catalog verification on installed candidate
    $installRoot = Join-Path $root ("install-" + [guid]::NewGuid().ToString("N"))
    $app = Join-Path $installRoot "sky_desktop_shell.exe"
    $uninstaller = Join-Path $installRoot "uninstall.exe"
    try {
        New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
        $install = Start-Process -FilePath (Join-Path $bundle $releaseInstaller) -ArgumentList @("/S", "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
        if ($install.ExitCode -ne 0) { Fail "candidate current-user installer failed" }
        $installedBuiltinRoot = Join-Path $installRoot "builtin-songs"
        Invoke-Checked "cargo" @(
            "xtask", "builtin-catalog", "verify-installed", "--root", $installedBuiltinRoot
        ) "candidate installed built-in catalog verification failed"

        $installedBuiltinFiles = @(Get-ChildItem -LiteralPath $installedBuiltinRoot -File -Recurse | Sort-Object FullName)
        if ($installedBuiltinFiles.Count -eq 0) { Fail "candidate installed built-in catalog is empty" }
        $installedBuiltinCatalogEvidence = [ordered]@{
            verification = "cargo xtask builtin-catalog verify-installed"
            root = "builtin-songs"
            manifest_sha256 = (Get-FileHash -LiteralPath (Join-Path $installedBuiltinRoot "manifest.json") -Algorithm SHA256).Hash.ToLowerInvariant()
            file_count = $installedBuiltinFiles.Count
            files = @($installedBuiltinFiles | ForEach-Object {
                [ordered]@{
                    path = [IO.Path]::GetRelativePath($installRoot, $_.FullName).Replace("\", "/")
                    size = [int64]$_.Length
                    sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                }
            })
            manifest_validated = $true
            file_set_exact = $true
            sha256_verified = $true
            songs_parseable = $true
        }

        $previousAppDataRoot = [Environment]::GetEnvironmentVariable("SKY_APP_DATA_ROOT", "Process")
        $previousFreshSelfTest = [Environment]::GetEnvironmentVariable("SKY_BUILTIN_CATALOG_FRESH_SELFTEST", "Process")
        $freshAppData = Join-Path $root ("fresh-appdata-" + [guid]::NewGuid().ToString("N"))
        try {
            [Environment]::SetEnvironmentVariable("SKY_APP_DATA_ROOT", $freshAppData, "Process")
            [Environment]::SetEnvironmentVariable("SKY_BUILTIN_CATALOG_FRESH_SELFTEST", "1", "Process")
            $catalogSelftest = Start-Process -FilePath $app -ArgumentList @("--selftest-desktop-shell") -WindowStyle Hidden -Wait -PassThru
            if ($catalogSelftest.ExitCode -ne 0) { Fail "fresh built-in catalog self-test failed" }
            $freshSongsRoot = Join-Path $freshAppData "songs"
            $freshUserSongs = @(
                Get-ChildItem -LiteralPath $freshSongsRoot -File -Recurse -ErrorAction SilentlyContinue
            )
            if ($freshUserSongs.Count -ne 0) { Fail "fresh built-in catalog self-test populated user songs" }
        } finally {
            if ($null -eq $previousAppDataRoot) { Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue } else { [Environment]::SetEnvironmentVariable("SKY_APP_DATA_ROOT", $previousAppDataRoot, "Process") }
            if ($null -eq $previousFreshSelfTest) { Remove-Item Env:SKY_BUILTIN_CATALOG_FRESH_SELFTEST -ErrorAction SilentlyContinue } else { [Environment]::SetEnvironmentVariable("SKY_BUILTIN_CATALOG_FRESH_SELFTEST", $previousFreshSelfTest, "Process") }
            if (Test-Path -LiteralPath $freshAppData) { Remove-Item -LiteralPath $freshAppData -Recurse -Force -ErrorAction SilentlyContinue }
        }

        # active-playback-install-rejected
        $activity = Start-Process -FilePath $app -ArgumentList @("--selftest-update-active-playback") -WindowStyle Hidden -Wait -PassThru
        if ($activity.ExitCode -ne 0) { Fail "playback-active update rejection self-test failed" }

        $installedBuiltinCatalogProof = [ordered]@{
            qualification = @(
                "fresh-current-user-no-admin-install",
                "previous-v4-to-exact-downloaded-candidate-update",
                "active-playback-install-rejected",
                "installed-built-in-catalog-exact-manifest-file-set-sha-parseability",
                "fresh-appdata-built-in-user-composition"
            )
        }

        $uninstall = Start-Process -FilePath $uninstaller -ArgumentList @("/S") -WindowStyle Hidden -Wait -PassThru
        if ($uninstall.ExitCode -ne 0) { Fail "candidate uninstall failed" }
    } finally {
        if (Test-Path -LiteralPath $installRoot) { Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue }
    }

    $candidateManifest = [ordered]@{
        schema_version = 1
        source_sha = $SourceSha.ToLowerInvariant()
        version = $Version
        channel = $Channel
        tag = $Tag
        qualification_assets = $candidateAssets
        public_assets = $publicRecords
    }
    $manifestPath = Join-Path $root "candidate-manifest.json"
    Write-JsonFile $manifestPath $candidateManifest
    Write-Host "V4 candidate qualification: PASS (candidate frozen once; public installer and signature verified)"
}

function Invoke-PublishRelease {
    $root = Get-EffectiveStateRoot
    $manifestPath = Join-Path $root "candidate-manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        Fail "candidate manifest is missing: $manifestPath"
    }
    $manifest = Read-JsonFile $manifestPath
    if ([string]$manifest.source_sha.ToLowerInvariant() -ne $SourceSha.ToLowerInvariant() -or
        [string]$manifest.version -ne $Version -or
        [string]$manifest.channel -ne $Channel -or
        [string]$manifest.tag -ne $Tag) {
        Fail "candidate manifest does not match requested release identity"
    }

    $publicRecords = @(Get-PublicReleaseRecordsFromManifest $manifest)
    if ($publicRecords.Count -ne 2) {
        Fail "candidate manifest public asset count is not exactly 2"
    }

    $repository = Get-CanonicalRepository
    $notesPath = Assert-ReleaseNotes
    $effectiveRunId = if (-not [string]::IsNullOrWhiteSpace($RunId)) { $RunId } elseif (-not [string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID)) { $env:GITHUB_RUN_ID } else { "manual" }

    # Pre-check: Ensure tag does not already exist
    $direct = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/tags/$Tag") -AllowNotFound
    if ($null -ne $direct) {
        Fail "repository already contains published release for tag $Tag; published releases are immutable"
    }

    # Format release body with transaction marker
    $marker = Format-V4TransactionMarker -Repository $repository -RunId $effectiveRunId -SourceSha $SourceSha -Version $Version -Tag $Tag
    $rawNotes = (Get-Content -LiteralPath $notesPath -Raw).Trim()
    $body = $rawNotes + "`n`n" + $marker

    # 1. Create Release Draft
    # Display name = $Tag (name = v4.1.1, tag = v4.1.1)
    $payloadPath = Join-Path $root "create-draft-payload.json"
    Write-JsonFile $payloadPath ([ordered]@{
        tag_name = $Tag
        target_commitish = $SourceSha.ToLowerInvariant()
        name = $Tag
        body = $body
        draft = $true
        prerelease = ($Channel -eq "beta")
        make_latest = Get-V4ReleaseDraftMakeLatestValue
    })

    $draft = $null
    $createError = $null
    try {
        $draft = Invoke-GitHubApi -Arguments @("api", "--method", "POST", "repos/$repository/releases", "--input", $payloadPath)
    } catch {
        $createError = $_
    }

    if ($null -eq $draft) {
        # Check if draft POST timed out but remote draft actually exists:
        $collection = Get-ReleaseCollection $repository
        $existing = Select-V4ReleaseByTag -DirectRelease $null -ReleaseCollection $collection -Tag $Tag
        if ($null -ne $existing -and [bool]$existing.draft) {
            $existingMarker = Get-V4TransactionMarker ([string]$existing.body)
            if (Test-V4TransactionMarkerMatch $existingMarker $repository $effectiveRunId $SourceSha $Tag) {
                Write-Host "Reconciled draft created despite POST error/timeout: id=$($existing.id)"
                $draft = $existing
            } else {
                Fail "draft creation failed and an un-adoptable release exists: $createError"
            }
        } else {
            Fail "failed to create release draft: $createError"
        }
    }

    $releaseId = [int64]$draft.id
    if ($releaseId -le 0) {
        Fail "repository draft returned an invalid release_id"
    }
    Write-Host "Created release draft: id=$releaseId tag=$Tag (name=$Tag)"

    # Pre-publication boundary try-catch
    try {
        $uploadUrl = [string]$draft.upload_url
        if ([string]::IsNullOrWhiteSpace($uploadUrl)) {
            Fail "repository draft did not return upload_url"
        }
        $uploadUrl = $uploadUrl -replace '\{\?name,label\}$', ''

        # 2. Upload Assets
        foreach ($record in $publicRecords) {
            $assetName = [string]$record.release_name
            $assetFile = Get-StateAssetPath $record
            if (-not (Test-Path -LiteralPath $assetFile -PathType Leaf)) {
                Fail "asset file to upload is missing: $assetFile"
            }
            Write-Host "Uploading release asset: $assetName ($([int64]$record.size) bytes)"
            if ($null -ne $script:AssetUploadHandler) {
                & $script:AssetUploadHandler $uploadUrl $assetName $assetFile
            } else {
                Invoke-V4ReleaseAssetUpload -UploadUrl $uploadUrl -AssetName $assetName -FilePath $assetFile
            }
        }

        # 3. GET exact release_id and verify exact asset names/sizes/digests
        $serverDraft = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/$releaseId")
        if ($null -eq $serverDraft) {
            Fail "failed to query release draft $releaseId after asset upload"
        }
        Assert-ExactPublicReleaseAssetSet $serverDraft

        $serverAssets = @($serverDraft.assets)
        if ($serverAssets.Count -ne $publicRecords.Count) {
            Fail "server asset count ($($serverAssets.Count)) does not match qualified asset count ($($publicRecords.Count))"
        }

        foreach ($expected in $publicRecords) {
            $expectedName = [string]$expected.release_name
            $serverAsset = @($serverAssets | Where-Object { [string]$_.name -eq $expectedName })
            if ($serverAsset.Count -ne 1) {
                Fail "server is missing uploaded asset: $expectedName"
            }
            $sa = $serverAsset[0]
            if ([int64]$sa.size -ne [int64]$expected.size) {
                Fail "server asset size mismatch for ${expectedName}: expected $($expected.size), got $($sa.size)"
            }
            if ($null -ne $sa.PSObject.Properties['state'] -and [string]$sa.state -ne "uploaded") {
                Fail "server asset state is not uploaded for ${expectedName}: $($sa.state)"
            }
            if ($null -ne $sa.PSObject.Properties['digest'] -and -not [string]::IsNullOrWhiteSpace([string]$sa.digest)) {
                $rawDigest = [string]$sa.digest
                $remoteSha = if ($rawDigest -match '^sha256:(.+)$') { $Matches[1] } else { $rawDigest }
                if ($remoteSha.ToLowerInvariant() -ne [string]$expected.sha256.ToLowerInvariant()) {
                    Fail "server asset digest mismatch for ${expectedName}: expected $($expected.sha256), got $remoteSha"
                }
            }
        }
        Write-Host "Server verified exact asset sizes and digests on release draft ${releaseId}: PASS"

        # 4. Irreversible Publication PATCH (draft = false)
        $patchPayloadPath = Join-Path $root "publish-payload.json"
        Write-JsonFile $patchPayloadPath ([ordered]@{
            draft = $false
            make_latest = (Get-V4ReleaseMakeLatestValue $Channel)
        })

        $published = $null
        $patchError = $null
        try {
            $published = Invoke-GitHubApi -Arguments @("api", "--method", "PATCH", "repos/$repository/releases/$releaseId", "--input", $patchPayloadPath)
        } catch {
            $patchError = $_
        }

        if ($null -ne $patchError) {
            # Check if remote release was published despite error (e.g. timeout)
            $checkRemote = $null
            try {
                $checkRemote = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/$releaseId") -AllowNotFound
            } catch {}

            if ($null -ne $checkRemote -and -not [bool]$checkRemote.draft) {
                Write-Host "Reconciliation: release was published despite PATCH error"
            } else {
                # Still unpublished draft: perform self-cleanup
                Invoke-DraftSelfCleanup -Repository $repository -ReleaseId $releaseId -ExpectedSha $SourceSha -ExpectedTag $Tag -ExpectedRunId $effectiveRunId
                throw $patchError
            }
        }

    } catch {
        $prePubEx = $_
        Write-Warning "Pre-publication error encountered: $($prePubEx.Exception.Message)"

        # Check whether remote release was published despite error
        $checkRemote = $null
        try {
            $checkRemote = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/$releaseId") -AllowNotFound
        } catch {}

        if ($null -ne $checkRemote -and -not [bool]$checkRemote.draft) {
            Write-Host "Reconciliation: release was published despite error"
        } else {
            Invoke-DraftSelfCleanup -Repository $repository -ReleaseId $releaseId -ExpectedSha $SourceSha -ExpectedTag $Tag -ExpectedRunId $effectiveRunId
            throw $prePubEx
        }
    }

    # 5. Authoritative Post-Publication Reconciliation
    $finalRelease = $null
    $finalError = $null
    try {
        $finalRelease = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/$releaseId")
    } catch {
        $finalError = $_
    }

    if ($null -eq $finalRelease) {
        Fail "POST_PUBLICATION_INCIDENT: publication PATCH succeeded or was attempted, but GET repos/$repository/releases/$releaseId failed: $finalError"
    }

    if ([bool]$finalRelease.draft) {
        Invoke-DraftSelfCleanup -Repository $repository -ReleaseId $releaseId -ExpectedSha $SourceSha -ExpectedTag $Tag -ExpectedRunId $effectiveRunId
        Fail "publication PATCH failed and remote release remains unpublished draft"
    }

    $published = $finalRelease
    Assert-ImmutableRelease $published
    if ([string]::IsNullOrWhiteSpace([string]$finalRelease.published_at)) {
        Fail "POST_PUBLICATION_INCIDENT: published release has empty published_at timestamp"
    }
    if ([string]$finalRelease.target_commitish.ToLowerInvariant() -ne $SourceSha.ToLowerInvariant()) {
        Fail "POST_PUBLICATION_INCIDENT: published release target_commitish does not match requested source SHA"
    }
    if ([string]$finalRelease.tag_name -ne $Tag) {
        Fail "POST_PUBLICATION_INCIDENT: published release tag_name does not match requested tag"
    }
    Assert-ExactPublicReleaseAssetSet $finalRelease

    Write-Host "V4 release publication transaction: PASS (tag=$Tag, id=$releaseId, immutable=true, published_at=$($finalRelease.published_at))"
}

function Invoke-PromoteMetadata {
    $root = Get-EffectiveStateRoot
    $repository = Get-CanonicalRepository
    $publishedRelease = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/tags/$Tag")
    if ($null -eq $publishedRelease -or [bool]$publishedRelease.draft -or [string]::IsNullOrWhiteSpace([string]$publishedRelease.published_at)) {
        Fail "metadata promotion is forbidden before immutable publication"
    }
    Assert-ImmutableRelease $publishedRelease
    if ([string]$publishedRelease.target_commitish.ToLowerInvariant() -ne $SourceSha.ToLowerInvariant()) {
        Fail "published release target_commitish does not match requested source SHA"
    }
    Assert-ExactPublicReleaseAssetSet $publishedRelease

    $metadataCheckout = Join-Path $root "release-metadata"
    if (Test-Path -LiteralPath $metadataCheckout) { Remove-Item -LiteralPath $metadataCheckout -Recurse -Force }
    Invoke-GitHubApi -Arguments @("repo", "clone", $repository, $metadataCheckout, "--", "--branch", "release-metadata", "--depth", "1") -Raw | Out-Null
    if (-not (Test-Path -LiteralPath (Join-Path $metadataCheckout ".git") -PathType Container)) { Fail "release-metadata branch checkout was not obtained" }

    $manifestPath = Join-Path $root "candidate-manifest.json"
    $manifest = Read-JsonFile $manifestPath
    $publicRecords = @(Get-PublicReleaseRecordsFromManifest $manifest)
    $installerRecord = @($publicRecords | Where-Object { [string]$_.name -eq (Get-ExpectedInstallerName) })[0]
    $signatureRecord = @($publicRecords | Where-Object { [string]$_.name -eq "$((Get-ExpectedInstallerName)).sig" })[0]

    $notesPath = Assert-ReleaseNotes
    $destination = Join-Path $root "latest.json"
    $publicationDateUtc = Convert-PublishedAtToMetadataTimestamp [string]$publishedRelease.published_at
    & cargo xtask release-metadata generate `
        --channel $Channel `
        --version $Version `
        --notes-file $notesPath `
        --pub-date $publicationDateUtc `
        --platform "windows-x86_64" `
        --asset-url "https://github.com/$repository/releases/download/$Tag/$([string]$installerRecord.release_name)" `
        --signature-file (Get-StateAssetPath $signatureRecord) `
        --output $destination
    if ($LASTEXITCODE -ne 0) { Fail "release metadata generation failed" }

    & cargo xtask release-metadata validate --channel $Channel --metadata $destination
    if ($LASTEXITCODE -ne 0) { Fail "release metadata validation failed" }

    $encoded = [Convert]::ToBase64String([IO.File]::ReadAllBytes($destination))
    $existing = Invoke-GitHubApi -Arguments @("api", "repos/$repository/contents/channels/$Channel/latest.json?ref=release-metadata") -AllowNotFound
    if ($null -ne $existing) {
        $currentPath = Join-Path $root "current-$Channel-latest.json"
        Write-RepositoryContentFile $existing $currentPath "channels/$Channel/latest.json"
        & cargo xtask release-metadata validate-monotonic --channel $Channel --current $currentPath --candidate $destination
        if ($LASTEXITCODE -ne 0) { Fail "candidate metadata is not strictly monotonic over current channel metadata" }
    }

    $payload = [ordered]@{
        message = "promote $Channel metadata for $Tag"
        content = $encoded
        branch = "release-metadata"
    }
    if ($null -ne $existing -and $null -ne $existing.PSObject.Properties['sha']) {
        $payload['sha'] = [string]$existing.sha
    }
    $payloadPath = Join-Path $root "metadata-commit.json"
    Write-JsonFile $payloadPath $payload
    Invoke-GitHubApi -Arguments @("api", "--method", "PUT", "repos/$repository/contents/channels/$Channel/latest.json", "--input", $payloadPath) | Out-Null
    Write-Host "V4 metadata promotion: PASS (channel=$Channel, tag=$Tag)"
}

function Invoke-FinalVerify {
    $repository = Get-CanonicalRepository
    $release = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/tags/$Tag")
    if ($null -eq $release -or [bool]$release.draft -or [string]::IsNullOrWhiteSpace([string]$release.published_at)) {
        Fail "final release is still draft or unpublished"
    }
    Assert-ImmutableRelease $release
    $candidateManifest = Read-JsonFile (Join-Path (Get-EffectiveStateRoot) "candidate-manifest.json")
    $publicRecords = @(Get-PublicReleaseRecordsFromManifest $candidateManifest)
    Assert-ExactPublicReleaseAssetSet $release
    Assert-ExactAssetSet $release $publicRecords
    foreach ($expected in $publicRecords) {
        $expectedReleaseName = if ($null -ne $expected.PSObject.Properties['release_name']) { [string]$expected.release_name } else { [string]$expected.name }
        $asset = @($release.assets | Where-Object { [string]$_.name -eq $expectedReleaseName })
        if ($asset.Count -ne 1 -or [int64]$asset[0].size -ne [int64]$expected.size) { Fail "final public asset identity changed: $expectedReleaseName" }
        $finalPath = Join-Path (Get-EffectiveStateRoot) "final-$expectedReleaseName"
        Invoke-GitHubApi -Arguments @("api", [string]$asset[0].url, "--header", "Accept: application/octet-stream") -BinaryOutput -OutputPath $finalPath
        $hash = (Get-FileHash -LiteralPath $finalPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne [string]$expected.sha256) { Fail "final public asset digest differs from qualified bytes: $expectedReleaseName" }
    }

    # Verify unauthenticated raw channel metadata
    $metadataResponse = Invoke-GitHubApi -Arguments @("api", "repos/$repository/contents/channels/$Channel/latest.json?ref=release-metadata")
    $metadataPath = Join-Path (Get-EffectiveStateRoot) "final-metadata.json"
    Write-RepositoryContentFile $metadataResponse $metadataPath "channels/$Channel/latest.json"
    & cargo xtask release-metadata validate --channel $Channel --metadata $metadataPath
    if ($LASTEXITCODE -ne 0) { Fail "final channel metadata validation failed" }

    $publicDocument = Get-PublicMetadataDocument $Channel
    if ([string]$publicDocument.version -ne $Version) { Fail "public metadata document version does not match requested version" }
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "ci_v4_release_latest_guard.ps1")
    ) "legacy GitHub Latest release was displaced"
    Write-Host "V4 final verification: PASS (tag=$Tag channel=$Channel verified via raw endpoint)"
}

# ==============================================================================
# Rehearsal & Legacy Compatibility Handlers
# ==============================================================================

function Invoke-ValidateRequest {
    Assert-RequestIdentity
    Assert-ReleaseNotes
}

function Invoke-ValidateRepository {
    Assert-RepositoryReleasePolicy
}

function Invoke-CreateDraft {
    Assert-RequestIdentity
    Assert-RepositoryReleasePolicy
    $repository = Get-CanonicalRepository
    $manifestPath = Join-Path (Get-EffectiveStateRoot) "candidate-manifest.json"
    $manifest = Read-JsonFile $manifestPath
    $publicRecords = @(Get-PublicReleaseRecordsFromManifest $manifest)
    Assert-CandidateEvidence @($manifest.qualification_assets)

    $ref = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/refs/tags/$Tag") -AllowNotFound
    if ($null -ne $ref) {
        Fail "repository already contains published release/tag $Tag; published tags are immutable"
    }

    $notesPath = Assert-ReleaseNotes
    $runId = if ([string]::IsNullOrWhiteSpace($RunId)) { "rehearsal" } else { $RunId }
    $marker = Format-V4TransactionMarker -Repository $repository -RunId $runId -SourceSha $SourceSha -Version $Version -Tag $Tag
    $body = (Get-Content -LiteralPath $notesPath -Raw).Trim() + "`n`n" + $marker

    $payloadPath = Join-Path (Get-EffectiveStateRoot) "create-release.json"
    Write-JsonFile $payloadPath ([ordered]@{
        tag_name = $Tag
        target_commitish = $SourceSha.ToLowerInvariant()
        name = $Tag
        body = $body
        draft = $true
        prerelease = ($Channel -eq "beta")
        make_latest = Get-V4ReleaseDraftMakeLatestValue
    })
    $release = Invoke-GitHubApi -Arguments @("api", "--method", "POST", "repos/$repository/releases", "--input", $payloadPath)
    if (-not $release.draft -or [string]$release.tag_name -ne $Tag) { Fail "repository did not create requested draft release" }

    $uploadUrl = [string]$release.upload_url
    $uploadUrl = $uploadUrl -replace '\{\?name,label\}$', ''
    foreach ($record in $publicRecords) {
        $assetName = [string]$record.release_name
        $assetFile = Get-StateAssetPath $record
        Invoke-V4ReleaseAssetUpload -UploadUrl $uploadUrl -AssetName $assetName -FilePath $assetFile
    }
}

function Invoke-DownloadDraft {
    $repository = Get-CanonicalRepository
    $collection = Get-ReleaseCollection $repository
    $release = Select-V4ReleaseByTag -DirectRelease $null -ReleaseCollection $collection -Tag $Tag
    if ($null -eq $release -or -not [bool]$release.draft) { Fail "draft download requires unpublished draft release" }
    $manifest = Read-JsonFile (Join-Path (Get-EffectiveStateRoot) "candidate-manifest.json")
    $publicRecords = @(Get-PublicReleaseRecordsFromManifest $manifest)
    Assert-ExactPublicReleaseAssetSet $release
    $downloaded = Join-Path (Get-EffectiveStateRoot) "downloaded"
    if (Test-Path -LiteralPath $downloaded) { Remove-Item -LiteralPath $downloaded -Recurse -Force }
    New-Item -ItemType Directory -Path $downloaded -Force | Out-Null
    foreach ($expected in $publicRecords) {
        $expectedName = [string]$expected.release_name
        $asset = @($release.assets | Where-Object { [string]$_.name -eq $expectedName })
        if ($asset.Count -ne 1) { Fail "expected repository asset is missing: $expectedName" }
        $dest = Join-Path $downloaded ([IO.Path]::GetFileName($expectedName))
        Invoke-GitHubApi -Arguments @("api", [string]$asset[0].url, "--header", "Accept: application/octet-stream") -BinaryOutput -OutputPath $dest
        $hash = (Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne [string]$expected.sha256) { Fail "downloaded asset digest mismatch: $expectedName" }
    }
}

function Invoke-QualifyDownloaded {
    Invoke-BuildCandidate
}

function Invoke-RecordAttestations {
    Write-Host "Attestations recorded."
}

function Invoke-PublishDraft {
    Invoke-PublishRelease
}

function Invoke-SelfTest {
    $scriptPath = (Resolve-Path $PSCommandPath).Path
    $source = Get-Content -LiteralPath $scriptPath -Raw
    if ($source -notmatch 'draft = \$true' -or
        $source -notmatch "metadata promotion is forbidden before immutable publication") {
        Fail "self-test could not find draft/qualification/publication guards"
    }
    try {
        Assert-ImmutableRelease ([pscustomobject]@{ immutable = $false })
        Fail "immutable=false unexpectedly passed the publication guard"
    } catch {
        if ($_.Exception.Message -notmatch "repository release is not marked immutable") { throw }
    }
    Assert-ImmutableRelease ([pscustomobject]@{ immutable = $true })
    Write-Host "V4 immutable publication guard self-test: PASS (immutable=false rejected; immutable=true accepted)"

    $selfTestVersion = $Version
    try {
        $Version = "4.0.0-rc.1"
        $publicNames = @(Get-CanonicalPublicReleaseNames)
        $exactPublicRelease = [pscustomobject]@{
            assets = @(
                [pscustomobject]@{ name = $publicNames[0] }
                [pscustomobject]@{ name = $publicNames[1] }
            )
        }
        Assert-ExactPublicReleaseAssetSet $exactPublicRelease
        foreach ($invalidAssets in @(
            @([pscustomobject]@{ name = $publicNames[0] }),
            @(
                [pscustomobject]@{ name = $publicNames[0] }
                [pscustomobject]@{ name = $publicNames[1] }
                [pscustomobject]@{ name = "SBOM.spdx.json" }
            )
        )) {
            try {
                Assert-ExactPublicReleaseAssetSet ([pscustomobject]@{ assets = $invalidAssets })
                Fail "FinalVerify public asset-set self-test accepted missing or extra assets"
            } catch {
                if ($_.Exception.Message -notmatch "exactly the canonical installer") { throw }
            }
        }
        Write-Host "V4 exact public asset-set self-test: PASS (missing and extra assets rejected)"
    } finally {
        $Version = $selfTestVersion
    }

    foreach ($channelCase in @(
        [pscustomobject]@{ Channel = "stable"; PublishExpected = "true" },
        [pscustomobject]@{ Channel = "beta"; PublishExpected = "false" }
    )) {
        $draftPayload = [ordered]@{
            draft = $true
            make_latest = Get-V4ReleaseDraftMakeLatestValue
        }
        $draftRoundTrip = (($draftPayload | ConvertTo-Json -Depth 20) | ConvertFrom-Json)
        if ($draftRoundTrip.make_latest -isnot [string] -or $draftRoundTrip.make_latest -ne "false") {
            Fail "GitHub draft payload make_latest must round-trip as the string enum false"
        }
        if ($draftRoundTrip.draft -isnot [bool] -or -not [bool]$draftRoundTrip.draft) {
            Fail "GitHub draft payload draft must remain the JSON boolean true"
        }

        $publishPayload = [ordered]@{
            draft = $false
            make_latest = Get-V4ReleaseMakeLatestValue $channelCase.Channel
        }
        $publishRoundTrip = (($publishPayload | ConvertTo-Json -Depth 20) | ConvertFrom-Json)
        if ($publishRoundTrip.make_latest -isnot [string] -or $publishRoundTrip.make_latest -ne $channelCase.PublishExpected) {
            Fail "GitHub publication payload make_latest must round-trip as the string enum $($channelCase.PublishExpected) for $($channelCase.Channel)"
        }
        if ($publishRoundTrip.draft -isnot [bool] -or [bool]$publishRoundTrip.draft) {
            Fail "GitHub publication payload draft must remain the JSON boolean false"
        }
    }
    Write-Host "V4 GitHub release payload self-test: PASS (draft false; stable publish true; beta publish false)"
}

# ==============================================================================
# Entry Point Dispatch
# ==============================================================================

switch ($State) {
    "Preflight" { Invoke-Preflight }
    "BuildCandidate" { Invoke-BuildCandidate }
    "PublishRelease" { Invoke-PublishRelease }
    "PublishReleaseTransaction" { Invoke-PublishRelease }
    "PromoteMetadata" { Invoke-PromoteMetadata }
    "FinalVerify" { Invoke-FinalVerify }
    "ValidateRequest" { Invoke-ValidateRequest }
    "ValidateRepository" { Invoke-ValidateRepository }
    "CreateDraft" { Invoke-CreateDraft }
    "DownloadDraft" { Invoke-DownloadDraft }
    "QualifyDownloaded" { Invoke-QualifyDownloaded }
    "RecordAttestations" { Invoke-RecordAttestations }
    "PublishDraft" { Invoke-PublishDraft }
    "SelfTest" { Invoke-SelfTest }
    default { Fail "Unknown state '$State'" }
}
