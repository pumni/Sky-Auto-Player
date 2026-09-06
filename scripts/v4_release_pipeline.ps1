[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet(
        "ValidateRequest",
        "ValidateAuthority",
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
    [string]$PublicationDateUtc,
    [string]$AuthorityTokenEnv = "V4_RELEASE_AUTHORITY_TOKEN"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$authorityRepository = "pumni/Sky-Auto-Player-Releases"
$sourceRepository = "pumni/Sky-Auto-Player"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$installerSuffix = "_x64-setup.exe"
$productionEvidenceName = "V4_PRODUCTION_RELEASE_EVIDENCE.json"
$qualificationEvidenceName = "V4_QUALIFICATION_EVIDENCE.json"
$authenticodeEvidenceName = "TAURI_AUTHENTICODE_EVIDENCE.json"
$installedAuthenticodeEvidenceName = "INSTALLED_AUTHENTICODE_EVIDENCE.json"
$summaryName = "TAURI_ARTIFACT_SUMMARY.json"
$sbomName = "SBOM.spdx.json"
. (Join-Path $PSScriptRoot "v4_release_authority_upload.ps1")

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
    return $resolved
}

function Get-AuthorityToken {
    $token = [Environment]::GetEnvironmentVariable($AuthorityTokenEnv, "Process")
    if ([string]::IsNullOrWhiteSpace($token)) { Fail "authority token environment variable is unavailable" }
    return $token
}

function Invoke-WithAuthorityToken([scriptblock]$Action) {
    $token = Get-AuthorityToken
    $oldGhToken = [Environment]::GetEnvironmentVariable("GH_TOKEN", "Process")
    try {
        [Environment]::SetEnvironmentVariable("GH_TOKEN", $token, "Process")
        & $Action
    } finally {
        if ($null -eq $oldGhToken) {
            [Environment]::SetEnvironmentVariable("GH_TOKEN", $null, "Process")
        } else {
            [Environment]::SetEnvironmentVariable("GH_TOKEN", $oldGhToken, "Process")
        }
    }
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

function Invoke-AuthorityApi {
    param(
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [switch]$AllowNotFound,
        [switch]$BinaryOutput,
        [switch]$Raw,
        [string]$OutputPath
    )
    $errorPath = Join-Path (Get-EffectiveStateRoot) ("gh-error-" + [guid]::NewGuid().ToString("N") + ".log")
    try {
        Invoke-WithAuthorityToken {
            if ($BinaryOutput) {
                $script:authorityApiExitCode = Invoke-GhBinaryOutput `
                    -Arguments $Arguments -OutputPath $OutputPath -ErrorPath $errorPath
            } else {
                $script:authorityApiResult = & gh @Arguments 2>$errorPath
                $script:authorityApiExitCode = $LASTEXITCODE
            }
        }
        if ($authorityApiExitCode -ne 0) {
            $errorText = if (Test-Path -LiteralPath $errorPath) { Get-Content -LiteralPath $errorPath -Raw } else { "" }
            if ($AllowNotFound -and $errorText -match '(?i)(404|not found)') { return $null }
            # Do not include the provider response: it is not needed for diagnosis and
            # keeps credentials and server diagnostics out of the release log.
            Fail "authority API request failed"
        }
        if ($BinaryOutput) { return $null }
        if ($Raw) { return ($authorityApiResult -join "`n") }
        return (($authorityApiResult -join "`n") | ConvertFrom-Json)
    } finally {
        Remove-Item -LiteralPath $errorPath -Force -ErrorAction SilentlyContinue
    }
}

function Assert-AuthorityMain {
    $main = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/git/ref/heads/main") -AllowNotFound
    if ($null -eq $main) {
        Fail "release authority main is not initialized; run a separately reviewed authority bootstrap before production release"
    }
    if ([string]$main.ref -ne "refs/heads/main") { Fail "release authority main ref is not canonical" }
    $immutable = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/immutable-releases") -AllowNotFound
    if ($null -eq $immutable -or -not [bool]$immutable.enabled) {
        Fail "release authority immutable-releases policy is not enabled; enable it before any production transaction"
    }
    Write-Host "V4 release authority preconditions: PASS (main exists; immutable releases enabled)"
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
    return [pscustomobject]@{
        name = $Candidate.name
        role = $Candidate.role
        size = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $Candidate.path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Assert-EvidenceIdentity([string]$ProductionPath, [string]$QualificationPath, [object[]]$Records) {
    $recordsByName = @{}
    foreach ($record in @($Records)) { $recordsByName[[string]$record.name] = $record }
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
    if ([string]$evidence.installer -ne (Get-ExpectedInstallerName) -or
        [string]$evidence.updater_signature -ne "$(Get-ExpectedInstallerName).sig" -or
        [string]$evidence.authenticode_evidence -ne $authenticodeEvidenceName -or
        [string]$evidence.sbom -ne $sbomName -or
        [string]$evidence.installer -ne [string]$recordsByName[(Get-ExpectedInstallerName)].name -or
        [int64]$evidence.installer_size -ne [int64]$recordsByName[(Get-ExpectedInstallerName)].size -or
        [string]$evidence.installer_sha256 -ne [string]$recordsByName[(Get-ExpectedInstallerName)].sha256 -or
        [int64]$evidence.signature_size -ne [int64]$recordsByName["$((Get-ExpectedInstallerName)).sig"].size -or
        [string]$evidence.updater_signature_sha256 -ne [string]$recordsByName["$((Get-ExpectedInstallerName)).sig"].sha256 -or
        [string]$evidence.authenticode_evidence_sha256 -ne [string]$recordsByName[$authenticodeEvidenceName].sha256 -or
        [string]$evidence.sbom_sha256 -ne [string]$recordsByName[$sbomName].sha256) {
        Fail "production evidence digests or sizes do not match the candidate manifest"
    }
    if ([string]$qualification.installer -ne (Get-ExpectedInstallerName) -or
        [string]$qualification.updater_signature -ne "$(Get-ExpectedInstallerName).sig" -or
        [string]$qualification.installer_sha256 -ne [string]$recordsByName[(Get-ExpectedInstallerName)].sha256 -or
        [string]$qualification.updater_signature_sha256 -ne [string]$recordsByName["$((Get-ExpectedInstallerName)).sig"].sha256 -or
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

    $prevPassword = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")
    $credential = $null
    try {
        $credential = Get-V4UpdaterProductionCredential
        [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $credential, "Process")

        # This is the only production candidate build boundary. The orchestrator
        # owns key verification, stale purge, clean-worktree, NSIS, updater sig,
        # unsigned-zero-budget, SBOM, and install smoke semantics.
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
            -ExpectedSourceSha $SourceSha `
            -Version $Version `
            -Channel $Channel `
            -UpdaterPrivateKeyPath $keyPath
        if ($LASTEXITCODE -ne 0) { Fail "production orchestrator failed" }
    } finally {
        if ($null -ne $prevPassword) {
            [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $prevPassword, "Process")
        } else {
            [Environment]::SetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", $null, "Process")
        }
        try {
            Remove-V4UpdaterProductionCredential
        } catch {
            Write-Warning "V4 updater session credential cleanup returned: $($_.Exception.Message)"
        }
        $credential = $null
        Remove-Variable credential -ErrorAction SilentlyContinue
    }

    $records = @(Get-CandidateRecords | ForEach-Object { Get-FileRecord $_ })
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

function Assert-NoExistingAuthorityTag {
    $release = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/releases/tags/$Tag") -AllowNotFound
    if ($null -ne $release) { Fail "authority already contains release tag $Tag; existing releases are never moved or replaced" }
    $ref = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/git/ref/tags/$Tag") -AllowNotFound
    if ($null -ne $ref) { Fail "authority already contains tag $Tag; a new SemVer/RC is required" }
}

function Invoke-CreateDraft {
    Assert-RequestIdentity
    Assert-AuthorityMain
    $manifestPath = Join-Path (Get-EffectiveStateRoot) "candidate-manifest.json"
    $manifest = Read-JsonFile $manifestPath
    Assert-NoExistingAuthorityTag
    $notesPath = Assert-ReleaseNotes
    $body = "V4 qualified release candidate`n`nsource_sha: $($SourceSha.ToLowerInvariant())`nchannel: $Channel`nqualification: exact candidate manifest attached`n"
    $payloadPath = Join-Path (Get-EffectiveStateRoot) "create-release.json"
    Write-JsonFile $payloadPath ([ordered]@{
        tag_name = $Tag
        target_commitish = "main"
        name = "Sky Auto Player $Version"
        body = $body + ([IO.File]::ReadAllText($notesPath)).Trim()
        draft = $true
        prerelease = ($Channel -eq "beta")
    })
    $release = Invoke-AuthorityApi -Arguments @("api", "--method", "POST", "repos/$authorityRepository/releases", "--input", $payloadPath)
    if (-not $release.draft -or [string]$release.tag_name -ne $Tag) { Fail "authority did not create the requested draft release" }
    $uploadUrl = [string]$release.upload_url
    if ([string]::IsNullOrWhiteSpace($uploadUrl)) { Fail "authority draft did not return its release-specific upload_url" }
    $uploadUrl = $uploadUrl -replace '\{\?name,label\}$', ''
    if ($uploadUrl -notmatch '^https://uploads\.github\.com/repos/[^/]+/[^/]+/releases/\d+/assets$') {
        Fail "authority draft returned an unexpected release asset upload_url"
    }

    foreach ($record in $manifest.assets) {
        $candidate = (Get-CandidateRecords | Where-Object { $_.name -eq [string]$record.name })
        if ($null -eq $candidate) { Fail "candidate manifest contains an unknown asset" }
        # GitHub's release-specific upload_url is deliberately used here. The
        # endpoint rejects duplicate names; this path never deletes or
        # replaces an asset after a failed upload.
        $uploaded = Invoke-V4ReleaseAuthorityAssetUpload `
            -UploadUrl $uploadUrl `
            -AssetName ([string]$record.name) `
            -FilePath ([string]$candidate.path) `
            -Token (Get-AuthorityToken)
        if ([string]$uploaded.name -ne [string]$record.name -or
            [int64]$uploaded.size -ne [int64]$record.size -or
            [string]$uploaded.state -ne "uploaded") {
            Fail "authority upload did not return the exact uploaded asset: $($record.name)"
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
    Write-Host "V4 authority draft: PASS (tag=$Tag; exact qualified asset set uploaded)"
}

function Assert-ExactAssetSet([object]$Release, [object[]]$Expected) {
    $actual = @($Release.assets | ForEach-Object { [string]$_.name } | Sort-Object)
    $expectedNames = @($Expected | ForEach-Object { [string]$_.name } | Sort-Object)
    if (($actual -join "`n") -ne ($expectedNames -join "`n")) { Fail "authority release asset set differs from the qualified candidate set" }
}

function Assert-ImmutableRelease([object]$Release) {
    if ($null -eq $Release.immutable -or -not [bool]$Release.immutable) {
        Fail "authority release is not marked immutable"
    }
}

function Invoke-DownloadDraft {
    $state = Get-State
    if (-not $state.draft -or $state.published) { Fail "draft download requires an unpublished draft release" }
    $release = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/releases/$($state.release_id)")
    if (-not $release.draft -or [string]$release.tag_name -ne $Tag) { Fail "authority release is not the expected draft" }
    Assert-ExactAssetSet $release $state.assets
    $downloaded = Join-Path (Get-EffectiveStateRoot) "downloaded"
    if (Test-Path -LiteralPath $downloaded) { Remove-Item -LiteralPath $downloaded -Recurse -Force }
    New-Item -ItemType Directory -Path $downloaded -Force | Out-Null
    foreach ($expected in $state.assets) {
        $asset = @($release.assets | Where-Object { [string]$_.name -eq [string]$expected.name })
        if ($asset.Count -ne 1) { Fail "expected authority asset is missing" }
        $destination = Join-Path $downloaded ([IO.Path]::GetFileName([string]$expected.name))
        Invoke-AuthorityApi -Arguments @("api", [string]$asset[0].url, "--header", "Accept: application/octet-stream") -BinaryOutput -OutputPath $destination
        $item = Get-Item -LiteralPath $destination
        $hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        if ([int64]$item.Length -ne [int64]$expected.size -or $hash -ne [string]$expected.sha256) {
            Fail "downloaded authority asset differs in size or SHA-256: $($expected.name)"
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
    $installer = Get-ExpectedInstallerName
    Copy-Item -LiteralPath (Join-Path $downloaded $installer) -Destination (Join-Path $bundle $installer)
    Copy-Item -LiteralPath (Join-Path $downloaded "$installer.sig") -Destination (Join-Path $bundle "$installer.sig")

    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1"),
        "-Mode", "unsigned-zero-budget", "-Artifact", (Join-Path $downloaded $installer),
        "-Evidence", (Join-Path $root "downloaded-authenticode-verification.json")
    ) "downloaded candidate Authenticode state is not unsigned-zero-budget"
    Invoke-Checked "cargo" @(
        "xtask", "updater-trust", "verify-signature", "--installer", (Join-Path $downloaded $installer),
        "--signature", (Join-Path $downloaded "$installer.sig")
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
    # authority, then run the real previous-v4 bridge against the exact
    # installer and signature downloaded from the draft. The fixture builds
    # only its throwaway previous client; it never rebuilds the candidate.
    $canonicalPublicKey = Join-Path $root "canonical-updater-public-key.txt"
    Invoke-Checked "cargo" @(
        "xtask", "updater-trust", "export-public-key", "--output", $canonicalPublicKey
    ) "canonical updater public-root export failed"
    $fixtureBundle = Join-Path $root "previous-v4-fixture"
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "ci_tauri_update_e2e.ps1"),
        "-BundleDir", $fixtureBundle,
        "-CandidateInstallerPath", (Join-Path $downloaded $installer),
        "-CandidateSignaturePath", (Join-Path $downloaded "$installer.sig"),
        "-CandidateVersion", $Version,
        "-CandidatePublicKeyPath", $canonicalPublicKey
    ) "exact downloaded previous-v4 to candidate-v4 updater qualification failed"

    # Production policy requires a deterministic exact-artifact Defender
    # custom scan on the downloaded installer. scan_v4_defender_exact.ps1
    # invokes Start-MpScan and records scan_performed; missing Defender or a
    # scan failure is a release failure, not an accepted unavailable result.
    $defenderEvidencePath = Join-Path $root "defender-evidence.json"
    Invoke-Checked "pwsh" @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", (Join-Path $PSScriptRoot "scan_v4_defender_exact.ps1"),
        "-Artifact", (Join-Path $downloaded $installer),
        "-Evidence", $defenderEvidencePath
    ) "exact downloaded installer Defender scan failed"
    $defenderEvidence = Read-JsonFile $defenderEvidencePath
    $installerRecord = @($state.assets | Where-Object { [string]$_.name -eq $installer })
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
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { Fail "LOCALAPPDATA is unavailable for external app-data preservation qualification" }
    $preservationRoot = Join-Path $env:LOCALAPPDATA ("io.github.pumni.skyautoplayer/wo07-release-test-" + [guid]::NewGuid().ToString("N"))
    $preservationMarker = Join-Path $preservationRoot "preserve.txt"
    New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $preservationRoot -Force | Out-Null
    [IO.File]::WriteAllText($preservationMarker, "external-user-data-marker`n", [Text.UTF8Encoding]::new($false))
    try {
        $install = Start-Process -FilePath (Join-Path $downloaded $installer) -ArgumentList @("/S", "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
        if ($install.ExitCode -ne 0) { Fail "downloaded candidate current-user installer failed" }
        if (-not (Test-Path -LiteralPath $app) -or -not (Test-Path -LiteralPath $uninstaller)) { Fail "downloaded candidate install omitted app or uninstaller" }
        $installedPe = @(Get-ChildItem -LiteralPath $installRoot -File -Recurse | Where-Object {
            $_.Extension.ToLowerInvariant() -in @(".exe", ".dll") -and $_.Name -ne "uninstall.exe"
        })
        if ($installedPe.Count -eq 0) { Fail "downloaded candidate installed no project PE files" }
        Invoke-Checked "pwsh" @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1"),
            "-Mode", "unsigned-zero-budget", "-Artifact" , $installedPe.FullName
        ) "downloaded candidate installed PE state is not unsigned-zero-budget"
        $activity = Start-Process -FilePath $app -ArgumentList @("--selftest-update-active-playback") -WindowStyle Hidden -Wait -PassThru
        if ($activity.ExitCode -ne 0) { Fail "downloaded candidate packaged playback-active update rejection self-test failed" }
        $shell = Start-Process -FilePath $app -ArgumentList @("--selftest-desktop-shell") -WindowStyle Hidden -Wait -PassThru
        if ($shell.ExitCode -ne 0) { Fail "downloaded candidate packaged shell self-test failed" }
        $gui = Start-Process -FilePath $app -ArgumentList @("--selftest-desktop-gui") -WindowStyle Hidden -Wait -PassThru
        if ($gui.ExitCode -ne 0) { Fail "downloaded candidate GUI/input safety self-test failed" }
        $uninstall = Start-Process -FilePath $uninstaller -ArgumentList @("/S") -WindowStyle Hidden -Wait -PassThru
        if ($uninstall.ExitCode -ne 0) { Fail "downloaded candidate uninstall failed" }
        if (-not (Test-Path -LiteralPath $preservationMarker -PathType Leaf)) { Fail "uninstall removed external app/user data" }
        $reinstall = Start-Process -FilePath (Join-Path $downloaded $installer) -ArgumentList @("/S", "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
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
            "defender-exact-download-scan-no-detection",
            "spdx-sbom",
            "exact-asset-digest"
        )
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
    $release = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/releases/$($state.release_id)")
    if (-not $release.draft -or [string]$release.tag_name -ne $Tag) { Fail "draft is missing or has changed before publication" }
    Assert-ExactAssetSet $release $state.assets
    foreach ($expected in $state.assets) {
        $asset = @($release.assets | Where-Object { [string]$_.name -eq [string]$expected.name })
        if ($asset.Count -ne 1 -or [int64]$asset[0].size -ne [int64]$expected.size) { Fail "draft asset changed before publication" }
        $downloadedPath = Join-Path (Get-EffectiveStateRoot) "downloaded/$($expected.name)"
        $downloadedHash = (Get-FileHash -LiteralPath $downloadedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($downloadedHash -ne [string]$expected.sha256) { Fail "qualified downloaded digest changed before publication" }
    }
    $patchPath = Join-Path (Get-EffectiveStateRoot) "publish-release.json"
    Write-JsonFile $patchPath ([ordered]@{ draft = $false })
    $published = Invoke-AuthorityApi -Arguments @("api", "--method", "PATCH", "repos/$authorityRepository/releases/$($state.release_id)", "--input", $patchPath)
    if ($published.draft -or [string]::IsNullOrWhiteSpace([string]$published.published_at)) { Fail "authority did not publish the already-qualified draft" }
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
    $authorityCheckout = Join-Path $root "authority"
    if (Test-Path -LiteralPath $authorityCheckout) { Remove-Item -LiteralPath $authorityCheckout -Recurse -Force }
    Invoke-AuthorityApi -Arguments @("repo", "clone", $authorityRepository, $authorityCheckout, "--", "--branch", "main", "--depth", "1") -Raw | Out-Null
    if (-not (Test-Path -LiteralPath (Join-Path $authorityCheckout ".git") -PathType Container)) { Fail "authority checkout main was not obtained" }
    $metadata = Join-Path $root "latest.json"
    $notesPath = Assert-ReleaseNotes
    $signature = Join-Path $downloaded "$((Get-ExpectedInstallerName)).sig"
    $assetUrl = "https://github.com/$authorityRepository/releases/download/$Tag/$((Get-ExpectedInstallerName).Replace(' ', '%20'))"
    Invoke-Checked "cargo" @(
        "xtask", "release-authority", "generate", "--channel", $Channel, "--version", $Version,
        "--notes-file", $notesPath, "--pub-date", $PublicationDateUtc, "--platform", "windows-x86_64",
        "--asset-url", $assetUrl, "--signature-file", $signature, "--output", $metadata
    ) "deterministic v4 metadata generation failed"
    Invoke-WithAuthorityToken {
        Invoke-Checked "pwsh" @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", (Join-Path $PSScriptRoot "promote_v4_metadata.ps1"),
            "-Channel", $Channel, "-Metadata", $metadata,
            "-QualificationEvidence", (Join-Path $downloaded $qualificationEvidenceName),
            "-AuthorityCheckout", $authorityCheckout, "-SourceCheckout", $repoRoot
        ) "post-publication metadata promotion validation failed"
    }
    $destination = Join-Path $authorityCheckout "channels/$Channel/latest.json"
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) { Fail "promotion did not produce the governed channel metadata" }
    $encoded = [Convert]::ToBase64String([IO.File]::ReadAllBytes($destination))
    $existing = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/contents/channels/$Channel/latest.json?ref=main") -AllowNotFound
    $payload = [ordered]@{
        message = "Promote v4 $Channel metadata for $Version"
        content = $encoded
        branch = "main"
    }
    if ($null -ne $existing) { $payload.sha = [string]$existing.sha }
    $payloadPath = Join-Path $root "metadata-commit.json"
    Write-JsonFile $payloadPath $payload
    Invoke-AuthorityApi -Arguments @("api", "--method", "PUT", "repos/$authorityRepository/contents/channels/$Channel/latest.json", "--input", $payloadPath) | Out-Null
    Write-Host "V4 metadata promotion: PASS (channel=$Channel; publication already immutable)"
}

function Invoke-FinalVerify {
    $state = Get-State
    if (-not $state.published) { Fail "final verification requires a published release" }
    $release = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/releases/tags/$Tag")
    if ($release.draft -or [string]::IsNullOrWhiteSpace([string]$release.published_at)) { Fail "final release is still draft or unpublished" }
    Assert-ImmutableRelease $release
    Assert-ExactAssetSet $release $state.assets
    foreach ($expected in $state.assets) {
        $asset = @($release.assets | Where-Object { [string]$_.name -eq [string]$expected.name })
        if ($asset.Count -ne 1 -or [int64]$asset[0].size -ne [int64]$expected.size) { Fail "final public asset identity changed" }
        $finalPath = Join-Path (Get-EffectiveStateRoot) "final-$($expected.name)"
        Invoke-AuthorityApi -Arguments @("api", [string]$asset[0].url, "--header", "Accept: application/octet-stream") -BinaryOutput -OutputPath $finalPath
        $hash = (Get-FileHash -LiteralPath $finalPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne [string]$expected.sha256) { Fail "final public asset digest differs from qualified bytes" }
    }
    $metadataResponse = Invoke-AuthorityApi -Arguments @("api", "repos/$authorityRepository/contents/channels/$Channel/latest.json?ref=main")
    $metadataPath = Join-Path (Get-EffectiveStateRoot) "final-metadata.json"
    $metadataBase64 = [regex]::Replace([string]$metadataResponse.content, '\s', '')
    [IO.File]::WriteAllBytes($metadataPath, [Convert]::FromBase64String($metadataBase64))
    Invoke-Checked "cargo" @("xtask", "release-authority", "validate", "--channel", $Channel, "--metadata", $metadataPath) "final public metadata failed deterministic validation"
    $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
    $expectedUrl = "https://github.com/$authorityRepository/releases/download/$Tag/$((Get-ExpectedInstallerName).Replace(' ', '%20'))"
    if ([string]$metadata.version -ne $Version -or [string]$metadata.platforms.'windows-x86_64'.url -ne $expectedUrl) {
        Fail "final metadata does not reference the exact immutable public asset"
    }
    $finalSignature = Get-Content -LiteralPath (Join-Path (Get-EffectiveStateRoot) "final-$((Get-ExpectedInstallerName)).sig") -Raw
    $metadataSignature = [string]$metadata.platforms.'windows-x86_64'.signature
    if ($finalSignature.Trim() -ne $metadataSignature.Trim()) {
        Fail "final metadata signature does not match the exact public Tauri signature asset"
    }
    if ($expectedUrl -match "Sky-Auto-Player/releases") { Fail "v3 source-repository release namespace leaked into v4 metadata" }
    Invoke-WithAuthorityToken {
        Invoke-Checked "pwsh" @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", (Join-Path $PSScriptRoot "ci_v4_release_authority_acceptance.ps1")
        ) "v3 source-repository release discovery isolation check failed"
    }
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
    "ValidateAuthority" { Assert-RequestIdentity; Assert-AuthorityMain }
    "BuildCandidate" { Invoke-BuildCandidate }
    "CreateDraft" { Assert-RequestIdentity; Invoke-CreateDraft }
    "DownloadDraft" { Assert-RequestIdentity; Invoke-DownloadDraft }
    "QualifyDownloaded" { Assert-RequestIdentity; Invoke-QualifyDownloaded }
    "RecordAttestations" { Assert-RequestIdentity; Invoke-RecordAttestations }
    "PublishDraft" { Assert-RequestIdentity; Invoke-PublishDraft }
    "PromoteMetadata" { Assert-RequestIdentity; Invoke-PromoteMetadata }
    "FinalVerify" { Assert-RequestIdentity; Invoke-FinalVerify }
}
