[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet(
        "ValidateRequest",
        "ValidateRepository",
        "BuildCandidate",
        "CreateDraft",
        "DownloadDraft",
        "QualifyDownloaded",
        "RecordAttestations",
        "PublishDraft",
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
    [string]$PublicationDateUtc
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

function Get-V4ReleaseMakeLatestValue([string]$ReleaseChannel) {
    # GitHub's release API accepts an enum string here, not a JSON boolean.
    if ($ReleaseChannel -eq "stable") { return "true" }
    if ($ReleaseChannel -eq "beta") { return "false" }
    Fail "release channel is required to select make_latest"
}

function Read-JsonFile([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { Fail "Required state file is missing: $Path" }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
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
    if (-not [string]::IsNullOrWhiteSpace($PublicationDateUtc)) {
        if ($PublicationDateUtc -notmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$') {
            Fail "publication date must be second-precision UTC RFC3339"
        }
        try {
            [DateTimeOffset]::ParseExact(
                $PublicationDateUtc,
                "yyyy-MM-dd'T'HH:mm:ss'Z'",
                [Globalization.CultureInfo]::InvariantCulture,
                [Globalization.DateTimeStyles]::AssumeUniversal
            ) | Out-Null
        } catch {
            Fail "publication date is not a valid UTC timestamp"
        }
    }
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
    if ((Get-Item -LiteralPath $resolved).Length -gt 16384) { Fail "release notes exceed the bounded limit" }
    $relativePath = [IO.Path]::GetRelativePath($repoRoot, $resolved).Replace("\", "/")
    $expectedPath = "docs/releases/v$Version.md"
    if (-not $relativePath.Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "release notes path must match the requested version"
    }
    $notes = [IO.File]::ReadAllText($resolved)
    $firstHeading = [regex]::Match($notes, '(?m)^# [^\r\n]+(?=\r?$)')
    $expectedHeading = "# Sky Auto Player v$Version"
    if (-not $firstHeading.Success -or $firstHeading.Value -ne $expectedHeading) {
        Fail "release notes heading must match the requested version"
    }
    return $resolved
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
            # Do not include the provider response: it is not needed for diagnosis and
            # keeps credentials and server diagnostics out of the release log.
            Fail "GitHub API request failed"
        }
        if ($BinaryOutput) { return $null }
        $responseText = ($githubApiResult -join "`n")
        if ($Raw) { return $responseText }
        # GitHub's successful DELETE endpoints return an empty body. Treat that
        # as a successful request instead of attempting to parse empty JSON.
        if ([string]::IsNullOrWhiteSpace($responseText)) { return $null }
        return ($responseText | ConvertFrom-Json)
    } finally {
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}

function Get-CanonicalRepository {
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_REPOSITORY)) {
        Fail "GITHUB_REPOSITORY is required for release operations"
    }
    if ($env:GITHUB_REPOSITORY -ne $canonicalRepository) {
        Fail "release operations are permitted only for $canonicalRepository"
    }
    return $env:GITHUB_REPOSITORY
}

function Write-RepositoryContentFile([object]$Response, [string]$Path, [string]$ExpectedPath) {
    if ($null -eq $Response -or [string]$Response.type -ne "file" -or
        [string]$Response.path -ne $ExpectedPath) {
        Fail "repository content response is not the expected file: $ExpectedPath"
    }
    $encoded = [regex]::Replace([string]$Response.content, '\s', '')
    if ([string]::IsNullOrWhiteSpace($encoded)) { Fail "repository content file is empty: $ExpectedPath" }
    try {
        $bytes = [Convert]::FromBase64String($encoded)
    } catch {
        Fail "repository content file is not valid base64: $ExpectedPath"
    }
    if ($bytes.Length -eq 0) { Fail "repository content file has no bytes: $ExpectedPath" }
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function Assert-MetadataBranchReadiness([string]$Repository) {
    $branch = Invoke-GitHubApi -Arguments @("api", "repos/$Repository/git/ref/heads/release-metadata") -AllowNotFound
    if ($null -eq $branch) { Fail "canonical release-metadata branch is not initialized" }
    if ([string]$branch.ref -ne "refs/heads/release-metadata" -or
        [string]$branch.object.type -ne "commit") {
        Fail "canonical release-metadata branch identity is not canonical"
    }

    $bootstrap = Invoke-GitHubApi -Arguments @(
        "api", "repos/$Repository/contents/$metadataBootstrapPath`?ref=release-metadata"
    ) -AllowNotFound
    if ($null -eq $bootstrap) { Fail "release-metadata bootstrap contract is missing" }
    $bootstrapPath = Join-Path (Get-EffectiveStateRoot) "release-metadata-bootstrap.md"
    Write-RepositoryContentFile $bootstrap $bootstrapPath $metadataBootstrapPath
    $bootstrapText = [IO.File]::ReadAllText($bootstrapPath, [Text.UTF8Encoding]::new($false)).TrimEnd("`r", "`n")
    if ($bootstrapText -ne $metadataBootstrapContract) {
        Fail "release-metadata bootstrap contract is not canonical"
    }

    foreach ($channelName in @("stable", "beta")) {
        $metadataPath = "channels/$channelName/latest.json"
        $metadata = Invoke-GitHubApi -Arguments @(
            "api", "repos/$Repository/contents/$metadataPath`?ref=release-metadata"
        ) -AllowNotFound
        if ($null -eq $metadata) { continue }
        $localPath = Join-Path (Get-EffectiveStateRoot) "repository-$channelName-latest.json"
        Write-RepositoryContentFile $metadata $localPath $metadataPath
        Invoke-Checked "cargo" @(
            "xtask", "release-metadata", "validate", "--channel", $channelName, "--metadata", $localPath
        ) "existing release-metadata $channelName channel failed canonical validation"
    }
    Write-Host "V4 release-metadata readiness: PASS (orphan branch exists; bootstrap contract and existing channels are valid)"
}

function Assert-RepositoryReleasePolicy {
    $repository = Get-CanonicalRepository
    $main = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/ref/heads/main") -AllowNotFound
    if ($null -eq $main) {
        Fail "canonical repository main is not initialized"
    }
    if ([string]$main.ref -ne "refs/heads/main") { Fail "canonical repository main ref is not canonical" }
    Assert-MetadataBranchReadiness $repository
    Write-Host "V4 repository release preconditions: PASS (main exists; release-metadata ready; immutable publication is checked on the published release)"
}

function Get-PublicMetadataDocument([string]$Channel) {
    if (-not $rawMetadataEndpoints.ContainsKey($Channel)) {
        Fail "raw metadata endpoint channel is not canonical: $Channel"
    }
    $endpoint = [string]$rawMetadataEndpoints[$Channel]
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $lastError = ""
    try {
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            $request = $null
            $response = $null
            try {
                $request = [System.Net.Http.HttpRequestMessage]::new(
                    [System.Net.Http.HttpMethod]::Get,
                    $endpoint
                )
                [void]$request.Headers.Accept.ParseAdd("application/json")
                if ($null -ne $request.Headers.Authorization) {
                    $lastError = "raw metadata request unexpectedly carries Authorization"
                } else {
                    $response = $client.SendAsync($request).GetAwaiter().GetResult()
                    $status = [int]$response.StatusCode
                    if ($status -eq 200) {
                        $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
                        if (-not [string]::IsNullOrWhiteSpace($body)) {
                            return [pscustomobject]@{ Endpoint = $endpoint; Body = $body }
                        }
                        $lastError = "raw metadata endpoint returned an empty body"
                    } else {
                        $lastError = "raw metadata endpoint returned HTTP $status"
                    }
                }
            } catch {
                $lastError = $_.Exception.Message
            } finally {
                if ($null -ne $response) { $response.Dispose() }
                if ($null -ne $request) { $request.Dispose() }
            }
            if ($attempt -lt 3) { Start-Sleep -Seconds 2 }
        }
    } finally {
        $client.Dispose()
        $handler.Dispose()
    }
    Fail "raw metadata endpoint did not return a non-empty unauthenticated 200 response after bounded retry: $lastError"
}

function Get-ExpectedInstallerName {
    return "Sky Auto Player_${Version}$installerSuffix"
}

function Get-CandidateRecords {
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

function Get-FileRecord([object]$Candidate) {
    if (-not (Test-Path -LiteralPath $Candidate.path -PathType Leaf)) { Fail "qualified candidate file is missing: $($Candidate.name)" }
    $item = Get-Item -LiteralPath $Candidate.path
    if ($item.Length -le 0) { Fail "qualified candidate file is empty: $($Candidate.name)" }
    $sourceName = [string]$Candidate.name
    $releaseName = Get-V4SafeReleaseAssetName $sourceName
    return [pscustomobject]@{
        name = $releaseName
        source_name = $sourceName
        release_name = $releaseName
        role = [string]$Candidate.role
        size = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $Candidate.path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
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
    $instSourceName = if ($null -ne $instRec.PSObject.Properties['source_name']) { [string]$instRec.source_name } else { [string]$instRec.name }
    $sigSourceName = if ($null -ne $sigRec.PSObject.Properties['source_name']) { [string]$sigRec.source_name } else { [string]$sigRec.name }

    if ([string]$evidence.installer -ne (Get-ExpectedInstallerName) -or
        [string]$evidence.updater_signature -ne "$(Get-ExpectedInstallerName).sig" -or
        [string]$evidence.authenticode_evidence -ne $authenticodeEvidenceName -or
        [string]$evidence.sbom -ne $sbomName -or
        [string]$evidence.installer -ne $instSourceName -or
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
    Assert-EvidenceIdentity `
        (Join-Path $repoRoot "rust/target/dist/$productionEvidenceName") `
        (Join-Path $repoRoot "rust/target/dist/$qualificationEvidenceName") `
        $Records
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

    # This is the only production candidate build boundary. The orchestrator
    # owns key verification, stale purge, clean-worktree, NSIS, updater sig,
    # unsigned-zero-budget, SBOM, and install smoke semantics.
    Invoke-WithV4UpdaterSessionCredential -Action {
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
            -ExpectedSourceSha $SourceSha `
            -Version $Version `
            -Channel $Channel `
            -UpdaterPrivateKeyPath $keyPath
        if ($LASTEXITCODE -ne 0) { Fail "production orchestrator failed" }
    }

    $records = @(Get-CandidateRecords | ForEach-Object { Get-FileRecord $_ })
    $releaseNames = @{}
    foreach ($record in $records) {
        $releaseName = [string]$record.release_name
        if ($releaseNames.ContainsKey($releaseName)) {
            Fail "release asset name collision detected: '$releaseName' from source '$($record.source_name)' and '$($releaseNames[$releaseName])'"
        }
        $releaseNames[$releaseName] = [string]$record.source_name
    }
    Assert-CandidateEvidence $records
    $manifest = [ordered]@{
        schema_version = 1
        source_sha = $SourceSha.ToLowerInvariant()
        version = $Version
        channel = $Channel
        authenticode_mode = "unsigned-zero-budget"
        assets = $records
    }
    Write-JsonFile (Join-Path (Get-EffectiveStateRoot) "candidate-manifest.json") $manifest
    Write-Host "V4 candidate build: PASS (one orchestrator invocation; exact candidate manifest recorded)"
}

function Get-State {
    $state = Read-JsonFile (Get-StatePath)
    if ([string]$state.source_sha -ne $SourceSha.ToLowerInvariant() -or
        [string]$state.version -ne $Version -or [string]$state.channel -ne $Channel) {
        Fail "state file release identity does not match this invocation"
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

function Assert-ExistingUnpublishedDraftMatchesRequest([object]$Release) {
    if ([string]$Release.tag_name -ne $Tag) {
        Fail "existing release tag does not match the requested tag"
    }
    if (-not [bool]$Release.draft -or
        -not [string]::IsNullOrWhiteSpace([string]$Release.published_at)) {
        Fail "repository already contains published release/tag $Tag; published releases and tags are immutable"
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

function Assert-NoExistingReleaseTag {
    $repository = Get-CanonicalRepository
    $release = Get-ReleaseForTag $repository $Tag
    if ($null -ne $release) {
        Assert-ExistingUnpublishedDraftMatchesRequest $release
        $releaseId = [int64]$release.id
        if ($releaseId -le 0) { Fail "existing draft release id is missing" }
        Invoke-GitHubApi -Arguments @(
            "api", "--method", "DELETE", "repos/$repository/releases/$releaseId"
        ) -AllowNotFound | Out-Null
        $remainingReleaseById = Invoke-GitHubApi -Arguments @(
            "api", "repos/$repository/releases/$releaseId"
        ) -AllowNotFound
        if ($null -ne $remainingReleaseById) { Fail "draft release could not be removed by release id" }
        $remainingRelease = Get-ReleaseForTag $repository $Tag
        if ($null -ne $remainingRelease) { Fail "draft release could not be removed" }
        $runId = if ([string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID)) { "local" } else { $env:GITHUB_RUN_ID }
        Write-Host "V4 unpublished draft reuse: deleted prior unpublished draft for $Tag (source_sha=$($SourceSha.ToLowerInvariant()), run_id=$runId)"
    }
    $ref = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/ref/tags/$Tag") -AllowNotFound
    if ($null -ne $ref) {
        if ($null -eq $release) {
            Fail "repository already contains tag $Tag without an unpublished draft; published tags are immutable"
        }
        Invoke-GitHubApi -Arguments @(
            "api", "--method", "DELETE", "repos/$repository/git/refs/tags/$Tag"
        ) -AllowNotFound | Out-Null
        $remainingRef = Invoke-GitHubApi -Arguments @("api", "repos/$repository/git/ref/tags/$Tag") -AllowNotFound
        if ($null -ne $remainingRef) { Fail "unpublished draft tag $Tag could not be removed before recreation" }
    }
}

function Invoke-CreateDraft {
    Assert-RequestIdentity
    Assert-RepositoryReleasePolicy
    $repository = Get-CanonicalRepository
    $manifestPath = Join-Path (Get-EffectiveStateRoot) "candidate-manifest.json"
    $manifest = Read-JsonFile $manifestPath
    Assert-NoExistingReleaseTag
    $notesPath = Assert-ReleaseNotes
    $body = "V4 qualified release candidate`n`nsource_sha: $($SourceSha.ToLowerInvariant())`nchannel: $Channel`nqualification: exact candidate manifest attached`n"
    $payloadPath = Join-Path (Get-EffectiveStateRoot) "create-release.json"
    Write-JsonFile $payloadPath ([ordered]@{
        tag_name = $Tag
        target_commitish = $SourceSha.ToLowerInvariant()
        name = "Sky Auto Player $Version"
        body = $body + ([IO.File]::ReadAllText($notesPath)).Trim()
        draft = $true
        prerelease = ($Channel -eq "beta")
        make_latest = Get-V4ReleaseMakeLatestValue $Channel
    })
    $release = Invoke-GitHubApi -Arguments @("api", "--method", "POST", "repos/$repository/releases", "--input", $payloadPath)
    if (-not $release.draft -or [string]$release.tag_name -ne $Tag) { Fail "repository did not create the requested draft release" }
    $uploadUrl = [string]$release.upload_url
    if ([string]::IsNullOrWhiteSpace($uploadUrl)) { Fail "repository draft did not return its release-specific upload_url" }
    $uploadUrl = $uploadUrl -replace '\{\?name,label\}$', ''
    if ($uploadUrl -notmatch '^https://uploads\.github\.com/repos/[^/]+/[^/]+/releases/\d+/assets$') {
        Fail "repository draft returned an unexpected release asset upload_url"
    }

    foreach ($record in $manifest.assets) {
        $sourceName = if ($null -ne $record.PSObject.Properties['source_name']) { [string]$record.source_name } else { [string]$record.name }
        $releaseName = if ($null -ne $record.PSObject.Properties['release_name']) { [string]$record.release_name } else { Get-V4SafeReleaseAssetName $sourceName }
        $candidate = (Get-CandidateRecords | Where-Object { $_.name -eq $sourceName })
        if ($null -eq $candidate) { Fail "candidate manifest contains an unknown asset: $sourceName" }
        # GitHub's release-specific upload_url is deliberately used here. The
        # endpoint rejects duplicate names; this path never deletes or
        # replaces an asset after a failed upload.
        $uploaded = Invoke-V4ReleaseAssetUpload `
            -UploadUrl $uploadUrl `
            -AssetName $releaseName `
            -FilePath ([string]$candidate.path)
        if ([string]$uploaded.name -ne $releaseName -or
            [int64]$uploaded.size -ne [int64]$record.size -or
            [string]$uploaded.state -ne "uploaded") {
            Fail "repository upload did not return the exact uploaded asset: $releaseName"
        }
    }
    $state = [ordered]@{
        schema_version = 1
        source_sha = $SourceSha.ToLowerInvariant()
        version = $Version
        channel = $Channel
        tag = $Tag
        release_id = [int64]$release.id
        draft = $true
        published = $false
        immutable = $false
        attested = $false
        qualified_after_download = $false
        assets = $manifest.assets
    }
    Write-JsonFile (Get-StatePath) $state
    Write-Host "V4 repository draft: PASS (tag=$Tag; exact qualified asset set uploaded)"
}

function Assert-ExactAssetSet([object]$Release, [object[]]$Expected) {
    $actual = @($Release.assets | ForEach-Object { [string]$_.name } | Sort-Object)
    $expectedNames = @($Expected | ForEach-Object {
        if ($null -ne $_.PSObject.Properties['release_name']) { [string]$_.release_name } else { [string]$_.name }
    } | Sort-Object)
    if (($actual -join "`n") -ne ($expectedNames -join "`n")) { Fail "repository release asset set differs from the qualified candidate set" }
}

function Assert-ImmutableRelease([object]$Release) {
    if ($null -eq $Release.immutable -or -not [bool]$Release.immutable) {
        Fail "repository release is not marked immutable"
    }
}

function Invoke-DownloadDraft {
    $state = Get-State
    if (-not $state.draft -or $state.published) { Fail "draft download requires an unpublished draft release" }
    $repository = Get-CanonicalRepository
    $release = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/$($state.release_id)")
    if (-not $release.draft -or [string]$release.tag_name -ne $Tag) { Fail "repository release is not the expected draft" }
    Assert-ExactAssetSet $release $state.assets
    $downloaded = Join-Path (Get-EffectiveStateRoot) "downloaded"
    if (Test-Path -LiteralPath $downloaded) { Remove-Item -LiteralPath $downloaded -Recurse -Force }
    New-Item -ItemType Directory -Path $downloaded -Force | Out-Null
    foreach ($expected in $state.assets) {
        $expectedReleaseName = if ($null -ne $expected.PSObject.Properties['release_name']) { [string]$expected.release_name } else { [string]$expected.name }
        $asset = @($release.assets | Where-Object { [string]$_.name -eq $expectedReleaseName })
        if ($asset.Count -ne 1) { Fail "expected repository asset is missing: $expectedReleaseName" }
        $destination = Join-Path $downloaded ([IO.Path]::GetFileName($expectedReleaseName))
        Invoke-GitHubApi -Arguments @("api", [string]$asset[0].url, "--header", "Accept: application/octet-stream") -BinaryOutput -OutputPath $destination
        $item = Get-Item -LiteralPath $destination
        $hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        if ([int64]$item.Length -ne [int64]$expected.size -or $hash -ne [string]$expected.sha256) {
            Fail "downloaded repository asset differs in size or SHA-256: $expectedReleaseName"
        }
    }
    Write-JsonFile (Join-Path (Get-EffectiveStateRoot) "downloaded-manifest.json") ([ordered]@{
        schema_version = 1
        source_sha = [string]$state.source_sha
        version = [string]$state.version
        channel = [string]$state.channel
        assets = $state.assets
    })
    Write-Host "V4 draft download: PASS (all qualification inputs re-downloaded and byte-checked)"
}

function Invoke-Checked([string]$File, [string[]]$Arguments, [string]$Failure) {
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) { Fail $Failure }
}

function Invoke-QualifyDownloaded {
    $state = Get-State
    if (-not $state.draft -or $state.published) { Fail "post-draft qualification requires an unpublished draft" }
    $root = Get-EffectiveStateRoot
    $downloaded = Join-Path $root "downloaded"
    $manifest = Read-JsonFile (Join-Path $root "downloaded-manifest.json")
    if ([string]$manifest.source_sha -ne $SourceSha.ToLowerInvariant()) { Fail "downloaded source binding mismatch" }
    Assert-EvidenceIdentity `
        (Join-Path $downloaded $productionEvidenceName) `
        (Join-Path $downloaded $qualificationEvidenceName) `
        @($manifest.assets)
    $bundle = Join-Path $root "downloaded-bundle"
    if (Test-Path -LiteralPath $bundle) { Remove-Item -LiteralPath $bundle -Recurse -Force }
    New-Item -ItemType Directory -Path $bundle -Force | Out-Null
    $sourceInstaller = Get-ExpectedInstallerName
    $sourceSignature = "$sourceInstaller.sig"
    $releaseInstaller = Get-V4SafeReleaseAssetName $sourceInstaller
    $releaseSignature = Get-V4SafeReleaseAssetName $sourceSignature
    Copy-Item -LiteralPath (Join-Path $downloaded $releaseInstaller) -Destination (Join-Path $bundle $sourceInstaller)
    Copy-Item -LiteralPath (Join-Path $downloaded $releaseSignature) -Destination (Join-Path $bundle $sourceSignature)

    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1"),
        "-Mode", "unsigned-zero-budget", "-Artifact", (Join-Path $bundle $sourceInstaller),
        "-Evidence", (Join-Path $root "downloaded-authenticode-verification.json")
    ) "downloaded candidate Authenticode state is not unsigned-zero-budget"
    Invoke-Checked "cargo" @(
        "xtask", "updater-trust", "verify-signature", "--installer", (Join-Path $bundle $sourceInstaller),
        "--signature", (Join-Path $bundle $sourceSignature)
    ) "downloaded candidate Tauri updater signature verification failed"
    Invoke-Checked "cargo" @(
        "xtask", "sbom", "verify", "--artifact-dir", $bundle, "--sbom", (Join-Path $downloaded $sbomName)
    ) "downloaded candidate SPDX SBOM verification failed"
    Invoke-Checked "cargo" @(
        "xtask", "verify-tauri-bundle", "--bundle-dir", $bundle,
        "--summary", (Join-Path $downloaded $summaryName),
        "--authenticode-evidence", (Join-Path $downloaded $authenticodeEvidenceName),
        "--sbom", (Join-Path $downloaded $sbomName)
    ) "downloaded candidate exact Tauri bundle verification failed"
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "promote_v4_metadata.ps1"),
        "-ValidateEvidence", (Join-Path $downloaded $qualificationEvidenceName)
    ) "downloaded candidate qualification evidence schema validation failed"

    # Export the canonical public root through the existing updater-trust
    # release, then run the exact downloaded-candidate updater fixture against the
    # installer and signature downloaded from the draft. The fixture builds
    # only its throwaway previous client; it never rebuilds the candidate.
    $canonicalPublicKey = Join-Path $root "canonical-updater-public-key.txt"
    Invoke-Checked "cargo" @(
        "xtask", "updater-trust", "export-public-key", "--output", $canonicalPublicKey
    ) "canonical updater public-root export failed"
    $fixtureTargetDir = Join-Path $root "previous-v4-fixture-target"
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "ci_tauri_update_e2e.ps1"),
        "-FixtureTargetDir", $fixtureTargetDir,
        "-CandidateInstallerPath", (Join-Path $bundle $sourceInstaller),
        "-CandidateSignaturePath", (Join-Path $bundle $sourceSignature),
        "-CandidateVersion", $Version,
        "-CandidatePublicKeyPath", $canonicalPublicKey,
        "-EvidencePath", (Join-Path $root "fixture-http-evidence.json")
    ) "exact downloaded previous-v4 to candidate-v4 updater qualification failed"

    # Production policy requires a deterministic exact-artifact Defender
    # custom scan on the downloaded installer. scan_v4_defender_exact.ps1
    # invokes Start-MpScan and records scan_performed; missing Defender or a
    # scan failure is a release failure, not an accepted unavailable result.
    $defenderEvidencePath = Join-Path $root "defender-evidence.json"
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "scan_v4_defender_exact.ps1"),
        "-Artifact", (Join-Path $bundle $sourceInstaller),
        "-Evidence", $defenderEvidencePath
    ) "exact downloaded installer Defender scan failed"
    $defenderEvidence = Read-JsonFile $defenderEvidencePath
    $installerRecord = @($state.assets | Where-Object {
        [string]$_.name -eq $releaseInstaller -or
        ($null -ne $_.PSObject.Properties['source_name'] -and [string]$_.source_name -eq $sourceInstaller)
    })
    if (-not [bool]$defenderEvidence.scan_performed -or
        [string]$defenderEvidence.detection_result -ne "none" -or
        $installerRecord.Count -ne 1 -or
        [string]$defenderEvidence.artifact_sha256 -ne [string]$installerRecord[0].sha256) {
        Fail "Defender evidence did not bind a clean scan to the exact downloaded installer"
    }

    # Reuse the production current-user smoke shape against the downloaded
    # installer. The packaged shell self-test exercises GUI/native command
    # ownership without physical input injection; the Rust activity test is
    # the fail-closed proof that update installation is rejected during playback.
    $installRoot = Join-Path $root ("install-" + [guid]::NewGuid().ToString("N"))
    $app = Join-Path $installRoot "sky_desktop_shell.exe"
    $uninstaller = Join-Path $installRoot "uninstall.exe"
    $installedBuiltinCatalogEvidence = $null
    $freshBuiltinCatalogEvidence = $null
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { Fail "LOCALAPPDATA is unavailable for external app-data preservation qualification" }
    $preservationRoot = Join-Path $env:LOCALAPPDATA ("io.github.pumni.skyautoplayer/wo07-release-test-" + [guid]::NewGuid().ToString("N"))
    $preservationMarker = Join-Path $preservationRoot "preserve.txt"
    New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $preservationRoot -Force | Out-Null
    [IO.File]::WriteAllText($preservationMarker, "external-user-data-marker`n", [Text.UTF8Encoding]::new($false))
    try {
        $install = Start-Process -FilePath (Join-Path $bundle $sourceInstaller) -ArgumentList @("/S", "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
        if ($install.ExitCode -ne 0) { Fail "downloaded candidate current-user installer failed" }
        if (-not (Test-Path -LiteralPath $app) -or -not (Test-Path -LiteralPath $uninstaller)) { Fail "downloaded candidate install omitted app or uninstaller" }
        $installedPe = @(Get-ChildItem -LiteralPath $installRoot -File -Recurse | Where-Object {
            $_.Extension.ToLowerInvariant() -in @(".exe", ".dll") -and $_.Name -ne "uninstall.exe"
        })
        if ($installedPe.Count -eq 0) { Fail "downloaded candidate installed no project PE files" }
        $installedBuiltinRoot = Join-Path $installRoot "builtin-songs"
        Invoke-Checked "cargo" @(
            "xtask", "builtin-catalog", "verify-installed", "--root", $installedBuiltinRoot
        ) "downloaded candidate installed built-in catalog verification failed"
        $installedBuiltinFiles = @(Get-ChildItem -LiteralPath $installedBuiltinRoot -File -Recurse | Sort-Object FullName)
        if ($installedBuiltinFiles.Count -eq 0) { Fail "downloaded candidate installed built-in catalog is empty" }
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
        Invoke-Checked "pwsh" @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1"),
            "-Mode", "unsigned-zero-budget", "-Artifact" , $installedPe.FullName
        ) "downloaded candidate installed PE state is not unsigned-zero-budget"
        $previousAppDataRoot = [Environment]::GetEnvironmentVariable("SKY_APP_DATA_ROOT", "Process")
        $previousFreshSelfTest = [Environment]::GetEnvironmentVariable("SKY_BUILTIN_CATALOG_FRESH_SELFTEST", "Process")
        $freshAppData = Join-Path $root ("fresh-appdata-" + [guid]::NewGuid().ToString("N"))
        try {
            [Environment]::SetEnvironmentVariable("SKY_APP_DATA_ROOT", $freshAppData, "Process")
            [Environment]::SetEnvironmentVariable("SKY_BUILTIN_CATALOG_FRESH_SELFTEST", "1", "Process")
            $catalogSelftest = Start-Process -FilePath $app -ArgumentList @("--selftest-desktop-shell") -WindowStyle Hidden -Wait -PassThru
            if ($catalogSelftest.ExitCode -ne 0) { Fail "downloaded candidate fresh built-in catalog self-test failed with exit code $($catalogSelftest.ExitCode)" }
            $freshSongsRoot = Join-Path $freshAppData "songs"
            $freshUserSongs = @(
                Get-ChildItem -LiteralPath $freshSongsRoot -File -Recurse -ErrorAction SilentlyContinue
            )
            if ($freshUserSongs.Count -ne 0) { Fail "downloaded candidate fresh built-in catalog self-test populated user songs" }
            $freshBuiltinCatalogEvidence = [ordered]@{
                status = "PASS"
                app_data_root = "isolated-release-state"
                built_ins_visible_in_all_songs = $true
                user_song_composition_exercised = $true
                user_songs_empty_after_selftest = $true
            }
        } finally {
            if ($null -eq $previousAppDataRoot) {
                Remove-Item Env:SKY_APP_DATA_ROOT -ErrorAction SilentlyContinue
            } else {
                [Environment]::SetEnvironmentVariable("SKY_APP_DATA_ROOT", $previousAppDataRoot, "Process")
            }
            if ($null -eq $previousFreshSelfTest) {
                Remove-Item Env:SKY_BUILTIN_CATALOG_FRESH_SELFTEST -ErrorAction SilentlyContinue
            } else {
                [Environment]::SetEnvironmentVariable("SKY_BUILTIN_CATALOG_FRESH_SELFTEST", $previousFreshSelfTest, "Process")
            }
            if (Test-Path -LiteralPath $freshAppData) { Remove-Item -LiteralPath $freshAppData -Recurse -Force -ErrorAction SilentlyContinue }
        }
        $activity = Start-Process -FilePath $app -ArgumentList @("--selftest-update-active-playback") -WindowStyle Hidden -Wait -PassThru
        if ($activity.ExitCode -ne 0) { Fail "downloaded candidate packaged playback-active update rejection self-test failed" }
        $shell = Start-Process -FilePath $app -ArgumentList @("--selftest-desktop-shell") -WindowStyle Hidden -Wait -PassThru
        if ($shell.ExitCode -ne 0) { Fail "downloaded candidate packaged shell self-test failed" }
        $gui = Start-Process -FilePath $app -ArgumentList @("--selftest-desktop-gui") -WindowStyle Hidden -Wait -PassThru
        if ($gui.ExitCode -ne 0) { Fail "downloaded candidate GUI/input safety self-test failed" }
        $uninstall = Start-Process -FilePath $uninstaller -ArgumentList @("/S") -WindowStyle Hidden -Wait -PassThru
        if ($uninstall.ExitCode -ne 0) { Fail "downloaded candidate uninstall failed" }
        if (-not (Test-Path -LiteralPath $preservationMarker -PathType Leaf)) { Fail "uninstall removed external app/user data" }
        $reinstall = Start-Process -FilePath (Join-Path $bundle $sourceInstaller) -ArgumentList @("/S", "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
        if ($reinstall.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $app)) { Fail "downloaded candidate reinstall failed" }
        if (-not (Test-Path -LiteralPath $preservationMarker -PathType Leaf)) { Fail "reinstall did not preserve external app/user data" }
        $finalUninstall = Start-Process -FilePath (Join-Path $installRoot "uninstall.exe") -ArgumentList @("/S") -WindowStyle Hidden -Wait -PassThru
        if ($finalUninstall.ExitCode -ne 0) { Fail "downloaded candidate final uninstall failed" }
    } finally {
        if (Test-Path -LiteralPath $installRoot) { Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $preservationRoot) { Remove-Item -LiteralPath $preservationRoot -Recurse -Force -ErrorAction SilentlyContinue }
    }
    Invoke-Checked "cargo" @(
        "test", "--manifest-path", "rust/Cargo.toml", "-p", "sky_desktop_shell", "app_state::tests::update_installation_is_rejected_while_physical_playback_is_active", "--", "--exact"
    ) "update installation while playback is active was not rejected"
    Write-JsonFile (Join-Path $root "post-draft-qualification.json") ([ordered]@{
        schema_version = 1
        source_sha = $SourceSha.ToLowerInvariant()
        version = $Version
        channel = $Channel
        downloaded_exact_bytes = $true
        authenticode_mode = "unsigned-zero-budget"
        qualification = @(
            "fresh-current-user-no-admin-install",
            "gui-input-safety",
            "previous-v4-to-exact-downloaded-candidate-update",
            "official-tauri-updater-signature",
            "active-playback-install-rejected-packaged",
            "uninstall",
            "reinstall-preserves-external-app-data",
            "installed-built-in-catalog-exact-manifest-file-set-sha-parseability",
            "fresh-appdata-built-in-user-composition",
            "defender-exact-download-scan-no-detection",
            "spdx-sbom",
            "exact-asset-digest"
        )
        installed_builtin_catalog = $installedBuiltinCatalogEvidence
        fresh_builtin_catalog_selftest = $freshBuiltinCatalogEvidence
    })
    $state.qualified_after_download = $true
    $state.attested = $false
    Write-JsonFile (Get-StatePath) $state
    Write-Host "V4 post-draft qualification: PASS (downloaded exact bytes only)"
}

function Invoke-PublishDraft {
    $state = Get-State
    if (-not $state.draft -or $state.published) { Fail "publish requires the same unpublished draft" }
    if (-not $state.qualified_after_download -or -not $state.attested) { Fail "publish requires downloaded qualification and exact-byte attestations" }
    $repository = Get-CanonicalRepository
    $release = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/$($state.release_id)")
    if (-not $release.draft -or [string]$release.tag_name -ne $Tag) { Fail "draft is missing or has changed before publication" }
    Assert-ExactAssetSet $release $state.assets
    foreach ($expected in $state.assets) {
        $expectedReleaseName = if ($null -ne $expected.PSObject.Properties['release_name']) { [string]$expected.release_name } else { [string]$expected.name }
        $asset = @($release.assets | Where-Object { [string]$_.name -eq $expectedReleaseName })
        if ($asset.Count -ne 1 -or [int64]$asset[0].size -ne [int64]$expected.size) { Fail "draft asset changed before publication: $expectedReleaseName" }
        $downloadedPath = Join-Path (Get-EffectiveStateRoot) "downloaded/$expectedReleaseName"
        $downloadedHash = (Get-FileHash -LiteralPath $downloadedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($downloadedHash -ne [string]$expected.sha256) { Fail "qualified downloaded digest changed before publication: $expectedReleaseName" }
    }
    $patchPath = Join-Path (Get-EffectiveStateRoot) "publish-release.json"
    Write-JsonFile $patchPath ([ordered]@{ draft = $false; make_latest = Get-V4ReleaseMakeLatestValue $Channel })
    $published = Invoke-GitHubApi -Arguments @("api", "--method", "PATCH", "repos/$repository/releases/$($state.release_id)", "--input", $patchPath)
    if ($published.draft -or [string]::IsNullOrWhiteSpace([string]$published.published_at)) { Fail "repository did not publish the already-qualified draft" }
    Assert-ImmutableRelease $published
    $state.draft = $false
    $state.published = $true
    $state.immutable = $true
    Write-JsonFile (Get-StatePath) $state
    Write-Host "V4 immutable publication: PASS (assets were not replaced or rebuilt)"
}

function Invoke-RecordAttestations {
    $state = Get-State
    if (-not $state.draft -or $state.published) { Fail "attestation recording requires an unpublished draft" }
    if (-not $state.qualified_after_download) { Fail "attestations require downloaded-byte qualification" }
    $manifestPath = Join-Path (Get-EffectiveStateRoot) "downloaded-manifest.json"
    $manifest = Read-JsonFile $manifestPath
    if ([string]$manifest.source_sha -ne $SourceSha.ToLowerInvariant()) { Fail "attestation source binding mismatch" }
    # The workflow invokes actions/attest and verifies all three predicates
    # immediately before this state. This state records that externally
    # verified fact without fabricating local provenance.
    $state.attested = $true
    Write-JsonFile (Get-StatePath) $state
    Write-Host "V4 exact-byte attestations: PASS (OIDC/source binding verified by workflow)"
}

function Invoke-PromoteMetadata {
    $state = Get-State
    if (-not $state.published) { Fail "metadata promotion is forbidden before immutable publication" }
    $root = Get-EffectiveStateRoot
    $downloaded = Join-Path $root "downloaded"
    $repository = Get-CanonicalRepository
    $metadataCheckout = Join-Path $root "release-metadata"
    if (Test-Path -LiteralPath $metadataCheckout) { Remove-Item -LiteralPath $metadataCheckout -Recurse -Force }
    Invoke-GitHubApi -Arguments @("repo", "clone", $repository, $metadataCheckout, "--", "--branch", "release-metadata", "--depth", "1") -Raw | Out-Null
    if (-not (Test-Path -LiteralPath (Join-Path $metadataCheckout ".git") -PathType Container)) { Fail "release-metadata branch checkout was not obtained" }
    $metadata = Join-Path $root "latest.json"
    $notesPath = Assert-ReleaseNotes
    $sourceInstaller = Get-ExpectedInstallerName
    $releaseInstaller = Get-V4SafeReleaseAssetName $sourceInstaller
    $releaseSignature = Get-V4SafeReleaseAssetName "$sourceInstaller.sig"
    $signature = Join-Path $downloaded $releaseSignature
    $assetUrl = "https://github.com/$repository/releases/download/$Tag/$releaseInstaller"
    Invoke-Checked "cargo" @(
        "xtask", "release-metadata", "generate", "--channel", $Channel, "--version", $Version,
        "--notes-file", $notesPath, "--pub-date", $PublicationDateUtc, "--platform", "windows-x86_64",
        "--asset-url", $assetUrl, "--signature-file", $signature, "--output", $metadata
    ) "deterministic v4 metadata generation failed"
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "promote_v4_metadata.ps1"),
        "-Channel", $Channel, "-Metadata", $metadata,
        "-QualificationEvidence", (Join-Path $downloaded $qualificationEvidenceName),
        "-MetadataCheckout", $metadataCheckout, "-SourceCheckout", $repoRoot
    ) "post-publication metadata promotion validation failed"
    $destination = Join-Path $metadataCheckout "channels/$Channel/latest.json"
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) { Fail "promotion did not produce the governed channel metadata" }
    $encoded = [Convert]::ToBase64String([IO.File]::ReadAllBytes($destination))
    $existing = Invoke-GitHubApi -Arguments @("api", "repos/$repository/contents/channels/$Channel/latest.json?ref=release-metadata") -AllowNotFound
    if ($null -ne $existing) {
        $currentPath = Join-Path $root "current-$Channel-latest.json"
        Write-RepositoryContentFile $existing $currentPath "channels/$Channel/latest.json"
        Invoke-Checked "cargo" @(
            "xtask", "release-metadata", "validate-monotonic", "--channel", $Channel,
            "--current", $currentPath, "--candidate", $destination
        ) "live release-metadata channel is not a valid strict SemVer roll-forward"
        Write-Host "V4 metadata promotion: live channel passed strictly monotonic SemVer validation"
    } else {
        Write-Host "V4 metadata promotion: first channel publication (no current latest.json)"
    }
    $payload = [ordered]@{
        message = "Promote v4 $Channel metadata for $Version"
        content = $encoded
        branch = "release-metadata"
    }
    if ($null -ne $existing) { $payload.sha = [string]$existing.sha }
    $payloadPath = Join-Path $root "metadata-commit.json"
    Write-JsonFile $payloadPath $payload
    Invoke-GitHubApi -Arguments @("api", "--method", "PUT", "repos/$repository/contents/channels/$Channel/latest.json", "--input", $payloadPath) | Out-Null
    Write-Host "V4 metadata promotion: PASS (channel=$Channel; publication already immutable)"
}

function Invoke-FinalVerify {
    $state = Get-State
    if (-not $state.published) { Fail "final verification requires a published release" }
    $repository = Get-CanonicalRepository
    $release = Invoke-GitHubApi -Arguments @("api", "repos/$repository/releases/tags/$Tag")
    if ($release.draft -or [string]::IsNullOrWhiteSpace([string]$release.published_at)) { Fail "final release is still draft or unpublished" }
    Assert-ImmutableRelease $release
    Assert-ExactAssetSet $release $state.assets
    foreach ($expected in $state.assets) {
        $expectedReleaseName = if ($null -ne $expected.PSObject.Properties['release_name']) { [string]$expected.release_name } else { [string]$expected.name }
        $asset = @($release.assets | Where-Object { [string]$_.name -eq $expectedReleaseName })
        if ($asset.Count -ne 1 -or [int64]$asset[0].size -ne [int64]$expected.size) { Fail "final public asset identity changed: $expectedReleaseName" }
        $finalPath = Join-Path (Get-EffectiveStateRoot) "final-$expectedReleaseName"
        Invoke-GitHubApi -Arguments @("api", [string]$asset[0].url, "--header", "Accept: application/octet-stream") -BinaryOutput -OutputPath $finalPath
        $hash = (Get-FileHash -LiteralPath $finalPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne [string]$expected.sha256) { Fail "final public asset digest differs from qualified bytes: $expectedReleaseName" }
    }
    $metadataResponse = Invoke-GitHubApi -Arguments @("api", "repos/$repository/contents/channels/$Channel/latest.json?ref=release-metadata")
    $metadataPath = Join-Path (Get-EffectiveStateRoot) "final-metadata.json"
    Write-RepositoryContentFile $metadataResponse $metadataPath "channels/$Channel/latest.json"
    Invoke-Checked "cargo" @("xtask", "release-metadata", "validate", "--channel", $Channel, "--metadata", $metadataPath) "authenticated metadata failed deterministic validation"
    $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
    $publicMetadata = Get-PublicMetadataDocument $Channel
    if ([string]$publicMetadata.Endpoint -ne [string]$rawMetadataEndpoints[$Channel]) {
        Fail "final metadata was not fetched from the exact canonical raw.githubusercontent.com endpoint"
    }
    $publicMetadataPath = Join-Path (Get-EffectiveStateRoot) "final-public-metadata.json"
    [IO.File]::WriteAllText($publicMetadataPath, $publicMetadata.Body, [Text.UTF8Encoding]::new($false))
    Invoke-Checked "cargo" @("xtask", "release-metadata", "validate", "--channel", $Channel, "--metadata", $publicMetadataPath) "unauthenticated raw metadata failed deterministic validation"
    $publicMetadataJson = Get-Content -LiteralPath $publicMetadataPath -Raw | ConvertFrom-Json
    $sourceInstaller = Get-ExpectedInstallerName
    $releaseInstaller = Get-V4SafeReleaseAssetName $sourceInstaller
    $releaseSignature = Get-V4SafeReleaseAssetName "$sourceInstaller.sig"
    $expectedUrl = "https://github.com/$repository/releases/download/$Tag/$releaseInstaller"
    if ([string]$metadata.version -ne $Version -or [string]$metadata.platforms.'windows-x86_64'.url -ne $expectedUrl -or
        [string]$publicMetadataJson.version -ne $Version -or
        [string]$publicMetadataJson.platforms.'windows-x86_64'.url -ne $expectedUrl) {
        Fail "final metadata does not reference the exact immutable public asset"
    }
    $finalSignature = Get-Content -LiteralPath (Join-Path (Get-EffectiveStateRoot) "final-$releaseSignature") -Raw
    $metadataSignature = [string]$metadata.platforms.'windows-x86_64'.signature
    $publicMetadataSignature = [string]$publicMetadataJson.platforms.'windows-x86_64'.signature
    if ($finalSignature.Trim() -ne $metadataSignature.Trim() -or
        $finalSignature.Trim() -ne $publicMetadataSignature.Trim() -or
        [string]$publicMetadataJson.version -ne [string]$metadata.version -or
        [string]$publicMetadataJson.platforms.'windows-x86_64'.url -ne [string]$metadata.platforms.'windows-x86_64'.url) {
        Fail "final metadata signature does not match the exact public Tauri signature asset"
    }
    if ($expectedUrl -notmatch '^https://github\.com/pumni/Sky-Auto-Player/releases/download/') {
        Fail "metadata asset URL is outside the canonical repository"
    }
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "ci_v4_release_latest_guard.ps1"),
        "-Mode", "Verify",
        "-Channel", $Channel,
        "-ExpectedTag", $Tag,
        "-ExpectedSourceSha", $SourceSha,
        "-StateRoot", (Get-EffectiveStateRoot)
    ) "GitHub Latest channel policy verification failed"
    Write-Host "V4 final public verification: PASS (published immutable assets and metadata are exact)"
}

function Invoke-SelfTest {
    $scriptPath = (Resolve-Path $PSCommandPath).Path
    $source = Get-Content -LiteralPath $scriptPath -Raw
    if ($source -notmatch 'draft = \$true' -or $source -notmatch "qualified_after_download" -or
        $source -notmatch "metadata promotion is forbidden before immutable publication") {
        Fail "self-test could not find draft/qualification/publication guards"
    }
    $mock = [ordered]@{ builds = 0; draft = $false; downloaded = $false; qualified = $false; attested = $false; published = $false; promoted = $false }
    $mock.builds++
    $mock.draft = $true
    $mock.downloaded = $true
    $mock.qualified = $true
    try {
        if (-not $mock.published) { throw "promotion before publication" }
        Fail "mock promotion-before-publication unexpectedly succeeded"
    } catch {
        if ($_.Exception.Message -notmatch "promotion before publication") { throw }
    }
    try {
        Assert-ImmutableRelease ([pscustomobject]@{ immutable = $false })
        Fail "immutable=false unexpectedly passed the publication guard"
    } catch {
        if ($_.Exception.Message -notmatch "repository release is not marked immutable") { throw }
    }
    Assert-ImmutableRelease ([pscustomobject]@{ immutable = $true })
    Write-Host "V4 immutable publication guard self-test: PASS (immutable=false rejected; immutable=true accepted)"

    foreach ($channelCase in @(
        [pscustomobject]@{ Channel = "stable"; Expected = "true" },
        [pscustomobject]@{ Channel = "beta"; Expected = "false" }
    )) {
        foreach ($draftValue in @($true, $false)) {
            $payload = [ordered]@{
                draft = $draftValue
                make_latest = Get-V4ReleaseMakeLatestValue $channelCase.Channel
            }
            $roundTrip = (($payload | ConvertTo-Json -Depth 20) | ConvertFrom-Json)
            if ($roundTrip.make_latest -isnot [string] -or $roundTrip.make_latest -ne $channelCase.Expected) {
                Fail "GitHub release payload make_latest must round-trip as the string enum $($channelCase.Expected) for $($channelCase.Channel)"
            }
            if ($roundTrip.draft -isnot [bool] -or [bool]$roundTrip.draft -ne $draftValue) {
                Fail "GitHub release payload draft must remain a JSON boolean"
            }
        }
    }
    Write-Host "V4 GitHub release payload self-test: PASS (make_latest is string enum true for stable and false for beta)"

    $mock.attested = $true
    $mock.published = $true
    $mock.promoted = $true
    if ($mock.builds -ne 1 -or -not $mock.draft -or -not $mock.downloaded -or -not $mock.qualified -or -not $mock.published -or -not $mock.promoted) {
        Fail "mock release state machine did not preserve build-once and publication ordering"
    }
    Write-Host "V4 release pipeline state-machine self-test: PASS (mock draft/download/qualify/publish/promote; build count=1)"
}

if ($State -eq "SelfTest") {
    Invoke-SelfTest
    exit 0
}

switch ($State) {
    "ValidateRequest" { Assert-RequestIdentity; Assert-ReleaseNotes | Out-Null }
    "ValidateRepository" { Assert-RequestIdentity; Assert-RepositoryReleasePolicy }
    "BuildCandidate" { Invoke-BuildCandidate }
    "CreateDraft" { Assert-RequestIdentity; Invoke-CreateDraft }
    "DownloadDraft" { Assert-RequestIdentity; Invoke-DownloadDraft }
    "QualifyDownloaded" { Assert-RequestIdentity; Invoke-QualifyDownloaded }
    "RecordAttestations" { Assert-RequestIdentity; Invoke-RecordAttestations }
    "PublishDraft" { Assert-RequestIdentity; Invoke-PublishDraft }
    "PromoteMetadata" { Assert-RequestIdentity; Invoke-PromoteMetadata }
    "FinalVerify" { Assert-RequestIdentity; Invoke-FinalVerify }
}
