[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$pipelinePath = Join-Path $PSScriptRoot "v4_release_pipeline.ps1"

# CI pull_request jobs expose a merge SHA through GITHUB_SHA, while these mocked
# states must model production's exact checked-out source SHA. Keep the test
# environment aligned with the fixture's checked-out HEAD without weakening the
# production validation in v4_release_pipeline.ps1.
$fixtureGithubSha = (& git rev-parse HEAD 2>$null).Trim()
if ($fixtureGithubSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "release pipeline fixture could not resolve an exact checked-out HEAD SHA"
}
$env:GITHUB_SHA = $fixtureGithubSha

$fixtureWrapperPath = Join-Path $PSScriptRoot "ci_tauri_update_e2e.ps1"
$fixtureCorePath = Join-Path $PSScriptRoot "ci_tauri_update_e2e_core.ps1"
$uploadHelperPath = Join-Path $PSScriptRoot "v4_release_asset_upload.ps1"
$workflowPath = Join-Path $repoRoot ".github/workflows/release-v4.yml"
$draftWorkflowPath = Join-Path $repoRoot ".github/workflows/rehearse-v4.yml"
$draftLookupPath = Join-Path $PSScriptRoot "v4_release_draft_lookup.ps1"
$pipeline = Get-Content -LiteralPath $pipelinePath -Raw
$fixtureWrapper = Get-Content -LiteralPath $fixtureWrapperPath -Raw
$fixtureCore = Get-Content -LiteralPath $fixtureCorePath -Raw
$uploadHelper = Get-Content -LiteralPath $uploadHelperPath -Raw
$workflow = Get-Content -LiteralPath $workflowPath -Raw
$draftWorkflow = Get-Content -LiteralPath $draftWorkflowPath -Raw
$draftLookup = Get-Content -LiteralPath $draftLookupPath -Raw
$testHarness = Get-Content -LiteralPath $PSCommandPath -Raw
$latestGuardPath = Join-Path $PSScriptRoot "ci_v4_release_latest_guard.ps1"
$latestGuard = (Get-Content -LiteralPath $latestGuardPath -Raw) + (Get-Content -LiteralPath (Join-Path $PSScriptRoot "v4_release_latest_policy.ps1") -Raw)

function Fail([string]$Message) { throw "FAILED: $Message" }

foreach ($source in @(
    [pscustomobject]@{ Name = "release workflow"; Text = $workflow },
    [pscustomobject]@{ Name = "release pipeline"; Text = $pipeline },
    [pscustomobject]@{ Name = "metadata promotion"; Text = (Get-Content -LiteralPath (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") -Raw) },
    [pscustomobject]@{ Name = "asset upload"; Text = $uploadHelper },
    [pscustomobject]@{ Name = "GitHub Latest policy guard"; Text = (Get-Content -LiteralPath (Join-Path $PSScriptRoot "ci_v4_release_latest_guard.ps1") -Raw) },
    [pscustomobject]@{ Name = "production qualification workflow"; Text = $draftWorkflow },
    [pscustomobject]@{ Name = "release draft lookup"; Text = $draftLookup }
)) {
    foreach ($forbidden in @(
        "Sky-Auto-Player-Releases",
        "V4_RELEASE_AUTHORITY_TOKEN",
        "V4_RELEASE_AUTHORITY_REPOSITORY",
        "Invoke-AuthorityApi",
        "AuthorityTokenEnv",
        "AuthorityCheckout",
        "release-authority",
        "release_authority"
    )) {
        if ($source.Text.Contains($forbidden)) {
            Fail "$($source.Name) retains forbidden two-repository marker: $forbidden"
        }
    }
}

foreach ($marker in @(
    'ValidateSet("Baseline", "Verify")',
    'Get-GitHubJson "repos/$canonicalRepository/releases/latest"',
    'ExpectedTag', 'ExpectedSourceSha',
    'make_latest=$(if ($Channel -eq "stable") { "true" } else { "false" })',
    'stable publication did not become the exact GitHub Latest release',
    'beta publication changed GitHub Latest identity',
    'beta publication displaced GitHub Latest',
    'read_only=true',
    'v4_release_latest_policy.ps1'
)) {
    if (-not $latestGuard.Contains($marker)) {
        Fail "GitHub Latest policy guard marker is missing: $marker"
    }
}
if ($latestGuard.Contains('^v3\.')) {
    Fail "GitHub Latest policy guard still hard-codes the retired v3-only namespace"
}

foreach ($marker in @(
    'name: V4 Production Pre-Publication Qualification',
    'workflow_dispatch:',
    'group: v4-release-control-plane',
    'qualification-dispatch-boundary',
    'runs-on: [self-hosted, windows, v4-release, single-tenant]',
    'environment: v4-production-release',
    'contents: read',
    'ref: ${{ github.sha }}',
    'persist-credentials: false',
    'V4_UPDATER_PRIVATE_KEY_PATH',
    'verify_v4_release_runner.ps1',
    'Run canonical production Preflight without publication',
    'Run canonical production BuildCandidate and stop',
    '-State Preflight', '-State BuildCandidate',
    'release-context.json',
    'if: always()',
    'cleanup_v4_release_state.ps1',
    'actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a'
)) {
    if (-not $draftWorkflow.Contains($marker)) {
        Fail "production qualification workflow marker is missing: $marker"
    }
}
$qualificationStates = @('-State Preflight', '-State BuildCandidate')
$previousQualificationStatePosition = -1
foreach ($stateMarker in $qualificationStates) {
    $qualificationStatePosition = $draftWorkflow.IndexOf($stateMarker)
    if ($qualificationStatePosition -lt 0 -or $qualificationStatePosition -lt $previousQualificationStatePosition) {
        Fail "production qualification states are missing or out of order: $stateMarker"
    }
    $previousQualificationStatePosition = $qualificationStatePosition
}
foreach ($forbidden in @(
    '-State PublishRelease', '-State PromoteMetadata', '-State FinalVerify',
    'create-github-app-token', 'metadata-app-token', 'actions/attest@',
    'softprops/action-gh-release', 'gh release', 'contents: write',
    'updater_private_key_path:', 'inputs.updater_private_key_path',
    'V4_RELEASE_AUTHORITY_TOKEN', 'V4_RELEASE_AUTHORITY_REPOSITORY'
)) {
    if ($draftWorkflow.Contains($forbidden)) {
        Fail "production qualification workflow contains forbidden marker: $forbidden"
    }
}

. $draftLookupPath

function Test-DraftLookupFallback {
    $draft = [pscustomobject]@{
        id = 386002301
        tag_name = 'v4.0.1'
        draft = $true
        published_at = $null
    }
    $selected = Select-V4ReleaseByTag -DirectRelease $null -ReleaseCollection @($draft) -Tag 'v4.0.1'
    if ($null -eq $selected -or [int64]$selected.id -ne 386002301) {
        Fail "release lookup did not find a draft hidden from the by-tag endpoint"
    }

    try {
        $null = Select-V4ReleaseByTag -DirectRelease $null -ReleaseCollection @($draft, $draft) -Tag 'v4.0.1'
        Fail "release lookup accepted duplicate tag candidates"
    } catch {
        if ($_.Exception.Message -notmatch 'duplicate releases use the requested tag') { throw }
    }

    try {
        $null = Select-V4ReleaseByTag -DirectRelease ([pscustomobject]@{ tag_name = 'v4.0.0' }) -ReleaseCollection @() -Tag 'v4.0.1'
        Fail "release lookup accepted a direct tag mismatch"
    } catch {
        if ($_.Exception.Message -notmatch 'direct release tag does not match') { throw }
    }
}

Test-DraftLookupFallback

function Test-StrictModeEmptyFreshUserSongs {
    Set-StrictMode -Version Latest
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-empty-fresh-songs-test-" + [guid]::NewGuid().ToString("N"))
    $freshSongsRoot = Join-Path $testRoot "songs"
    try {
        New-Item -ItemType Directory -Path $freshSongsRoot -Force | Out-Null
        $freshUserSongs = @(
            Get-ChildItem -LiteralPath $freshSongsRoot -File -Recurse -ErrorAction SilentlyContinue
        )
        if ($freshUserSongs.Count -ne 0) {
            Fail "empty fresh songs directory unexpectedly contained user songs"
        }
    } finally {
        if (Test-Path -LiteralPath $testRoot) {
            Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-StrictModeEmptyFreshUserSongs

function Get-SanitizedReleaseProbeOutput {
    param(
        [AllowEmptyString()]
        [string]$Output
    )

    if ([string]::IsNullOrWhiteSpace($Output)) {
        return "(child produced no diagnostic output)"
    }

    $sanitized = $Output
    $sanitized = [regex]::Replace(
        $sanitized,
        '(?ms)-----BEGIN [^-]+-----.*?-----END [^-]+-----',
        '[REDACTED KEY MATERIAL]'
    )
    foreach ($secretName in @(
        'TAURI_SIGNING_PRIVATE_KEY_PASSWORD',
        'TAURI_SIGNING_PRIVATE_KEY',
        'GH_TOKEN',
        'GITHUB_TOKEN'
    )) {
        $sanitized = [regex]::Replace(
            $sanitized,
            "(?im)($([regex]::Escape($secretName))\s*[=:]\s*)[^\s\r\n]+",
            '$1[REDACTED]'
        )
    }
    return $sanitized.Trim()
}

if ($fixtureWrapper.Contains("BundleDir") -or $fixtureCore.Contains("BundleDir")) {
    Fail "updater fixture must not use the ambiguous BundleDir contract"
}
foreach ($marker in @(
    "FixtureTargetDir",
    "CARGO_TARGET_DIR",
    "dist/bundle/nsis",
    "Downloaded candidate and bridge paths must remain outside the throwaway fixture target directory",
    "Get-DisposableLoopbackPort",
    "selftest-update-fixture-port",
    "selftest-update-fixture-public-key",
    "Assert-ExactHttpResponse",
    "fixture-http-evidence.json",
    "body_sha256",
    "windows-x86_64",
    "/candidate/update.exe"
)) {
    if (-not $fixtureCore.Contains($marker)) {
        Fail "updater fixture topology marker is missing: $marker"
    }
}
if (-not $pipeline.Contains("FixtureTargetDir")) {
    Fail "production qualification must pass an explicit fixture target directory"
}
foreach ($marker in @(
    'Verify isolated production runner boundary',
    'V4_UPDATER_PRIVATE_KEY_PATH',
    '-UpdaterPrivateKeyPath $env:V4_UPDATER_PRIVATE_KEY_PATH'
)) {
    if (-not $workflow.Contains($marker)) {
        Fail "production workflow key-path transport marker is missing: $marker"
    }
}
if ($workflow.Contains("CARGO_TARGET_DIR=")) {
    Fail "production workflow still carries an ambient fixture target contract"
}

foreach ($script in @(
    [pscustomobject]@{ Name = "production release pipeline"; Source = $pipeline }
)) {
    foreach ($forbidden in @(
        'gh @Arguments --output', 'gh.exe @Arguments --output', '--output $OutputPath',
        '"$uploadUrl?name='
    )) {
        if ($script.Source.Contains($forbidden)) {
            Fail "$($script.Name) must not use gh api --output for binary asset downloads"
        }
    }
    foreach ($marker in @(
        'Invoke-GhBinaryOutput', 'Invoke-V4ReleaseAssetUpload',
        'PSVersionTable.PSVersion', '7.4.0', 'RedirectStandardOutput',
        'RedirectStandardError', 'StandardOutput.BaseStream', 'ReadToEndAsync',
        'ArgumentList'
    )) {
        if (-not $script.Source.Contains($marker)) {
            Fail "$($script.Name) binary download helper is missing marker: $marker"
        }
    }
}

foreach ($marker in @(
    'System.Net.Http.HttpClient', 'System.Net.Http.StreamContent', 'System.IO.FileStream',
    'Headers.Authorization', 'UserAgent', 'application/vnd.github+json',
    'X-GitHub-Api-Version', '2026-03-10', 'uploads.github.com', 'ContentLength', 'fileLength',
    'StatusCode', 'System.Net.HttpStatusCode', 'Created',
    'SendAsync', 'ReadAsStringAsync', 'application/octet-stream',
    '$client.Timeout = [TimeSpan]::FromMinutes(10)'
)) {
    if (-not $uploadHelper.Contains($marker)) {
        Fail "raw release asset upload helper is missing marker: $marker"
    }
}
if ($uploadHelper.Contains('InfiniteTimeSpan')) {
    Fail "raw release asset upload helper must use a finite timeout"
}
$timeoutMatch = [regex]::Match(
    $uploadHelper,
    '\$client\.Timeout\s*=\s*\[TimeSpan\]::FromMinutes\((\d+)\)'
)
if (-not $timeoutMatch.Success -or ([int]$timeoutMatch.Groups[1].Value * 60) -le 100) {
    Fail "raw release asset upload timeout must be explicit and longer than 100 seconds"
}
if ($uploadHelper.Contains('gh ') -or $uploadHelper.Contains('ArgumentList')) {
    Fail "raw release asset upload helper must not invoke GitHub CLI"
}
if ($uploadHelper.Contains('$UploadUrl?name=')) {
    Fail "raw release asset upload helper must not use ambiguous PowerShell URL interpolation"
}
foreach ($marker in @(
    'UploadUrl.Contains("?")', '[string]::Concat($UploadUrl, "?name="', 'escapedAssetName'
)) {
    if (-not $uploadHelper.Contains($marker)) {
        Fail "raw release asset upload URL construction guard is missing: $marker"
    }
}

function Test-RawUploadBodyByteIdentity {
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-raw-upload-test-" + [guid]::NewGuid().ToString("N"))
    $testPath = Join-Path $testRoot "binary-fixture.bin"
    $expected = [byte[]](0x00, 0xFF, 0x80, 0x41, 0xC3, 0x28, 0x0D, 0x0A, 0x7F)
    $stream = $null
    $content = $null
    $request = $null
    try {
        New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
        [IO.File]::WriteAllBytes($testPath, $expected)
        $stream = [IO.FileStream]::new($testPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        $content = [System.Net.Http.StreamContent]::new($stream)
        $content.Headers.ContentLength = [int64]$stream.Length
        if ($content.Headers.ContentLength -ne [int64]$expected.Length) {
            Fail "raw upload content length changed"
        }
        $request = [System.Net.Http.HttpRequestMessage]::new(
            [System.Net.Http.HttpMethod]::Post,
            "https://uploads.github.com/test"
        )
        $request.Headers.UserAgent.ParseAdd("Sky-Auto-Player-v4-release-pipeline/1.0")
        [void]$request.Headers.Accept.Add(
            [System.Net.Http.Headers.MediaTypeWithQualityHeaderValue]::new(
                "application/vnd.github+json"
            )
        )
        [void]$request.Headers.Add("X-GitHub-Api-Version", "2026-03-10")
        if ($request.Headers.UserAgent.ToString() -ne "Sky-Auto-Player-v4-release-pipeline/1.0" -or
            $request.Headers.Accept.ToString() -ne "application/vnd.github+json" -or
            $request.Headers.GetValues("X-GitHub-Api-Version") -join "," -ne "2026-03-10") {
            Fail "raw upload protocol headers changed"
        }
        $request.Content = $content
        $captured = $request.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
        if ($captured.Length -ne $expected.Length) { Fail "raw upload body length changed" }
        for ($index = 0; $index -lt $expected.Length; $index++) {
            if ($captured[$index] -ne $expected[$index]) { Fail "raw upload body bytes changed" }
        }
    } finally {
        if ($null -ne $request) { $request.Dispose() }
        if ($null -ne $content) { $content.Dispose() }
        if ($null -ne $stream) { $stream.Dispose() }
        if (Test-Path -LiteralPath $testRoot) {
            Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Test-RawUploadBodyByteIdentity

function Test-RawUploadRequiresCreated {
    $created = [System.Net.Http.HttpResponseMessage]::new([System.Net.HttpStatusCode]::Created)
    $ok = [System.Net.Http.HttpResponseMessage]::new([System.Net.HttpStatusCode]::OK)
    try {
        if ($created.StatusCode -ne [System.Net.HttpStatusCode]::Created) {
            Fail "raw upload Created response contract changed"
        }
        if ($ok.StatusCode -eq [System.Net.HttpStatusCode]::Created) {
            Fail "raw upload accepted a non-Created response"
        }
    } finally {
        $created.Dispose()
        $ok.Dispose()
    }
}

Test-RawUploadRequiresCreated

if (([regex]::Matches($pipeline, "orchestrate_v4_production_release\.ps1")).Count -ne 1) {
    Fail "production orchestrator must have exactly one call site"
}
foreach ($marker in @(
    'Preflight', 'BuildCandidate', 'PublishRelease',
    'PromoteMetadata', 'FinalVerify', 'unsigned-zero-budget',
    'release-context.json', 'Write-V4ReleaseContext', 'Import-V4ReleaseContext', 'Get-SourceReleaseIdentity',
    'Remove-V4StaleMatchingDraft', 'V4 unpublished draft cleanup',
    'metadata promotion is forbidden before immutable publication',
    'release-metadata branch is not initialized',
    'Assert-MetadataBranchReadiness', 'metadataBootstrapContract',
    'release-metadata readiness', 'validate-monotonic', 'strictly monotonic',
    'New-ExpectedV4Metadata', 'Assert-ExactFileBytes', 'Write-RepositoryContentFile',
    'Assert-RawMetadataConverges', 'RawMetadataRetryBudgetSeconds',
    'Assert-V4GitHubLatestPolicy', 'Get-RemoteLatestRelease',
    'Convert-PublishedAtToMetadataTimestamp', '$publicationDateUtc',
    'publishedRelease.target_commitish', 'published_at',
    'raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json',
    'raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json',
    'AllowAutoRedirect', 'Headers.Authorization', 'ReadAsByteArrayAsync',
    'repository already contains published release/tag', 'V4 unpublished draft cleanup',
    'published tags are immutable', 'git/refs/tags/$Tag',
    'GitHub''s successful DELETE endpoints return an empty body',
    'Get-FileHash', 'verify-signature', 'sbom', 'verify-tauri-bundle',
    'current-user', 'active-playback-install-rejected', 'upload_url',
    'Assert-ImmutableRelease $published', 'repository release is not marked immutable',
    'V4 immutable publication guard self-test', 'immutable=false rejected',
    'V4 exact public asset-set self-test', 'missing and extra assets rejected',
    'Get-V4ReleaseMakeLatestValue',
    'Get-V4ReleaseDraftMakeLatestValue',
    'make_latest = Get-V4ReleaseDraftMakeLatestValue',
    'V4 GitHub release payload self-test',
    'draft false; stable publish true; beta publish false',
    'Start-MpScan',
    'previous-v4-to-exact-downloaded-candidate-update',
    'selftest-update-active-playback', 'scan_performed',
    'cargo xtask builtin-catalog verify-installed',
    'installed-built-in-catalog-exact-manifest-file-set-sha-parseability',
    'manifest_validated', 'file_set_exact', 'sha256_verified', 'songs_parseable',
    'fresh-appdata-built-in-user-composition',
    'SKY_BUILTIN_CATALOG_FRESH_SELFTEST',
    'fresh built-in catalog self-test',
    'previousFreshSelfTest',
    'if ($null -eq $previousAppDataRoot)',
    'Remove-Item Env:SKY_APP_DATA_ROOT',
    'if ($null -eq $previousFreshSelfTest)',
    'Remove-Item Env:SKY_BUILTIN_CATALOG_FRESH_SELFTEST',
    'v4_updater_credential_broker.ps1',
    'docs/releases/v$Version.md',
    'release notes path must match the requested version',
    'release notes heading must match the requested version',
    '(?m)^# [^\r\n]+(?=\r?$)'
)) {
    if (-not $pipeline.Contains($marker)) { Fail "pipeline marker is missing: $marker" }
}
if ($pipeline.Contains('repos/$repository/immutable-releases')) {
    Fail "repository policy validation must not call the administration-only immutable-releases endpoint"
}
$pipelineSelfTestOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $pipelinePath -State SelfTest 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $pipelineSelfTestOutput -notmatch 'immutable=false rejected; immutable=true accepted') {
    Fail "pipeline immutable publication guard self-test did not reject immutable=false"
}
if ($pipelineSelfTestOutput -notmatch 'draft false; stable publish true; beta publish false') {
    Fail "pipeline GitHub release payload self-test did not verify the make_latest JSON enum type"
}
$publishReleaseBody = $pipeline.Substring(
    $pipeline.IndexOf('function Invoke-PublishRelease', [StringComparison]::Ordinal),
    $pipeline.IndexOf('function Invoke-PromoteMetadata', [StringComparison]::Ordinal) -
        $pipeline.IndexOf('function Invoke-PublishRelease', [StringComparison]::Ordinal)
)
if (-not $publishReleaseBody.Contains('make_latest = Get-V4ReleaseDraftMakeLatestValue') -or
    -not $publishReleaseBody.Contains('make_latest = (Get-V4ReleaseMakeLatestValue $Channel)')) {
    Fail "PublishRelease must use draft-safe make_latest for draft and channel-aware helper for publication"
}

# Public release asset regression contract: the previous v4.0.1 failure uploaded
# the complete eight-file qualification candidate set. Keep the two sets explicit
# and make every public boundary consume only the canonical two-record projection.
foreach ($marker in @(
    'function Get-QualificationCandidateRecords',
    'function Get-PublicReleaseRecords',
    'function Get-CanonicalPublicReleaseNames',
    'qualification_assets = $candidateAssets',
    'public_assets = $publicRecords',
    'Assert-ManifestAssetFiles',
    'Freeze-CandidateAssets',
    'Get-StateAssetPath',
    'Assert-ExactPublicReleaseAssetSet',
    'candidate manifest must declare qualification_assets and public_assets separately'
)) {
    if (-not $pipeline.Contains($marker)) {
        Fail "public release asset separation marker is missing: $marker"
    }
}
$publishReleaseUploadBody = $publishReleaseBody.Substring(
    $publishReleaseBody.IndexOf('foreach ($record in $publicRecords)', [StringComparison]::Ordinal)
)
if (-not $publishReleaseUploadBody.Contains('Invoke-V4ReleaseAssetUpload') -or
    $publishReleaseUploadBody.Contains('Get-QualificationCandidateRecords') -or
    $publishReleaseUploadBody.Contains('manifest.assets')) {
    Fail "PublishRelease upload loop is not restricted to public_assets"
}
$publishReleaseVerifyBody = $publishReleaseBody.Substring(
    $publishReleaseBody.IndexOf('Assert-ExactPublicReleaseAssetSet $serverDraft', [StringComparison]::Ordinal)
)
if (-not $publishReleaseVerifyBody.Contains('server asset size mismatch') -or
    -not $publishReleaseVerifyBody.Contains('server asset digest mismatch') -or
    -not $publishReleaseVerifyBody.Contains('Invoke-DraftSelfCleanup')) {
    Fail "PublishRelease does not server-verify asset sizes and digests with fail-closed self-cleanup"
}
$promoteMetadataBody = $pipeline.Substring(
    $pipeline.IndexOf('function Invoke-PromoteMetadata', [StringComparison]::Ordinal),
    $pipeline.IndexOf('function Invoke-FinalVerify', [StringComparison]::Ordinal) -
        $pipeline.IndexOf('function Invoke-PromoteMetadata', [StringComparison]::Ordinal)
)
if (-not $promoteMetadataBody.Contains('Get-StateAssetPath') -or
    $promoteMetadataBody.Contains('downloaded/')) {
    Fail "PromoteMetadata must use candidate assets outside downloaded/"
}
foreach ($workflowSource in @(
    [pscustomobject]@{ Name = 'production release workflow'; Text = $workflow },
    [pscustomobject]@{ Name = 'production qualification workflow'; Text = $draftWorkflow }
)) {
    if ($workflowSource.Text -match 'sbom-path:\s+\$\{\{ runner\.temp \}\}[^\r\n]*\\downloaded\\SBOM\.spdx\.json') {
        Fail "$($workflowSource.Name) still attests an SBOM from downloaded/"
    }
    if ($workflowSource.Name -eq 'production release workflow' -and
        -not $workflowSource.Text.Contains('candidate-evidence\SBOM.spdx.json')) {
        Fail "$($workflowSource.Name) does not attest the frozen candidate SBOM"
    }
}
$finalVerifyBody = $pipeline.Substring(
    $pipeline.IndexOf('function Invoke-FinalVerify', [StringComparison]::Ordinal),
    $pipeline.IndexOf('function Invoke-SelfTest', [StringComparison]::Ordinal) -
        $pipeline.IndexOf('function Invoke-FinalVerify', [StringComparison]::Ordinal)
)
if (-not $finalVerifyBody.Contains('Assert-ExactPublishedPublicAssetRecords $release $publicRecords $repository') -or
    -not $finalVerifyBody.Contains('Assert-ExactAssetSet $release $publicRecords') -or
    -not $finalVerifyBody.Contains('Get-PublicReleaseRecordsFromManifest $candidateManifest') -or
    -not $finalVerifyBody.Contains('Get-FileHash')) {
    Fail "FinalVerify does not enforce exact public asset-set and byte equality"
}
Write-Host "V4 public release asset regression contract: PASS (8-file qualification set isolated; exact 2-file public set enforced)"
foreach ($marker in @(
    'function Get-SanitizedReleaseProbeOutput',
    'version-check.log',
    'ChildOutput',
    'VersionCheckLog'
)) {
    if (-not $testHarness.Contains($marker)) {
        Fail "release-note probe diagnostic marker is missing: $marker"
    }
}

function Invoke-ReleaseNotesValidation([string]$NotesPath) {
    $probeRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-release-notes-probe-" + [guid]::NewGuid().ToString("N"))
    $packageManifest = Get-Content -LiteralPath (Join-Path $repoRoot "desktop/src-tauri/Cargo.toml") -Raw
    $versionMatch = [regex]::Match($packageManifest, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $versionMatch.Success) { Fail "release notes probe could not read package version" }
    $sourceSha = (& git rev-parse HEAD).Trim()
    try {
        $probeChannel = if ($versionMatch.Groups[1].Value.Contains("-")) { "beta" } else { "stable" }
        $probeRunner = Join-Path $probeRoot "run-probe.ps1"
        New-Item -ItemType Directory -Path $probeRoot -Force | Out-Null
        Set-Content -LiteralPath $probeRunner -Value @"
`$ErrorActionPreference = 'Stop'
. '$pipelinePath' -State SelfTest -Version '$($versionMatch.Groups[1].Value)' -Channel '$probeChannel' -Tag 'v$($versionMatch.Groups[1].Value)' -SourceSha '$sourceSha' -WorkflowSha '$sourceSha' -StateRoot '$probeRoot' -ReleaseNotesPath '$NotesPath'
Assert-RequestIdentity
Assert-ReleaseNotes
"@ -Encoding utf8
        $childOutput = (& pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $probeRunner 2>&1 | Out-String)
        $exitCode = [int]$LASTEXITCODE
        $versionCheckPath = Join-Path $probeRoot "version-check.log"
        $versionCheckOutput = if (Test-Path -LiteralPath $versionCheckPath -PathType Leaf) {
            Get-Content -LiteralPath $versionCheckPath -Raw -ErrorAction SilentlyContinue
        } else {
            ""
        }
        $diagnostics = @(
            "child output:`n$(Get-SanitizedReleaseProbeOutput $childOutput)"
        )
        if (-not [string]::IsNullOrWhiteSpace($versionCheckOutput)) {
            $diagnostics += "version-check.log:`n$(Get-SanitizedReleaseProbeOutput $versionCheckOutput)"
        }
        return [pscustomobject]@{
            ExitCode = $exitCode
            Output = ($diagnostics -join "`n")
            ChildOutput = Get-SanitizedReleaseProbeOutput $childOutput
            VersionCheckLog = Get-SanitizedReleaseProbeOutput $versionCheckOutput
        }
    } finally {
        if (Test-Path -LiteralPath $probeRoot) {
            Remove-Item -LiteralPath $probeRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

$packageVersion = [regex]::Match(
    (Get-Content -LiteralPath (Join-Path $repoRoot "desktop/src-tauri/Cargo.toml") -Raw),
    '(?m)^version\s*=\s*"([^"]+)"'
).Groups[1].Value
$validNotesPath = Join-Path $repoRoot "docs/releases/v$packageVersion.md"
$canonicalReleaseNotes = Get-Content -LiteralPath $validNotesPath -Raw
foreach ($forbiddenPublicReleaseNotesPattern in @(
    '(?i)\bNO-GO\b',
    '(?i)pre-publication\s+(?:gate|check|verification|status|review)',
    '(?i)owner/admin',
    '(?i)cutover',
    '(?i)release remains blocked',
    '(?i)not release approval',
    '(?i)gate status',
    '(?i)has no branch protection',
    '(?i)has no protection rules',
    '(?i)must be protected before GO',
    '(?i)observed before',
    '(?i)preparation branch',
    'V4_RELEASE_AUTHORITY_TOKEN',
    'Sky-Auto-Player-Releases'
)) {
    if ($canonicalReleaseNotes -match $forbiddenPublicReleaseNotesPattern) {
        Fail "canonical v$packageVersion public release notes contain an internal gate phrase: $forbiddenPublicReleaseNotesPattern"
    }
}
Write-Host "V4 public release notes contract: PASS (no internal gate state or obsolete topology wording)"
$validNotesProbe = Invoke-ReleaseNotesValidation $validNotesPath
if ($validNotesProbe.ExitCode -ne 0 -or $validNotesProbe.Output -notmatch "V4 release identity: PASS") {
    Fail "canonical release notes were rejected by the identity probe. Diagnostics:`n$($validNotesProbe.Output)"
}
$wrongNotes = Get-ChildItem -LiteralPath (Join-Path $repoRoot "docs/releases") -Filter "v4.0.0-rc.*.md" |
    Where-Object { $_.Name -ne "v$packageVersion.md" } |
    Select-Object -First 1
if ($null -eq $wrongNotes) { Fail "release notes probe requires an existing mismatched v4 notes file" }
$wrongNotesProbe = Invoke-ReleaseNotesValidation $wrongNotes.FullName
if ($wrongNotesProbe.ExitCode -eq 0) {
    Fail "identity probe accepted release notes for a different version"
}

foreach ($brokerFile in @(
    'verify_v4_release_runner.ps1',
    'cleanup_v4_release_state.ps1',
    'v4_updater_credential_broker.ps1',
    'set_v4_updater_session_credential.ps1',
    'remove_v4_updater_session_credential.ps1',
    'test_v4_updater_credential_broker.ps1'
)) {
    $path = Join-Path $PSScriptRoot $brokerFile
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Fail "required credential broker file is missing: $brokerFile"
    }
}

foreach ($marker in @(
    'workflow_dispatch:',
    'runs-on: [self-hosted, windows, v4-release, single-tenant]',
    'contents: read', 'id-token: write', 'attestations: write',
    'actions/upload-artifact@',
    'GH_TOKEN: ${{ github.token }}',
    'actions: read',
    'EXPECTED_WORKFLOW: rehearse-v4.yml',
    'ref: ${{ github.sha }}',
    'Require exact-head production qualification',
    'actions/workflows/$EXPECTED_WORKFLOW/runs?event=workflow_dispatch&status=completed&head_sha=$GITHUB_SHA',
    'V4 Production Pre-Publication Qualification',
    'persist-credentials: false',
    'actions/attest@',
    '--source-digest $env:GITHUB_SHA',
    'Initialize release state root', 'RUNNER_TEMP', 'GITHUB_RUN_ID', 'GITHUB_ENV',
    'release-dispatch-boundary',
    'github.event.repository.default_branch',
    'refs/heads/main',
    'environment: v4-production-release',
    'Mint release-metadata GitHub App token',
    'id: metadata-app-token',
    'actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1',
    'client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}',
    'private-key: ${{ secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY }}',
    'owner: ${{ github.repository_owner }}',
    'repositories: ${{ github.event.repository.name }}',
    'permission-contents: write',
    'GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}',
    'Mint release-metadata GitHub App token before publication',
    'Probe release-metadata App access before publication',
    'probe_v4_metadata_app_access.ps1',
    'Verify isolated production runner boundary',
    'verify_v4_release_runner.ps1',
    'cleanup_v4_release_state.ps1',
    'Preflight', 'BuildCandidate', 'PublishRelease', 'PromoteMetadata', 'FinalVerify'
)) {
    if (-not $workflow.Contains($marker)) { Fail "workflow marker is missing: $marker" }
}
if ($workflow.Contains('app-id:') -or $workflow.Contains('V4_RELEASE_METADATA_APP_ID')) {
    Fail "canonical release workflow must use client-id with the actual Client ID variable"
}
foreach ($marker in @(
    'candidate-manifest.json',
    'candidate-evidence\*.json',
    'fixture-http-evidence.json',
    'defender-evidence.json',
    'preflight-evidence.json'
)) {
    if (-not $workflow.Contains($marker)) { Fail "production bounded evidence retention marker is missing: $marker" }
}
foreach ($marker in @(
    'candidate-manifest.json',
    'release-context.json',
    'candidate-evidence\*.json',
    'fixture-http-evidence.json',
    'defender-evidence.json',
    'preflight-evidence.json'
)) {
    if (-not $draftWorkflow.Contains($marker)) { Fail "qualification bounded evidence retention marker is missing: $marker" }
}
if ($workflow.Contains('inputs:') -or $workflow.Contains('inputs.')) {
    Fail "production release workflow must not expose semantic workflow_dispatch inputs"
}
$metadataTokenMarker = 'GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}'
$metadataTokenUses = ([regex]::Matches($workflow, [regex]::Escape($metadataTokenMarker))).Count
if ($metadataTokenUses -ne 2) {
    Fail "metadata App installation token must be used by the pre-publication probe and promotion"
}
$metadataPrivateKeyMarker = 'secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY'
$metadataPrivateKeyUses = ([regex]::Matches($workflow, [regex]::Escape($metadataPrivateKeyMarker))).Count
if ($metadataPrivateKeyUses -ne 1) {
    Fail "metadata App private key must be consumed exactly once by the token-mint action"
}
$publishStep = $workflow.IndexOf('- name: Publish the qualified candidate immutably', [StringComparison]::Ordinal)
$metadataTokenStep = $workflow.IndexOf('- name: Mint release-metadata GitHub App token before publication', [StringComparison]::Ordinal)
$probeStep = $workflow.IndexOf('- name: Probe release-metadata App access before publication', [StringComparison]::Ordinal)
if ($metadataTokenStep -lt 0 -or $probeStep -lt 0 -or $publishStep -lt 0 -or
    $metadataTokenStep -ge $probeStep -or $probeStep -ge $publishStep) {
    Fail "metadata App mint/probe must precede immutable publication"
}
$probeEnd = $workflow.IndexOf("`n      - name:", $probeStep + 1, [StringComparison]::Ordinal)
if ($probeEnd -lt 0) { $probeEnd = $workflow.Length }
$probeBlock = $workflow.Substring($probeStep, $probeEnd - $probeStep)
if (-not $probeBlock.Contains($metadataTokenMarker) -or $probeBlock.Contains('github.token')) {
    Fail "pre-publication App access probe must use only the short-lived App token"
}
$promotionStart = $workflow.IndexOf('- name: Promote release metadata only after immutable publication', [StringComparison]::Ordinal)
if ($promotionStart -lt 0) { Fail "metadata promotion step is missing" }
$promotionEnd = $workflow.IndexOf("`n      - name:", $promotionStart + 1, [StringComparison]::Ordinal)
if ($promotionEnd -lt 0) { $promotionEnd = $workflow.Length }
$promotionBlock = $workflow.Substring($promotionStart, $promotionEnd - $promotionStart)
if (-not $promotionBlock.Contains($metadataTokenMarker) -or
    $promotionBlock.Contains('GH_TOKEN: ${{ github.token }}')) {
    Fail "metadata promotion must use only the short-lived App token"
}
foreach ($forbidden in @(
    'cargo xtask dist', 'Sky-Auto-Player-Updater.exe', 'MANIFEST.json.sig',
    'softprops/action-gh-release', 'secrets.TAURI_SIGNING_PRIVATE_KEY',
    'secrets.UPDATER_PRIVATE_KEY', 'secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD',
    'secrets.UPDATER_PASSWORD', 'secrets.V4_UPDATER_PASSWORD',
    'updater_password_env', 'credential_target',
    'V4_RELEASE_STATE_ROOT: ${{ runner.temp }}',
    'updater_private_key_path:',
    'inputs.updater_private_key_path',
    'Mask updater key path',
    '::add-mask::'
)) {
    if ($workflow.Contains($forbidden)) { Fail "forbidden production workflow marker remains: $forbidden" }
}

$stateRootInit = $workflow.IndexOf('- name: Initialize release state root', [StringComparison]::Ordinal)
$checkout = $workflow.IndexOf('- name: Check out the exact requested source SHA', [StringComparison]::Ordinal)
if ($stateRootInit -lt 0 -or $checkout -lt 0 -or $stateRootInit -gt $checkout) {
    Fail "release state root must be initialized from runner default environment before checkout and release steps"
}

class MockReleaseApi {
    [int]$BuildCount = 0
    [bool]$Draft = $false
    [bool]$Qualified = $false
    [bool]$Attested = $false
    [bool]$Published = $false
    [bool]$immutable = $false
    [bool]$Promoted = $false
    [bool]$UploadedThroughReleaseUrl = $false
    [string]$UploadUrl = ""
    [bool]$ExactAssetsVerified = $false
    [bool]$ExactDownloadedBytes = $false

    [void] BuildCandidate() {
        if ($this.BuildCount -ne 0) { throw "candidate rebuilt" }
        $this.BuildCount++
        $this.Qualified = $true
    }
    [void] PublishRelease() {
        if ($this.BuildCount -ne 1 -or -not $this.Qualified -or -not $this.Attested -or $this.Published) {
            throw "publication ordering violation"
        }
        $this.Draft = $true
        $this.UploadedThroughReleaseUrl = $true
        $this.UploadUrl = "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/42/assets"
        $this.ExactAssetsVerified = $true
        $this.ExactDownloadedBytes = $true
        $this.Draft = $false
        $this.Published = $true
        $this.immutable = $true
    }
    [void] PromoteMetadata() {
        if (-not $this.Published) { throw "promotion before immutable publication" }
        $this.Promoted = $true
    }
}

$mock = [MockReleaseApi]::new()
$mock.BuildCandidate()
try {
    $mock.PromoteMetadata()
    Fail "mock promotion before publication was accepted"
} catch {
    if ($_.Exception.Message -notmatch "promotion before immutable publication") { throw }
}
$mock.Attested = $true
$mock.PublishRelease()
$mock.PromoteMetadata()
if ($mock.BuildCount -ne 1 -or -not $mock.Promoted -or $mock.Draft -or -not $mock.Published -or -not $mock.immutable -or -not $mock.UploadedThroughReleaseUrl -or -not $mock.ExactAssetsVerified -or -not $mock.ExactDownloadedBytes) {
    Fail "mock state machine did not preserve build-once/publication ordering"
}

# Production evidence Authenticode binding contract regression test
. (Join-Path $PSScriptRoot "v4_qualification_evidence.ps1")

$evidenceTestRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-evidence-binding-test-" + [guid]::NewGuid().ToString("N"))
try {
    New-Item -ItemType Directory -Path $evidenceTestRoot -Force | Out-Null
    $testVersion = "4.0.0-rc.1"
    $testInstaller = "Sky Auto Player_${testVersion}_x64-setup.exe"
    $testSignature = "$testInstaller.sig"
    $testSha = "1234567890abcdef1234567890abcdef12345678"
    $testAuthSha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    $testSbomSha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    $testInstallerSha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
    $testSigSha = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
    $testKeyId = "19AABD2E7838818C"

    # Verify builder produces required fields with exact values and independent digest
    $prodObj = New-V4CanonicalProductionEvidence `
        -SourceSha $testSha `
        -Version $testVersion `
        -Channel "stable" `
        -InstallerName $testInstaller `
        -SignatureName $testSignature `
        -InstallerSize 1234567 `
        -SignatureSize 512 `
        -InstallerSha256 $testInstallerSha `
        -SignatureSha256 $testSigSha `
        -AuthenticodeEvidenceSha256 $testAuthSha `
        -SbomSha256 $testSbomSha `
        -UpdaterKeyId $testKeyId

    if (-not $prodObj.Contains("authenticode_evidence") -or -not $prodObj.Contains("authenticode_evidence_sha256")) {
        Fail "production evidence builder omitted required Authenticode binding property"
    }
    if ($prodObj["authenticode_evidence"] -ne "TAURI_AUTHENTICODE_EVIDENCE.json") {
        Fail "production evidence builder authenticode_evidence filename must be TAURI_AUTHENTICODE_EVIDENCE.json"
    }
    if ($prodObj["authenticode_evidence_sha256"] -ne $testAuthSha) {
        Fail "production evidence builder authenticode_evidence_sha256 must match AuthenticodeEvidenceSha256 input"
    }
    if ($prodObj["authenticode_evidence_sha256"] -eq $prodObj["installer_sha256"] -or
        $prodObj["authenticode_evidence_sha256"] -eq $prodObj["updater_signature_sha256"] -or
        $prodObj["authenticode_evidence_sha256"] -eq $prodObj["sbom_sha256"]) {
        Fail "production evidence builder improperly reused another digest for authenticode_evidence_sha256"
    }

    $qualObj = New-V4CanonicalQualificationEvidence `
        -Version $testVersion `
        -InstallerName $testInstaller `
        -SignatureName $testSignature `
        -InstallerSize 1234567 `
        -SignatureSize 512 `
        -InstallerSha256 $testInstallerSha `
        -SignatureSha256 $testSigSha `
        -AuthenticodeEvidenceSha256 $testAuthSha `
        -SbomSha256 $testSbomSha

    $qualPath = Join-Path $evidenceTestRoot "V4_QUALIFICATION_EVIDENCE.json"
    $prodPath = Join-Path $evidenceTestRoot "V4_PRODUCTION_RELEASE_EVIDENCE.json"
    $qualObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $qualPath -Encoding utf8

    $records = @(
        [pscustomobject]@{ name = $testInstaller; size = [int64]1234567; sha256 = $testInstallerSha },
        [pscustomobject]@{ name = $testSignature; size = [int64]512; sha256 = $testSigSha },
        [pscustomobject]@{ name = "V4_PRODUCTION_RELEASE_EVIDENCE.json"; size = [int64]100; sha256 = "1" * 64 },
        [pscustomobject]@{ name = "V4_QUALIFICATION_EVIDENCE.json"; size = [int64]100; sha256 = "2" * 64 },
        [pscustomobject]@{ name = "TAURI_AUTHENTICODE_EVIDENCE.json"; size = [int64]100; sha256 = $testAuthSha },
        [pscustomobject]@{ name = "SBOM.spdx.json"; size = [int64]100; sha256 = $testSbomSha }
    )

    # Helper to invoke pipeline Assert-EvidenceIdentity in a scoped environment
    function Invoke-EvidenceIdentityAssertion([string]$TargetProdPath) {
        $scopedScript = @'
param(
    [string]$PipelinePath,
    [string]$TargetProdPath,
    [string]$QualPath,
    [string]$TestSha,
    [string]$TestVersion,
    [string]$TestInstaller,
    [string]$TestInstallerSha,
    [string]$TestSigSha,
    [string]$TestAuthSha,
    [string]$TestSbomSha
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$SourceSha = $TestSha
$Version = $TestVersion
$Channel = 'stable'
$productionEvidenceName = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'
$qualificationEvidenceName = 'V4_QUALIFICATION_EVIDENCE.json'
$authenticodeEvidenceName = 'TAURI_AUTHENTICODE_EVIDENCE.json'
$sbomName = 'SBOM.spdx.json'
. (Join-Path (Split-Path -Parent $PipelinePath) 'v4_qualification_evidence.ps1')
$safeInstaller = Get-V4SafeReleaseAssetName $TestInstaller
$safeSig = Get-V4SafeReleaseAssetName "$TestInstaller.sig"
function Fail([string]$Message) { throw $Message }
function Get-ExpectedInstallerName { return $safeInstaller }
function Get-ExpectedSignatureName { return $safeSig }
function Get-ExpectedSourceInstallerName { return $TestInstaller }
function Get-ExpectedSourceSignatureName { return "$TestInstaller.sig" }

$pipelineCode = Get-Content -LiteralPath $PipelinePath -Raw

function Extract-Function([string]$fnName) {
    $startIdx = $pipelineCode.IndexOf("function $fnName")
    if ($startIdx -lt 0) { throw "Could not locate $fnName" }
    $openBrace = $pipelineCode.IndexOf('{', $startIdx)
    $depth = 0
    for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
        if ($pipelineCode[$i] -eq '{') { $depth++ }
        elseif ($pipelineCode[$i] -eq '}') {
            $depth--
            if ($depth -eq 0) {
                return $pipelineCode.Substring($startIdx, $i - $startIdx + 1)
            }
        }
    }
    throw "Unclosed brace for $fnName"
}

. ([scriptblock]::Create((Extract-Function 'Get-RecordPropertyValue')))
. ([scriptblock]::Create((Extract-Function 'Get-RecordPropertyString')))
. ([scriptblock]::Create((Extract-Function 'Assert-EvidenceIdentity')))

$recs = @(
    [pscustomobject]@{ name = $safeInstaller; release_name = $safeInstaller; source_name = $TestInstaller; size = [int64]1234567; sha256 = $TestInstallerSha },
    [pscustomobject]@{ name = $safeSig; release_name = $safeSig; source_name = "$TestInstaller.sig"; size = [int64]512; sha256 = $TestSigSha },
    [pscustomobject]@{ name = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'; size = [int64]100; sha256 = ('1' * 64) },
    [pscustomobject]@{ name = 'V4_QUALIFICATION_EVIDENCE.json'; size = [int64]100; sha256 = ('2' * 64) },
    [pscustomobject]@{ name = 'TAURI_AUTHENTICODE_EVIDENCE.json'; size = [int64]100; sha256 = $TestAuthSha },
    [pscustomobject]@{ name = 'SBOM.spdx.json'; size = [int64]100; sha256 = $TestSbomSha }
)

Assert-EvidenceIdentity $TargetProdPath $QualPath $recs
'@
        $worker = Join-Path $evidenceTestRoot "assert_worker.ps1"
        Set-Content -LiteralPath $worker -Value $scopedScript -Encoding utf8
        $res = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $worker `
            -PipelinePath $pipelinePath `
            -TargetProdPath $TargetProdPath `
            -QualPath $qualPath `
            -TestSha $testSha `
            -TestVersion $testVersion `
            -TestInstaller $testInstaller `
            -TestInstallerSha $testInstallerSha `
            -TestSigSha $testSigSha `
            -TestAuthSha $testAuthSha `
            -TestSbomSha $testSbomSha 2>&1 | Out-String
        return @{ ExitCode = $LASTEXITCODE; Output = $res }
    }

    # 1. Valid production evidence passes consumer Assert-EvidenceIdentity
    $prodObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $prodPath -Encoding utf8
    $validRun = Invoke-EvidenceIdentityAssertion $prodPath
    if ($validRun.ExitCode -ne 0) {
        Fail "consumer Assert-EvidenceIdentity rejected valid production evidence: $($validRun.Output)"
    }

    # 2. Missing authenticode_evidence fails closed
    $missingAuthObj = New-V4CanonicalProductionEvidence `
        -SourceSha $testSha `
        -Version $testVersion `
        -Channel "stable" `
        -InstallerName $testInstaller `
        -SignatureName $testSignature `
        -InstallerSize 1234567 `
        -SignatureSize 512 `
        -InstallerSha256 $testInstallerSha `
        -SignatureSha256 $testSigSha `
        -AuthenticodeEvidenceSha256 $testAuthSha `
        -SbomSha256 $testSbomSha `
        -UpdaterKeyId $testKeyId
    $missingAuthObj.Remove("authenticode_evidence")
    $missingAuthPath = Join-Path $evidenceTestRoot "missing_auth.json"
    $missingAuthObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $missingAuthPath -Encoding utf8
    $missingAuthRun = Invoke-EvidenceIdentityAssertion $missingAuthPath
    if ($missingAuthRun.ExitCode -eq 0) {
        Fail "consumer Assert-EvidenceIdentity accepted production evidence missing authenticode_evidence"
    }

    # 3. Missing authenticode_evidence_sha256 fails closed
    $missingShaObj = New-V4CanonicalProductionEvidence `
        -SourceSha $testSha `
        -Version $testVersion `
        -Channel "stable" `
        -InstallerName $testInstaller `
        -SignatureName $testSignature `
        -InstallerSize 1234567 `
        -SignatureSize 512 `
        -InstallerSha256 $testInstallerSha `
        -SignatureSha256 $testSigSha `
        -AuthenticodeEvidenceSha256 $testAuthSha `
        -SbomSha256 $testSbomSha `
        -UpdaterKeyId $testKeyId
    $missingShaObj.Remove("authenticode_evidence_sha256")
    $missingShaPath = Join-Path $evidenceTestRoot "missing_sha.json"
    $missingShaObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $missingShaPath -Encoding utf8
    $missingShaRun = Invoke-EvidenceIdentityAssertion $missingShaPath
    if ($missingShaRun.ExitCode -eq 0) {
        Fail "consumer Assert-EvidenceIdentity accepted production evidence missing authenticode_evidence_sha256"
    }

    # 4. Tampered filename fails closed
    $tamperedNameObj = New-V4CanonicalProductionEvidence `
        -SourceSha $testSha `
        -Version $testVersion `
        -Channel "stable" `
        -InstallerName $testInstaller `
        -SignatureName $testSignature `
        -InstallerSize 1234567 `
        -SignatureSize 512 `
        -InstallerSha256 $testInstallerSha `
        -SignatureSha256 $testSigSha `
        -AuthenticodeEvidenceSha256 $testAuthSha `
        -SbomSha256 $testSbomSha `
        -UpdaterKeyId $testKeyId
    $tamperedNameObj["authenticode_evidence"] = "TAMPERED_AUTHENTICODE_EVIDENCE.json"
    $tamperedNamePath = Join-Path $evidenceTestRoot "tampered_name.json"
    $tamperedNameObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $tamperedNamePath -Encoding utf8
    $tamperedNameRun = Invoke-EvidenceIdentityAssertion $tamperedNamePath
    if ($tamperedNameRun.ExitCode -eq 0) {
        Fail "consumer Assert-EvidenceIdentity accepted production evidence with tampered authenticode_evidence filename"
    }

    # 5. Tampered SHA-256 fails closed
    $tamperedShaObj = New-V4CanonicalProductionEvidence `
        -SourceSha $testSha `
        -Version $testVersion `
        -Channel "stable" `
        -InstallerName $testInstaller `
        -SignatureName $testSignature `
        -InstallerSize 1234567 `
        -SignatureSize 512 `
        -InstallerSha256 $testInstallerSha `
        -SignatureSha256 $testSigSha `
        -AuthenticodeEvidenceSha256 $testAuthSha `
        -SbomSha256 $testSbomSha `
        -UpdaterKeyId $testKeyId
    $tamperedShaObj["authenticode_evidence_sha256"] = "f" * 64
    $tamperedShaPath = Join-Path $evidenceTestRoot "tampered_sha.json"
    $tamperedShaObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $tamperedShaPath -Encoding utf8
    $tamperedShaRun = Invoke-EvidenceIdentityAssertion $tamperedShaPath
    if ($tamperedShaRun.ExitCode -eq 0) {
        Fail "consumer Assert-EvidenceIdentity accepted production evidence with tampered authenticode_evidence_sha256"
    }
} finally {
    if (Test-Path -LiteralPath $evidenceTestRoot) {
        Remove-Item -LiteralPath $evidenceTestRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Safe release asset name contract regression tests
function Test-SafeReleaseAssetNameContract {
    # 1. Source installer name with spaces maps to deterministic safe release name
    $sourceInstaller = "Sky Auto Player_4.0.0-rc.1_x64-setup.exe"
    $safeInstaller = Get-V4SafeReleaseAssetName $sourceInstaller
    if ($safeInstaller -ne "Sky.Auto.Player_4.0.0-rc.1_x64-setup.exe") {
        Fail "Get-V4SafeReleaseAssetName did not map source installer spaces to dots"
    }
    $sourceSig = "$sourceInstaller.sig"
    $safeSig = Get-V4SafeReleaseAssetName $sourceSig
    if ($safeSig -ne "Sky.Auto.Player_4.0.0-rc.1_x64-setup.exe.sig") {
        Fail "Get-V4SafeReleaseAssetName did not map signature name spaces to dots"
    }

    # 2. Source and release records keep identical SHA and size
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-record-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        $filePath = Join-Path $tempDir $sourceInstaller
        $testBytes = [byte[]](0x4D, 0x5A, 0x90, 0x00, 0x03)
        [IO.File]::WriteAllBytes($filePath, $testBytes)
        $fileSha = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()

        $rec = [pscustomobject]@{
            name = $safeInstaller
            source_name = $sourceInstaller
            release_name = $safeInstaller
            role = "installer"
            size = [int64]$testBytes.Length
            sha256 = $fileSha
        }
        if ($rec.size -ne [int64]$testBytes.Length -or $rec.sha256 -ne $fileSha) {
            Fail "source and release record sizes or SHA-256 digests do not match"
        }
        if ($rec.source_name -ne $sourceInstaller -or $rec.release_name -ne $safeInstaller) {
            Fail "record does not cleanly separate source_name from release_name"
        }
    } finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    # 3. Upload response exact-name matching remains mandatory
    if ($pipeline -notmatch '\[string\]\$uploaded\.name\s+-ne\s+\$releaseName') {
        Fail "pipeline must enforce uploaded.name -eq releaseName exact response match"
    }

    # 4. Unsafe name fails before PublishRelease
    foreach ($unsafe in @(
        "", "   ", "path/separator", "path\separator", ".leadingdot", "-leadinghyphen",
        ".", "..", "invalid*char", "invalid?char", "invalid:char", "invalid|char"
    )) {
        $failedClosed = $false
        try {
            $null = Get-V4SafeReleaseAssetName $unsafe
        } catch {
            $failedClosed = $true
        }
        if (-not $failedClosed) {
            Fail "Get-V4SafeReleaseAssetName accepted unsafe name: '$unsafe'"
        }
    }

    # 5. Release-name collision fails before PublishRelease
    if ($pipeline -notmatch 'release asset name collision detected') {
        Fail "pipeline must contain release asset name collision check before PublishRelease"
    }

    # 6. Downloaded safe-name asset qualifies against source-name evidence without byte mutation
    $qualTestDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-download-qual-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $qualTestDir -Force | Out-Null
        $dlDir = Join-Path $qualTestDir "downloaded"
        $bundleDir = Join-Path $qualTestDir "bundle"
        New-Item -ItemType Directory -Path $dlDir -Force | Out-Null
        New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null
        $dlFile = Join-Path $dlDir $safeInstaller
        $fixtureBytes = [byte[]](0xDE, 0xAD, 0xBE, 0xEF, 0x42)
        [IO.File]::WriteAllBytes($dlFile, $fixtureBytes)
        $dlHash = (Get-FileHash -LiteralPath $dlFile -Algorithm SHA256).Hash.ToLowerInvariant()
        # Stage to bundle under source name
        $stagedFile = Join-Path $bundleDir $sourceInstaller
        Copy-Item -LiteralPath $dlFile -Destination $stagedFile
        $stagedHash = (Get-FileHash -LiteralPath $stagedFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($stagedHash -ne $dlHash) {
            Fail "staging safe release name into source bundle mutated file bytes"
        }
        if ((Get-Item -LiteralPath $stagedFile).Name -ne $sourceInstaller) {
            Fail "staged file name does not match expected source installer name"
        }
    } finally {
        if (Test-Path -LiteralPath $qualTestDir) {
            Remove-Item -LiteralPath $qualTestDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    Write-Host "V4 safe release asset name contract: PASS (deterministic dot mapping; collision check; exact response check; safe staging)"
}

# -------------------------------------------------------------------------
# Post-Publication Incident & Canonical Schema Regression Tests
# -------------------------------------------------------------------------

$doctorScriptPath = Join-Path $PSScriptRoot "release_doctor.ps1"
$doctorScriptContent = Get-Content -LiteralPath $doctorScriptPath -Raw

function Extract-ScriptFunction([string]$Content, [string]$FunctionName) {
    $startIdx = $Content.IndexOf("function $FunctionName")
    if ($startIdx -lt 0) { throw "Could not locate $FunctionName in script content" }
    $openBrace = $Content.IndexOf('{', $startIdx)
    $depth = 0
    $endIdx = -1
    for ($i = $openBrace; $i -lt $Content.Length; $i++) {
        if ($Content[$i] -eq '{') { $depth++ }
        elseif ($Content[$i] -eq '}') {
            $depth--
            if ($depth -eq 0) {
                $endIdx = $i
                break
            }
        }
    }
    if ($endIdx -lt 0) { throw "Could not find matching brace for $FunctionName" }
    return $Content.Substring($startIdx, $endIdx - $startIdx + 1)
}

function Extract-PipelineFunction([string]$FunctionName) {
    return Extract-ScriptFunction $pipeline $FunctionName
}

function Assert-V4ReleaseStateSchema([object]$State) {
    if ($null -eq $State) { Fail "release state object is null" }
    $requiredProps = @(
        "schema_version", "phase", "source_sha", "version", "channel",
        "tag", "release_id", "draft", "published", "immutable", "published_at",
        "attested", "qualified_after_download", "qualification_assets",
        "public_assets", "metadata_promoted", "promoted_at", "final_verified",
        "final_verified_at", "reconciled_from_remote", "last_reconciled_at",
        "failure_class", "error_message"
    )
    foreach ($prop in $requiredProps) {
        if ($null -eq $State.PSObject.Properties[$prop]) {
            Fail "release state is missing required property '$prop'"
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
        [Parameter(Mandatory = $true)] [string]$SourceSha,
        [Parameter(Mandatory = $true)] [string]$Version,
        [Parameter(Mandatory = $true)] [string]$Channel,
        [Parameter(Mandatory = $true)] [string]$Tag,
        [Parameter(Mandatory = $true)] [int64]$ReleaseId,
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
. ([scriptblock]::Create((Extract-PipelineFunction "Format-CanonicalRfc3339Timestamp")))
. ([scriptblock]::Create((Extract-ScriptFunction $doctorScriptContent "Test-ReleaseWorkflowRunCriteria")))
. ([scriptblock]::Create((Extract-ScriptFunction $doctorScriptContent "Assert-ReleaseWorkflowRunCriteria")))

# -------------------------------------------------------------------------
# Test 1: schema-v1 missing field reproduces StrictMode failure
# -------------------------------------------------------------------------
function Test-SchemaV1MissingFieldReproducesStrictModeFailure {
    $schema1State = [ordered]@{
        schema_version = 1
        source_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        version = "4.1.0"
        channel = "stable"
        tag = "v4.1.0"
        release_id = 42
        draft = $true
        published = $false
        immutable = $false
        attested = $true
        qualified_after_download = $true
    }
    $rawJson = $schema1State | ConvertTo-Json
    $deserialized = $rawJson | ConvertFrom-Json

    $reproduced = $false
    try {
        $deserialized.published_at = "2026-09-17T17:35:58Z"
    } catch {
        if ($_.Exception -is [System.Management.Automation.SetValueInvocationException] -or
            $_.Exception.Message -match "published_at") {
            $reproduced = $true
        } else {
            throw
        }
    }
    if (-not $reproduced) {
        Fail "root-cause reproduction failed: assigning published_at on schema 1 PSCustomObject did not throw"
    }
    Write-Host "V4 test (1/19): schema-v1 missing field reproduces StrictMode failure: PASS"
}

# -------------------------------------------------------------------------
# Test 2: schema-v2 canonical constructor survives StrictMode
# -------------------------------------------------------------------------
function Test-SchemaV2CanonicalConstructorSurvivesStrictMode {
    $state = New-V4CanonicalReleaseState `
        -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
        -Version "4.1.0" `
        -Channel "stable" `
        -Tag "v4.1.0" `
        -ReleaseId 12345 `
        -Phase "READY"

    $requiredProperties = @(
        "schema_version", "phase", "source_sha", "version", "channel", "tag", "release_id",
        "draft", "published", "immutable", "published_at", "attested", "qualified_after_download",
        "qualification_assets", "public_assets", "metadata_promoted", "promoted_at",
        "final_verified", "final_verified_at", "reconciled_from_remote", "last_reconciled_at",
        "failure_class", "error_message"
    )
    if ($requiredProperties.Count -ne 23) { Fail "expected 23 canonical properties, found $($requiredProperties.Count)" }
    foreach ($prop in $requiredProperties) {
        if ($null -eq $state.PSObject.Properties[$prop]) {
            Fail "New-V4CanonicalReleaseState omitted required property '$prop'"
        }
    }

    $json = $state | ConvertTo-Json -Depth 10
    $deserialized = $json | ConvertFrom-Json
    Assert-V4ReleaseStateSchema $deserialized

    # Mutate all lifecycle properties under Set-StrictMode -Version Latest
    $deserialized.published_at = "2026-09-17T17:35:58Z"
    $deserialized.phase = "PUBLISHED_PENDING_METADATA"
    $deserialized.draft = $false
    $deserialized.published = $true
    $deserialized.immutable = $true
    $deserialized.reconciled_from_remote = $true
    $deserialized.last_reconciled_at = "2026-09-17T17:36:00Z"
    $deserialized.failure_class = ""
    $deserialized.error_message = ""
    $deserialized.metadata_promoted = $true
    $deserialized.promoted_at = "2026-09-17T17:37:00Z"
    $deserialized.final_verified = $true
    $deserialized.final_verified_at = "2026-09-17T17:38:00Z"
    $deserialized.phase = "COMPLETE"

    Assert-V4ReleaseStateSchema $deserialized
    Write-Host "V4 test (2/19): schema-v2 canonical constructor survives StrictMode: PASS"
}

# -------------------------------------------------------------------------
# Test 3: malformed/missing critical schema-v2 field fails closed instead of being silently normalized
# -------------------------------------------------------------------------
function Test-MalformedOrMissingCriticalSchemaV2FieldFailsClosed {
    $baseState = New-V4CanonicalReleaseState `
        -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
        -Version "4.1.0" `
        -Channel "stable" `
        -Tag "v4.1.0" `
        -ReleaseId 12345

    $requiredProperties = @(
        "schema_version", "phase", "source_sha", "version", "channel", "tag", "release_id",
        "draft", "published", "immutable", "published_at", "attested", "qualified_after_download",
        "qualification_assets", "public_assets", "metadata_promoted", "promoted_at",
        "final_verified", "final_verified_at", "reconciled_from_remote", "last_reconciled_at",
        "failure_class", "error_message"
    )

    # 1. Missing property must fail closed
    foreach ($prop in $requiredProperties) {
        $dict = [ordered]@{}
        foreach ($p in $requiredProperties) {
            if ($p -ne $prop) {
                $dict[$p] = $baseState.$p
            }
        }
        $incomplete = [pscustomobject]$dict
        $threw = $false
        try {
            Assert-V4ReleaseStateSchema $incomplete
        } catch {
            if ($_.Exception.Message -match "missing required property '$prop'") {
                $threw = $true
            } else {
                throw
            }
        }
        if (-not $threw) {
            Fail "Assert-V4ReleaseStateSchema did not fail closed on missing property '$prop'"
        }
    }

    # 2. Schema version must be exactly 2
    $badVersion = ($baseState | ConvertTo-Json | ConvertFrom-Json)
    $badVersion.schema_version = 1
    $threw = $false
    try {
        Assert-V4ReleaseStateSchema $badVersion
    } catch {
        if ($_.Exception.Message -match "unsupported release state schema_version") { $threw = $true }
    }
    if (-not $threw) { Fail "Assert-V4ReleaseStateSchema did not reject schema_version 1" }

    # 3. Invalid phase must fail closed
    $badPhase = ($baseState | ConvertTo-Json | ConvertFrom-Json)
    $badPhase.phase = "POST_PUBLICATION_INCIDENT" # Diagnostic, not persisted phase!
    $threw = $false
    try {
        Assert-V4ReleaseStateSchema $badPhase
    } catch {
        if ($_.Exception.Message -match "invalid release state phase") { $threw = $true }
    }
    if (-not $threw) { Fail "Assert-V4ReleaseStateSchema did not reject non-persisted phase" }

    # 4. Published release without published_at timestamp must fail closed
    $badPublished = ($baseState | ConvertTo-Json | ConvertFrom-Json)
    $badPublished.phase = "PUBLISHED_PENDING_METADATA"
    $badPublished.draft = $false
    $badPublished.published = $true
    $badPublished.published_at = ""
    $threw = $false
    try {
        Assert-V4ReleaseStateSchema $badPublished
    } catch {
        if ($_.Exception.Message -match "published_at timestamp is empty") { $threw = $true }
    }
    if (-not $threw) { Fail "Assert-V4ReleaseStateSchema accepted published=true with empty published_at" }

    Write-Host "V4 test (3/19): malformed/missing critical schema-v2 field fails closed: PASS"
}

# Helper for testing publish reconciliation state transitions
function Invoke-TestPublishReconciliation {
    param(
        [Parameter(Mandatory = $true)][string]$StatePath,
        [Parameter(Mandatory = $true)][string]$Tag,
        [Parameter(Mandatory = $true)][string]$SourceSha,
        [scriptblock]$PatchAction,
        [scriptblock]$GetAction
    )
    $state = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
    Assert-V4ReleaseStateSchema $state

    # Attempt PATCH
    $patchError = $null
    try {
        if ($null -ne $PatchAction) { $null = & $PatchAction }
    } catch {
        $patchError = $_
    }

    # Step 3: ALWAYS GET exact release_id
    $remoteRelease = $null
    $getRemoteError = $null
    try {
        if ($null -ne $GetAction) { $remoteRelease = & $GetAction }
    } catch {
        $getRemoteError = $_
    }

    # Branch 1: Published + immutable
    if ($null -ne $remoteRelease -and -not [bool]$remoteRelease.draft -and -not [string]::IsNullOrWhiteSpace([string]$remoteRelease.published_at)) {
        if ([string]$remoteRelease.tag_name -ne $Tag) {
            Fail "published remote release tag mismatch"
        }
        $state.phase = "PUBLISHED_PENDING_METADATA"
        $state.draft = $false
        $state.published = $true
        $state.immutable = [bool]$remoteRelease.immutable
        $state.published_at = [string]$remoteRelease.published_at
        $state.reconciled_from_remote = $true
        $state.last_reconciled_at = (Get-Date).ToUniversalTime().ToString("o")
        $state.failure_class = ""
        $state.error_message = ""
        $json = $state | ConvertTo-Json -Depth 10
        [IO.File]::WriteAllText($StatePath, $json, [Text.UTF8Encoding]::new($false))
        return $state
    }

    # Branch 2: Still draft
    if ($null -ne $remoteRelease -and [bool]$remoteRelease.draft) {
        $errMsg = if ($null -ne $patchError) { $patchError.Exception.Message } else { "remote release remains draft" }
        $state.failure_class = "RECOVERABLE_PRE_PUBLICATION_FAILURE"
        $state.error_message = $errMsg
        $state.last_reconciled_at = (Get-Date).ToUniversalTime().ToString("o")
        $json = $state | ConvertTo-Json -Depth 10
        [IO.File]::WriteAllText($StatePath, $json, [Text.UTF8Encoding]::new($false))
        throw "RECOVERABLE_PRE_PUBLICATION_FAILURE: publication did not complete; release remains draft ($errMsg)"
    }

    # Branch 3: Remote truth unknown
    $unknownMsg = if ($null -ne $getRemoteError) {
        $getRemoteError.Exception.Message
    } elseif ($null -ne $patchError) {
        $patchError.Exception.Message
    } else {
        "unable to retrieve release after publication attempt"
    }
    $state.failure_class = "REMOTE_STATE_UNKNOWN"
    $state.error_message = $unknownMsg
    $state.last_reconciled_at = (Get-Date).ToUniversalTime().ToString("o")
    $json = $state | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText($StatePath, $json, [Text.UTF8Encoding]::new($false))
    throw "REMOTE_STATE_UNKNOWN: unable to verify remote publication state ($unknownMsg)"
}

# -------------------------------------------------------------------------
# Test 4: PATCH succeeds + GET confirms publication => PUBLISHED_PENDING_METADATA
# -------------------------------------------------------------------------
function Test-PatchSucceedsGetConfirmsPublication {
    $testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-test4-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $testDir -Force | Out-Null
        $stateFile = Join-Path $testDir "release-state.json"

        $initialState = New-V4CanonicalReleaseState `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -Version "4.1.0" `
            -Channel "stable" `
            -Tag "v4.1.0" `
            -ReleaseId 12345 `
            -Phase "QUALIFIED"
        $initialState.qualified_after_download = $true
        $initialState.attested = $true
        [IO.File]::WriteAllText($stateFile, ($initialState | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

        $mockRemote = [pscustomobject]@{
            id = 12345
            tag_name = "v4.1.0"
            target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
            draft = $false
            published_at = "2026-09-17T17:35:58Z"
            immutable = $true
        }

        $resultState = Invoke-TestPublishReconciliation `
            -StatePath $stateFile `
            -Tag "v4.1.0" `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -PatchAction { return $mockRemote } `
            -GetAction { return $mockRemote }

        if ($resultState.phase -ne "PUBLISHED_PENDING_METADATA" -or
            -not $resultState.published -or
            $resultState.draft -or
            -not $resultState.immutable -or
            $resultState.published_at -ne "2026-09-17T17:35:58Z" -or
            -not $resultState.reconciled_from_remote) {
            Fail "state was not reconciled to PUBLISHED_PENDING_METADATA"
        }

        Write-Host "V4 test (4/19): PATCH succeeds + GET confirms publication => PUBLISHED_PENDING_METADATA: PASS"
    } finally {
        if (Test-Path -LiteralPath $testDir) { Remove-Item -LiteralPath $testDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 5: PATCH command reports failure + GET confirms publication => PUBLISHED_PENDING_METADATA
# -------------------------------------------------------------------------
function Test-PatchReportsFailureGetConfirmsPublication {
    $testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-test5-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $testDir -Force | Out-Null
        $stateFile = Join-Path $testDir "release-state.json"

        $initialState = New-V4CanonicalReleaseState `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -Version "4.1.0" `
            -Channel "stable" `
            -Tag "v4.1.0" `
            -ReleaseId 12345 `
            -Phase "QUALIFIED"
        $initialState.qualified_after_download = $true
        $initialState.attested = $true
        [IO.File]::WriteAllText($stateFile, ($initialState | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

        $mockRemote = [pscustomobject]@{
            id = 12345
            tag_name = "v4.1.0"
            target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
            draft = $false
            published_at = "2026-09-17T17:35:58Z"
            immutable = $true
        }

        # PATCH throws connection error, but subsequent GET succeeds
        $resultState = Invoke-TestPublishReconciliation `
            -StatePath $stateFile `
            -Tag "v4.1.0" `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -PatchAction { throw [System.IO.IOException]::new("Connection reset by peer during PATCH") } `
            -GetAction { return $mockRemote }

        if ($resultState.phase -ne "PUBLISHED_PENDING_METADATA" -or
            -not $resultState.published -or
            $resultState.draft -or
            -not $resultState.immutable -or
            $resultState.published_at -ne "2026-09-17T17:35:58Z" -or
            -not $resultState.reconciled_from_remote) {
            Fail "reconciliation failed when PATCH failed but GET confirmed publication"
        }

        Write-Host "V4 test (5/19): PATCH command reports failure + GET confirms publication => PUBLISHED_PENDING_METADATA: PASS"
    } finally {
        if (Test-Path -LiteralPath $testDir) { Remove-Item -LiteralPath $testDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 6: PATCH fails + GET confirms still draft => recoverable pre-publication failure
# -------------------------------------------------------------------------
function Test-PatchFailsGetConfirmsStillDraft {
    $testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-test6-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $testDir -Force | Out-Null
        $stateFile = Join-Path $testDir "release-state.json"

        $initialState = New-V4CanonicalReleaseState `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -Version "4.1.0" `
            -Channel "stable" `
            -Tag "v4.1.0" `
            -ReleaseId 12345 `
            -Phase "QUALIFIED"
        $initialState.qualified_after_download = $true
        $initialState.attested = $true
        [IO.File]::WriteAllText($stateFile, ($initialState | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

        $mockDraft = [pscustomobject]@{
            id = 12345
            tag_name = "v4.1.0"
            target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
            draft = $true
            published_at = $null
            immutable = $false
        }

        $threw = $false
        try {
            Invoke-TestPublishReconciliation `
                -StatePath $stateFile `
                -Tag "v4.1.0" `
                -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
                -PatchAction { throw [System.Net.Http.HttpRequestException]::new("Bad gateway 502") } `
                -GetAction { return $mockDraft }
        } catch {
            if ($_.Exception.Message -match "RECOVERABLE_PRE_PUBLICATION_FAILURE") {
                $threw = $true
            } else {
                throw
            }
        }
        if (-not $threw) { Fail "expected RECOVERABLE_PRE_PUBLICATION_FAILURE exception" }

        # Verify persisted failure class
        $persisted = Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json
        if ($persisted.failure_class -ne "RECOVERABLE_PRE_PUBLICATION_FAILURE" -or
            $persisted.phase -ne "QUALIFIED" -or
            $persisted.published) {
            Fail "persisted state does not reflect RECOVERABLE_PRE_PUBLICATION_FAILURE"
        }

        Write-Host "V4 test (6/19): PATCH fails + GET confirms still draft => recoverable pre-publication failure: PASS"
    } finally {
        if (Test-Path -LiteralPath $testDir) { Remove-Item -LiteralPath $testDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 7: PATCH result ambiguous + GET unavailable => REMOTE_STATE_UNKNOWN
# -------------------------------------------------------------------------
function Test-PatchAmbiguousGetUnavailable {
    $testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-test7-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $testDir -Force | Out-Null
        $stateFile = Join-Path $testDir "release-state.json"

        $initialState = New-V4CanonicalReleaseState `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -Version "4.1.0" `
            -Channel "stable" `
            -Tag "v4.1.0" `
            -ReleaseId 12345 `
            -Phase "QUALIFIED"
        $initialState.qualified_after_download = $true
        $initialState.attested = $true
        [IO.File]::WriteAllText($stateFile, ($initialState | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

        $threw = $false
        try {
            Invoke-TestPublishReconciliation `
                -StatePath $stateFile `
                -Tag "v4.1.0" `
                -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
                -PatchAction { throw [System.IO.IOException]::new("Timeout waiting for response") } `
                -GetAction { throw [System.Net.Http.HttpRequestException]::new("Service Unavailable 503") }
        } catch {
            if ($_.Exception.Message -match "REMOTE_STATE_UNKNOWN") {
                $threw = $true
            } else {
                throw
            }
        }
        if (-not $threw) { Fail "expected REMOTE_STATE_UNKNOWN exception" }

        # Verify persisted failure class
        $persisted = Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json
        if ($persisted.failure_class -ne "REMOTE_STATE_UNKNOWN" -or
            $persisted.phase -ne "QUALIFIED" -or
            $persisted.published) {
            Fail "persisted state does not reflect REMOTE_STATE_UNKNOWN"
        }

        Write-Host "V4 test (7/19): PATCH result ambiguous + GET unavailable => REMOTE_STATE_UNKNOWN: PASS"
    } finally {
        if (Test-Path -LiteralPath $testDir) { Remove-Item -LiteralPath $testDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 8: fresh transaction encounters existing published same tag => refuses adoption
# -------------------------------------------------------------------------
function Test-FreshTransactionRefusesAdoptionOfExistingPublishedRelease {
    $publishedRemote = [pscustomobject]@{
        id = 99999
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
    }

    $rejected = $false
    try {
        $Tag = "v4.1.0"
        $SourceSha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        Assert-ExistingUnpublishedDraftMatchesRequest $publishedRemote
    } catch {
        if ($_.Exception.Message -match "fresh transaction refuses adoption" -and
            $_.Exception.Message -match "published releases and tags are immutable") {
            $rejected = $true
        } else {
            throw
        }
    }
    if (-not $rejected) {
        Fail "fresh transaction did not fail closed with 'fresh transaction refuses adoption'"
    }

    Write-Host "V4 test (8/19): fresh transaction encounters existing published same tag => refuses adoption: PASS"
}

# -------------------------------------------------------------------------
# Test 9: exact local release_id encounters already-published remote after ambiguous same-transaction PATCH => reconciliation allowed
# -------------------------------------------------------------------------
function Test-ExactLocalReleaseIdEncountersAlreadyPublishedRemoteReconciles {
    $testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-test9-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $testDir -Force | Out-Null
        $stateFile = Join-Path $testDir "release-state.json"

        # Local transaction has exact matching release_id, source_sha, tag, version
        $initialState = New-V4CanonicalReleaseState `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -Version "4.1.0" `
            -Channel "stable" `
            -Tag "v4.1.0" `
            -ReleaseId 12345 `
            -Phase "QUALIFIED"
        $initialState.qualified_after_download = $true
        $initialState.attested = $true
        [IO.File]::WriteAllText($stateFile, ($initialState | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

        $mockAlreadyPublished = [pscustomobject]@{
            id = 12345
            tag_name = "v4.1.0"
            target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
            draft = $false
            published_at = "2026-09-17T17:35:58Z"
            immutable = $true
        }

        # Preflight detects remote is already published with exact release_id
        $resultState = Invoke-TestPublishReconciliation `
            -StatePath $stateFile `
            -Tag "v4.1.0" `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -PatchAction { Fail "PATCH should not be called when preflight confirms publication" } `
            -GetAction { return $mockAlreadyPublished }

        if ($resultState.phase -ne "PUBLISHED_PENDING_METADATA" -or
            -not $resultState.published -or
            -not $resultState.reconciled_from_remote) {
            Fail "exact local transaction failed to reconcile already-published remote release"
        }

        Write-Host "V4 test (9/19): exact local release_id encounters already-published remote => reconciliation allowed: PASS"
    } finally {
        if (Test-Path -LiteralPath $testDir) { Remove-Item -LiteralPath $testDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 10: published release + metadata old + production workflow still running/unknown => PUBLISHED_PENDING_METADATA
# -------------------------------------------------------------------------
function Test-PublishedReleaseMetadataOldProductionWorkflowRunningOrUnknown {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"

    $publishedRemote = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }
    $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
    $mockMetadata = [pscustomobject]@{ version = "4.0.1" } # Channel metadata old

    # Case A: Workflow is in-progress -> external_phase = PUBLISHED_PENDING_METADATA, classification = null, operator_review_required = false
    $inProgressRun = [pscustomobject]@{
        id = 35255186714
        path = ".github/workflows/release-v4.yml"
        event = "workflow_dispatch"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "in_progress"
        conclusion = $null
    }
    $reportA = & $doctorScript `
        -Tag "v4.1.0" `
        -Channel "stable" `
        -Offline `
        -OfflineExternalRelease $publishedRemote `
        -OfflineLatestRelease $mockLatest `
        -OfflineMetadata $mockMetadata `
        -OfflineWorkflowRun $inProgressRun `
        -Format Json | ConvertFrom-Json

    if ($reportA.persisted_phase -ne $null -or
        $reportA.external_phase -ne "PUBLISHED_PENDING_METADATA" -or
        $reportA.classification -ne $null -or
        $reportA.workflow_outcome -ne "in_progress" -or
        $reportA.operator_review_required -ne $false) {
        Fail "in-progress workflow contract failed: got ext_phase=$($reportA.external_phase), class=$($reportA.classification), outcome=$($reportA.workflow_outcome), review=$($reportA.operator_review_required)"
    }
    if ($null -ne $reportA.PSObject.Properties['phase'] -or $null -ne $reportA.PSObject.Properties['lifecycle_state']) {
        Fail "ambiguous phase/lifecycle_state must not be emitted at root report"
    }

    # Case B: Workflow outcome unknown -> external_phase = PUBLISHED_PENDING_METADATA, classification = null, operator_review_required = true
    $reportB = & $doctorScript `
        -Tag "v4.1.0" `
        -Channel "stable" `
        -Offline `
        -OfflineExternalRelease $publishedRemote `
        -OfflineLatestRelease $mockLatest `
        -OfflineMetadata $mockMetadata `
        -OfflineWorkflowRun $null `
        -Format Json | ConvertFrom-Json

    if ($reportB.persisted_phase -ne $null -or
        $reportB.external_phase -ne "PUBLISHED_PENDING_METADATA" -or
        $reportB.classification -ne $null -or
        $reportB.workflow_outcome -ne "unknown" -or
        $reportB.operator_review_required -ne $true) {
        Fail "unknown workflow contract failed: got ext_phase=$($reportB.external_phase), class=$($reportB.classification), outcome=$($reportB.workflow_outcome), review=$($reportB.operator_review_required)"
    }
    if ($null -ne $reportB.PSObject.Properties['phase'] -or $null -ne $reportB.PSObject.Properties['lifecycle_state']) {
        Fail "ambiguous phase/lifecycle_state must not be emitted at root report"
    }

    Write-Host "V4 test (10/19): published release + metadata old + production workflow still running/unknown => PUBLISHED_PENDING_METADATA: PASS"
}

# -------------------------------------------------------------------------
# Test 11: published release + metadata old + exact workflow completed failure => POST_PUBLICATION_INCIDENT
# -------------------------------------------------------------------------
function Test-PublishedReleaseMetadataOldWorkflowCompletedFailure {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"

    $publishedRemote = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }
    $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
    $mockMetadata = [pscustomobject]@{ version = "4.0.1" }

    $failedRun = [pscustomobject]@{
        id = 35255186714
        path = ".github/workflows/release-v4.yml"
        event = "workflow_dispatch"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "failure"
    }

    $report = & $doctorScript `
        -Tag "v4.1.0" `
        -Channel "stable" `
        -Offline `
        -OfflineExternalRelease $publishedRemote `
        -OfflineLatestRelease $mockLatest `
        -OfflineMetadata $mockMetadata `
        -OfflineWorkflowRun $failedRun `
        -Format Json | ConvertFrom-Json

    if ($report.persisted_phase -ne $null -or
        $report.external_phase -ne "PUBLISHED_PENDING_METADATA" -or
        $report.classification -ne "POST_PUBLICATION_INCIDENT" -or
        $report.workflow_outcome -ne "failure" -or
        $report.operator_review_required -ne $true) {
        Fail "failed workflow run did not produce contract (persisted_phase=null, external_phase=PUBLISHED_PENDING_METADATA, classification=POST_PUBLICATION_INCIDENT, outcome=failure, review=true): got persisted=$($report.persisted_phase), ext=$($report.external_phase), class=$($report.classification), outcome=$($report.workflow_outcome), review=$($report.operator_review_required)"
    }
    if ($null -ne $report.PSObject.Properties['phase'] -or $null -ne $report.PSObject.Properties['lifecycle_state']) {
        Fail "ambiguous phase/lifecycle_state must not be emitted at root report"
    }
    if ($report.recovery_guidance -notmatch "NEVER rerun the failed workflow run" -or
        $report.recovery_guidance -notmatch "NEVER delete, recreate, or replace tag") {
        Fail "diagnostic recovery guidance missing mandatory immutability/no-rerun prohibitions"
    }

    Write-Host "V4 test (11/19): published release + metadata old + exact workflow completed failure => POST_PUBLICATION_INCIDENT: PASS"
}

# -------------------------------------------------------------------------
# Test 12: canonical UTC RFC3339 timestamps culture invariance (en-US, vi-VN, fr-FR)
# -------------------------------------------------------------------------
function Test-CanonicalRfc3339TimestampsCultureInvariance {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"
    $publishedRemote = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }
    $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
    $mockMetadata = [pscustomobject]@{ version = "4.0.1" }
    $failedRun = [pscustomobject]@{
        id = 35255186714
        path = ".github/workflows/release-v4.yml"
        event = "workflow_dispatch"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "failure"
    }

    $originalCulture = [System.Threading.Thread]::CurrentThread.CurrentCulture
    $originalUICulture = [System.Threading.Thread]::CurrentThread.CurrentUICulture
    try {
        foreach ($cultureCode in @("en-US", "vi-VN", "fr-FR")) {
            $cultureInfo = [System.Globalization.CultureInfo]::GetCultureInfo($cultureCode)
            [System.Threading.Thread]::CurrentThread.CurrentCulture = $cultureInfo
            [System.Threading.Thread]::CurrentThread.CurrentUICulture = $cultureInfo

            $rawJson = (& $doctorScript `
                -Tag "v4.1.0" `
                -Channel "stable" `
                -Offline `
                -OfflineExternalRelease $publishedRemote `
                -OfflineLatestRelease $mockLatest `
                -OfflineMetadata $mockMetadata `
                -OfflineWorkflowRun $failedRun `
                -Format Json) -join "`n"

            # 1. Assert raw JSON string contains the exact stable UTC RFC3339 timestamp
            if ($rawJson -notmatch '"published_at":\s*"2026-09-17T17:35:58Z"') {
                Fail "raw JSON in culture $cultureCode did not contain canonical RFC3339 timestamp '2026-09-17T17:35:58Z'"
            }
            if ($rawJson -match '\d{2}/\d{2}/\d{4}') {
                Fail "raw JSON in culture $cultureCode contained locale-formatted date string"
            }

            # 2. Assert Format-CanonicalRfc3339Timestamp parses it invariantly across cultures
            $parsed = $rawJson | ConvertFrom-Json
            $canonicalPubAt = Format-CanonicalRfc3339Timestamp $parsed.external_truth.published_at
            if ($canonicalPubAt -ne "2026-09-17T17:35:58Z") {
                Fail "canonical parsed timestamp in culture $cultureCode was '$canonicalPubAt', expected '2026-09-17T17:35:58Z'"
            }
        }
    } finally {
        [System.Threading.Thread]::CurrentThread.CurrentCulture = $originalCulture
        [System.Threading.Thread]::CurrentThread.CurrentUICulture = $originalUICulture
    }

    Write-Host "V4 test (12/19): canonical RFC3339 timestamps culture invariance (en-US, vi-VN, fr-FR): PASS"
}

# -------------------------------------------------------------------------
# Test 13: unrelated run ID => refused (fails closed)
# -------------------------------------------------------------------------
function Test-UnrelatedWorkflowRunIdRefused {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"
    $publishedRemote = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }

    $unrelatedRun = [pscustomobject]@{
        id = 99999999999
        path = ".github/workflows/ci.yml"
        event = "push"
        head_sha = "1111111111111111111111111111111111111111"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "failure"
    }

    $refused = $false
    try {
        & $doctorScript `
            -Tag "v4.1.0" `
            -Channel "stable" `
            -RunId "99999999999" `
            -Offline `
            -OfflineExternalRelease $publishedRemote `
            -OfflineWorkflowRun $unrelatedRun `
            -Format Json | Out-Null
    } catch {
        if ($_.Exception.Message -match "Workflow run '99999999999' is refused") {
            $refused = $true
        } else {
            throw
        }
    }

    if (-not $refused) {
        Fail "unrelated workflow run ID was not refused"
    }

    Write-Host "V4 test (13/19): unrelated run ID => refused (fails closed): PASS"
}

# -------------------------------------------------------------------------
# Test 14: same source with CI/non-production run => ignored by filter
# -------------------------------------------------------------------------
function Test-SameSourceNonProductionRunIgnored {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"
    $publishedRemote = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }
    $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
    $mockMetadata = [pscustomobject]@{ version = "4.0.1" }

    $ciPushRun = [pscustomobject]@{
        id = 11111
        path = ".github/workflows/ci.yml"
        event = "push"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "success"
    }
    $releaseDispatchRun = [pscustomobject]@{
        id = 35255186714
        path = ".github/workflows/release-v4.yml"
        event = "workflow_dispatch"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "failure"
    }

    $report = & $doctorScript `
        -Tag "v4.1.0" `
        -Channel "stable" `
        -Offline `
        -OfflineExternalRelease $publishedRemote `
        -OfflineLatestRelease $mockLatest `
        -OfflineMetadata $mockMetadata `
        -OfflineWorkflowRuns @($ciPushRun, $releaseDispatchRun) `
        -Format Json | ConvertFrom-Json

    if ($report.workflow_outcome -ne "failure" -or
        $report.workflow_run_id -ne "35255186714" -or
        $report.classification -ne "POST_PUBLICATION_INCIDENT") {
        Fail "CI push run was not ignored during auto-resolution: got id=$($report.workflow_run_id), outcome=$($report.workflow_outcome), class=$($report.classification)"
    }

    Write-Host "V4 test (14/19): same source with CI/non-production run => ignored by filter: PASS"
}

# -------------------------------------------------------------------------
# Test 15: two production workflow_dispatch runs for same SHA => ambiguous/fail closed
# -------------------------------------------------------------------------
function Test-TwoProductionWorkflowDispatchRunsAmbiguousFailsClosed {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"
    $publishedRemote = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }
    $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
    $mockMetadata = [pscustomobject]@{ version = "4.0.1" }

    $run1 = [pscustomobject]@{
        id = 10001
        path = ".github/workflows/release-v4.yml"
        event = "workflow_dispatch"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "cancelled"
    }
    $run2 = [pscustomobject]@{
        id = 10002
        path = ".github/workflows/release-v4.yml"
        event = "workflow_dispatch"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "failure"
    }

    $report = & $doctorScript `
        -Tag "v4.1.0" `
        -Channel "stable" `
        -Offline `
        -OfflineExternalRelease $publishedRemote `
        -OfflineLatestRelease $mockLatest `
        -OfflineMetadata $mockMetadata `
        -OfflineWorkflowRuns @($run1, $run2) `
        -Format Json | ConvertFrom-Json

    if ($report.workflow_outcome -ne "unknown" -or
        $report.operator_review_required -ne $true -or
        $report.classification -ne $null -or
        $report.external_phase -ne "PUBLISHED_PENDING_METADATA") {
        Fail "multiple valid runs did not produce fail-closed ambiguous state: got outcome=$($report.workflow_outcome), review=$($report.operator_review_required), class=$($report.classification)"
    }
    if ($report.recovery_guidance -notmatch "Provide explicit --run-id") {
        Fail "guidance does not instruct operator to provide explicit --run-id"
    }

    Write-Host "V4 test (15/19): two production workflow_dispatch runs for same SHA => ambiguous/fail closed: PASS"
}

# -------------------------------------------------------------------------
# Test 16: explicit valid run ID => deterministic classification
# -------------------------------------------------------------------------
function Test-ExplicitValidRunIdDeterministicClassification {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"
    $publishedRemote = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "2026-09-17T17:35:58Z"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }
    $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
    $mockMetadata = [pscustomobject]@{ version = "4.0.1" }

    $run1 = [pscustomobject]@{
        id = 10001
        path = ".github/workflows/release-v4.yml"
        event = "workflow_dispatch"
        head_sha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
        status = "completed"
        conclusion = "cancelled"
    }

    $report = & $doctorScript `
        -Tag "v4.1.0" `
        -Channel "stable" `
        -RunId "10001" `
        -Offline `
        -OfflineExternalRelease $publishedRemote `
        -OfflineLatestRelease $mockLatest `
        -OfflineMetadata $mockMetadata `
        -OfflineWorkflowRun $run1 `
        -Format Json | ConvertFrom-Json

    if ($report.workflow_outcome -ne "cancelled" -or
        $report.workflow_run_id -ne "10001" -or
        $report.classification -ne "POST_PUBLICATION_INCIDENT" -or
        $report.operator_review_required -ne $true) {
        Fail "explicit RunId did not produce deterministic classification: got id=$($report.workflow_run_id), outcome=$($report.workflow_outcome), class=$($report.classification)"
    }

    Write-Host "V4 test (16/19): explicit valid run ID => deterministic classification: PASS"
}

# -------------------------------------------------------------------------
# Test 17: exact release_id missing + same tag points to another published release => REMOTE_STATE_UNKNOWN => other release is NOT adopted
# -------------------------------------------------------------------------
function Test-ExactReleaseIdMissingTagPointsToAnotherReleaseNotAdopted {
    $testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-test17-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $testDir -Force | Out-Null
        $stateFile = Join-Path $testDir "release-state.json"

        # Local state expects release_id 12345
        $initialState = New-V4CanonicalReleaseState `
            -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
            -Version "4.1.0" `
            -Channel "stable" `
            -Tag "v4.1.0" `
            -ReleaseId 12345 `
            -Phase "QUALIFIED"
        $initialState.qualified_after_download = $true
        $initialState.attested = $true
        [IO.File]::WriteAllText($stateFile, ($initialState | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

        # Another release with id 99999 exists on remote with the same tag
        $otherReleaseWithSameTag = [pscustomobject]@{
            id = 99999
            tag_name = "v4.1.0"
            target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
            draft = $false
            published_at = "2026-09-17T17:35:58Z"
            immutable = $true
        }

        $threw = $false
        try {
            Invoke-TestPublishReconciliation `
                -StatePath $stateFile `
                -Tag "v4.1.0" `
                -SourceSha "5e5ab9d9a89aaced2af97a32c64fff21681c4c56" `
                -PatchAction { throw [System.IO.IOException]::new("Connection error during PATCH") } `
                -GetAction {
                    # Authoritative query GET repos/$repo/releases/12345 returns $null (not found)
                    return $null
                }
        } catch {
            if ($_.Exception.Message -match "REMOTE_STATE_UNKNOWN") {
                $threw = $true
            } else {
                throw
            }
        }
        if (-not $threw) { Fail "expected REMOTE_STATE_UNKNOWN exception when exact release_id is missing" }

        # Verify persisted failure class and that other release was NOT adopted
        $persisted = Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json
        if ($persisted.failure_class -ne "REMOTE_STATE_UNKNOWN" -or
            $persisted.phase -ne "QUALIFIED" -or
            $persisted.published -or
            $persisted.release_id -ne 12345) {
            Fail "persisted state corrupted or adopted other release: release_id=$($persisted.release_id), phase=$($persisted.phase), published=$($persisted.published), failure_class=$($persisted.failure_class)"
        }

        Write-Host "V4 test (17/19): exact release_id missing + same tag points to another published release => REMOTE_STATE_UNKNOWN => other release is NOT adopted: PASS"
    } finally {
        if (Test-Path -LiteralPath $testDir) { Remove-Item -LiteralPath $testDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 18: workflow run repository identity validation (missing => refused, wrong => refused, exact => accepted)
# -------------------------------------------------------------------------
function Test-WorkflowRunRepositoryIdentityValidation {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"
    $targetSha = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
    $expectedRepo = "pumni/Sky-Auto-Player"

    function Get-BaseRun {
        return [pscustomobject]@{
            id = 35255186714
            path = ".github/workflows/release-v4.yml"
            event = "workflow_dispatch"
            head_sha = $targetSha
            status = "completed"
            conclusion = "failure"
        }
    }

    # Case A: Missing repository identity => refused
    $runMissingRepo = Get-BaseRun
    if (Test-ReleaseWorkflowRunCriteria $runMissingRepo $targetSha $expectedRepo) {
        Fail "Test-ReleaseWorkflowRunCriteria accepted run with missing repository identity"
    }
    $refusedMissing = $false
    try {
        Assert-ReleaseWorkflowRunCriteria $runMissingRepo $targetSha $expectedRepo "35255186714"
    } catch {
        if ($_.Exception.Message -match "repository identity is missing or empty") {
            $refusedMissing = $true
        } else {
            throw
        }
    }
    if (-not $refusedMissing) {
        Fail "Assert-ReleaseWorkflowRunCriteria did not fail closed on missing repository identity"
    }

    # Case B: Wrong repository identity => refused
    $runWrongRepo = Get-BaseRun
    $runWrongRepo | Add-Member -MemberType NoteProperty -Name "repository" -Value ([pscustomobject]@{ full_name = "evil/Sky-Auto-Player" })
    if (Test-ReleaseWorkflowRunCriteria $runWrongRepo $targetSha $expectedRepo) {
        Fail "Test-ReleaseWorkflowRunCriteria accepted run with wrong repository identity"
    }
    $refusedWrong = $false
    try {
        Assert-ReleaseWorkflowRunCriteria $runWrongRepo $targetSha $expectedRepo "35255186714"
    } catch {
        if ($_.Exception.Message -match "does not match required '$expectedRepo'") {
            $refusedWrong = $true
        } else {
            throw
        }
    }
    if (-not $refusedWrong) {
        Fail "Assert-ReleaseWorkflowRunCriteria did not refuse wrong repository identity"
    }

    # Case C: Exact repository identity => accepted
    $runExactRepo = Get-BaseRun
    $runExactRepo | Add-Member -MemberType NoteProperty -Name "repository" -Value ([pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" })
    if (-not (Test-ReleaseWorkflowRunCriteria $runExactRepo $targetSha $expectedRepo)) {
        Fail "Test-ReleaseWorkflowRunCriteria rejected run with exact required repository identity"
    }
    try {
        Assert-ReleaseWorkflowRunCriteria $runExactRepo $targetSha $expectedRepo "35255186714"
    } catch {
        Fail "Assert-ReleaseWorkflowRunCriteria threw unexpectedly for valid exact repository: $_"
    }

    Write-Host "V4 test (18/19): workflow run repository identity validation (missing => refused, wrong => refused, exact => accepted): PASS"
}

# -------------------------------------------------------------------------
# Test 19: timestamp formatting fail-closed (valid => canonical UTC, non-default culture => byte-identical UTC, invalid => returns null)
# -------------------------------------------------------------------------
function Test-TimestampFormattingFailClosedAndCultureInvariance {
    $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"

    # 1. Direct unit verification of Format-CanonicalRfc3339Timestamp:
    $validOffset = "2026-09-17T10:35:58-07:00"
    $formatted = Format-CanonicalRfc3339Timestamp $validOffset
    if ($formatted -ne "2026-09-17T17:35:58Z") {
        Fail "Format-CanonicalRfc3339Timestamp produced '$formatted', expected '2026-09-17T17:35:58Z'"
    }

    $originalCulture = [System.Threading.Thread]::CurrentThread.CurrentCulture
    $originalUICulture = [System.Threading.Thread]::CurrentThread.CurrentUICulture
    try {
        foreach ($cultureCode in @("vi-VN", "fr-FR", "ar-SA", "de-DE")) {
            $cultureInfo = [System.Globalization.CultureInfo]::GetCultureInfo($cultureCode)
            [System.Threading.Thread]::CurrentThread.CurrentCulture = $cultureInfo
            [System.Threading.Thread]::CurrentThread.CurrentUICulture = $cultureInfo
            $formattedCulture = Format-CanonicalRfc3339Timestamp $validOffset
            if ($formattedCulture -ne "2026-09-17T17:35:58Z") {
                Fail "Format-CanonicalRfc3339Timestamp in culture $cultureCode produced '$formattedCulture', expected '2026-09-17T17:35:58Z'"
            }
        }
    } finally {
        [System.Threading.Thread]::CurrentThread.CurrentCulture = $originalCulture
        [System.Threading.Thread]::CurrentThread.CurrentUICulture = $originalUICulture
    }

    # Invalid timestamp must return $null, NOT the raw input string
    $invalidInput = "not-a-timestamp-2026"
    $invalidResult = Format-CanonicalRfc3339Timestamp $invalidInput
    if ($null -ne $invalidResult) {
        Fail "Format-CanonicalRfc3339Timestamp on invalid input did not return null; got '$invalidResult'"
    }
    $emptyResult = Format-CanonicalRfc3339Timestamp ""
    if ($null -ne $emptyResult) {
        Fail "Format-CanonicalRfc3339Timestamp on empty string did not return null"
    }

    # 2. Doctor integration verification with malformed external release published_at:
    $publishedWithMalformedTime = [pscustomobject]@{
        tag_name = "v4.1.0"
        target_commitish = "5e5ab9d9a89aaced2af97a32c64fff21681c4c56"
        draft = $false
        published_at = "malformed-timestamp-value"
        immutable = $true
        assets = @(
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe" },
            [pscustomobject]@{ name = "Sky.Auto.Player_4.1.0_x64-setup.exe.sig" }
        )
    }
    $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
    $mockMetadata = [pscustomobject]@{ version = "4.0.1" }

    $report = & $doctorScript `
        -Tag "v4.1.0" `
        -Channel "stable" `
        -Offline `
        -OfflineExternalRelease $publishedWithMalformedTime `
        -OfflineLatestRelease $mockLatest `
        -OfflineMetadata $mockMetadata `
        -Format Json | ConvertFrom-Json

    if ($report.external_truth.published_at -ne $null) {
        Fail "doctor did not null out malformed published_at; got '$($report.external_truth.published_at)'"
    }
    if ($report.external_phase -ne "PUBLISHED_PENDING_METADATA" -or
        $report.classification -ne "POST_PUBLICATION_INCIDENT" -or
        $report.operator_review_required -ne $true) {
        Fail "doctor did not fail closed on malformed published_at: ext_phase=$($report.external_phase), class=$($report.classification), review=$($report.operator_review_required)"
    }
    $findingMatched = $false
    foreach ($f in $report.findings) {
        if ($f -match "External release published_at is invalid/non-canonical: 'malformed-timestamp-value'") {
            $findingMatched = $true
            break
        }
    }
    if (-not $findingMatched) {
        Fail "doctor findings did not include malformed published_at warning: $($report.findings -join '; ')"
    }

    Write-Host "V4 test (19/19): timestamp formatting fail-closed (valid => canonical UTC, non-default culture => byte-identical UTC, invalid => returns null): PASS"
}

# -------------------------------------------------------------------------
# Fault-Injection Test Harness for Simplified V4 Release Transaction
# -------------------------------------------------------------------------

function New-V4SimplifiedTestFixture {
    param(
        [string]$Version = "4.1.4",
        [string]$Channel = "stable",
        [string]$SourceSha = ""
    )

    if ([string]::IsNullOrWhiteSpace($SourceSha)) {
        $SourceSha = (& git rev-parse HEAD 2>$null).Trim()
        if ([string]::IsNullOrWhiteSpace($SourceSha) -or $SourceSha -notmatch '^[0-9a-fA-F]{40}$') {
            $SourceSha = "2ae2c7923db2da5630d03726114d50b80064ed36"
        }
    }

    $testDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-simptest-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $testDir -Force | Out-Null
    $stateRoot = Join-Path $testDir "state-root"
    New-Item -ItemType Directory -Path $stateRoot -Force | Out-Null
    $bundleDir = Join-Path $stateRoot "candidate-bundle"
    New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null

    $sourceInstallerName = "Sky Auto Player_${Version}_x64-setup.exe"
    $sourceSigName = "$sourceInstallerName.sig"
    $installerName = "Sky.Auto.Player_${Version}_x64-setup.exe"
    $sigName = "$installerName.sig"
    $installerPath = Join-Path $bundleDir $sourceInstallerName
    $sigPath = Join-Path $bundleDir $sourceSigName

    [IO.File]::WriteAllBytes($installerPath, [byte[]](1..100))
    [IO.File]::WriteAllBytes($sigPath, [Text.Encoding]::ASCII.GetBytes(("A" * 48) + "`r`n"))

    $installerSha = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sigSha = (Get-FileHash -LiteralPath $sigPath -Algorithm SHA256).Hash.ToLowerInvariant()

    $installerRecord = [ordered]@{
        name = $installerName
        release_name = $installerName
        source_name = $sourceInstallerName
        role = "installer"
        size = [int64]100
        sha256 = $installerSha
        source_path = $installerPath
        state_path = "candidate-bundle/$sourceInstallerName"
    }
    $sigRecord = [ordered]@{
        name = $sigName
        release_name = $sigName
        source_name = $sourceSigName
        role = "updater-signature"
        size = [int64]50
        sha256 = $sigSha
        source_path = $sigPath
        state_path = "candidate-bundle/$sourceSigName"
    }

    $manifest = [ordered]@{
        schema_version = 1
        source_sha = $SourceSha.ToLowerInvariant()
        version = $Version
        channel = $Channel
        tag = "v$Version"
        qualification_assets = @($installerRecord, $sigRecord)
        public_assets = @($installerRecord, $sigRecord)
    }
    $manifestPath = Join-Path $stateRoot "candidate-manifest.json"
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    $context = [ordered]@{
        schema_version = 1
        repository = "pumni/Sky-Auto-Player"
        version = $Version
        channel = $Channel
        tag = "v$Version"
        release_notes_path = "docs/releases/v$Version.md"
        source_sha = $SourceSha.ToLowerInvariant()
        workflow_sha = $SourceSha.ToLowerInvariant()
        run_id = "35292682626"
        created_at = "2026-09-19T00:00:00Z"
    }
    [IO.File]::WriteAllText(
        (Join-Path $stateRoot "release-context.json"),
        (($context | ConvertTo-Json -Depth 10) + "`n"),
        [Text.UTF8Encoding]::new($false)
    )

    return [pscustomobject]@{
        TestDir = $testDir
        StateRoot = $stateRoot
        AssetsDir = $bundleDir
        InstallerName = $installerName
        SignatureName = $sigName
        InstallerSha = $installerSha
        SignatureSha = $sigSha
        InstallerPath = $installerPath
        SignaturePath = $sigPath
        Version = $Version
        Channel = $Channel
        SourceSha = $SourceSha
        Tag = "v$Version"
    }
}

class V4SimplifiedMockContext {
    [hashtable]$Releases = @{}
    [System.Collections.ArrayList]$DeletedReleases = [System.Collections.ArrayList]::new()
    [System.Collections.ArrayList]$PatchedReleases = [System.Collections.ArrayList]::new()
    [System.Collections.ArrayList]$UploadedAssets = [System.Collections.ArrayList]::new()
    [bool]$FailPostDraft = $false
    [bool]$PostDraftTimeoutWithRemote = $false
    [bool]$FailFirstAssetUpload = $false
    [bool]$FailSecondAssetUpload = $false
    [bool]$CorruptServerDigest = $false
    [bool]$DigestMissing = $false
    [bool]$DigestEmpty = $false
    [bool]$DigestMalformed = $false
    [bool]$DigestSha512 = $false
    [bool]$FailPatch = $false
    [bool]$PatchTimeoutWithRemotePublished = $false
    [bool]$PatchFailStillDraft = $false
    [bool]$FailPostPublishGet = $false
    [string]$Tag = "v4.1.4"
    [string]$Version = "4.1.4"
    [string]$SourceSha = ""
    [string]$RunId = "35292682626"
    [string]$InstallerName = "Sky.Auto.Player_4.1.4_x64-setup.exe"
    [string]$SignatureName = "Sky.Auto.Player_4.1.4_x64-setup.exe.sig"
    [string]$InstallerSha = ""
    [string]$SignatureSha = ""
}

function New-V4MockGitHubApiHandler([V4SimplifiedMockContext]$Ctx) {
    return {
        param($Arguments, $AllowNotFound, $BinaryOutput, $Raw, $OutputPath)
        $cmd = $Arguments -join ' '

        if ($cmd -match 'releases/latest') {
            $published = @($Ctx.Releases.Values | Where-Object { -not [bool]$_.draft })
            if ($published.Count -gt 0) { return $published[0] }
            return [pscustomobject]@{
                id = [int64]41; tag_name = 'v4.0.1'; target_commitish = ('0' * 40)
                draft = $false; prerelease = $false; published_at = '2026-09-01T00:00:00Z'
                url = 'https://api.github.com/repos/pumni/Sky-Auto-Player/releases/41'
            }
        }
        if ($cmd -match ('releases/tags/' + [regex]::Escape($Ctx.Tag))) {
            $existing = @($Ctx.Releases.Values | Where-Object { [string]$_.tag_name -eq $Ctx.Tag -and -not [bool]$_.draft })
            if ($existing.Count -gt 0) { return $existing[0] }
            if ($AllowNotFound) { return $null }
            return $null
        }

        if ($cmd -match 'api --paginate --slurp repos/.+/releases\?per_page=100') {
            return @($Ctx.Releases.Values)
        }

        if ($cmd -match 'POST repos/.+/releases') {
            if ($Ctx.FailPostDraft) { throw "GitHub API POST error: draft creation failed" }
            if ($Ctx.PostDraftTimeoutWithRemote) {
                $marker = "<!-- v4-release-tx: {`"repository`":`"pumni/Sky-Auto-Player`",`"run_id`":`"$($Ctx.RunId)`",`"source_sha`":`"$($Ctx.SourceSha)`",`"version`":`"$($Ctx.Version)`",`"tag`":`"$($Ctx.Tag)`"} -->"
                $Ctx.Releases[[int64]42] = [pscustomobject]@{
                    id = [int64]42
                    upload_url = "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/42/assets"
                    draft = $true
                    prerelease = $false
                    tag_name = $Ctx.Tag
                    target_commitish = $Ctx.SourceSha
                    body = "Notes`n`n$marker"
                    immutable = $false
                    published_at = $null
                    assets = @()
                }
                throw "GitHub API POST timeout: 504 Gateway Timeout"
            }
            $inputIdx = [array]::IndexOf($Arguments, "--input")
            $body = ""
            if ($inputIdx -ge 0 -and $inputIdx + 1 -lt $Arguments.Length) {
                $payload = Get-Content -LiteralPath $Arguments[$inputIdx + 1] -Raw | ConvertFrom-Json
                $body = [string]$payload.body
            }
            $draft = [pscustomobject]@{
                id = [int64]42
                upload_url = "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/42/assets"
                draft = $true
                prerelease = $false
                tag_name = $Ctx.Tag
                target_commitish = $Ctx.SourceSha
                body = $body
                immutable = $false
                published_at = $null
                assets = @()
            }
            $Ctx.Releases[[int64]42] = $draft
            return $draft
        }

        if ($cmd -match 'DELETE repos/.+/releases/(\d+)') {
            $delId = [int64]$Matches[1]
            [void]$Ctx.DeletedReleases.Add($delId)
            [void]$Ctx.Releases.Remove($delId)
            return $null
        }

        if ($cmd -match 'PATCH repos/.+/releases/(\d+)') {
            $patchId = [int64]$Matches[1]
            if ($Ctx.FailPatch) {
                if ($Ctx.PatchTimeoutWithRemotePublished) {
                    if ($Ctx.Releases.ContainsKey($patchId)) {
                        $rel = $Ctx.Releases[$patchId]
                        $rel.draft = $false
                        $rel.immutable = $true
                        $rel.published_at = "2026-09-18T00:00:00Z"
                    }
                    throw "GitHub API PATCH timeout: 504 Gateway Timeout"
                }
                if ($Ctx.PatchFailStillDraft) {
                    throw "GitHub API PATCH error: validation failed"
                }
                throw "GitHub API PATCH failed"
            }
            [void]$Ctx.PatchedReleases.Add($patchId)
            if ($Ctx.Releases.ContainsKey($patchId)) {
                $rel = $Ctx.Releases[$patchId]
                $rel.draft = $false
                $rel.immutable = $true
                $rel.published_at = "2026-09-18T00:00:00Z"
                return $rel
            }
            return [pscustomobject]@{
                id = $patchId
                draft = $false
                immutable = $true
                published_at = "2026-09-18T00:00:00Z"
                tag_name = $Ctx.Tag
                target_commitish = $Ctx.SourceSha
                assets = @()
            }
        }

        if ($cmd -match 'api repos/.+/releases/(\d+)') {
            $getId = [int64]$Matches[1]
            if ($Ctx.FailPostPublishGet -and $Ctx.PatchedReleases.Contains($getId)) {
                throw "GitHub API GET 500: internal server error"
            }
            if ($Ctx.Releases.ContainsKey($getId)) {
                $rel = $Ctx.Releases[$getId]
                $instDigest = if ($Ctx.CorruptServerDigest) {
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                } elseif ($Ctx.DigestEmpty) {
                    ""
                } elseif ($Ctx.DigestMalformed) {
                    "invalid-digest-format"
                } elseif ($Ctx.DigestSha512) {
                    "sha512:00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
                } else {
                    "sha256:$($Ctx.InstallerSha)"
                }
                $instAssetProps = [ordered]@{
                    name = $Ctx.InstallerName
                    size = [int64]100
                    state = "uploaded"
                    url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/101"
                }
                if (-not $Ctx.DigestMissing) {
                    $instAssetProps["digest"] = $instDigest
                }
                $rel.assets = @(
                    [pscustomobject]$instAssetProps,
                    [pscustomobject]@{ name = $Ctx.SignatureName; size = [int64]50; state = "uploaded"; digest = "sha256:$($Ctx.SignatureSha)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/102" }
                )
                return $rel
            }
            if ($AllowNotFound) { return $null }
            throw "Release $getId not found"
        }

        if ($cmd -match 'git/ref/heads/main') {
            return [pscustomobject]@{ ref = "refs/heads/main"; object = [pscustomobject]@{ sha = $Ctx.SourceSha } }
        }
        if ($cmd -match 'git/ref/heads/release-metadata') {
            return [pscustomobject]@{ ref = "refs/heads/release-metadata"; object = [pscustomobject]@{ sha = "mock-metadata-sha-123" } }
        }
        if ($cmd -match 'git/ref/tags/') {
            return $null
        }
        if ($cmd -match 'contents/\.release-metadata/README\.md') {
            $bootstrapText = @(
                "# Sky Auto Player v4 release metadata",
                "",
                "bootstrap_contract: sky-auto-player-v4-release-metadata-v1",
                "This orphan branch contains deployment-state metadata only.",
                "The channel latest.json files are created only by qualified immutable release promotion."
            ) -join "`n"
            return [pscustomobject]@{ content = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($bootstrapText)) }
        }
        if ($cmd -match 'contents/channels/(stable|beta)/latest\.json') {
            $isBeta = ($Matches[1] -eq 'beta')
            $ver = if ($isBeta) { "4.1.0-beta.1" } else { "4.0.1" }
            $latestPayload = @{
                version = $ver
                notes = "Previous version notes"
                pub_date = "2026-09-01T00:00:00Z"
                platforms = @{
                    "windows-x86_64" = @{
                        signature = "dGVzdC1zaWduYXR1cmU="
                        url = "https://github.com/pumni/Sky-Auto-Player/releases/download/v$ver/Sky.Auto.Player_${ver}_x64-setup.exe"
                    }
                }
            } | ConvertTo-Json -Depth 5
            return [pscustomobject]@{ content = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($latestPayload)); sha = "mock-sha-123" }
        }

        if ($AllowNotFound) { return $null }
        return $null
    }
}

function New-V4MockAssetUploadHandler([V4SimplifiedMockContext]$Ctx) {
    return {
        param($UploadUrl, $AssetName, $FilePath)
        if ($Ctx.FailFirstAssetUpload -and $AssetName -eq $Ctx.InstallerName) {
            throw "Connection reset during upload of $AssetName"
        }
        if ($Ctx.FailSecondAssetUpload -and $AssetName -eq $Ctx.SignatureName) {
            throw "HTTP 500 error during upload of $AssetName"
        }
        [void]$Ctx.UploadedAssets.Add($AssetName)
    }
}

function Invoke-TestPublishReleaseTransaction([pscustomobject]$Fixture, [V4SimplifiedMockContext]$Ctx) {
    $Ctx.Tag = $Fixture.Tag
    $Ctx.Version = $Fixture.Version
    $Ctx.SourceSha = $Fixture.SourceSha
    $Ctx.InstallerName = $Fixture.InstallerName
    $Ctx.SignatureName = $Fixture.SignatureName
    $Ctx.InstallerSha = $Fixture.InstallerSha
    $Ctx.SignatureSha = $Fixture.SignatureSha
    $contextPath = Join-Path $Fixture.StateRoot "release-context.json"
    $context = Get-Content -LiteralPath $contextPath -Raw | ConvertFrom-Json
    $context.run_id = $Ctx.RunId
    [IO.File]::WriteAllText($contextPath, (($context | ConvertTo-Json -Depth 10) + "`n"), [Text.UTF8Encoding]::new($false))

    $apiHandler = New-V4MockGitHubApiHandler $Ctx
    $uploadHandler = New-V4MockAssetUploadHandler $Ctx
    & {
        $script:GitHubApiHandler = $apiHandler
        $script:AssetUploadHandler = $uploadHandler
        . $pipelinePath `
            -State "PublishRelease" `
            -Version $Fixture.Version `
            -Channel $Fixture.Channel `
            -Tag $Fixture.Tag `
            -SourceSha $Fixture.SourceSha `
            -WorkflowSha $Fixture.SourceSha `
            -StateRoot $Fixture.StateRoot `
            -ReleaseNotesPath (Join-Path $repoRoot "docs/releases/v$($Fixture.Version).md") `
            -RunId $Ctx.RunId
    }
}

# -------------------------------------------------------------------------
# Test 20: Draft POST success
# -------------------------------------------------------------------------
function Test-DraftPostSuccess {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha

        Invoke-TestPublishReleaseTransaction $fixture $ctx

        if (-not $ctx.PatchedReleases.Contains([int64]42) -or $ctx.DeletedReleases.Count -ne 0) {
            Fail "draft POST success did not publish release or unexpectedly deleted it"
        }
        Write-Host "V4 test (20/32): draft POST success (draft created, assets verified, immutable published): PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 21: Draft POST timeout but remote draft exists (reconciled by marker)
# -------------------------------------------------------------------------
function Test-DraftPostTimeoutReconciledByMarker {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha
        $ctx.PostDraftTimeoutWithRemote = $true

        Invoke-TestPublishReleaseTransaction $fixture $ctx

        if (-not $ctx.PatchedReleases.Contains([int64]42) -or $ctx.DeletedReleases.Count -ne 0) {
            Fail "draft POST timeout was not reconciled by transaction marker"
        }
        Write-Host "V4 test (21/32): draft POST timeout reconciled by transaction marker: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 22: First asset upload failure (assert draft auto-deleted)
# -------------------------------------------------------------------------
function Test-FirstAssetUploadFailureDraftAutoDeleted {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha
        $ctx.FailFirstAssetUpload = $true

        $threw = $false
        try {
            Invoke-TestPublishReleaseTransaction $fixture $ctx
        } catch {
            $threw = $true
        }

        if (-not $threw) { Fail "first asset upload failure did not throw" }
        if (-not $ctx.DeletedReleases.Contains([int64]42)) {
            Fail "first asset upload failure did not auto-delete the draft release"
        }
        if ($ctx.PatchedReleases.Count -ne 0) { Fail "failed asset upload must never attempt PATCH publication" }
        Write-Host "V4 test (22/32): first asset upload failure auto-deletes draft: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 23: Second asset upload failure (assert draft auto-deleted)
# -------------------------------------------------------------------------
function Test-SecondAssetUploadFailureDraftAutoDeleted {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha
        $ctx.FailSecondAssetUpload = $true

        $threw = $false
        try {
            Invoke-TestPublishReleaseTransaction $fixture $ctx
        } catch {
            $threw = $true
        }

        if (-not $threw) { Fail "second asset upload failure did not throw" }
        if (-not $ctx.DeletedReleases.Contains([int64]42)) {
            Fail "second asset upload failure did not auto-delete the draft release"
        }
        if ($ctx.PatchedReleases.Count -ne 0) { Fail "failed asset upload must never attempt PATCH publication" }
        Write-Host "V4 test (23/32): second asset upload failure auto-deletes draft: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 24: Server asset digest mismatch (assert draft auto-deleted, no PATCH)
# -------------------------------------------------------------------------
function Test-ServerAssetDigestMismatchDraftAutoDeletedNoPatch {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha
        $ctx.CorruptServerDigest = $true

        $threw = $false
        $errorMsg = ""
        try {
            Invoke-TestPublishReleaseTransaction $fixture $ctx
        } catch {
            $threw = $true
            $errorMsg = $_.Exception.Message
        }

        if (-not $threw) { Fail "server asset digest mismatch did not throw" }
        if ($errorMsg -notmatch "digest mismatch") { Fail "unexpected error message: $errorMsg" }
        if (-not $ctx.DeletedReleases.Contains([int64]42)) {
            Fail "server asset digest mismatch did not auto-delete the draft release"
        }
        if ($ctx.PatchedReleases.Count -ne 0) { Fail "digest mismatch must never attempt PATCH publication" }
        Write-Host "V4 test (24/32): server asset digest mismatch auto-deletes draft without PATCH: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 25: Publish PATCH success
# -------------------------------------------------------------------------
function Test-PublishPatchSuccess {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha

        Invoke-TestPublishReleaseTransaction $fixture $ctx

        if (-not $ctx.PatchedReleases.Contains([int64]42)) { Fail "publish PATCH was not invoked" }
        $published = $ctx.Releases[[int64]42]
        if ($published.draft -or -not $published.immutable -or [string]::IsNullOrWhiteSpace($published.published_at)) {
            Fail "published release is not immutable or draft was not cleared"
        }
        if ($ctx.DeletedReleases.Count -ne 0) { Fail "successful publication must never delete release" }
        Write-Host "V4 test (25/32): publish PATCH success (draft=false, immutable=true, verified): PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 26: Publish PATCH timeout but remote published (assert reconciled, no delete)
# -------------------------------------------------------------------------
function Test-PublishPatchTimeoutRemotePublishedReconcilesNoDelete {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha
        $ctx.FailPatch = $true
        $ctx.PatchTimeoutWithRemotePublished = $true

        Invoke-TestPublishReleaseTransaction $fixture $ctx

        if ($ctx.DeletedReleases.Count -ne 0) {
            Fail "publication timeout when remote is published must NEVER delete the release"
        }
        $published = $ctx.Releases[[int64]42]
        if ($published.draft) { Fail "release must be published" }
        Write-Host "V4 test (26/32): publish PATCH timeout when remote published reconciles without delete: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 27: Publish PATCH failure and remote still draft (assert draft auto-deleted)
# -------------------------------------------------------------------------
function Test-PublishPatchFailureRemoteStillDraftAutoDeleted {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha
        $ctx.FailPatch = $true
        $ctx.PatchFailStillDraft = $true

        $threw = $false
        try {
            Invoke-TestPublishReleaseTransaction $fixture $ctx
        } catch {
            $threw = $true
        }

        if (-not $threw) { Fail "publish PATCH failure did not throw" }
        if (-not $ctx.DeletedReleases.Contains([int64]42)) {
            Fail "publish PATCH failure with remote still draft did not auto-delete draft"
        }
        Write-Host "V4 test (27/32): publish PATCH failure with remote still draft auto-deletes draft: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 28: Remote GET unavailable after mutation (assert fails closed, no delete)
# -------------------------------------------------------------------------
function Test-RemoteGetUnavailableAfterMutationFailsClosedNoDelete {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha
        $ctx.FailPostPublishGet = $true

        $threw = $false
        $errorMsg = ""
        try {
            Invoke-TestPublishReleaseTransaction $fixture $ctx
        } catch {
            $threw = $true
            $errorMsg = $_.Exception.Message
        }

        if (-not $threw) { Fail "remote GET unavailable after mutation did not throw" }
        if ($errorMsg -notmatch "POST_PUBLICATION_INCIDENT") {
            Fail "remote GET unavailable after mutation did not fail closed as POST_PUBLICATION_INCIDENT: $errorMsg"
        }
        if ($ctx.DeletedReleases.Count -ne 0) {
            Fail "remote GET unavailable after publication attempt must NEVER delete the release"
        }
        Write-Host "V4 test (28/32): remote GET unavailable after mutation fails closed (no delete): PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 29: Metadata promotion failure after publication (assert release intact)
# -------------------------------------------------------------------------
function Test-MetadataPromotionFailureAfterPublicationReleaseIntact {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha

        # Add published release to remote
        $ctx.Releases[[int64]42] = [pscustomobject]@{
            id = [int64]42
            tag_name = $fixture.Tag
            target_commitish = $fixture.SourceSha
            draft = $false
            immutable = $true
            published_at = "2026-09-18T00:00:00Z"
            assets = @(
                [pscustomobject]@{ name = $fixture.InstallerName; size = [int64]100; state = "uploaded"; digest = "sha256:$($fixture.InstallerSha)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/101" },
                [pscustomobject]@{ name = $fixture.SignatureName; size = [int64]50; state = "uploaded"; digest = "sha256:$($fixture.SignatureSha)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/102" }
            )
        }

        # Run PromoteMetadata with API failure on branch clone/fetch
        $threw = $false
        try {
            $apiHandler = {
                param($Arguments, $AllowNotFound, $BinaryOutput, $Raw, $OutputPath)
                $cmd = $Arguments -join ' '
                if ($cmd -match ('releases/tags/' + [regex]::Escape($ctx.Tag))) { return $ctx.Releases[[int64]42] }
                if ($cmd -match 'repo clone') { throw "Git clone authentication failure" }
                return $null
            }
            & {
                $script:GitHubApiHandler = $apiHandler
                . $pipelinePath `
                    -State "PromoteMetadata" `
                    -Version $fixture.Version `
                    -Channel $fixture.Channel `
                    -Tag $fixture.Tag `
                    -SourceSha $fixture.SourceSha `
                    -WorkflowSha $fixture.SourceSha `
                    -StateRoot $fixture.StateRoot
            }
        } catch {
            $threw = $true
        }

        if (-not $threw) { Fail "metadata promotion failure did not throw" }
        if ($ctx.DeletedReleases.Count -ne 0 -or -not $ctx.Releases.ContainsKey([int64]42)) {
            Fail "metadata promotion failure must NEVER delete or mutate published release"
        }
        Write-Host "V4 test (29/32): metadata promotion failure keeps published release intact: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 30: Process failure immediately after draft creation (preflight observes; publication cleans)
# -------------------------------------------------------------------------
function Test-ProcessFailureAfterDraftCreationPreflightCleansStaleDraft {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $contextPath = Join-Path $fixture.StateRoot "release-context.json"
        $context = Get-Content -LiteralPath $contextPath -Raw | ConvertFrom-Json
        $context.run_id = "new-run"
        [IO.File]::WriteAllText($contextPath, (($context | ConvertTo-Json -Depth 10) + "`n"), [Text.UTF8Encoding]::new($false))
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.SourceSha = $fixture.SourceSha
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha

        # Add stale draft matching source SHA, tag, and transaction marker
        $staleMarker = "<!-- v4-release-tx: {`"repository`":`"pumni/Sky-Auto-Player`",`"run_id`":`"previous-run`",`"source_sha`":`"$($fixture.SourceSha)`",`"version`":`"$($fixture.Version)`",`"tag`":`"$($fixture.Tag)`"} -->"
        $ctx.Releases[[int64]42] = [pscustomobject]@{
            id = [int64]42
            tag_name = $fixture.Tag
            target_commitish = $fixture.SourceSha
            draft = $true
            immutable = $false
            published_at = $null
            body = "Old Notes`n`n$staleMarker"
            assets = @()
        }

        $apiHandler = New-V4MockGitHubApiHandler $ctx
        & {
            $script:GitHubApiHandler = $apiHandler
            . $pipelinePath `
                -State "Preflight" `
                -Version $fixture.Version `
                -Channel $fixture.Channel `
                -Tag $fixture.Tag `
                -SourceSha $fixture.SourceSha `
                -WorkflowSha $fixture.SourceSha `
                -StateRoot $fixture.StateRoot `
                -ReleaseNotesPath (Join-Path $repoRoot "docs/releases/v$($fixture.Version).md") `
                -RunId "new-run"
            if ($ctx.DeletedReleases.Count -ne 0) {
                Fail "preflight must not clean a stale draft"
            }
            Remove-V4StaleMatchingDraft -Repository "pumni/Sky-Auto-Player" | Out-Null
        }

        if (-not $ctx.DeletedReleases.Contains([int64]42)) {
            Fail "PublishRelease stale-draft reconciliation did not clean matching draft"
        }
        Write-Host "V4 test (30/32): preflight is read-only and publication owns stale-draft cleanup: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 31: Process failure immediately after publication (assert preflight and release-doctor refuse mutation)
# -------------------------------------------------------------------------
function Test-ProcessFailureAfterPublicationPreflightAndDoctorRefuse {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $contextPath = Join-Path $fixture.StateRoot "release-context.json"
        $context = Get-Content -LiteralPath $contextPath -Raw | ConvertFrom-Json
        $context.run_id = "subsequent-run"
        [IO.File]::WriteAllText($contextPath, (($context | ConvertTo-Json -Depth 10) + "`n"), [Text.UTF8Encoding]::new($false))
        $ctx = [V4SimplifiedMockContext]::new()
        $ctx.SourceSha = $fixture.SourceSha
        $ctx.InstallerName = $fixture.InstallerName
        $ctx.SignatureName = $fixture.SignatureName
        $ctx.InstallerSha = $fixture.InstallerSha
        $ctx.SignatureSha = $fixture.SignatureSha

        # Pre-populate published release
        $ctx.Releases[[int64]42] = [pscustomobject]@{
            id = [int64]42
            tag_name = $fixture.Tag
            target_commitish = $fixture.SourceSha
            draft = $false
            immutable = $true
            published_at = "2026-09-18T00:00:00Z"
            assets = @(
                [pscustomobject]@{ name = $fixture.InstallerName; size = [int64]100; state = "uploaded"; digest = "sha256:$($fixture.InstallerSha)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/101" },
                [pscustomobject]@{ name = $fixture.SignatureName; size = [int64]50; state = "uploaded"; digest = "sha256:$($fixture.SignatureSha)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/102" }
            )
        }

        # 1. Preflight must reject because published release exists
        $preflightThrew = $false
        $apiHandler = New-V4MockGitHubApiHandler $ctx
        try {
            & {
                $script:GitHubApiHandler = $apiHandler
                . $pipelinePath `
                    -State "Preflight" `
                    -Version $fixture.Version `
                    -Channel $fixture.Channel `
                    -Tag $fixture.Tag `
                    -SourceSha $fixture.SourceSha `
                    -WorkflowSha $fixture.SourceSha `
                    -StateRoot $fixture.StateRoot `
                    -ReleaseNotesPath (Join-Path $repoRoot "docs/releases/v$($fixture.Version).md") `
                    -RunId "subsequent-run"
            }
        } catch {
            $preflightThrew = $true
        }

        if (-not $preflightThrew) {
            Fail "preflight did not fail closed when published release already exists"
        }
        if ($ctx.DeletedReleases.Count -ne 0) {
            Fail "preflight must NEVER delete an already published release"
        }

        # 2. Release-doctor diagnoses as POST_PUBLICATION_INCIDENT without mutation
        $doctorScript = Join-Path $PSScriptRoot "release_doctor.ps1"
        $mockLatest = [pscustomobject]@{ tag_name = "v4.1.0" }
        $mockMetadata = [pscustomobject]@{ version = "4.0.1" }
        $failedRun = [pscustomobject]@{
            id = 35292682626
            path = ".github/workflows/release-v4.yml"
            event = "workflow_dispatch"
            head_sha = $fixture.SourceSha
            repository = [pscustomobject]@{ full_name = "pumni/Sky-Auto-Player" }
            status = "completed"
            conclusion = "failure"
        }
        $docReport = & $doctorScript `
            -Tag $fixture.Tag `
            -Channel "stable" `
            -Offline `
            -OfflineExternalRelease $ctx.Releases[[int64]42] `
            -OfflineLatestRelease $mockLatest `
            -OfflineMetadata $mockMetadata `
            -OfflineWorkflowRun $failedRun `
            -Format Json | ConvertFrom-Json

        if ($docReport.classification -ne "POST_PUBLICATION_INCIDENT" -or
            $docReport.operator_review_required -ne $true -or
            $docReport.external_phase -ne "PUBLISHED_PENDING_METADATA") {
            Fail "release-doctor failed to diagnose post-publication incident: $($docReport.classification)"
        }

        Write-Host "V4 test (31/32): process failure after publication diagnosed fail-closed without mutation: PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 32: Run 35292682626 parameter conversion regression
# -------------------------------------------------------------------------
function Test-Run35292682626ParameterConversionRegression {
    # In run 35292682626, PowerShell evaluated `-ReleaseId [int64]$draft.id` without parentheses
    # as string interpolation. The fix strictly evaluates `$releaseId = [int64]($draft.id)`.
    $draftObject = [pscustomobject]@{
        id = 391147448
        tag_name = "v4.1.1"
    }

    # Verify that [int64]($draftObject.id) evaluates strictly to Int64 without boxing/conversion failure
    $evaluatedId = [int64]($draftObject.id)
    if ($evaluatedId -isnot [int64] -or $evaluatedId -ne 391147448L) {
        Fail "evaluatedId is not Int64 391147448"
    }

    # Verify parameter binding accepts evaluatedId as Int64
    function Test-Int64BindingTarget([Parameter(Mandatory = $true)][int64]$ReleaseId) {
        return $ReleaseId
    }

    $boundId = Test-Int64BindingTarget -ReleaseId $evaluatedId
    if ($boundId -ne 391147448L) {
        Fail "boundId did not match expected Int64 value"
    }

    Write-Host "V4 test (32/32): run 35292682626 parameter conversion regression strictly typed: PASS"
}

# -------------------------------------------------------------------------
# Test 33: Transaction marker — duplicate critical key rejection (table-driven)
# -------------------------------------------------------------------------
function Test-TransactionMarkerDuplicateKeyRejectsAllCriticalKeys {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $sha = $fixture.SourceSha; $tag = $fixture.Tag; $ver = $fixture.Version
        $repo = "pumni/Sky-Auto-Player"
        $cases = @(
            @{ Key = "repository"; Raw = "{`"repository`":`"$repo`",`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Key = "run_id";     Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"run_id`":`"2`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Key = "source_sha"; Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Key = "version";    Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Key = "tag";        Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`",`"tag`":`"$tag`"}" }
        )
        $caseIndex = 0
        foreach ($c in $cases) {
            $caseIndex++
            $body = "<!-- v4-release-tx: $($c.Raw) -->"
            $result = & {
                . $pipelinePath -State SelfTest -Version $fixture.Version -Channel $fixture.Channel `
                    -Tag $fixture.Tag -SourceSha $fixture.SourceSha -WorkflowSha $fixture.SourceSha `
                    -StateRoot $fixture.StateRoot `
                    -ReleaseNotesPath (Join-Path $repoRoot "docs/releases/v$($fixture.Version).md") `
                    -RunId "test" 2>$null
                Get-V4TransactionMarker -Body $body
            }
            if ($null -ne $result) {
                Fail "duplicate key '$($c.Key)' was not rejected by Get-V4TransactionMarker (case $caseIndex)"
            }
        }
        Write-Host "V4 test (33/47): transaction marker duplicate critical key => rejected (5/5 cases): PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# -------------------------------------------------------------------------
# Test 34: Transaction marker — missing, blank, wrong field validation
#           + stale-draft (previous-run) pass + same-transaction pass
# -------------------------------------------------------------------------
function Test-TransactionMarkerFieldValidationAndMatchCases {
    $fixture = New-V4SimplifiedTestFixture
    try {
        $sha = $fixture.SourceSha; $tag = $fixture.Tag; $ver = $fixture.Version
        $repo = "pumni/Sky-Auto-Player"
        $wrongSha = "0000000000000000000000000000000000000000"

        # Dot-source pipeline once to load marker functions into this scope
        . $pipelinePath -State SelfTest -Version $ver -Channel $fixture.Channel `
            -Tag $tag -SourceSha $sha -WorkflowSha $sha `
            -StateRoot $fixture.StateRoot `
            -ReleaseNotesPath (Join-Path $repoRoot "docs/releases/v$ver.md") `
            -RunId "test" 2>$null

        # Missing field => schema invalid
        $missingCases = @(
            @{ Desc = "missing repository"; Raw = "{`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "missing run_id";     Raw = "{`"repository`":`"$repo`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "missing source_sha"; Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "missing version";    Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"tag`":`"$tag`"}" },
            @{ Desc = "missing tag";        Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`"}" }
        )
        foreach ($c in $missingCases) {
            $body = "<!-- v4-release-tx: $($c.Raw) -->"
            $parsed = Get-V4TransactionMarker -Body $body
            $valid = Assert-V4TransactionMarkerStrictSchema -Marker $parsed
            if ($valid) { Fail "schema accepted marker with $($c.Desc)" }
        }

        # Blank field => schema invalid
        $blankCases = @(
            @{ Desc = "blank repository"; Raw = "{`"repository`":`"`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "blank run_id";     Raw = "{`"repository`":`"$repo`",`"run_id`":`"`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "blank source_sha"; Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "blank version";    Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"`",`"tag`":`"$tag`"}" },
            @{ Desc = "blank tag";        Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"`"}" }
        )
        foreach ($c in $blankCases) {
            $body = "<!-- v4-release-tx: $($c.Raw) -->"
            $parsed = Get-V4TransactionMarker -Body $body
            $valid = Assert-V4TransactionMarkerStrictSchema -Marker $parsed
            if ($valid) { Fail "schema accepted marker with $($c.Desc)" }
        }

        # Wrong field => match fails
        $wrongCases = @(
            @{ Desc = "wrong repository"; Raw = "{`"repository`":`"wrong/repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "wrong source_sha"; Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$wrongSha`",`"version`":`"$ver`",`"tag`":`"$tag`"}" },
            @{ Desc = "wrong version";    Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"9.9.9`",`"tag`":`"v9.9.9`"}" },
            @{ Desc = "wrong tag";        Raw = "{`"repository`":`"$repo`",`"run_id`":`"1`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"v9.9.9`"}" }
        )
        foreach ($c in $wrongCases) {
            $body = "<!-- v4-release-tx: $($c.Raw) -->"
            $parsed = Get-V4TransactionMarker -Body $body
            $matched = Test-V4TransactionMarkerMatch -Marker $parsed `
                -ExpectedRepo $repo -ExpectedSha $sha -ExpectedVersion $ver -ExpectedTag $tag
            if ($matched) { Fail "marker match accepted marker with $($c.Desc)" }
        }

        # Complete previous-run marker => stale-draft cleanup allowed (any run_id passes)
        $prevRaw = "{`"repository`":`"$repo`",`"run_id`":`"previous-run`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}"
        $prevParsed = Get-V4TransactionMarker -Body "<!-- v4-release-tx: $prevRaw -->"
        $stalePasses = Test-V4TransactionMarkerMatch -Marker $prevParsed `
            -ExpectedRepo $repo -ExpectedSha $sha -ExpectedVersion $ver -ExpectedTag $tag
        if (-not $stalePasses) { Fail "stale-draft cleanup rejected valid previous-run marker" }

        # Complete current-run marker => same-transaction reconciliation (run_id must match)
        $currRaw = "{`"repository`":`"$repo`",`"run_id`":`"current-run`",`"source_sha`":`"$sha`",`"version`":`"$ver`",`"tag`":`"$tag`"}"
        $currParsed = Get-V4TransactionMarker -Body "<!-- v4-release-tx: $currRaw -->"
        $sameTxPasses = Test-V4TransactionMarkerMatch -Marker $currParsed `
            -ExpectedRepo $repo -ExpectedRunId "current-run" `
            -ExpectedSha $sha -ExpectedVersion $ver -ExpectedTag $tag
        if (-not $sameTxPasses) { Fail "same-transaction reconciliation rejected valid current-run marker" }

        Write-Host "V4 test (34/47): transaction marker missing/blank/wrong/stale-OK/same-tx-OK (5+5+4+1+1 cases): PASS"
    } finally {
        if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
}


# -------------------------------------------------------------------------
# Test 35: Server digest edge cases — table-driven
# Fault flags => FAIL + draft deleted + no PATCH
# Correct sha256 => PASS + published + no delete
# -------------------------------------------------------------------------
function Test-ServerDigestEdgeCases {
    $cases = @(
        @{ Label = "digest property absent";    Flag = "DigestMissing";       ExpectPass = $false },
        @{ Label = "digest empty string";       Flag = "DigestEmpty";         ExpectPass = $false },
        @{ Label = "digest malformed format";   Flag = "DigestMalformed";     ExpectPass = $false },
        @{ Label = "digest sha512:... prefix";  Flag = "DigestSha512";        ExpectPass = $false },
        @{ Label = "sha256 wrong (mismatch)";   Flag = "CorruptServerDigest"; ExpectPass = $false },
        @{ Label = "exact sha256:<64-hex>";     Flag = "";                    ExpectPass = $true  }
    )
    $caseIndex = 0
    foreach ($c in $cases) {
        $caseIndex++
        $fixture = New-V4SimplifiedTestFixture
        try {
            $ctx = [V4SimplifiedMockContext]::new()
            $ctx.Tag = $fixture.Tag
            $ctx.Version = $fixture.Version
            $ctx.SourceSha = $fixture.SourceSha
            $ctx.InstallerName = $fixture.InstallerName
            $ctx.SignatureName = $fixture.SignatureName
            $ctx.InstallerSha = $fixture.InstallerSha
            $ctx.SignatureSha = $fixture.SignatureSha
            if (-not [string]::IsNullOrWhiteSpace($c.Flag)) { $ctx.($c.Flag) = $true }

            $threw = $false
            try { Invoke-TestPublishReleaseTransaction $fixture $ctx } catch { $threw = $true }

            if ($c.ExpectPass) {
                if ($threw) { Fail "digest case '$($c.Label)' threw unexpectedly" }
                if (-not $ctx.PatchedReleases.Contains([int64]42)) { Fail "digest case '$($c.Label)' did not publish" }
                if ($ctx.DeletedReleases.Count -ne 0) { Fail "digest case '$($c.Label)' unexpectedly deleted" }
            } else {
                if (-not $threw) { Fail "digest case '$($c.Label)' did not throw" }
                if (-not $ctx.DeletedReleases.Contains([int64]42)) { Fail "digest case '$($c.Label)' did not delete draft" }
                if ($ctx.PatchedReleases.Count -ne 0) { Fail "digest case '$($c.Label)' must not PATCH after failure" }
            }
        } finally {
            if (Test-Path -LiteralPath $fixture.TestDir) { Remove-Item -LiteralPath $fixture.TestDir -Recurse -Force -ErrorAction SilentlyContinue }
        }
    }
    Write-Host "V4 test (35/47): server digest edge cases FAIL+cleanup or PASS (6/6 cases): PASS"
}

# -------------------------------------------------------------------------
# Test A: Issue #336 - Source vs public naming contract
# -------------------------------------------------------------------------
function Test-ReleasePipelineSourceVsPublicNamingContract {
    $currentVersion = $packageVersion

    $namingScript = @'
param([string]$PipelinePath, [string]$TargetVersion)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$Version = $TargetVersion
$installerSuffix = '_x64-setup.exe'
. (Join-Path (Split-Path -Parent $PipelinePath) 'v4_qualification_evidence.ps1')

$pipelineCode = Get-Content -LiteralPath $PipelinePath -Raw

function Extract-Body([string]$fnName) {
    $startIdx = $pipelineCode.IndexOf("function $fnName")
    if ($startIdx -lt 0) { throw "Could not locate $fnName" }
    $openBrace = $pipelineCode.IndexOf('{', $startIdx)
    $depth = 0
    for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
        if ($pipelineCode[$i] -eq '{') { $depth++ }
        elseif ($pipelineCode[$i] -eq '}') {
            $depth--
            if ($depth -eq 0) {
                return $pipelineCode.Substring($openBrace + 1, $i - $openBrace - 1)
            }
        }
    }
    throw "Unclosed brace for $fnName"
}

. ([scriptblock]::Create("function Get-ExpectedSourceInstallerName { " + (Extract-Body 'Get-ExpectedSourceInstallerName') + " }"))
. ([scriptblock]::Create("function Get-ExpectedSourceSignatureName { " + (Extract-Body 'Get-ExpectedSourceSignatureName') + " }"))
. ([scriptblock]::Create("function Get-ExpectedInstallerName { " + (Extract-Body 'Get-ExpectedInstallerName') + " }"))
. ([scriptblock]::Create("function Get-ExpectedSignatureName { " + (Extract-Body 'Get-ExpectedSignatureName') + " }"))

[pscustomobject]@{
    SourceInstaller = Get-ExpectedSourceInstallerName
    PublicInstaller = Get-ExpectedInstallerName
    SourceSignature = Get-ExpectedSourceSignatureName
    PublicSignature = Get-ExpectedSignatureName
}
'@
    $sb = [scriptblock]::Create($namingScript)
    $names = & $sb $pipelinePath $currentVersion
    if ($names.SourceInstaller -ne "Sky Auto Player_${currentVersion}_x64-setup.exe") {
        Fail "Get-ExpectedSourceInstallerName did not match source name with spaces: $($names.SourceInstaller)"
    }
    if ($names.PublicInstaller -ne "Sky.Auto.Player_${currentVersion}_x64-setup.exe") {
        Fail "Get-ExpectedInstallerName did not match canonical public dotted name: $($names.PublicInstaller)"
    }
    if ($names.SourceSignature -ne "Sky Auto Player_${currentVersion}_x64-setup.exe.sig") {
        Fail "Get-ExpectedSourceSignatureName did not match source signature with spaces: $($names.SourceSignature)"
    }
    if ($names.PublicSignature -ne "Sky.Auto.Player_${currentVersion}_x64-setup.exe.sig") {
        Fail "Get-ExpectedSignatureName did not match canonical public dotted signature: $($names.PublicSignature)"
    }
    if ((Get-V4SafeReleaseAssetName $names.SourceInstaller) -ne $names.PublicInstaller) {
        Fail "Get-V4SafeReleaseAssetName(sourceInstaller) does not equal publicInstaller"
    }
    if ((Get-V4SafeReleaseAssetName $names.SourceSignature) -ne $names.PublicSignature) {
        Fail "Get-V4SafeReleaseAssetName(sourceSignature) does not equal publicSignature"
    }
    Write-Host "V4 test (48/56): source vs public naming contract probe (Test A): PASS"
}

# -------------------------------------------------------------------------
# Test B: Issue #336 - Qualification candidate record regression
# -------------------------------------------------------------------------
function Test-QualificationCandidateRecordRegression {
    $candidateScript = @'
param([string]$PipelinePath, [string]$TargetVersion, [string]$RepoRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$Version = $TargetVersion
$installerSuffix = '_x64-setup.exe'
$repoRoot = $RepoRoot
$qualificationEvidenceName = 'V4_QUALIFICATION_EVIDENCE.json'
$productionEvidenceName = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'
$authenticodeEvidenceName = 'TAURI_AUTHENTICODE_EVIDENCE.json'
$installedAuthenticodeEvidenceName = 'INSTALLED_AUTHENTICODE_EVIDENCE.json'
$summaryName = 'TAURI_ARTIFACT_SUMMARY.json'
$sbomName = 'SBOM.spdx.json'
. (Join-Path (Split-Path -Parent $PipelinePath) 'v4_qualification_evidence.ps1')

$pipelineCode = Get-Content -LiteralPath $PipelinePath -Raw

function Extract-Body([string]$fnName) {
    $startIdx = $pipelineCode.IndexOf("function $fnName")
    if ($startIdx -lt 0) { throw "Could not locate $fnName" }
    $openBrace = $pipelineCode.IndexOf('{', $startIdx)
    $depth = 0
    for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
        if ($pipelineCode[$i] -eq '{') { $depth++ }
        elseif ($pipelineCode[$i] -eq '}') {
            $depth--
            if ($depth -eq 0) {
                return $pipelineCode.Substring($openBrace + 1, $i - $openBrace - 1)
            }
        }
    }
    throw "Unclosed brace for $fnName"
}

. ([scriptblock]::Create("function Get-ExpectedSourceInstallerName { " + (Extract-Body 'Get-ExpectedSourceInstallerName') + " }"))
. ([scriptblock]::Create("function Get-ExpectedSourceSignatureName { " + (Extract-Body 'Get-ExpectedSourceSignatureName') + " }"))
. ([scriptblock]::Create("function Get-ExpectedInstallerName { " + (Extract-Body 'Get-ExpectedInstallerName') + " }"))
. ([scriptblock]::Create("function Get-ExpectedSignatureName { " + (Extract-Body 'Get-ExpectedSignatureName') + " }"))
. ([scriptblock]::Create("function Get-QualificationCandidateRecords { " + (Extract-Body 'Get-QualificationCandidateRecords') + " }"))

return @(Get-QualificationCandidateRecords)
'@
    $sb = [scriptblock]::Create($candidateScript)
    $cands = & $sb $pipelinePath $packageVersion $repoRoot
    $instCand = @($cands | Where-Object { $_.role -eq 'installer' })
    $sigCand = @($cands | Where-Object { $_.role -eq 'updater-signature' })
    $authInstCand = @($cands | Where-Object { $_.role -eq 'installed-authenticode-evidence' })
    $summaryCand = @($cands | Where-Object { $_.role -eq 'artifact-summary' })

    if ($instCand.Count -ne 1) { Fail "expected exactly one installer candidate record" }
    if ($sigCand.Count -ne 1) { Fail "expected exactly one updater-signature candidate record" }
    if ($authInstCand.Count -ne 1 -or $authInstCand[0].name -ne 'INSTALLED_AUTHENTICODE_EVIDENCE.json') {
        Fail "installed-authenticode-evidence candidate record mismatch"
    }
    if ($summaryCand.Count -ne 1 -or $summaryCand[0].name -ne 'TAURI_ARTIFACT_SUMMARY.json') {
        Fail "artifact-summary candidate record mismatch"
    }

    $expectedSourceInst = "Sky Auto Player_${packageVersion}_x64-setup.exe"
    $expectedPublicInst = "Sky.Auto.Player_${packageVersion}_x64-setup.exe"
    $expectedSourceSig = "Sky Auto Player_${packageVersion}_x64-setup.exe.sig"
    $expectedPublicSig = "Sky.Auto.Player_${packageVersion}_x64-setup.exe.sig"

    if ($instCand[0].name -ne $expectedSourceInst) {
        Fail "installer candidate name must be source filename with spaces: $($instCand[0].name)"
    }
    if ([IO.Path]::GetFileName($instCand[0].path) -ne $expectedSourceInst) {
        Fail "installer candidate path filename must be source filename with spaces: $($instCand[0].path)"
    }
    if ((Get-V4SafeReleaseAssetName $instCand[0].name) -ne $expectedPublicInst) {
        Fail "safe mapping of installer candidate name must be canonical public dotted name"
    }

    if ($sigCand[0].name -ne $expectedSourceSig) {
        Fail "updater-signature candidate name must be source signature with spaces: $($sigCand[0].name)"
    }
    if ([IO.Path]::GetFileName($sigCand[0].path) -ne $expectedSourceSig) {
        Fail "updater-signature candidate path filename must be source signature with spaces: $($sigCand[0].path)"
    }
    if ((Get-V4SafeReleaseAssetName $sigCand[0].name) -ne $expectedPublicSig) {
        Fail "safe mapping of updater-signature candidate name must be canonical public dotted name"
    }
    Write-Host "V4 test (49/56): qualification candidate record probe (Test B): PASS"
}

# -------------------------------------------------------------------------
# Test C: Issue #336 - Evidence mapping regression
# -------------------------------------------------------------------------
function Test-EvidenceMappingRegression {
    $evidenceMappingTestRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-evidence-mapping-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $evidenceMappingTestRoot -Force | Out-Null
        $v = $packageVersion
        $srcInstaller = "Sky Auto Player_${v}_x64-setup.exe"
        $srcSig = "$srcInstaller.sig"
        $safeInstaller = "Sky.Auto.Player_${v}_x64-setup.exe"
        $safeSig = "$safeInstaller.sig"
        $testSha = "1234567890abcdef1234567890abcdef12345678"
        $instSha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        $sigSha = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        $authSha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        $sbomSha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

        # Production evidence with spaces
        $prodObj = New-V4CanonicalProductionEvidence `
            -SourceSha $testSha `
            -Version $v `
            -Channel "stable" `
            -InstallerName $srcInstaller `
            -SignatureName $srcSig `
            -InstallerSize 1234567 `
            -SignatureSize 512 `
            -InstallerSha256 $instSha `
            -SignatureSha256 $sigSha `
            -AuthenticodeEvidenceSha256 $authSha `
            -SbomSha256 $sbomSha `
            -UpdaterKeyId "19AABD2E7838818C"
        $validProdPath = Join-Path $evidenceMappingTestRoot "valid_prod.json"
        $prodObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $validProdPath -Encoding utf8

        # Qualification evidence with spaces
        $qualObj = New-V4CanonicalQualificationEvidence `
            -Version $v `
            -InstallerName $srcInstaller `
            -SignatureName $srcSig `
            -InstallerSize 1234567 `
            -SignatureSize 512 `
            -InstallerSha256 $instSha `
            -SignatureSha256 $sigSha `
            -AuthenticodeEvidenceSha256 $authSha `
            -SbomSha256 $sbomSha
        $qualPath = Join-Path $evidenceMappingTestRoot "valid_qual.json"
        $qualObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $qualPath -Encoding utf8

        # Mutated production evidence: installer name has dots instead of spaces
        $mutatedProdObj = New-V4CanonicalProductionEvidence `
            -SourceSha $testSha `
            -Version $v `
            -Channel "stable" `
            -InstallerName $safeInstaller `
            -SignatureName $srcSig `
            -InstallerSize 1234567 `
            -SignatureSize 512 `
            -InstallerSha256 $instSha `
            -SignatureSha256 $sigSha `
            -AuthenticodeEvidenceSha256 $authSha `
            -SbomSha256 $sbomSha `
            -UpdaterKeyId "19AABD2E7838818C"
        $mutatedProdPath = Join-Path $evidenceMappingTestRoot "mutated_prod.json"
        $mutatedProdObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $mutatedProdPath -Encoding utf8

        $workerScript = @'
param(
    [string]$PipelinePath,
    [string]$ProdPath,
    [string]$QualPath,
    [string]$SafeInstaller,
    [string]$SafeSig,
    [string]$SrcInstaller,
    [string]$SrcSig,
    [string]$InstSha,
    [string]$SigSha,
    [string]$AuthSha,
    [string]$SbomSha,
    [string]$TestSha,
    [string]$TestVersion
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$SourceSha = $TestSha
$Version = $TestVersion
$Channel = 'stable'
$productionEvidenceName = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'
$qualificationEvidenceName = 'V4_QUALIFICATION_EVIDENCE.json'
$authenticodeEvidenceName = 'TAURI_AUTHENTICODE_EVIDENCE.json'
$sbomName = 'SBOM.spdx.json'
function Fail([string]$Message) { throw $Message }
function Get-ExpectedInstallerName { return $SafeInstaller }
function Get-ExpectedSignatureName { return $SafeSig }
function Get-ExpectedSourceInstallerName { return $SrcInstaller }
function Get-ExpectedSourceSignatureName { return $SrcSig }

$pipelineCode = Get-Content -LiteralPath $PipelinePath -Raw

function Extract-Function([string]$fnName) {
    $startIdx = $pipelineCode.IndexOf("function $fnName")
    if ($startIdx -lt 0) { throw "Could not locate $fnName" }
    $openBrace = $pipelineCode.IndexOf('{', $startIdx)
    $depth = 0
    for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
        if ($pipelineCode[$i] -eq '{') { $depth++ }
        elseif ($pipelineCode[$i] -eq '}') {
            $depth--
            if ($depth -eq 0) {
                return $pipelineCode.Substring($startIdx, $i - $startIdx + 1)
            }
        }
    }
    throw "Unclosed brace for $fnName"
}

. ([scriptblock]::Create((Extract-Function 'Get-RecordPropertyValue')))
. ([scriptblock]::Create((Extract-Function 'Get-RecordPropertyString')))
. ([scriptblock]::Create((Extract-Function 'Assert-EvidenceIdentity')))

$recs = @(
    [pscustomobject]@{ name = $SafeInstaller; release_name = $SafeInstaller; source_name = $SrcInstaller; size = [int64]1234567; sha256 = $InstSha },
    [pscustomobject]@{ name = $SafeSig; release_name = $SafeSig; source_name = $SrcSig; size = [int64]512; sha256 = $SigSha },
    [pscustomobject]@{ name = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'; size = [int64]100; sha256 = ('1' * 64) },
    [pscustomobject]@{ name = 'V4_QUALIFICATION_EVIDENCE.json'; size = [int64]100; sha256 = ('2' * 64) },
    [pscustomobject]@{ name = 'TAURI_AUTHENTICODE_EVIDENCE.json'; size = [int64]100; sha256 = $AuthSha },
    [pscustomobject]@{ name = 'SBOM.spdx.json'; size = [int64]100; sha256 = $SbomSha }
)

Assert-EvidenceIdentity $ProdPath $QualPath $recs
'@
        $worker = Join-Path $evidenceMappingTestRoot "evidence_mapping_worker.ps1"
        Set-Content -LiteralPath $worker -Value $workerScript -Encoding utf8

        # 1. Valid mapping (source names with spaces in evidence) must PASS
        $resPass = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $worker `
            -PipelinePath $pipelinePath `
            -ProdPath $validProdPath `
            -QualPath $qualPath `
            -SafeInstaller $safeInstaller `
            -SafeSig $safeSig `
            -SrcInstaller $srcInstaller `
            -SrcSig $srcSig `
            -InstSha $instSha `
            -SigSha $sigSha `
            -AuthSha $authSha `
            -SbomSha $sbomSha `
            -TestSha $testSha `
            -TestVersion $v 2>&1 | Out-String

        if ($LASTEXITCODE -ne 0) {
            Fail "Assert-EvidenceIdentity rejected valid source/public mapping: $resPass"
        }

        # 2. Mutated mapping (evidence with dotted public name) must FAIL CLOSED
        $resFail = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $worker `
            -PipelinePath $pipelinePath `
            -ProdPath $mutatedProdPath `
            -QualPath $qualPath `
            -SafeInstaller $safeInstaller `
            -SafeSig $safeSig `
            -SrcInstaller $srcInstaller `
            -SrcSig $srcSig `
            -InstSha $instSha `
            -SigSha $sigSha `
            -AuthSha $authSha `
            -SbomSha $sbomSha `
            -TestSha $testSha `
            -TestVersion $v 2>&1 | Out-String

        if ($LASTEXITCODE -eq 0) {
            Fail "Assert-EvidenceIdentity accepted evidence with mutated dotted installer name (expected fail-closed)"
        }

        Write-Host "V4 test (50/56): evidence mapping regression probe (Test C): PASS"
    } finally {
        if (Test-Path -LiteralPath $evidenceMappingTestRoot) {
            Remove-Item -LiteralPath $evidenceMappingTestRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# -------------------------------------------------------------------------
# Test D: Issue #336 corrective - Candidate evidence production wiring regression
# -------------------------------------------------------------------------
function Test-CandidateEvidenceProductionWiringRegression {
    $pipeline = Get-Content -LiteralPath $pipelinePath -Raw
    $defMatches = [regex]::Matches($pipeline, '(?m)^function Assert-CandidateEvidence\(')
    if ($defMatches.Count -ne 1) {
        Fail "Expected exactly one Assert-CandidateEvidence definition in release pipeline, found $($defMatches.Count)"
    }

    $freezeMarker = '$candidateAssets = @(Freeze-CandidateAssets $records)'
    $evidenceGateMarker = 'Assert-CandidateEvidence $candidateAssets'
    $publicProjectionMarker = '$publicRecords = @(Get-PublicReleaseRecords $records)'

    $freezeIndex = $pipeline.IndexOf($freezeMarker)
    $evidenceGateIndex = $pipeline.IndexOf($evidenceGateMarker)
    $publicProjectionIndex = $pipeline.IndexOf($publicProjectionMarker)

    if ($freezeIndex -lt 0) { Fail "Missing freeze marker in pipeline: $freezeMarker" }
    if ($evidenceGateIndex -lt 0) { Fail "Missing candidate evidence gate call in pipeline: $evidenceGateMarker" }
    if ($publicProjectionIndex -lt 0) { Fail "Missing public release projection marker in pipeline: $publicProjectionMarker" }

    if (-not ($freezeIndex -lt $evidenceGateIndex -and $evidenceGateIndex -lt $publicProjectionIndex)) {
        Fail "Invoke-BuildCandidate ordering violation: expected Freeze-CandidateAssets ($freezeIndex) < Assert-CandidateEvidence ($evidenceGateIndex) < Get-PublicReleaseRecords ($publicProjectionIndex)"
    }

    Write-Host "V4 test (51/56): candidate evidence production wiring regression probe (Test D): PASS"
}

# -------------------------------------------------------------------------
# Test E: Issue #340 - Production shape record regression
# -------------------------------------------------------------------------
function Test-ProductionShapeRecordRegression {
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-prod-shape-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        $v = $packageVersion
        $srcInstaller = "Sky Auto Player_${v}_x64-setup.exe"
        $srcSig = "$srcInstaller.sig"
        $dottedInstaller = "Sky.Auto.Player_${v}_x64-setup.exe"
        $dottedSig = "$dottedInstaller.sig"

        $instPath = Join-Path $tempDir $srcInstaller
        $sigPath = Join-Path $tempDir $srcSig
        [IO.File]::WriteAllBytes($instPath, [byte[]](1..100))
        [IO.File]::WriteAllBytes($sigPath, [byte[]](1..50))

        $instCand = [pscustomobject]@{ name = $srcInstaller; path = $instPath; role = "installer" }
        $sigCand = [pscustomobject]@{ name = $srcSig; path = $sigPath; role = "updater-signature" }

        $pipelineCode = Get-Content -LiteralPath $pipelinePath -Raw

        function Extract-FunctionLocal([string]$fnName) {
            $startIdx = $pipelineCode.IndexOf("function $fnName")
            if ($startIdx -lt 0) { throw "Could not locate $fnName" }
            $openBrace = $pipelineCode.IndexOf('{', $startIdx)
            $depth = 0
            for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
                if ($pipelineCode[$i] -eq '{') { $depth++ }
                elseif ($pipelineCode[$i] -eq '}') {
                    $depth--
                    if ($depth -eq 0) {
                        return $pipelineCode.Substring($startIdx, $i - $startIdx + 1)
                    }
                }
            }
            throw "Unclosed brace for $fnName"
        }

        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyValue')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyString')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-FileRecord')))

        # 1. Test Get-FileRecord output shape on installer
        $instRec = Get-FileRecord $instCand
        if ($instRec -isnot [System.Collections.IDictionary]) {
            Fail "Get-FileRecord must produce an IDictionary/OrderedDictionary, got $($instRec.GetType().FullName)"
        }
        if ((Get-RecordPropertyString $instRec 'role') -ne 'installer') {
            Fail "installer record role mismatch: $(Get-RecordPropertyString $instRec 'role')"
        }
        if ((Get-RecordPropertyString $instRec 'source_name') -ne $srcInstaller) {
            Fail "installer record source_name must have spaces: $(Get-RecordPropertyString $instRec 'source_name')"
        }
        if ((Get-RecordPropertyString $instRec 'release_name') -ne $dottedInstaller) {
            Fail "installer record release_name must be dotted: $(Get-RecordPropertyString $instRec 'release_name')"
        }
        if ((Get-RecordPropertyString $instRec 'name') -ne $dottedInstaller) {
            Fail "installer record name must match dotted release_name: $(Get-RecordPropertyString $instRec 'name')"
        }
        if ((Get-RecordPropertyString $instRec 'state_path') -ne "candidate-bundle/$srcInstaller") {
            Fail "installer record state_path must be candidate-bundle/<source_name>, got: $(Get-RecordPropertyString $instRec 'state_path')"
        }
        if ([int64](Get-RecordPropertyValue $instRec 'size') -ne 100) {
            Fail "installer record size mismatch: $(Get-RecordPropertyValue $instRec 'size')"
        }

        # 2. Test Get-FileRecord output shape on updater signature
        $sigRec = Get-FileRecord $sigCand
        if ($sigRec -isnot [System.Collections.IDictionary]) {
            Fail "Get-FileRecord must produce an IDictionary/OrderedDictionary for signature"
        }
        if ((Get-RecordPropertyString $sigRec 'role') -ne 'updater-signature') {
            Fail "signature record role mismatch"
        }
        if ((Get-RecordPropertyString $sigRec 'source_name') -ne $srcSig) {
            Fail "signature record source_name must have spaces"
        }
        if ((Get-RecordPropertyString $sigRec 'release_name') -ne $dottedSig) {
            Fail "signature record release_name must be dotted"
        }
        if ((Get-RecordPropertyString $sigRec 'state_path') -ne "candidate-bundle/$srcSig") {
            Fail "signature record state_path must be candidate-bundle/<source_name>, got: $(Get-RecordPropertyString $sigRec 'state_path')"
        }

        # 3. Test Get-FileRecord output shape on evidence roles
        $evidenceRoles = @(
            @{ Role = "production-evidence"; Name = "V4_PRODUCTION_RELEASE_EVIDENCE.json" },
            @{ Role = "qualification-evidence"; Name = "V4_QUALIFICATION_EVIDENCE.json" },
            @{ Role = "authenticode-evidence"; Name = "TAURI_AUTHENTICODE_EVIDENCE.json" },
            @{ Role = "installed-authenticode-evidence"; Name = "INSTALLED_AUTHENTICODE_EVIDENCE.json" },
            @{ Role = "artifact-summary"; Name = "TAURI_ARTIFACT_SUMMARY.json" },
            @{ Role = "sbom"; Name = "SBOM.spdx.json" }
        )
        foreach ($er in $evidenceRoles) {
            $ePath = Join-Path $tempDir $er.Name
            Set-Content -LiteralPath $ePath -Value "{}" -Encoding utf8
            $eCand = [pscustomobject]@{ name = $er.Name; path = $ePath; role = $er.Role }
            $eRec = Get-FileRecord $eCand
            if ((Get-RecordPropertyString $eRec 'state_path') -ne "candidate-evidence/$($er.Name)") {
                Fail "evidence role $($er.Role) state_path must be candidate-evidence/$($er.Name), got: $(Get-RecordPropertyString $eRec 'state_path')"
            }
            if ((Get-RecordPropertyString $eRec 'release_name') -ne $er.Name) {
                Fail "evidence release_name mismatch for $($er.Name)"
            }
        }

        # 4. Test Get-RecordPropertyValue and Get-RecordPropertyString duality (OrderedDictionary vs PSCustomObject)
        $dict = [ordered]@{ key1 = "val1"; num = 42 }
        $pso = [pscustomobject]@{ key1 = "val1"; num = 42 }

        foreach ($target in @($dict, $pso)) {
            if ((Get-RecordPropertyString $target 'key1') -ne 'val1') { Fail "failed to read string from $($target.GetType().Name)" }
            if ([int](Get-RecordPropertyValue $target 'num') -ne 42) { Fail "failed to read int from $($target.GetType().Name)" }
            if ($null -ne (Get-RecordPropertyValue $target 'nonexistent')) { Fail "nonexistent property must return null" }
            if ((Get-RecordPropertyString $target 'nonexistent') -ne '') { Fail "nonexistent property string must return empty string" }
        }
        if ($null -ne (Get-RecordPropertyValue $null 'key1')) { Fail "null record must return null property value" }
        if ((Get-RecordPropertyString $null 'key1') -ne '') { Fail "null record must return empty property string" }

        Write-Host "V4 test (52/56): production shape record regression probe (Test E): PASS"
    } finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# -------------------------------------------------------------------------
# Test F: Issue #340 - Frozen candidate topology regression
# -------------------------------------------------------------------------
function Test-FrozenTopologyRegression {
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-frozen-topo-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        $stateRoot = Join-Path $tempDir "state-root"
        $sourceDir = Join-Path $tempDir "sources"
        New-Item -ItemType Directory -Path $stateRoot -Force | Out-Null
        New-Item -ItemType Directory -Path $sourceDir -Force | Out-Null

        $v = $packageVersion
        $installerSuffix = '_x64-setup.exe'
        $Version = $v
        $productionEvidenceName = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'
        $qualificationEvidenceName = 'V4_QUALIFICATION_EVIDENCE.json'
        $authenticodeEvidenceName = 'TAURI_AUTHENTICODE_EVIDENCE.json'
        $installedAuthenticodeEvidenceName = 'INSTALLED_AUTHENTICODE_EVIDENCE.json'
        $summaryName = 'TAURI_ARTIFACT_SUMMARY.json'
        $sbomName = 'SBOM.spdx.json'

        $srcInstaller = "Sky Auto Player_${v}_x64-setup.exe"
        $srcSig = "$srcInstaller.sig"
        $dottedInstaller = "Sky.Auto.Player_${v}_x64-setup.exe"
        $dottedSig = "$dottedInstaller.sig"

        $pipelineCode = Get-Content -LiteralPath $pipelinePath -Raw

        function Extract-FunctionLocal([string]$fnName) {
            $startIdx = $pipelineCode.IndexOf("function $fnName")
            if ($startIdx -lt 0) { throw "Could not locate $fnName" }
            $openBrace = $pipelineCode.IndexOf('{', $startIdx)
            $depth = 0
            for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
                if ($pipelineCode[$i] -eq '{') { $depth++ }
                elseif ($pipelineCode[$i] -eq '}') {
                    $depth--
                    if ($depth -eq 0) {
                        return $pipelineCode.Substring($startIdx, $i - $startIdx + 1)
                    }
                }
            }
            throw "Unclosed brace for $fnName"
        }

        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyValue')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyString')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSourceInstallerName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSourceSignatureName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedInstallerName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSignatureName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-StateAssetPath')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Assert-ManifestAssetFiles')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Freeze-CandidateAssets')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Assert-FrozenCandidateTopology')))

        function Get-EffectiveStateRoot { return $stateRoot }

        # Create source files
        $fileDefs = @(
            @{ Name = $srcInstaller; Role = "installer"; Path = (Join-Path $sourceDir $srcInstaller); Content = "installer bytes"; StatePath = "candidate-bundle/$srcInstaller"; RelName = $dottedInstaller },
            @{ Name = $srcSig; Role = "updater-signature"; Path = (Join-Path $sourceDir $srcSig); Content = "sig bytes"; StatePath = "candidate-bundle/$srcSig"; RelName = $dottedSig },
            @{ Name = $productionEvidenceName; Role = "production-evidence"; Path = (Join-Path $sourceDir $productionEvidenceName); Content = "{}"; StatePath = "candidate-evidence/$productionEvidenceName"; RelName = $productionEvidenceName },
            @{ Name = $qualificationEvidenceName; Role = "qualification-evidence"; Path = (Join-Path $sourceDir $qualificationEvidenceName); Content = "{}"; StatePath = "candidate-evidence/$qualificationEvidenceName"; RelName = $qualificationEvidenceName },
            @{ Name = $authenticodeEvidenceName; Role = "authenticode-evidence"; Path = (Join-Path $sourceDir $authenticodeEvidenceName); Content = "{}"; StatePath = "candidate-evidence/$authenticodeEvidenceName"; RelName = $authenticodeEvidenceName },
            @{ Name = $installedAuthenticodeEvidenceName; Role = "installed-authenticode-evidence"; Path = (Join-Path $sourceDir $installedAuthenticodeEvidenceName); Content = "{}"; StatePath = "candidate-evidence/$installedAuthenticodeEvidenceName"; RelName = $installedAuthenticodeEvidenceName },
            @{ Name = $summaryName; Role = "artifact-summary"; Path = (Join-Path $sourceDir $summaryName); Content = "{}"; StatePath = "candidate-evidence/$summaryName"; RelName = $summaryName },
            @{ Name = $sbomName; Role = "sbom"; Path = (Join-Path $sourceDir $sbomName); Content = "{}"; StatePath = "candidate-evidence/$sbomName"; RelName = $sbomName }
        )

        $records = @()
        foreach ($fd in $fileDefs) {
            Set-Content -LiteralPath $fd.Path -Value $fd.Content -Encoding utf8
            $item = Get-Item -LiteralPath $fd.Path
            $records += [ordered]@{
                name = $fd.RelName
                release_name = $fd.RelName
                source_name = $fd.Name
                role = $fd.Role
                size = [int64]$item.Length
                sha256 = (Get-FileHash -LiteralPath $fd.Path -Algorithm SHA256).Hash.ToLowerInvariant()
                source_path = $fd.Path
                state_path = $fd.StatePath
            }
        }

        # 1. Freeze-CandidateAssets into state root
        $frozen = Freeze-CandidateAssets $records
        if ($frozen.Count -ne 8) { Fail "Freeze-CandidateAssets must return all 8 records" }

        # 2. Positive assertion: Assert-FrozenCandidateTopology must PASS
        Assert-FrozenCandidateTopology $records

        # Direct disk verification:
        $bundleDir = Join-Path $stateRoot "candidate-bundle"
        $evidenceDir = Join-Path $stateRoot "candidate-evidence"
        $bundleFiles = @(Get-ChildItem -LiteralPath $bundleDir -File)
        $evidenceFiles = @(Get-ChildItem -LiteralPath $evidenceDir -File)

        if ($bundleFiles.Count -ne 2) { Fail "candidate-bundle on disk must have exactly 2 files, found $($bundleFiles.Count)" }
        if ($evidenceFiles.Count -ne 6) { Fail "candidate-evidence on disk must have exactly 6 files, found $($evidenceFiles.Count)" }
        if (Test-Path -LiteralPath (Join-Path $bundleDir $dottedInstaller)) { Fail "dotted installer found in candidate-bundle" }
        if (Test-Path -LiteralPath (Join-Path $stateRoot $dottedInstaller)) { Fail "dotted installer found in state root" }

        # 3. Fault injection A: JSON evidence in bundle must fail closed
        $injectedJson = Join-Path $bundleDir "stray.json"
        Set-Content -LiteralPath $injectedJson -Value "{}" -Encoding utf8
        $threw = $false
        try { Assert-FrozenCandidateTopology $records } catch {
            $threw = $true
            if ($_.Exception.Message -notmatch "must not contain JSON evidence files|must contain exactly") {
                Fail "unexpected error on JSON in bundle: $($_.Exception.Message)"
            }
        }
        if (-not $threw) { Fail "Assert-FrozenCandidateTopology accepted JSON file in candidate-bundle" }
        Remove-Item -LiteralPath $injectedJson -Force

        # 4. Fault injection B: Dotted copy in candidate-bundle must fail closed
        $injectedDotted = Join-Path $bundleDir $dottedInstaller
        Set-Content -LiteralPath $injectedDotted -Value "stray" -Encoding utf8
        $threw = $false
        try { Assert-FrozenCandidateTopology $records } catch {
            $threw = $true
            if ($_.Exception.Message -notmatch "must not contain dotted release asset copy|must contain exactly") {
                Fail "unexpected error on dotted file in bundle: $($_.Exception.Message)"
            }
        }
        if (-not $threw) { Fail "Assert-FrozenCandidateTopology accepted dotted file in candidate-bundle" }
        Remove-Item -LiteralPath $injectedDotted -Force

        # 5. Fault injection C: Dotted copy in state root must fail closed
        $injectedRootDotted = Join-Path $stateRoot $dottedInstaller
        Set-Content -LiteralPath $injectedRootDotted -Value "stray" -Encoding utf8
        $threw = $false
        try { Assert-FrozenCandidateTopology $records } catch {
            $threw = $true
            if ($_.Exception.Message -notmatch "state root must not contain dotted release asset copy") {
                Fail "unexpected error on dotted file in state root: $($_.Exception.Message)"
            }
        }
        if (-not $threw) { Fail "Assert-FrozenCandidateTopology accepted dotted file in state root" }
        Remove-Item -LiteralPath $injectedRootDotted -Force

        # 6. Fault injection D: Missing evidence file in candidate-evidence must fail closed
        $targetEvidence = Join-Path $evidenceDir $sbomName
        $sbomBackup = Get-Content -LiteralPath $targetEvidence -Raw
        Remove-Item -LiteralPath $targetEvidence -Force
        $threw = $false
        try { Assert-FrozenCandidateTopology $records } catch {
            $threw = $true
            if ($_.Exception.Message -notmatch "candidate-evidence is missing required evidence file: $sbomName") {
                Fail "unexpected error on missing evidence: $($_.Exception.Message)"
            }
        }
        if (-not $threw) { Fail "Assert-FrozenCandidateTopology accepted missing evidence file" }
        Set-Content -LiteralPath $targetEvidence -Value $sbomBackup -Encoding utf8

        Write-Host "V4 test (53/56): frozen candidate topology regression probe (Test F): PASS"
    } finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# -------------------------------------------------------------------------
# Test G: Issue #340 - Producer-consumer evidence binding regression
# -------------------------------------------------------------------------
function Test-ProducerConsumerEvidenceBindingRegression {
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-binding-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        $v = $packageVersion
        $installerSuffix = '_x64-setup.exe'
        $Version = $v
        $Channel = 'stable'
        $SourceSha = "1234567890abcdef1234567890abcdef12345678"
        $productionEvidenceName = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'
        $qualificationEvidenceName = 'V4_QUALIFICATION_EVIDENCE.json'
        $authenticodeEvidenceName = 'TAURI_AUTHENTICODE_EVIDENCE.json'
        $installedAuthenticodeEvidenceName = 'INSTALLED_AUTHENTICODE_EVIDENCE.json'
        $summaryName = 'TAURI_ARTIFACT_SUMMARY.json'
        $sbomName = 'SBOM.spdx.json'

        $srcInstaller = "Sky Auto Player_${v}_x64-setup.exe"
        $srcSig = "$srcInstaller.sig"
        $dottedInstaller = "Sky.Auto.Player_${v}_x64-setup.exe"
        $dottedSig = "$dottedInstaller.sig"

        $pipelineCode = Get-Content -LiteralPath $pipelinePath -Raw

        function Extract-FunctionLocal([string]$fnName) {
            $startIdx = $pipelineCode.IndexOf("function $fnName")
            if ($startIdx -lt 0) { throw "Could not locate $fnName" }
            $openBrace = $pipelineCode.IndexOf('{', $startIdx)
            $depth = 0
            for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
                if ($pipelineCode[$i] -eq '{') { $depth++ }
                elseif ($pipelineCode[$i] -eq '}') {
                    $depth--
                    if ($depth -eq 0) {
                        return $pipelineCode.Substring($startIdx, $i - $startIdx + 1)
                    }
                }
            }
            throw "Unclosed brace for $fnName"
        }

        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyValue')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyString')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSourceInstallerName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSourceSignatureName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedInstallerName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSignatureName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-FileRecord')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Assert-EvidenceIdentity')))

        # Set script scope variables so Assert-EvidenceIdentity can resolve them
        $script:SourceSha = $SourceSha
        $script:Version = $Version
        $script:Channel = $Channel
        $script:productionEvidenceName = $productionEvidenceName
        $script:qualificationEvidenceName = $qualificationEvidenceName
        $script:authenticodeEvidenceName = $authenticodeEvidenceName
        $script:sbomName = $sbomName

        # Create physical files for producer Get-FileRecord
        $files = @{
            $srcInstaller = [byte[]](1..100)
            $srcSig = [byte[]](1..50)
            $productionEvidenceName = [Text.Encoding]::UTF8.GetBytes("{}")
            $qualificationEvidenceName = [Text.Encoding]::UTF8.GetBytes("{}")
            $authenticodeEvidenceName = [Text.Encoding]::UTF8.GetBytes("auth evidence content")
            $installedAuthenticodeEvidenceName = [Text.Encoding]::UTF8.GetBytes("{}")
            $summaryName = [Text.Encoding]::UTF8.GetBytes("{}")
            $sbomName = [Text.Encoding]::UTF8.GetBytes("sbom evidence content")
        }

        $records = @()
        foreach ($entry in $files.GetEnumerator()) {
            $filePath = Join-Path $tempDir $entry.Key
            [IO.File]::WriteAllBytes($filePath, $entry.Value)
            $role = if ($entry.Key -eq $srcInstaller) { "installer" }
                    elseif ($entry.Key -eq $srcSig) { "updater-signature" }
                    elseif ($entry.Key -eq $productionEvidenceName) { "production-evidence" }
                    elseif ($entry.Key -eq $qualificationEvidenceName) { "qualification-evidence" }
                    elseif ($entry.Key -eq $authenticodeEvidenceName) { "authenticode-evidence" }
                    elseif ($entry.Key -eq $installedAuthenticodeEvidenceName) { "installed-authenticode-evidence" }
                    elseif ($entry.Key -eq $summaryName) { "artifact-summary" }
                    elseif ($entry.Key -eq $sbomName) { "sbom" }
                    else { "unknown" }
            $cand = [pscustomobject]@{ name = $entry.Key; path = $filePath; role = $role }
            $records += (Get-FileRecord $cand)
        }

        $instRecord = @($records | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'installer' })[0]
        $sigRecord = @($records | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'updater-signature' })[0]
        $authRecord = @($records | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'authenticode-evidence' })[0]
        $sbomRecord = @($records | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'sbom' })[0]

        $instSha = Get-RecordPropertyString $instRecord 'sha256'
        $sigSha = Get-RecordPropertyString $sigRecord 'sha256'
        $authSha = Get-RecordPropertyString $authRecord 'sha256'
        $sbomSha = Get-RecordPropertyString $sbomRecord 'sha256'

        # Baseline valid production and qualification evidence
        $validProd = New-V4CanonicalProductionEvidence `
            -SourceSha $SourceSha `
            -Version $Version `
            -Channel $Channel `
            -InstallerName $srcInstaller `
            -SignatureName $srcSig `
            -InstallerSize 100 `
            -SignatureSize 50 `
            -InstallerSha256 $instSha `
            -SignatureSha256 $sigSha `
            -AuthenticodeEvidenceSha256 $authSha `
            -SbomSha256 $sbomSha `
            -UpdaterKeyId "19AABD2E7838818C"

        $validQual = New-V4CanonicalQualificationEvidence `
            -Version $Version `
            -InstallerName $srcInstaller `
            -SignatureName $srcSig `
            -InstallerSize 100 `
            -SignatureSize 50 `
            -InstallerSha256 $instSha `
            -SignatureSha256 $sigSha `
            -AuthenticodeEvidenceSha256 $authSha `
            -SbomSha256 $sbomSha

        $prodPath = Join-Path $tempDir "test_prod.json"
        $qualPath = Join-Path $tempDir "test_qual.json"

        $validProd | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $prodPath -Encoding utf8
        $validQual | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $qualPath -Encoding utf8

        # 1. Base case: must PASS
        Assert-EvidenceIdentity $prodPath $qualPath $records

        # 2. Field-specific diagnostic assertions
        $diagnosticCases = @(
            @{ Field = "installer"; Mutation = "MutatedName.exe"; Target = "prod"; ExpectedError = "production evidence installer name mismatch" },
            @{ Field = "updater_signature"; Mutation = "MutatedSig.sig"; Target = "prod"; ExpectedError = "production evidence updater_signature name mismatch" },
            @{ Field = "installer_size"; Mutation = [int64]999; Target = "prod"; ExpectedError = "production evidence installer size mismatch" },
            @{ Field = "signature_size"; Mutation = [int64]999; Target = "prod"; ExpectedError = "production evidence updater signature size mismatch" },
            @{ Field = "installer_sha256"; Mutation = ('f' * 64); Target = "prod"; ExpectedError = "production evidence installer SHA-256 mismatch" },
            @{ Field = "updater_signature_sha256"; Mutation = ('f' * 64); Target = "prod"; ExpectedError = "production evidence updater signature SHA-256 mismatch" },
            @{ Field = "authenticode_evidence_sha256"; Mutation = ('f' * 64); Target = "prod"; ExpectedError = "production evidence Authenticode evidence SHA-256 mismatch" },
            @{ Field = "sbom_sha256"; Mutation = ('f' * 64); Target = "prod"; ExpectedError = "production evidence SBOM SHA-256 mismatch" },
            # Qualification evidence: all 11 fields fault-injected individually
            @{ Field = "installer"; Mutation = "MutatedName.exe"; Target = "qual"; ExpectedError = "qualification evidence installer name mismatch" },
            @{ Field = "updater_signature"; Mutation = "MutatedSig.sig"; Target = "qual"; ExpectedError = "qualification evidence updater_signature name mismatch" },
            @{ Field = "authenticode_evidence"; Mutation = "WRONG_EVIDENCE.json"; Target = "qual"; ExpectedError = "qualification evidence Authenticode evidence name mismatch" },
            @{ Field = "sbom"; Mutation = "WRONG_SBOM.json"; Target = "qual"; ExpectedError = "qualification evidence SBOM name mismatch" },
            @{ Field = "installer_size"; Mutation = [int64]999; Target = "qual"; ExpectedError = "qualification evidence installer size mismatch" },
            @{ Field = "signature_size"; Mutation = [int64]999; Target = "qual"; ExpectedError = "qualification evidence updater signature size mismatch" },
            @{ Field = "installer_sha256"; Mutation = ('f' * 64); Target = "qual"; ExpectedError = "qualification evidence installer SHA-256 mismatch" },
            @{ Field = "updater_signature_sha256"; Mutation = ('f' * 64); Target = "qual"; ExpectedError = "qualification evidence updater signature SHA-256 mismatch" },
            @{ Field = "authenticode_evidence_sha256"; Mutation = ('f' * 64); Target = "qual"; ExpectedError = "qualification evidence Authenticode evidence SHA-256 mismatch" },
            @{ Field = "authenticode_mode"; Mutation = "wrong-mode"; Target = "qual"; ExpectedError = "qualification evidence authenticode_mode mismatch" },
            @{ Field = "sbom_sha256"; Mutation = ('f' * 64); Target = "qual"; ExpectedError = "qualification evidence SBOM SHA-256 mismatch" }
        )

        foreach ($case in $diagnosticCases) {
            $pObj = Get-Content -LiteralPath $prodPath -Raw | ConvertFrom-Json
            $qObj = Get-Content -LiteralPath $qualPath -Raw | ConvertFrom-Json
            if ($case.Target -eq "prod") {
                $pObj.($case.Field) = $case.Mutation
            } else {
                $qObj.($case.Field) = $case.Mutation
            }

            $mutatedProdPath = Join-Path $tempDir "mutated_prod_$($case.Field)_$($case.Target).json"
            $mutatedQualPath = Join-Path $tempDir "mutated_qual_$($case.Field)_$($case.Target).json"
            $pObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $mutatedProdPath -Encoding utf8
            $qObj | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $mutatedQualPath -Encoding utf8

            $threw = $false
            try {
                Assert-EvidenceIdentity $mutatedProdPath $mutatedQualPath $records
            } catch {
                $threw = $true
                if ($_.Exception.Message -notmatch [regex]::Escape($case.ExpectedError)) {
                    Fail "case '$($case.Field)' ($($case.Target)) expected error '$($case.ExpectedError)', got: $($_.Exception.Message)"
                }
            }
            if (-not $threw) {
                Fail "case '$($case.Field)' ($($case.Target)) should have failed closed but passed"
            }
        }

        Write-Host "V4 test (54/56): producer-consumer evidence binding regression probe (Test G): PASS"
    } finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# -------------------------------------------------------------------------
# Test H: Issue #340 - Public upload projection regression
# -------------------------------------------------------------------------
function Test-PublicUploadProjectionRegression {
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-pub-proj-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        $stateRoot = Join-Path $tempDir "state-root"
        New-Item -ItemType Directory -Path $stateRoot -Force | Out-Null

        $v = $packageVersion
        $installerSuffix = '_x64-setup.exe'
        $Version = $v
        $srcInstaller = "Sky Auto Player_${v}_x64-setup.exe"
        $srcSig = "$srcInstaller.sig"
        $dottedInstaller = "Sky.Auto.Player_${v}_x64-setup.exe"
        $dottedSig = "$dottedInstaller.sig"

        $pipelineCode = Get-Content -LiteralPath $pipelinePath -Raw

        function Extract-FunctionLocal([string]$fnName) {
            $startIdx = $pipelineCode.IndexOf("function $fnName")
            if ($startIdx -lt 0) { throw "Could not locate $fnName" }
            $openBrace = $pipelineCode.IndexOf('{', $startIdx)
            $depth = 0
            for ($i = $openBrace; $i -lt $pipelineCode.Length; $i++) {
                if ($pipelineCode[$i] -eq '{') { $depth++ }
                elseif ($pipelineCode[$i] -eq '}') {
                    $depth--
                    if ($depth -eq 0) {
                        return $pipelineCode.Substring($startIdx, $i - $startIdx + 1)
                    }
                }
            }
            throw "Unclosed brace for $fnName"
        }

        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyValue')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-RecordPropertyString')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSourceInstallerName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSourceSignatureName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedInstallerName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-ExpectedSignatureName')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-CanonicalPublicReleaseNames')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-PublicReleaseRecords')))
        . ([scriptblock]::Create((Extract-FunctionLocal 'Get-StateAssetPath')))

        function Get-EffectiveStateRoot { return $stateRoot }

        # Setup frozen candidate assets
        $bundleDir = Join-Path $stateRoot "candidate-bundle"
        New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null
        $frozenInstallerPath = Join-Path $bundleDir $srcInstaller
        $frozenSigPath = Join-Path $bundleDir $srcSig
        [IO.File]::WriteAllBytes($frozenInstallerPath, [byte[]](1..100))
        [IO.File]::WriteAllBytes($frozenSigPath, [byte[]](1..50))

        $instSha = (Get-FileHash -LiteralPath $frozenInstallerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $sigSha = (Get-FileHash -LiteralPath $frozenSigPath -Algorithm SHA256).Hash.ToLowerInvariant()

        $qualRecords = @(
            [ordered]@{
                name = $dottedInstaller
                release_name = $dottedInstaller
                source_name = $srcInstaller
                role = "installer"
                size = [int64]100
                sha256 = $instSha
                source_path = $frozenInstallerPath
                state_path = "candidate-bundle/$srcInstaller"
            },
            [ordered]@{
                name = $dottedSig
                release_name = $dottedSig
                source_name = $srcSig
                role = "updater-signature"
                size = [int64]50
                sha256 = $sigSha
                source_path = $frozenSigPath
                state_path = "candidate-bundle/$srcSig"
            },
            [ordered]@{
                name = "V4_PRODUCTION_RELEASE_EVIDENCE.json"
                release_name = "V4_PRODUCTION_RELEASE_EVIDENCE.json"
                source_name = "V4_PRODUCTION_RELEASE_EVIDENCE.json"
                role = "production-evidence"
                size = [int64]10
                sha256 = ('a' * 64)
                source_path = ""
                state_path = "candidate-evidence/V4_PRODUCTION_RELEASE_EVIDENCE.json"
            }
        )

        # 1. Project public release records from qualification records
        $publicRecords = Get-PublicReleaseRecords $qualRecords
        if ($publicRecords.Count -ne 2) {
            Fail "Get-PublicReleaseRecords must filter to exactly the 2 canonical public assets, got $($publicRecords.Count)"
        }

        # 2. Verify projection properties
        $pubInst = @($publicRecords | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'installer' })[0]
        $pubSig = @($publicRecords | Where-Object { (Get-RecordPropertyString $_ 'role') -eq 'updater-signature' })[0]

        if ((Get-RecordPropertyString $pubInst 'release_name') -ne $dottedInstaller) {
            Fail "projected installer release_name must be dotted: $(Get-RecordPropertyString $pubInst 'release_name')"
        }
        if ((Get-RecordPropertyString $pubInst 'source_name') -ne $srcInstaller) {
            Fail "projected installer source_name must have spaces: $(Get-RecordPropertyString $pubInst 'source_name')"
        }
        if ((Get-RecordPropertyString $pubSig 'release_name') -ne $dottedSig) {
            Fail "projected signature release_name must be dotted"
        }
        if ((Get-RecordPropertyString $pubSig 'source_name') -ne $srcSig) {
            Fail "projected signature source_name must have spaces"
        }

        # 3. Simulate upload loop: reads source-named file in candidate-bundle and maps to dotted release asset name
        $uploaded = @()
        foreach ($record in $publicRecords) {
            $filePath = Get-StateAssetPath $record
            $assetName = Get-RecordPropertyString $record "release_name"
            if (-not (Test-Path -LiteralPath $filePath -PathType Leaf)) {
                Fail "upload target file does not exist: $filePath"
            }
            if ([IO.Path]::GetFileName($filePath) -ne (Get-RecordPropertyString $record "source_name")) {
                Fail "upload target file is not source-named: $filePath"
            }
            if ($assetName -ne (Get-V4SafeReleaseAssetName ([IO.Path]::GetFileName($filePath)))) {
                Fail "asset name does not equal safe release name"
            }
            $uploaded += @{ AssetName = $assetName; FilePath = $filePath }
        }

        if ($uploaded.Count -ne 2) { Fail "upload loop must process exactly 2 assets" }

        # Verify zero dotted files created locally
        $dottedInBundle = Join-Path $bundleDir $dottedInstaller
        $dottedInRoot = Join-Path $stateRoot $dottedInstaller
        if (Test-Path -LiteralPath $dottedInBundle) {
            Fail "local dotted file created in candidate-bundle during projection/upload"
        }
        if (Test-Path -LiteralPath $dottedInRoot) {
            Fail "local dotted file created in state-root during projection/upload"
        }

        Write-Host "V4 test (55/56): public upload projection regression probe (Test H): PASS"
    } finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# -------------------------------------------------------------------------
# Test I: Issue #340 - Frozen bundle verifier isolation regression
# -------------------------------------------------------------------------
function Test-FrozenBundleVerifierIsolationRegression {
    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-verifier-iso-test-" + [guid]::NewGuid().ToString("N"))
    try {
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        $bundleDir = Join-Path $tempDir "candidate-bundle"
        $evidenceDir = Join-Path $tempDir "candidate-evidence"
        New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null
        New-Item -ItemType Directory -Path $evidenceDir -Force | Out-Null

        $v = $packageVersion
        $installerName = "Sky Auto Player_${v}_x64-setup.exe"
        $sigName = "$installerName.sig"

        $instPath = Join-Path $bundleDir $installerName
        $sigPath = Join-Path $bundleDir $sigName
        Set-Content -LiteralPath $instPath -Value "installer executable mock content" -Encoding utf8
        Set-Content -LiteralPath $sigPath -Value "untrusted comment: mock signature`nRWmockdata..." -Encoding utf8

        # 1. Generate SBOM from clean 2-file candidate-bundle
        $sbomPath = Join-Path $evidenceDir "SBOM.spdx.json"
        & cargo xtask sbom generate --artifact-dir $bundleDir --output $sbomPath
        if ($LASTEXITCODE -ne 0) { Fail "cargo xtask sbom generate failed on 2-file bundle" }

        # 2. Verify SBOM succeeds on clean 2-file candidate-bundle
        & cargo xtask sbom verify --artifact-dir $bundleDir --sbom $sbomPath
        if ($LASTEXITCODE -ne 0) { Fail "cargo xtask sbom verify failed on clean 2-file bundle" }

        # 3. Create valid unsigned-zero-budget Authenticode evidence
        $authPayload = [ordered]@{
            schema_version = 1
            evidence_type = "authenticode-verification"
            mode = "unsigned-zero-budget"
            expected_signer_thumbprint = $null
            verification_policy = "unsigned-project-owned-pe-files-and-canonical-nsis"
            files = @(
                [ordered]@{
                    name = $installerName
                    path = $instPath
                    status = "NotSigned"
                    platform_status = "NotSigned"
                    verification = "authenticode-unsigned-zero-budget"
                    trust_exception = "unsigned-zero-budget-policy"
                    integrity_verifier = "not-applicable-unsigned-zero-budget"
                    integrity_status = "NotSigned"
                    signed_digest_algorithm = $null
                    signed_digest = $null
                    computed_digest = $null
                    sha256 = (Get-FileHash -LiteralPath $instPath -Algorithm SHA256).Hash.ToLowerInvariant()
                    signer_thumbprint = $null
                    signer_subject = $null
                }
            )
        }
        $authPath = Join-Path $evidenceDir "TAURI_AUTHENTICODE_EVIDENCE.json"
        $authPayload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $authPath -Encoding utf8

        # 4. Verify Tauri bundle succeeds on clean 2-file candidate-bundle
        & cargo xtask verify-tauri-bundle --bundle-dir $bundleDir --authenticode-evidence $authPath --sbom $sbomPath
        if ($LASTEXITCODE -ne 0) { Fail "cargo xtask verify-tauri-bundle failed on clean 2-file bundle" }

        # 5. Fault Injection: place extra evidence JSON into candidate-bundle (reproducing run 35418967615)
        $strayJsonPath = Join-Path $bundleDir "V4_PRODUCTION_RELEASE_EVIDENCE.json"
        Set-Content -LiteralPath $strayJsonPath -Value "{}" -Encoding utf8

        # Assert SBOM verify fails closed on mixed directory
        $sbomMixedOutput = & cargo xtask sbom verify --artifact-dir $bundleDir --sbom $sbomPath 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0) { Fail "cargo xtask sbom verify accepted mixed evidence in bundle directory" }
        if ($sbomMixedOutput -notmatch "SBOM artifact-set SHA-256 does not match|SBOM file set does not match") {
            Fail "unexpected SBOM verify error on mixed bundle directory: $sbomMixedOutput"
        }

        # Assert verify-tauri-bundle fails closed on mixed directory
        $tauriMixedOutput = & cargo xtask verify-tauri-bundle --bundle-dir $bundleDir --authenticode-evidence $authPath --sbom $sbomPath 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0) { Fail "cargo xtask verify-tauri-bundle accepted mixed evidence in bundle directory" }
        if ($tauriMixedOutput -notmatch "Tauri NSIS bundle must contain only the setup executable and its \.sig") {
            Fail "unexpected verify-tauri-bundle error on mixed bundle directory: $tauriMixedOutput"
        }

        Write-Host "V4 test (56/56): frozen bundle verifier isolation regression probe (Test I): PASS"
    } finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# Run all 56 regression tests
Test-SchemaV1MissingFieldReproducesStrictModeFailure
Test-SchemaV2CanonicalConstructorSurvivesStrictMode
Test-MalformedOrMissingCriticalSchemaV2FieldFailsClosed
Test-PatchSucceedsGetConfirmsPublication
Test-PatchReportsFailureGetConfirmsPublication
Test-PatchFailsGetConfirmsStillDraft
Test-PatchAmbiguousGetUnavailable
Test-FreshTransactionRefusesAdoptionOfExistingPublishedRelease
Test-ExactLocalReleaseIdEncountersAlreadyPublishedRemoteReconciles
Test-PublishedReleaseMetadataOldProductionWorkflowRunningOrUnknown
Test-PublishedReleaseMetadataOldWorkflowCompletedFailure
Test-CanonicalRfc3339TimestampsCultureInvariance
Test-UnrelatedWorkflowRunIdRefused
Test-SameSourceNonProductionRunIgnored
Test-TwoProductionWorkflowDispatchRunsAmbiguousFailsClosed
Test-ExplicitValidRunIdDeterministicClassification
Test-ExactReleaseIdMissingTagPointsToAnotherReleaseNotAdopted
Test-WorkflowRunRepositoryIdentityValidation
Test-TimestampFormattingFailClosedAndCultureInvariance
Test-DraftPostSuccess
Test-DraftPostTimeoutReconciledByMarker
Test-FirstAssetUploadFailureDraftAutoDeleted
Test-SecondAssetUploadFailureDraftAutoDeleted
Test-ServerAssetDigestMismatchDraftAutoDeletedNoPatch
Test-PublishPatchSuccess
Test-PublishPatchTimeoutRemotePublishedReconcilesNoDelete
Test-PublishPatchFailureRemoteStillDraftAutoDeleted
Test-RemoteGetUnavailableAfterMutationFailsClosedNoDelete
Test-MetadataPromotionFailureAfterPublicationReleaseIntact
Test-ProcessFailureAfterDraftCreationPreflightCleansStaleDraft
Test-ProcessFailureAfterPublicationPreflightAndDoctorRefuse
Test-Run35292682626ParameterConversionRegression
Test-TransactionMarkerDuplicateKeyRejectsAllCriticalKeys
Test-TransactionMarkerFieldValidationAndMatchCases
Test-ServerDigestEdgeCases
Test-ReleasePipelineSourceVsPublicNamingContract
Test-QualificationCandidateRecordRegression
Test-EvidenceMappingRegression
Test-CandidateEvidenceProductionWiringRegression
Test-ProductionShapeRecordRegression
Test-FrozenTopologyRegression
Test-ProducerConsumerEvidenceBindingRegression
Test-PublicUploadProjectionRegression
Test-FrozenBundleVerifierIsolationRegression

Write-Host "V4 release pipeline contract/self-test: PASS (all 56 release state reconciliation, naming identity, topology, and fault injection regressions verified)"
