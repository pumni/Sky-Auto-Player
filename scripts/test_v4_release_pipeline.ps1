[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$pipelinePath = Join-Path $PSScriptRoot "v4_release_pipeline.ps1"
$topologyRehearsalPath = Join-Path $PSScriptRoot "test_v4_production_topology_rehearsal.ps1"
$fixtureWrapperPath = Join-Path $PSScriptRoot "ci_tauri_update_e2e.ps1"
$fixtureCorePath = Join-Path $PSScriptRoot "ci_tauri_update_e2e_core.ps1"
$uploadHelperPath = Join-Path $PSScriptRoot "v4_release_asset_upload.ps1"
$workflowPath = Join-Path $repoRoot ".github/workflows/release-v4.yml"
$draftWorkflowPath = Join-Path $repoRoot ".github/workflows/rehearse-v4.yml"
$draftCleanupPath = Join-Path $PSScriptRoot "cleanup_v4_draft_rehearsal.ps1"
$externalStatePath = Join-Path $PSScriptRoot "v4_draft_rehearsal_external_state.ps1"
$draftLookupPath = Join-Path $PSScriptRoot "v4_release_draft_lookup.ps1"
$pipeline = Get-Content -LiteralPath $pipelinePath -Raw
$topologyRehearsal = Get-Content -LiteralPath $topologyRehearsalPath -Raw
$fixtureWrapper = Get-Content -LiteralPath $fixtureWrapperPath -Raw
$fixtureCore = Get-Content -LiteralPath $fixtureCorePath -Raw
$uploadHelper = Get-Content -LiteralPath $uploadHelperPath -Raw
$workflow = Get-Content -LiteralPath $workflowPath -Raw
$draftWorkflow = Get-Content -LiteralPath $draftWorkflowPath -Raw
$draftCleanup = Get-Content -LiteralPath $draftCleanupPath -Raw
$externalState = Get-Content -LiteralPath $externalStatePath -Raw
$draftLookup = Get-Content -LiteralPath $draftLookupPath -Raw
$testHarness = Get-Content -LiteralPath $PSCommandPath -Raw
$latestGuardPath = Join-Path $PSScriptRoot "ci_v4_release_latest_guard.ps1"
$latestGuard = Get-Content -LiteralPath $latestGuardPath -Raw

foreach ($source in @(
    [pscustomobject]@{ Name = "release workflow"; Text = $workflow },
    [pscustomobject]@{ Name = "release pipeline"; Text = $pipeline },
    [pscustomobject]@{ Name = "metadata promotion"; Text = (Get-Content -LiteralPath (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") -Raw) },
    [pscustomobject]@{ Name = "asset upload"; Text = $uploadHelper },
    [pscustomobject]@{ Name = "GitHub Latest policy guard"; Text = (Get-Content -LiteralPath (Join-Path $PSScriptRoot "ci_v4_release_latest_guard.ps1") -Raw) },
    [pscustomobject]@{ Name = "controlled draft rehearsal workflow"; Text = $draftWorkflow },
    [pscustomobject]@{ Name = "controlled draft cleanup"; Text = $draftCleanup },
    [pscustomobject]@{ Name = "controlled draft external-state check"; Text = $externalState },
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
    'ValidateSet("Baseline", "Capture", "Verify")',
    'Get-GitHubJson "repos/$canonicalRepository/releases/latest"',
    'github-latest-before.json',
    'ExpectedTag', 'ExpectedSourceSha',
    'make_latest=$(if ($Channel -eq "stable") { "true" } else { "false" })',
    'stable publication did not become the exact GitHub Latest release',
    'beta publication changed GitHub Latest identity',
    'read_only=true'
)) {
    if (-not $latestGuard.Contains($marker)) {
        Fail "GitHub Latest policy guard marker is missing: $marker"
    }
}
if ($latestGuard.Contains('^v3\.')) {
    Fail "GitHub Latest policy guard still hard-codes the retired v3-only namespace"
}

function Fail([string]$Message) { throw "FAILED: $Message" }

foreach ($marker in @(
    'name: V4 Controlled Same-Repository Draft Rehearsal',
    'workflow_dispatch:',
    'group: v4-release-${{ inputs.source_sha }}',
    'draft-rehearsal-dispatch-boundary',
    'runs-on: [self-hosted, windows, v4-release, single-tenant]',
    'environment: v4-production-release',
    'contents: write',
    'id-token: write',
    'attestations: write',
    'ref: ${{ inputs.source_sha }}',
    'persist-credentials: false',
    'ValidateRequest', 'ValidateRepository', 'BuildCandidate', 'CreateDraft',
    'DownloadDraft', 'QualifyDownloaded', 'RecordAttestations',
    'actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6',
    '--source-digest $env:GITHUB_SHA',
    '-Mode Capture', '-Mode Verify',
    'cleanup_v4_draft_rehearsal.ps1',
    'v4_draft_rehearsal_external_state.ps1',
    'if: always()',
    'actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a'
)) {
    if (-not $draftWorkflow.Contains($marker)) {
        Fail "controlled draft rehearsal workflow marker is missing: $marker"
    }
}
$draftStates = @(
    '-State ValidateRequest', '-State ValidateRepository', '-State BuildCandidate',
    '-State CreateDraft', '-State DownloadDraft', '-State QualifyDownloaded',
    '-State RecordAttestations'
)
$previousDraftStatePosition = -1
foreach ($stateMarker in $draftStates) {
    $draftStatePosition = $draftWorkflow.IndexOf($stateMarker)
    if ($draftStatePosition -lt 0 -or $draftStatePosition -lt $previousDraftStatePosition) {
        Fail "controlled draft rehearsal states are missing or out of order: $stateMarker"
    }
    $previousDraftStatePosition = $draftStatePosition
}
foreach ($forbidden in @(
    'PublishDraft', 'PromoteMetadata', 'FinalVerify',
    'create-github-app-token', 'metadata-app-token',
    'softprops/action-gh-release', 'gh release',
    'actions/create-github-app-token',
    'updater_private_key_path:', 'inputs.updater_private_key_path',
    'V4_RELEASE_AUTHORITY_TOKEN', 'V4_RELEASE_AUTHORITY_REPOSITORY',
    'make_latest = $true'
)) {
    if ($draftWorkflow.Contains($forbidden)) {
        Fail "controlled draft rehearsal workflow contains forbidden marker: $forbidden"
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

foreach ($marker in @(
    'RUNNER_TEMP', 'GITHUB_WORKSPACE', 'StateRoot must be a child of RUNNER_TEMP',
    'source_sha', 'published_at', 'git/ref/tags', '--method', 'DELETE',
    'remainingRelease', 'remainingTag', 'draft-cleanup-authorized.json',
    'release-state.json', 'releases/$releaseId', 'v4_release_draft_lookup.ps1',
    'Select-V4ReleaseByTag', '--paginate', '--slurp', 'releases?per_page=100',
    'refusing to delete a published release', 'mismatched source',
    'draft release could not be removed by release id'
)) {
    if (-not $draftCleanup.Contains($marker)) {
        Fail "controlled draft cleanup marker is missing: $marker"
    }
}
foreach ($forbidden in @('PublishDraft', 'PromoteMetadata', 'FinalVerify', 'Sky-Auto-Player-Releases', 'V4_RELEASE_AUTHORITY_')) {
    if ($draftCleanup.Contains($forbidden)) {
        Fail "controlled draft cleanup contains forbidden marker: $forbidden"
    }
}

foreach ($marker in @(
    'Capture', 'Verify',
    'raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json',
    'raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json',
    'releases/latest', '^v[0-9]+\.[0-9]+\.[0-9]+$', 'AllowAutoRedirect', 'Headers.Authorization',
    'StatusCode', 'sha256', 'external-state-before.json', 'external-state-after.json',
    'GITHUB_REPOSITORY', 'target_release_absent', 'target_tag_absent',
    'v4_release_draft_lookup.ps1', 'Select-V4ReleaseByTag', '--paginate',
    '--slurp', 'releases?per_page=100'
)) {
    if (-not $externalState.Contains($marker)) {
        Fail "controlled draft external-state marker is missing: $marker"
    }
}
foreach ($forbidden in @('--method', 'POST', 'PATCH', 'PUT', 'DELETE', 'gh release', 'Sky-Auto-Player-Releases', 'V4_RELEASE_AUTHORITY_')) {
    if ($externalState.Contains($forbidden)) {
        Fail "controlled draft external-state check contains forbidden mutation marker: $forbidden"
    }
}
if (-not $externalState.Contains('System.Net.Http.HttpMethod]::Get') -or
    $externalState.Contains('Headers.Authorization =')) {
    Fail "raw metadata endpoint check must be an explicit unauthenticated GET"
}

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
foreach ($marker in @(
    '"-State", "QualifyDownloaded"',
    '"-StateRoot", $effectiveStateRoot'
)) {
    if (-not $topologyRehearsal.Contains($marker)) {
        Fail "production-topology rehearsal marker is missing: $marker"
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
    'X-GitHub-Api-Version', '2026-03-10', 'ContentLength', 'fileLength',
    'StatusCode', 'System.Net.HttpStatusCode', 'Created',
    'SendAsync', 'ReadAsStringAsync', 'application/octet-stream'
)) {
    if (-not $uploadHelper.Contains($marker)) {
        Fail "raw release asset upload helper is missing marker: $marker"
    }
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
    'ValidateRequest', 'ValidateRepository', 'BuildCandidate', 'CreateDraft',
    'DownloadDraft', 'QualifyDownloaded', 'RecordAttestations', 'PublishDraft',
    'PromoteMetadata', 'FinalVerify', 'unsigned-zero-budget',
    'metadata promotion is forbidden before immutable publication',
    'release-metadata branch is not initialized',
    'Assert-MetadataBranchReadiness', 'metadataBootstrapContract',
    'release-metadata readiness', 'validate-monotonic', 'strictly monotonic',
    'Get-PublicMetadataDocument', 'Write-RepositoryContentFile',
    'Convert-PublishedAtToMetadataTimestamp', '$publicationDateUtc',
    'publishedRelease.target_commitish', 'published_at',
    'raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json',
    'raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json',
    'AllowAutoRedirect', 'Headers.Authorization',
    'repository already contains published release/tag', 'unpublished draft reuse',
    'published tags are immutable', 'git/refs/tags/$Tag',
    'GitHub''s successful DELETE endpoints return an empty body',
    'Get-FileHash', 'verify-signature', 'sbom', 'verify-tauri-bundle',
    'current-user', 'active-playback-install-rejected', 'upload_url',
    'Assert-ImmutableRelease $published', 'repository release is not marked immutable',
    'V4 immutable publication guard self-test', 'immutable=false rejected',
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
    Fail "ValidateRepository must not call the administration-only immutable-releases endpoint"
}
$pipelineSelfTestOutput = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $pipelinePath -State SelfTest 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $pipelineSelfTestOutput -notmatch 'immutable=false rejected; immutable=true accepted') {
    Fail "pipeline immutable publication guard self-test did not reject immutable=false"
}
if ($pipelineSelfTestOutput -notmatch 'draft false; stable publish true; beta publish false') {
    Fail "pipeline GitHub release payload self-test did not verify the make_latest JSON enum type"
}
$createDraftBody = $pipeline.Substring(
    $pipeline.IndexOf('function Invoke-CreateDraft', [StringComparison]::Ordinal),
    $pipeline.IndexOf('function Invoke-DownloadDraft', [StringComparison]::Ordinal) -
        $pipeline.IndexOf('function Invoke-CreateDraft', [StringComparison]::Ordinal)
)
$publishDraftBody = $pipeline.Substring(
    $pipeline.IndexOf('function Invoke-PublishDraft', [StringComparison]::Ordinal),
    $pipeline.IndexOf('function Invoke-RecordAttestations', [StringComparison]::Ordinal) -
        $pipeline.IndexOf('function Invoke-PublishDraft', [StringComparison]::Ordinal)
)
if (-not $createDraftBody.Contains('make_latest = Get-V4ReleaseDraftMakeLatestValue') -or
    $createDraftBody.Contains('make_latest = Get-V4ReleaseMakeLatestValue $Channel')) {
    Fail "CreateDraft must always use the draft-safe make_latest=false helper"
}
if (-not $publishDraftBody.Contains('make_latest = Get-V4ReleaseMakeLatestValue $Channel') -or
    $publishDraftBody.Contains('Get-V4ReleaseDraftMakeLatestValue')) {
    Fail "PublishDraft must use the channel-aware publication make_latest helper"
}
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
        $arguments = @(
            "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", $pipelinePath,
            "-State", "ValidateRequest",
            "-Version", $versionMatch.Groups[1].Value,
            "-Channel", $probeChannel,
            "-Tag", "v$($versionMatch.Groups[1].Value)",
            "-SourceSha", $sourceSha,
            "-WorkflowSha", $sourceSha,
            "-StateRoot", $probeRoot,
            "-ReleaseNotesPath", $NotesPath
        )
        $childOutput = (& pwsh @arguments 2>&1 | Out-String)
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
    '(?i)pre-publication',
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
        Fail "canonical v4.0.1 public release notes contain an internal gate phrase: $forbiddenPublicReleaseNotesPattern"
    }
}
Write-Host "V4.0.1 public release notes contract: PASS (no internal gate state or obsolete topology wording)"
$validNotesProbe = Invoke-ReleaseNotesValidation $validNotesPath
if ($validNotesProbe.ExitCode -ne 0 -or $validNotesProbe.Output -notmatch "V4 release identity: PASS") {
    Fail "canonical release notes were rejected by ValidateRequest. Diagnostics:`n$($validNotesProbe.Output)"
}
$wrongNotes = Get-ChildItem -LiteralPath (Join-Path $repoRoot "docs/releases") -Filter "v4.0.0-rc.*.md" |
    Where-Object { $_.Name -ne "v$packageVersion.md" } |
    Select-Object -First 1
if ($null -eq $wrongNotes) { Fail "release notes probe requires an existing mismatched v4 notes file" }
$wrongNotesProbe = Invoke-ReleaseNotesValidation $wrongNotes.FullName
if ($wrongNotesProbe.ExitCode -eq 0) {
    Fail "ValidateRequest accepted release notes for a different version"
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
    'ref: ${{ github.sha }}',
    'Derive exact release identity from checked-out source',
    'V4_RELEASE_SOURCE_SHA=$sourceSha',
    'V4_RELEASE_VERSION=$version',
    'V4_RELEASE_CHANNEL=$channel',
    'V4_RELEASE_TAG=$tag',
    'V4_RELEASE_NOTES_PATH=$notesPath',
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
    'actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349',
    'app-id: ${{ vars.V4_RELEASE_METADATA_APP_ID }}',
    'private-key: ${{ secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY }}',
    'owner: ${{ github.repository_owner }}',
    'repositories: ${{ github.event.repository.name }}',
    'permission-contents: write',
    'GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}',
    'Snapshot GitHub Latest before publication',
    'Verify GitHub Latest channel policy before metadata promotion',
    'scripts/ci_v4_release_latest_guard.ps1',
    'Verify isolated production runner boundary',
    'verify_v4_release_runner.ps1',
    'cleanup_v4_release_state.ps1',
    'RecordAttestations', 'PublishDraft', 'PromoteMetadata', 'FinalVerify'
)) {
    if (-not $workflow.Contains($marker)) { Fail "workflow marker is missing: $marker" }
}
if ($workflow.Contains('inputs:') -or $workflow.Contains('inputs.')) {
    Fail "production release workflow must not expose semantic workflow_dispatch inputs"
}
$metadataTokenMarker = 'GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}'
$metadataTokenUses = ([regex]::Matches($workflow, [regex]::Escape($metadataTokenMarker))).Count
if ($metadataTokenUses -ne 1) {
    Fail "metadata App installation token must be used exactly once"
}
$metadataPrivateKeyMarker = 'secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY'
$metadataPrivateKeyUses = ([regex]::Matches($workflow, [regex]::Escape($metadataPrivateKeyMarker))).Count
if ($metadataPrivateKeyUses -ne 1) {
    Fail "metadata App private key must be consumed exactly once by the token-mint action"
}
$captureLatestStep = $workflow.IndexOf('- name: Snapshot GitHub Latest before publication', [StringComparison]::Ordinal)
$publishStep = $workflow.IndexOf('- name: Publish the already-qualified draft immutably', [StringComparison]::Ordinal)
$latestPolicyStep = $workflow.IndexOf('- name: Verify GitHub Latest channel policy before metadata promotion', [StringComparison]::Ordinal)
$metadataTokenStep = $workflow.IndexOf('- name: Mint release-metadata GitHub App token', [StringComparison]::Ordinal)
if ($captureLatestStep -lt 0 -or $publishStep -lt 0 -or $latestPolicyStep -lt 0 -or $metadataTokenStep -lt 0 -or
    $captureLatestStep -ge $publishStep -or $publishStep -ge $latestPolicyStep -or $latestPolicyStep -ge $metadataTokenStep) {
    Fail "GitHub Latest capture/policy guard ordering is not fail-closed"
}
$latestPolicyStepEnd = $workflow.IndexOf("`n      - name:", $latestPolicyStep + 1, [StringComparison]::Ordinal)
if ($latestPolicyStepEnd -lt 0) { $latestPolicyStepEnd = $workflow.Length }
$latestPolicyBlock = $workflow.Substring($latestPolicyStep, $latestPolicyStepEnd - $latestPolicyStep)
$captureLatestStepEnd = $workflow.IndexOf("`n      - name:", $captureLatestStep + 1, [StringComparison]::Ordinal)
if ($captureLatestStepEnd -lt 0) { $captureLatestStepEnd = $workflow.Length }
$captureLatestBlock = $workflow.Substring($captureLatestStep, $captureLatestStepEnd - $captureLatestStep)
if (-not $captureLatestBlock.Contains('GH_TOKEN: ${{ github.token }}') -or
    -not $captureLatestBlock.Contains('scripts/ci_v4_release_latest_guard.ps1') -or
    -not $captureLatestBlock.Contains('-Mode Capture') -or
    -not $captureLatestBlock.Contains('-StateRoot')) {
    Fail "pre-publication Latest snapshot must be read-only and use the isolated state root"
}
if (-not $latestPolicyBlock.Contains('GH_TOKEN: ${{ github.token }}') -or
    -not $latestPolicyBlock.Contains('scripts/ci_v4_release_latest_guard.ps1') -or
    -not $latestPolicyBlock.Contains('-Mode Verify') -or
    -not $latestPolicyBlock.Contains('-ExpectedSourceSha')) {
    Fail "post-publication Latest policy guard must be read-only and verify exact source identity"
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
    [bool]$Downloaded = $false
    [bool]$Qualified = $false
    [bool]$Attested = $false
    [bool]$Published = $false
    [bool]$immutable = $false
    [bool]$Promoted = $false
    [bool]$UploadedThroughReleaseUrl = $false
    [string]$UploadUrl = ""
    [bool]$ExactDownloadedBytes = $false

    [void] BuildCandidate() {
        if ($this.BuildCount -ne 0) { throw "candidate rebuilt" }
        $this.BuildCount++
    }
    [void] CreateDraft() {
        if ($this.BuildCount -ne 1 -or $this.Draft) { throw "draft ordering violation" }
        $this.Draft = $true
        $this.UploadedThroughReleaseUrl = $true
        $this.UploadUrl = "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/42/assets"
    }
    [void] AssertExactDraftUpload() {
        if (-not $this.Draft -or -not $this.UploadedThroughReleaseUrl -or
            $this.UploadUrl -notmatch '^https://uploads\.github\.com/.+/assets$') {
            throw "release-specific upload_url was not used"
        }
    }
    [void] DownloadDraft() {
        if (-not $this.Draft -or $this.Published) { throw "download ordering violation" }
        $this.Downloaded = $true
        $this.ExactDownloadedBytes = $true
    }
    [void] QualifyDownloaded() {
        if (-not $this.Downloaded -or -not $this.ExactDownloadedBytes) { throw "qualification did not use downloaded bytes" }
        $this.Qualified = $true
    }
    [void] PublishDraft() {
        if (-not $this.Qualified -or -not $this.Attested -or $this.Published) { throw "publication ordering violation" }
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
$mock.CreateDraft()
$mock.AssertExactDraftUpload()
$mock.DownloadDraft()
$mock.QualifyDownloaded()
try {
    $mock.PromoteMetadata()
    Fail "mock promotion before publication was accepted"
} catch {
    if ($_.Exception.Message -notmatch "promotion before immutable publication") { throw }
}
$mock.Attested = $true
$mock.PublishDraft()
$mock.PromoteMetadata()
if ($mock.BuildCount -ne 1 -or -not $mock.Promoted -or $mock.Draft -or -not $mock.Published -or -not $mock.immutable -or -not $mock.UploadedThroughReleaseUrl -or -not $mock.ExactDownloadedBytes) {
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
function Fail([string]$Message) { throw $Message }
function Get-ExpectedInstallerName { return $TestInstaller }

$pipelineCode = Get-Content -LiteralPath $PipelinePath -Raw
$startIdx = $pipelineCode.IndexOf('function Assert-EvidenceIdentity(')
if ($startIdx -lt 0) { throw 'Could not locate Assert-EvidenceIdentity function start' }
$openBrace = $pipelineCode.IndexOf('{', $startIdx)
if ($openBrace -lt 0) { throw 'Could not locate Assert-EvidenceIdentity body start' }
$closeBrace = $pipelineCode.IndexOf("`n}", $openBrace)
if ($closeBrace -lt 0) { throw 'Could not locate Assert-EvidenceIdentity body end' }
$fnBody = $pipelineCode.Substring($openBrace + 1, $closeBrace - $openBrace - 1)

$fn = [scriptblock]::Create("param([string]`$ProductionPath, [string]`$QualificationPath, [object[]]`$Records)`n$fnBody")

$recs = @(
    [pscustomobject]@{ name = $TestInstaller; size = [int64]1234567; sha256 = $TestInstallerSha },
    [pscustomobject]@{ name = "$TestInstaller.sig"; size = [int64]512; sha256 = $TestSigSha },
    [pscustomobject]@{ name = 'V4_PRODUCTION_RELEASE_EVIDENCE.json'; size = [int64]100; sha256 = ('1' * 64) },
    [pscustomobject]@{ name = 'V4_QUALIFICATION_EVIDENCE.json'; size = [int64]100; sha256 = ('2' * 64) },
    [pscustomobject]@{ name = 'TAURI_AUTHENTICODE_EVIDENCE.json'; size = [int64]100; sha256 = $TestAuthSha },
    [pscustomobject]@{ name = 'SBOM.spdx.json'; size = [int64]100; sha256 = $TestSbomSha }
)

& $fn $TargetProdPath $QualPath $recs
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

    # 4. Unsafe name fails before CreateDraft
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

    # 5. Release-name collision fails before CreateDraft
    if ($pipeline -notmatch 'release asset name collision detected') {
        Fail "pipeline must contain release asset name collision check before CreateDraft"
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

Test-SafeReleaseAssetNameContract

Write-Host "V4 release pipeline contract/self-test: PASS (mock draft/download/qualify/attest/publish/promote; build count=1)"
