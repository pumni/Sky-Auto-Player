[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet(
        "Preflight",
        "BuildCandidate",
        "PublishRelease",
        "PromoteMetadata",
        "FinalVerify",
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
. (Join-Path $PSScriptRoot "v4_nsis_smoke_boundary.ps1")

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
    if ([string]::IsNullOrWhiteSpace($Repository)) { Fail "transaction marker requires repository" }
    if ([string]::IsNullOrWhiteSpace($RunId)) { Fail "transaction marker requires run_id" }
    if ([string]::IsNullOrWhiteSpace($SourceSha) -or $SourceSha -notmatch '^[0-9a-fA-F]{40}$') {
        Fail "transaction marker requires exact 40-character source_sha"
    }
    if ([string]::IsNullOrWhiteSpace($Version)) { Fail "transaction marker requires version" }
    if ([string]::IsNullOrWhiteSpace($Tag) -or $Tag -ne "v$Version") {
        Fail "transaction marker requires tag exactly matching v<version>"
    }
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
    $matches = [regex]::Matches($Body, '<!--\s*v4-release-tx:\s*(\{.*?\})\s*-->')
    if ($matches.Count -ne 1) { return $null }
    $rawJson = $matches[0].Groups[1].Value
    # Reject duplicate critical keys before ConvertFrom-Json collapses them.
    # ConvertFrom-Json silently keeps the last value for duplicate keys, so
    # duplicates must be detected on the raw JSON string.
    $criticalKeys = @('repository', 'run_id', 'source_sha', 'version', 'tag')
    foreach ($key in $criticalKeys) {
        $pattern = '"' + [regex]::Escape($key) + '"' + '\s*:'
        if ([regex]::Matches($rawJson, $pattern).Count -gt 1) { return $null }
    }
    try {
        return ($rawJson | ConvertFrom-Json)
    } catch {
        return $null
    }
}

function Assert-V4TransactionMarkerStrictSchema([object]$Marker) {
    if ($null -eq $Marker) { return $false }
    $requiredProps = @("repository", "run_id", "source_sha", "version", "tag")
    foreach ($prop in $requiredProps) {
        if ($null -eq $Marker.PSObject.Properties[$prop]) { return $false }
        $val = [string]$Marker.$prop
        if ([string]::IsNullOrWhiteSpace($val)) { return $false }
    }
    if ([string]$Marker.source_sha -notmatch '^[0-9a-fA-F]{40}$') { return $false }
    if ([string]$Marker.version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') { return $false }
    if ([string]$Marker.tag -ne "v$([string]$Marker.version)") { return $false }
    return $true
}

function Test-V4TransactionMarkerMatch {
    param(
        [object]$Marker,
        [Parameter(Mandatory = $true)] [string]$ExpectedRepo,
        [string]$ExpectedRunId = "",
        [Parameter(Mandatory = $true)] [string]$ExpectedSha,
        [Parameter(Mandatory = $true)] [string]$ExpectedVersion,
        [Parameter(Mandatory = $true)] [string]$ExpectedTag
    )
    if ($null -eq $Marker) { return $false }
    if (-not (Assert-V4TransactionMarkerStrictSchema $Marker)) { return $false }

    if ([string]$Marker.repository -ne $ExpectedRepo) { return $false }
    if ([string]$Marker.source_sha.ToLowerInvariant() -ne $ExpectedSha.ToLowerInvariant()) { return $false }
    if ([string]$Marker.version -ne $ExpectedVersion) { return $false }
    if ([string]$Marker.tag -ne $ExpectedTag) { return $false }

    # For same-transaction reconciliation, run_id must match exactly.
    # For stale cleanup across runs, run_id must exist and be non-empty (verified by schema).
    if (-not [string]::IsNullOrWhiteSpace($ExpectedRunId)) {
        if ([string]$Marker.run_id -ne $ExpectedRunId) { return $false }
    }
    return $true
}

function Invoke-DraftSelfCleanup {
    param(
        [Parameter(Mandatory = $true)] [string]$Repository,
        [Parameter(Mandatory = $true)] [int64]$ReleaseId,
        [Parameter(Mandatory = $true)] [string]$ExpectedSha,
        [Parameter(Mandatory = $true)] [string]$ExpectedVersion,
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
            (Test-V4TransactionMarkerMatch -Marker $marker -ExpectedRepo $Repository -ExpectedRunId $ExpectedRunId -ExpectedSha $ExpectedSha -ExpectedVersion $ExpectedVersion -ExpectedTag $ExpectedTag)) {
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

function Assert-RequestIdentity {
    if ([string]::IsNullOrWhiteSpace($Version)) { Fail "version is required" }
    if ([string]::IsNullOrWhiteSpace($Channel)) { Fail "channel is required" }
    if ([string]::IsNullOrWhiteSpace($Tag)) { Fail "tag is required" }
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

    $versionLog = Join-Path (Get-EffectiveStateRoot) "version-check.log"
    & cargo xtask version check --version $Version --channel $Channel --tag $Tag *> $versionLog
    if ($LASTEXITCODE -ne 0) {
        $detail = if (Test-Path -LiteralPath $versionLog) { (Get-Content -LiteralPath $versionLog -Raw).Trim() } else { "" }
        Fail "canonical cargo xtask version/channel/tag validation failed: $detail"
    }
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

function Get-RecordPropertyValue([object]$Record, [string]$PropertyName) {
    if ($null -eq $Record -or [string]::IsNullOrWhiteSpace($PropertyName)) { return $null }
    if ($Record -is [System.Collections.IDictionary]) {
        if ($Record.Contains($PropertyName)) { return $Record[$PropertyName] }
        return $null
    }
    $prop = $Record.PSObject.Properties[$PropertyName]
    if ($null -ne $prop) { return $prop.Value }
    return $null
}

function Get-RecordPropertyString([object]$Record, [string]$PropertyName) {
    $value = Get-RecordPropertyValue $Record $PropertyName
    if ($null -eq $value) { return "" }
    return [string]$value
}

function Assert-ExactAssetSet([object]$Release, [object[]]$Expected) {
    $actual = @($Release.assets | ForEach-Object { [string]$_.name } | Sort-Object)
    $expectedNames = @($Expected | ForEach-Object {
        $relName = Get-RecordPropertyString $_ 'release_name'
        if (-not [string]::IsNullOrWhiteSpace($relName)) { $relName } else { Get-RecordPropertyString $_ 'name' }
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

function Get-ExpectedSourceInstallerName {
    return "Sky Auto Player_${Version}${installerSuffix}"
}

function Get-ExpectedSourceSignatureName {
    return "$(Get-ExpectedSourceInstallerName).sig"
}

function Get-ExpectedInstallerName {
    return Get-V4SafeReleaseAssetName (Get-ExpectedSourceInstallerName)
}

function Get-ExpectedSignatureName {
    return Get-V4SafeReleaseAssetName (Get-ExpectedSourceSignatureName)
}

function Get-CanonicalPublicReleaseNames {
    return @((Get-ExpectedInstallerName), (Get-ExpectedSignatureName))
}

function Get-QualificationCandidateRecords {
    $installer = Get-ExpectedSourceInstallerName
    $signature = Get-ExpectedSourceSignatureName
    $bundle = Join-Path $repoRoot "rust/target/dist/bundle/nsis"
    $evidence = Join-Path $repoRoot "rust/target/dist"
    return @(
        [pscustomobject]@{ name = $installer; path = Join-Path $bundle $installer; role = "installer" },
        [pscustomobject]@{ name = $signature; path = Join-Path $bundle $signature; role = "updater-signature" },
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
    $records = @($QualificationRecords | Where-Object { $publicNames -contains (Get-RecordPropertyString $_ 'release_name') })
    if ($records.Count -ne 2) {
        Fail "manifest public assets must match exactly the canonical installer and signature"
    }
    return $records
}

function Get-FileRecord([object]$Candidate) {
    $candidatePath = [string](Get-RecordPropertyValue $Candidate 'path')
    if ([string]::IsNullOrWhiteSpace($candidatePath)) {
        $candidatePath = [string]$Candidate.path
    }
    $item = Get-Item -LiteralPath $candidatePath
    $candName = Get-RecordPropertyString $Candidate 'name'
    $sourceName = if (-not [string]::IsNullOrWhiteSpace($candName)) { $candName } else { [IO.Path]::GetFileName($candidatePath) }
    $releaseName = Get-V4SafeReleaseAssetName $sourceName
    $role = Get-RecordPropertyString $Candidate 'role'
    $relativeFolder = if ($role -in @("installer", "updater-signature")) {
        "candidate-bundle"
    } else {
        "candidate-evidence"
    }
    [ordered]@{
        name = $releaseName
        release_name = $releaseName
        source_name = $sourceName
        role = $role
        size = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash.ToLowerInvariant()
        source_path = $candidatePath
        state_path = (Join-Path $relativeFolder $sourceName).Replace("\", "/")
    }
}

function Get-StateAssetPath([object]$Record) {
    $relative = Get-RecordPropertyString $Record "state_path"
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
        (Get-RecordPropertyString $_ "source_name") -eq $SourceName -or
        ((Get-RecordPropertyString $_ "name") -eq $SourceName -and [string]::IsNullOrWhiteSpace((Get-RecordPropertyString $_ "source_name")))
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
        $releaseName = Get-RecordPropertyString $record "release_name"
        if ([string]::IsNullOrWhiteSpace($releaseName)) {
            $releaseName = Get-RecordPropertyString $record "name"
        }
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            Fail "frozen qualification asset is missing: $releaseName"
        }
        $item = Get-Item -LiteralPath $path
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        $expectedSize = [int64](Get-RecordPropertyValue $record "size")
        $expectedSha = Get-RecordPropertyString $record "sha256"
        if ([int64]$item.Length -ne $expectedSize -or $hash -ne $expectedSha) {
            Fail "frozen qualification asset differs from the candidate manifest: $releaseName"
        }
    }
}

function Freeze-CandidateAssets([object[]]$Records) {
    foreach ($record in @($Records)) {
        $destination = Get-StateAssetPath $record
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath ([string](Get-RecordPropertyString $record "source_path")) -Destination $destination -Force
    }
    Assert-ManifestAssetFiles $Records
    return @($Records)
}

function Assert-FrozenCandidateTopology([object[]]$Records) {
    $root = Get-EffectiveStateRoot
    $bundleDir = Join-Path $root "candidate-bundle"
    $evidenceDir = Join-Path $root "candidate-evidence"

    if (-not (Test-Path -LiteralPath $bundleDir -PathType Container)) {
        Fail "candidate-bundle directory is missing: $bundleDir"
    }
    if (-not (Test-Path -LiteralPath $evidenceDir -PathType Container)) {
        Fail "candidate-evidence directory is missing: $evidenceDir"
    }

    $sourceInstaller = Get-ExpectedSourceInstallerName
    $sourceSignature = Get-ExpectedSourceSignatureName
    $expectedBundleNames = @($sourceInstaller, $sourceSignature) | Sort-Object

    $bundleFiles = @(Get-ChildItem -LiteralPath $bundleDir -File | Sort-Object Name)
    $actualBundleNames = @($bundleFiles | ForEach-Object { $_.Name } | Sort-Object)

    if (($actualBundleNames -join "`n") -ne ($expectedBundleNames -join "`n")) {
        Fail "candidate-bundle must contain exactly the source installer and updater signature, found: $($actualBundleNames -join ', ')"
    }
    if ($bundleFiles.Count -ne 2) {
        Fail "candidate-bundle must contain exactly 2 files, found $($bundleFiles.Count)"
    }
    foreach ($file in $bundleFiles) {
        if ($file.Extension -eq ".json") {
            Fail "candidate-bundle must not contain JSON evidence files: $($file.Name)"
        }
    }

    $dottedInstaller = Get-ExpectedInstallerName
    $dottedSignature = Get-ExpectedSignatureName
    if (Test-Path -LiteralPath (Join-Path $bundleDir $dottedInstaller) -PathType Leaf) {
        Fail "candidate-bundle must not contain dotted release asset copy: $dottedInstaller"
    }
    if (Test-Path -LiteralPath (Join-Path $bundleDir $dottedSignature) -PathType Leaf) {
        Fail "candidate-bundle must not contain dotted release asset copy: $dottedSignature"
    }
    if (Test-Path -LiteralPath (Join-Path $root $dottedInstaller) -PathType Leaf) {
        Fail "state root must not contain dotted release asset copy: $dottedInstaller"
    }
    if (Test-Path -LiteralPath (Join-Path $root $dottedSignature) -PathType Leaf) {
        Fail "state root must not contain dotted release asset copy: $dottedSignature"
    }

    $expectedEvidenceNames = @(
        $productionEvidenceName,
        $qualificationEvidenceName,
        $authenticodeEvidenceName,
        $installedAuthenticodeEvidenceName,
        $summaryName,
        $sbomName
    ) | Sort-Object

    $evidenceFiles = @(Get-ChildItem -LiteralPath $evidenceDir -File | Sort-Object Name)
    $actualEvidenceNames = @($evidenceFiles | ForEach-Object { $_.Name } | Sort-Object)

    foreach ($name in $expectedEvidenceNames) {
        if ($actualEvidenceNames -notcontains $name) {
            Fail "candidate-evidence is missing required evidence file: $name"
        }
    }
}

function Assert-EvidenceIdentity([string]$ProductionPath, [string]$QualificationPath, [object[]]$Records) {
    $recordsByName = @{}
    foreach ($record in @($Records)) {
        $name = Get-RecordPropertyString $record "name"
        $sourceName = Get-RecordPropertyString $record "source_name"
        $releaseName = Get-RecordPropertyString $record "release_name"
        if (-not [string]::IsNullOrWhiteSpace($name)) {
            $recordsByName[$name] = $record
        }
        if (-not [string]::IsNullOrWhiteSpace($sourceName)) {
            $recordsByName[$sourceName] = $record
        }
        if (-not [string]::IsNullOrWhiteSpace($releaseName)) {
            $recordsByName[$releaseName] = $record
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

    $instExpectedPublic = if (Get-Command "Get-ExpectedInstallerName" -ErrorAction SilentlyContinue) { Get-ExpectedInstallerName } else { "" }
    $instExpectedSource = if (Get-Command "Get-ExpectedSourceInstallerName" -ErrorAction SilentlyContinue) { Get-ExpectedSourceInstallerName } else { "" }
    $sigExpectedPublic = if (Get-Command "Get-ExpectedSignatureName" -ErrorAction SilentlyContinue) { Get-ExpectedSignatureName } else { "" }
    $sigExpectedSource = if (Get-Command "Get-ExpectedSourceSignatureName" -ErrorAction SilentlyContinue) { Get-ExpectedSourceSignatureName } else { "" }

    $instRec = if (-not [string]::IsNullOrWhiteSpace($instExpectedPublic) -and $recordsByName.ContainsKey($instExpectedPublic)) {
        $recordsByName[$instExpectedPublic]
    } elseif (-not [string]::IsNullOrWhiteSpace($instExpectedSource) -and $recordsByName.ContainsKey($instExpectedSource)) {
        $recordsByName[$instExpectedSource]
    } else {
        $candidates = @($Records | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'installer' })
        if ($candidates.Count -eq 1) { $candidates[0] } else { $null }
    }
    $sigRec = if (-not [string]::IsNullOrWhiteSpace($sigExpectedPublic) -and $recordsByName.ContainsKey($sigExpectedPublic)) {
        $recordsByName[$sigExpectedPublic]
    } elseif (-not [string]::IsNullOrWhiteSpace($sigExpectedSource) -and $recordsByName.ContainsKey($sigExpectedSource)) {
        $recordsByName[$sigExpectedSource]
    } else {
        $candidates = @($Records | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'updater-signature' })
        if ($candidates.Count -eq 1) { $candidates[0] } else { $null }
    }
    if ($null -eq $instRec -or $null -eq $sigRec) {
        Fail "candidate manifest is missing the canonical installer or updater signature record"
    }

    $instSourceName = Get-RecordPropertyString $instRec "source_name"
    if ([string]::IsNullOrWhiteSpace($instSourceName)) {
        $instSourceName = Get-RecordPropertyString $instRec "name"
    }
    $sigSourceName = Get-RecordPropertyString $sigRec "source_name"
    if ([string]::IsNullOrWhiteSpace($sigSourceName)) {
        $sigSourceName = Get-RecordPropertyString $sigRec "name"
    }

    # Production evidence field-specific fail-closed assertions
    if ([string]$evidence.installer -ne $instSourceName) {
        Fail "production evidence installer name mismatch: expected '$instSourceName', got '$([string]$evidence.installer)'"
    }
    if ([string]$evidence.updater_signature -ne $sigSourceName) {
        Fail "production evidence updater_signature name mismatch: expected '$sigSourceName', got '$([string]$evidence.updater_signature)'"
    }
    if ([string]$evidence.authenticode_evidence -ne $authenticodeEvidenceName) {
        Fail "production evidence Authenticode evidence name mismatch: expected '$authenticodeEvidenceName', got '$([string]$evidence.authenticode_evidence)'"
    }
    if ([string]$evidence.sbom -ne $sbomName) {
        Fail "production evidence SBOM name mismatch: expected '$sbomName', got '$([string]$evidence.sbom)'"
    }
    $instExpectedSize = [int64](Get-RecordPropertyValue $instRec "size")
    if ([int64]$evidence.installer_size -ne $instExpectedSize) {
        Fail "production evidence installer size mismatch: expected $instExpectedSize, got $([int64]$evidence.installer_size)"
    }
    $instExpectedSha = Get-RecordPropertyString $instRec "sha256"
    if ([string]$evidence.installer_sha256 -ne $instExpectedSha) {
        Fail "production evidence installer SHA-256 mismatch: expected '$instExpectedSha', got '$([string]$evidence.installer_sha256)'"
    }
    $sigExpectedSize = [int64](Get-RecordPropertyValue $sigRec "size")
    if ([int64]$evidence.signature_size -ne $sigExpectedSize) {
        Fail "production evidence updater signature size mismatch: expected $sigExpectedSize, got $([int64]$evidence.signature_size)"
    }
    $sigExpectedSha = Get-RecordPropertyString $sigRec "sha256"
    if ([string]$evidence.updater_signature_sha256 -ne $sigExpectedSha) {
        Fail "production evidence updater signature SHA-256 mismatch: expected '$sigExpectedSha', got '$([string]$evidence.updater_signature_sha256)'"
    }
    $authExpectedSha = Get-RecordPropertyString $recordsByName[$authenticodeEvidenceName] "sha256"
    if ([string]$evidence.authenticode_evidence_sha256 -ne $authExpectedSha) {
        Fail "production evidence Authenticode evidence SHA-256 mismatch: expected '$authExpectedSha', got '$([string]$evidence.authenticode_evidence_sha256)'"
    }
    $sbomExpectedSha = Get-RecordPropertyString $recordsByName[$sbomName] "sha256"
    if ([string]$evidence.sbom_sha256 -ne $sbomExpectedSha) {
        Fail "production evidence SBOM SHA-256 mismatch: expected '$sbomExpectedSha', got '$([string]$evidence.sbom_sha256)'"
    }

    # Qualification evidence field-specific fail-closed assertions
    if ([string]$qualification.installer -ne $instSourceName) {
        Fail "qualification evidence installer name mismatch: expected '$instSourceName', got '$([string]$qualification.installer)'"
    }
    if ([string]$qualification.updater_signature -ne $sigSourceName) {
        Fail "qualification evidence updater_signature name mismatch: expected '$sigSourceName', got '$([string]$qualification.updater_signature)'"
    }
    if ([string]$qualification.authenticode_evidence -ne $authenticodeEvidenceName) {
        Fail "qualification evidence Authenticode evidence name mismatch: expected '$authenticodeEvidenceName', got '$([string]$qualification.authenticode_evidence)'"
    }
    if ([string]$qualification.sbom -ne $sbomName) {
        Fail "qualification evidence SBOM name mismatch: expected '$sbomName', got '$([string]$qualification.sbom)'"
    }
    if ([int64]$qualification.installer_size -ne $instExpectedSize) {
        Fail "qualification evidence installer size mismatch: expected $instExpectedSize, got $([int64]$qualification.installer_size)"
    }
    if ([int64]$qualification.signature_size -ne $sigExpectedSize) {
        Fail "qualification evidence updater signature size mismatch: expected $sigExpectedSize, got $([int64]$qualification.signature_size)"
    }
    if ([string]$qualification.installer_sha256 -ne $instExpectedSha) {
        Fail "qualification evidence installer SHA-256 mismatch: expected '$instExpectedSha', got '$([string]$qualification.installer_sha256)'"
    }
    if ([string]$qualification.updater_signature_sha256 -ne $sigExpectedSha) {
        Fail "qualification evidence updater signature SHA-256 mismatch: expected '$sigExpectedSha', got '$([string]$qualification.updater_signature_sha256)'"
    }
    if ([string]$qualification.authenticode_evidence_sha256 -ne $authExpectedSha) {
        Fail "qualification evidence Authenticode evidence SHA-256 mismatch: expected '$authExpectedSha', got '$([string]$qualification.authenticode_evidence_sha256)'"
    }
    if ([string]$qualification.authenticode_mode -ne "unsigned-zero-budget") {
        Fail "qualification evidence authenticode_mode mismatch: expected 'unsigned-zero-budget', got '$([string]$qualification.authenticode_mode)'"
    }
    if ([string]$qualification.sbom_sha256 -ne $sbomExpectedSha) {
        Fail "qualification evidence SBOM SHA-256 mismatch: expected '$sbomExpectedSha', got '$([string]$qualification.sbom_sha256)'"
    }
}

function Assert-CandidateEvidence([object[]]$Records) {
    Assert-ManifestAssetFiles $Records
    Assert-FrozenCandidateTopology $Records
    $productionRecord = @($Records | Where-Object { (Get-RecordPropertyString $_ 'source_name') -eq $productionEvidenceName })
    $qualificationRecord = @($Records | Where-Object { (Get-RecordPropertyString $_ 'source_name') -eq $qualificationEvidenceName })
    if ($productionRecord.Count -ne 1 -or $qualificationRecord.Count -ne 1) {
        Fail "candidate manifest is missing frozen production or qualification evidence"
    }
    Assert-EvidenceIdentity `
        (Get-StateAssetPath $productionRecord[0]) `
        (Get-StateAssetPath $qualificationRecord[0]) `
        $Records
}

function Get-PublicReleaseRecordsFromManifest([object]$Manifest) {
    $qualAssets = Get-RecordPropertyValue $Manifest 'qualification_assets'
    $pubAssets = Get-RecordPropertyValue $Manifest 'public_assets'
    if ($null -eq $qualAssets -or $null -eq $pubAssets) {
        Fail "candidate manifest must declare qualification_assets and public_assets separately"
    }
    $qualificationRecords = @($qualAssets)
    $derived = @(Get-PublicReleaseRecords $qualificationRecords)
    $declared = @($pubAssets)
    if ($declared.Count -ne $derived.Count) {
        Fail "candidate manifest public_assets must contain exactly the canonical installer and signature"
    }
    for ($index = 0; $index -lt $derived.Count; $index++) {
        $decl = $declared[$index]
        $der = $derived[$index]
        if ((Get-RecordPropertyString $decl 'name') -ne (Get-RecordPropertyString $der 'name') -or
            (Get-RecordPropertyString $decl 'release_name') -ne (Get-RecordPropertyString $der 'release_name') -or
            (Get-RecordPropertyString $decl 'source_name') -ne (Get-RecordPropertyString $der 'source_name') -or
            (Get-RecordPropertyString $decl 'role') -ne (Get-RecordPropertyString $der 'role') -or
            (Get-RecordPropertyString $decl 'state_path') -ne (Get-RecordPropertyString $der 'state_path') -or
            (Get-RecordPropertyString $decl 'sha256') -ne (Get-RecordPropertyString $der 'sha256') -or
            [int64](Get-RecordPropertyValue $decl 'size') -ne [int64](Get-RecordPropertyValue $der 'size')) {
            Fail "candidate manifest public_assets are not an exact projection of qualification_assets"
        }
    }
    return $derived
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
            (Test-V4TransactionMarkerMatch -Marker $marker -ExpectedRepo $repository -ExpectedRunId "" -ExpectedSha $SourceSha -ExpectedVersion $Version -ExpectedTag $Tag)) {
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
    Assert-CandidateEvidence $candidateAssets
    $publicRecords = @(Get-PublicReleaseRecords $records)

    $root = Get-EffectiveStateRoot
    $bundle = Join-Path $root "candidate-bundle"
    $evidenceDir = Join-Path $root "candidate-evidence"
    $sourceInstaller = Get-ExpectedSourceInstallerName
    $sourceSignature = Get-ExpectedSourceSignatureName
    $releaseInstaller = Get-ExpectedInstallerName
    $releaseSignature = Get-ExpectedSignatureName
    $frozenInstaller = Join-Path $bundle $sourceInstaller
    $frozenSignature = Join-Path $bundle $sourceSignature
    $frozenSbom = Join-Path $evidenceDir $sbomName
    $frozenArtifactSummary = Join-Path $evidenceDir $summaryName
    $frozenAuthenticodeEvidence = Join-Path $evidenceDir $authenticodeEvidenceName
    $frozenQualificationEvidence = Join-Path $evidenceDir $qualificationEvidenceName

    # Local qualification against frozen candidate assets
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1"),
        "-Mode", "unsigned-zero-budget", "-Artifact", $frozenInstaller,
        "-Evidence", (Join-Path $root "downloaded-authenticode-verification.json")
    ) "candidate Authenticode state is not unsigned-zero-budget"

    Invoke-Checked "cargo" @(
        "xtask", "updater-trust", "verify-signature", "--installer", $frozenInstaller,
        "--signature", $frozenSignature
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
        "-CandidateInstallerPath", $frozenInstaller,
        "-CandidateSignaturePath", $frozenSignature,
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
        "-Artifact", $frozenInstaller,
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
    $smokeScope = Enter-V4NsisSmokeScope -InstallRoot $installRoot
    try {
        New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
        $install = Start-Process -FilePath $frozenInstaller -ArgumentList @("/S", "/NS", "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
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
            if (Test-Path -LiteralPath $freshAppData) { Remove-V4DirectoryWithRetry -Path $freshAppData }
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
        Exit-V4NsisSmokeScope -Scope $smokeScope
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
            if (Test-V4TransactionMarkerMatch -Marker $existingMarker -ExpectedRepo $repository -ExpectedRunId $effectiveRunId -ExpectedSha $SourceSha -ExpectedVersion $Version -ExpectedTag $Tag) {
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
            if ($null -eq $sa.PSObject.Properties['digest'] -or [string]::IsNullOrWhiteSpace([string]$sa.digest)) {
                Fail "server asset is missing mandatory digest: $expectedName"
            }
            $rawDigest = [string]$sa.digest
            if ($rawDigest -notmatch '^sha256:([0-9a-fA-F]{64})$') {
                Fail "server asset has invalid or unsupported digest format for ${expectedName}: '$rawDigest' (must be sha256:<64-hex>)"
            }
            $remoteSha = $Matches[1]
            if ($remoteSha.ToLowerInvariant() -ne [string]$expected.sha256.ToLowerInvariant()) {
                Fail "server asset digest mismatch for ${expectedName}: expected $($expected.sha256), got $remoteSha"
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
                Invoke-DraftSelfCleanup -Repository $repository -ReleaseId $releaseId -ExpectedSha $SourceSha -ExpectedVersion $Version -ExpectedTag $Tag -ExpectedRunId $effectiveRunId
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
            Invoke-DraftSelfCleanup -Repository $repository -ReleaseId $releaseId -ExpectedSha $SourceSha -ExpectedVersion $Version -ExpectedTag $Tag -ExpectedRunId $effectiveRunId
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
        Invoke-DraftSelfCleanup -Repository $repository -ReleaseId $releaseId -ExpectedSha $SourceSha -ExpectedVersion $Version -ExpectedTag $Tag -ExpectedRunId $effectiveRunId
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
    $expectedInstaller = Get-ExpectedInstallerName
    $expectedSignature = Get-ExpectedSignatureName
    $installerRecord = @($publicRecords | Where-Object { (Get-RecordPropertyString $_ 'name') -eq $expectedInstaller -or (Get-RecordPropertyString $_ 'release_name') -eq $expectedInstaller })[0]
    $signatureRecord = @($publicRecords | Where-Object { (Get-RecordPropertyString $_ 'name') -eq $expectedSignature -or (Get-RecordPropertyString $_ 'release_name') -eq $expectedSignature })[0]

    $notesPath = Assert-ReleaseNotes
    $destination = Join-Path $root "latest.json"
    $publicationDateUtc = Convert-PublishedAtToMetadataTimestamp [string]$publishedRelease.published_at
    $installerReleaseName = Get-RecordPropertyString $installerRecord 'release_name'
    & cargo xtask release-metadata generate `
        --channel $Channel `
        --version $Version `
        --notes-file $notesPath `
        --pub-date $publicationDateUtc `
        --platform "windows-x86_64" `
        --asset-url "https://github.com/$repository/releases/download/$Tag/$installerReleaseName" `
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
        $expectedReleaseName = Get-RecordPropertyString $expected 'release_name'
        if ([string]::IsNullOrWhiteSpace($expectedReleaseName)) {
            $expectedReleaseName = Get-RecordPropertyString $expected 'name'
        }
        $asset = @($release.assets | Where-Object { [string]$_.name -eq $expectedReleaseName })
        $expectedSize = [int64](Get-RecordPropertyValue $expected 'size')
        if ($asset.Count -ne 1 -or [int64]$asset[0].size -ne $expectedSize) { Fail "final public asset identity changed: $expectedReleaseName" }
        $finalPath = Join-Path (Get-EffectiveStateRoot) "final-$expectedReleaseName"
        Invoke-GitHubApi -Arguments @("api", [string]$asset[0].url, "--header", "Accept: application/octet-stream") -BinaryOutput -OutputPath $finalPath
        $hash = (Get-FileHash -LiteralPath $finalPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $expectedSha = Get-RecordPropertyString $expected 'sha256'
        if ($hash -ne $expectedSha) { Fail "final public asset digest differs from qualified bytes: $expectedReleaseName" }
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
    ) "GitHub Latest guard verification failed"
    Write-Host "V4 final verification: PASS (tag=$Tag channel=$Channel verified via raw endpoint)"
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
    "PromoteMetadata" { Invoke-PromoteMetadata }
    "FinalVerify" { Invoke-FinalVerify }
    "SelfTest" { Invoke-SelfTest }
    default { Fail "Unknown state '$State'" }
}
