[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$pipelinePath = Join-Path $PSScriptRoot "v4_release_pipeline.ps1"
$workflowPath = Join-Path $repoRoot ".github/workflows/release-v4.yml"
$pipeline = Get-Content -LiteralPath $pipelinePath -Raw
$workflow = Get-Content -LiteralPath $workflowPath -Raw

function Fail([string]$Message) { throw "FAILED: $Message" }

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
    'selftest-update-active-playback', 'scan_performed'
)) {
    if (-not $pipeline.Contains($marker)) { Fail "pipeline marker is missing: $marker" }
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
