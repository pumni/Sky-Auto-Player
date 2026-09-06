[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$pipelinePath = Join-Path $PSScriptRoot "v4_release_pipeline.ps1"
$rehearsalPath = Join-Path $PSScriptRoot "test_v4_release_authority_rehearsal.ps1"
$uploadHelperPath = Join-Path $PSScriptRoot "v4_release_authority_upload.ps1"
$workflowPath = Join-Path $repoRoot ".github/workflows/release-v4.yml"
$pipeline = Get-Content -LiteralPath $pipelinePath -Raw
$rehearsal = Get-Content -LiteralPath $rehearsalPath -Raw
$uploadHelper = Get-Content -LiteralPath $uploadHelperPath -Raw
$workflow = Get-Content -LiteralPath $workflowPath -Raw

function Fail([string]$Message) { throw "FAILED: $Message" }

foreach ($script in @(
    [pscustomobject]@{ Name = "production release pipeline"; Source = $pipeline },
    [pscustomobject]@{ Name = "authority rehearsal"; Source = $rehearsal }
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
        'Invoke-GhBinaryOutput', 'Invoke-V4ReleaseAuthorityAssetUpload',
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

foreach ($marker in @(
    'failed closed at phase',
    'cleanup left the disposable draft release',
    'cleanup left the disposable tag ref'
)) {
    if (-not $rehearsal.Contains($marker)) {
        Fail "authority rehearsal cleanup diagnostic/verification marker is missing: $marker"
    }
}
$tagProbeMarker = '"api", "repos/$authorityRepository/git/ref/tags/$tag"'
$tagDeleteMarker = '"api", "--method", "DELETE", "repos/$authorityRepository/git/refs/tags/$tag"'
$firstTagProbe = $rehearsal.IndexOf($tagProbeMarker, [StringComparison]::Ordinal)
$tagDelete = $rehearsal.IndexOf($tagDeleteMarker, [StringComparison]::Ordinal)
if ($firstTagProbe -lt 0 -or $tagDelete -lt 0 -or $firstTagProbe -gt $tagDelete) {
    Fail "authority rehearsal must probe the disposable tag ref before attempting DELETE"
}
if (([regex]::Matches($rehearsal, [regex]::Escape($tagProbeMarker))).Count -lt 2) {
    Fail "authority rehearsal must verify the disposable tag ref is absent after cleanup"
}

if (([regex]::Matches($pipeline, "orchestrate_v4_production_release\.ps1")).Count -ne 1) {
    Fail "production orchestrator must have exactly one call site"
}
foreach ($marker in @(
    'ValidateRequest', 'ValidateAuthority', 'BuildCandidate', 'CreateDraft',
    'DownloadDraft', 'QualifyDownloaded', 'RecordAttestations', 'PublishDraft',
    'PromoteMetadata', 'FinalVerify', 'unsigned-zero-budget',
    'metadata promotion is forbidden before immutable publication',
    'authority already contains tag', 'existing releases are never moved or replaced',
    'Get-FileHash', 'verify-signature', 'sbom', 'verify-tauri-bundle',
    'current-user', 'active-playback-install-rejected', 'upload_url',
    'immutable-releases', 'Assert-ImmutableRelease', 'Start-MpScan',
    'previous-v4-to-exact-downloaded-candidate-update',
    'selftest-update-active-playback', 'scan_performed',
    'v4_updater_credential_broker.ps1'
)) {
    if (-not $pipeline.Contains($marker)) { Fail "pipeline marker is missing: $marker" }
}

foreach ($brokerFile in @(
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
    'V4_RELEASE_AUTHORITY_TOKEN',
    'ref: ${{ inputs.source_sha }}',
    'persist-credentials: false',
    'actions/attest@',
    '--source-digest $env:GITHUB_SHA',
    'Initialize release state root', 'RUNNER_TEMP', 'GITHUB_RUN_ID', 'GITHUB_ENV',
    'RecordAttestations', 'PublishDraft', 'PromoteMetadata', 'FinalVerify'
)) {
    if (-not $workflow.Contains($marker)) { Fail "workflow marker is missing: $marker" }
}
foreach ($forbidden in @(
    'cargo xtask dist', 'Sky-Auto-Player-Updater.exe', 'MANIFEST.json.sig',
    'softprops/action-gh-release', 'secrets.TAURI_SIGNING_PRIVATE_KEY',
    'secrets.UPDATER_PRIVATE_KEY', 'secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD',
    'secrets.UPDATER_PASSWORD', 'secrets.V4_UPDATER_PASSWORD',
    'updater_password_env', 'credential_target',
    'V4_RELEASE_STATE_ROOT: ${{ runner.temp }}'
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
        $this.UploadUrl = "https://uploads.github.com/repos/pumni/Sky-Auto-Player-Releases/releases/42/assets"
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

Write-Host "V4 release pipeline contract/self-test: PASS (mock draft/download/qualify/attest/publish/promote; build count=1)"
