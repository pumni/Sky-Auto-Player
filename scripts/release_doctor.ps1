[CmdletBinding()]
param(
    [string]$Tag,
    [string]$Version,
    [ValidateSet("stable", "beta")]
    [string]$Channel,
    [string]$Repository = "pumni/Sky-Auto-Player",
    [string]$StateRoot,
    [string]$StatePath,
    [string]$RunId,
    [string]$WorkflowSha,
    [ValidateSet("Text", "Json")]
    [string]$Format = "Text",

    # Offline hooks for isolated, network-free unit/regression testing
    [switch]$Offline,
    [object]$OfflineExternalRelease,
    [object]$OfflineLatestRelease,
    [object]$OfflineMetadata,
    [object]$OfflinePublicMetadata,
    [object]$OfflineTagRef,
    [object]$OfflineWorkflowRun,
    [object[]]$OfflineWorkflowRuns
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

function Format-CanonicalRfc3339Timestamp([object]$timestamp) {
    if ($null -eq $timestamp -or [string]::IsNullOrWhiteSpace([string]$timestamp)) {
        return $null
    }
    if ($timestamp -is [DateTimeOffset]) {
        return $timestamp.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [System.Globalization.CultureInfo]::InvariantCulture)
    }
    if ($timestamp -is [DateTime]) {
        return $timestamp.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [System.Globalization.CultureInfo]::InvariantCulture)
    }
    $str = [string]$timestamp
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

function Test-ReleaseWorkflowRunCriteria([object]$run, [string]$expectedSha, [string]$expectedRepo) {
    if ($null -eq $run) { return $false }

    # 1. workflow path == .github/workflows/release-v4.yml
    $runPath = ""
    if ($null -ne $run.PSObject.Properties['path'] -and -not [string]::IsNullOrWhiteSpace([string]$run.path)) {
        $runPath = [string]$run.path
    } elseif ($null -ne $run.PSObject.Properties['workflow_url'] -and -not [string]::IsNullOrWhiteSpace([string]$run.workflow_url)) {
        $runPath = [string]$run.workflow_url
    }
    $isReleaseWorkflow = ($runPath -match '(^|[/\\])\.github[/\\]workflows[/\\]release-v4\.yml$' -or $runPath -eq ".github/workflows/release-v4.yml")
    if (-not $isReleaseWorkflow) { return $false }

    # 2. event == workflow_dispatch
    $event = if ($null -ne $run.PSObject.Properties['event']) { [string]$run.event } else { "" }
    if ($event -ne "workflow_dispatch") { return $false }

    # 3. head_sha == exact target source SHA
    if (-not [string]::IsNullOrWhiteSpace($expectedSha)) {
        $headSha = if ($null -ne $run.PSObject.Properties['head_sha']) { [string]$run.head_sha } else { "" }
        if ($headSha.ToLowerInvariant() -ne $expectedSha.ToLowerInvariant()) { return $false }
    }

    # 4. repository == pumni/Sky-Auto-Player (missing repository is rejected, not implicitly trusted)
    $repoName = ""
    if ($null -ne $run.PSObject.Properties['repository']) {
        if ($run.repository -is [string]) {
            $repoName = [string]$run.repository
        } elseif ($null -ne $run.repository.PSObject.Properties['full_name']) {
            $repoName = [string]$run.repository.full_name
        }
    }
    if ([string]::IsNullOrWhiteSpace($repoName)) { return $false }
    if ($repoName.ToLowerInvariant() -ne $expectedRepo.ToLowerInvariant()) { return $false }

    return $true
}

function Assert-ReleaseWorkflowRunCriteria([object]$run, [string]$expectedSha, [string]$expectedRepo, [string]$runId) {
    if ($null -eq $run) {
        throw "Specified workflow run ID '$runId' was not found in repository '$expectedRepo'."
    }

    $runPath = ""
    if ($null -ne $run.PSObject.Properties['path'] -and -not [string]::IsNullOrWhiteSpace([string]$run.path)) {
        $runPath = [string]$run.path
    } elseif ($null -ne $run.PSObject.Properties['workflow_url'] -and -not [string]::IsNullOrWhiteSpace([string]$run.workflow_url)) {
        $runPath = [string]$run.workflow_url
    }
    $isReleaseWorkflow = ($runPath -match '(^|[/\\])\.github[/\\]workflows[/\\]release-v4\.yml$' -or $runPath -eq ".github/workflows/release-v4.yml")
    if (-not $isReleaseWorkflow) {
        throw "Workflow run '$runId' is refused: workflow path '$runPath' does not match required '.github/workflows/release-v4.yml'"
    }

    $event = if ($null -ne $run.PSObject.Properties['event']) { [string]$run.event } else { "" }
    if ($event -ne "workflow_dispatch") {
        throw "Workflow run '$runId' is refused: event '$event' does not match required 'workflow_dispatch'"
    }

    if (-not [string]::IsNullOrWhiteSpace($expectedSha)) {
        $headSha = if ($null -ne $run.PSObject.Properties['head_sha']) { [string]$run.head_sha } else { "" }
        if ($headSha.ToLowerInvariant() -ne $expectedSha.ToLowerInvariant()) {
            throw "Workflow run '$runId' is refused: head_sha '$headSha' does not match target source SHA '$expectedSha'"
        }
    }

    $repoName = ""
    if ($null -ne $run.PSObject.Properties['repository']) {
        if ($run.repository -is [string]) {
            $repoName = [string]$run.repository
        } elseif ($null -ne $run.repository.PSObject.Properties['full_name']) {
            $repoName = [string]$run.repository.full_name
        }
    }
    if ([string]::IsNullOrWhiteSpace($repoName)) {
        throw "Workflow run '$runId' is refused: repository identity is missing or empty"
    }
    if ($repoName.ToLowerInvariant() -ne $expectedRepo.ToLowerInvariant()) {
        throw "Workflow run '$runId' is refused: repository '$repoName' does not match required '$expectedRepo'"
    }
}

# 1. Resolve identity
if ([string]::IsNullOrWhiteSpace($Tag) -and [string]::IsNullOrWhiteSpace($Version)) {
    $cargoPath = Join-Path $repoRoot "desktop/src-tauri/Cargo.toml"
    if (Test-Path -LiteralPath $cargoPath -PathType Leaf) {
        $cargo = Get-Content -LiteralPath $cargoPath -Raw
        if ($cargo -match '(?m)^version\s*=\s*"([^"]+)"') {
            $Version = $Matches[1]
        }
    }
}

if ([string]::IsNullOrWhiteSpace($Version) -and -not [string]::IsNullOrWhiteSpace($Tag)) {
    $Version = if ($Tag.StartsWith("v", [StringComparison]::OrdinalIgnoreCase)) { $Tag.Substring(1) } else { $Tag }
}
if ([string]::IsNullOrWhiteSpace($Tag) -and -not [string]::IsNullOrWhiteSpace($Version)) {
    $Tag = "v$Version"
}

if ([string]::IsNullOrWhiteSpace($Version) -or [string]::IsNullOrWhiteSpace($Tag)) {
    throw "release-doctor requires Tag or Version"
}

if ([string]::IsNullOrWhiteSpace($Channel)) {
    $Channel = if ($Version.Contains("-")) { "beta" } else { "stable" }
}

# 2. Inspect local state if provided
$resolvedStatePath = $null
if (-not [string]::IsNullOrWhiteSpace($StatePath)) {
    $resolvedStatePath = [IO.Path]::GetFullPath($StatePath)
} elseif (-not [string]::IsNullOrWhiteSpace($StateRoot)) {
    $resolvedStatePath = Join-Path ([IO.Path]::GetFullPath($StateRoot)) "release-state.json"
}

$localState = $null
if ($null -ne $resolvedStatePath -and (Test-Path -LiteralPath $resolvedStatePath -PathType Leaf)) {
    try {
        $localState = Get-Content -LiteralPath $resolvedStatePath -Raw | ConvertFrom-Json
    } catch {
        $localState = $null
    }
}

# 3. Read-only external queries (safe, read-only GET requests only)
function Invoke-ReadOnlyGet([string]$Endpoint) {
    if ($Offline) { return $null }
    $out = & gh api $Endpoint --header "Accept: application/vnd.github+json" 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    $text = ($out -join "`n")
    if ([string]::IsNullOrWhiteSpace($text)) { return $null }
    try {
        return ($text | ConvertFrom-Json)
    } catch {
        return $null
    }
}

$externalRelease = if ($null -ne $OfflineExternalRelease) {
    $OfflineExternalRelease
} else {
    Invoke-ReadOnlyGet "repos/$Repository/releases/tags/$Tag"
}

$tagRef = if ($null -ne $OfflineTagRef) {
    $OfflineTagRef
} else {
    Invoke-ReadOnlyGet "repos/$Repository/git/ref/tags/$Tag"
}

$latestRelease = if ($null -ne $OfflineLatestRelease) {
    $OfflineLatestRelease
} else {
    Invoke-ReadOnlyGet "repos/$Repository/releases/latest"
}

$channelMetadataResponse = if ($null -ne $OfflineMetadata) {
    $OfflineMetadata
} else {
    Invoke-ReadOnlyGet "repos/$Repository/contents/channels/$Channel/latest.json?ref=release-metadata"
}

$rawPublicMetadata = if ($null -ne $OfflinePublicMetadata) {
    $OfflinePublicMetadata
} else {
    if (-not $Offline) {
        try {
            $rawUrl = "https://raw.githubusercontent.com/$Repository/release-metadata/channels/$Channel/latest.json"
            $resp = Invoke-RestMethod -Uri $rawUrl -Method Get -TimeoutSec 10 -ErrorAction SilentlyContinue
            $resp
        } catch {
            $null
        }
    } else {
        $null
    }
}

# Parse channel metadata content if returned as GitHub contents object
$channelMetadata = $null
if ($null -ne $channelMetadataResponse) {
    if ($null -ne $channelMetadataResponse.PSObject.Properties['content']) {
        try {
            $decodedBytes = [Convert]::FromBase64String([string]$channelMetadataResponse.content)
            $decodedText = [Text.Encoding]::UTF8.GetString($decodedBytes)
            $channelMetadata = $decodedText | ConvertFrom-Json
        } catch {
            $channelMetadata = $null
        }
    } elseif ($null -ne $channelMetadataResponse.PSObject.Properties['version']) {
        $channelMetadata = $channelMetadataResponse
    }
}

# 4. Assess external truth
$releaseExists = ($null -ne $externalRelease)
$isDraft = ($releaseExists -and $null -ne $externalRelease.PSObject.Properties['draft'] -and [bool]$externalRelease.draft)
$rawPublishedAt = if ($releaseExists -and $null -ne $externalRelease.PSObject.Properties['published_at'] -and -not [string]::IsNullOrWhiteSpace([string]$externalRelease.published_at)) { [string]$externalRelease.published_at } else { "" }
$publishedAt = Format-CanonicalRfc3339Timestamp $rawPublishedAt
$isMalformedPublishedAt = ($releaseExists -and -not $isDraft -and -not [string]::IsNullOrWhiteSpace($rawPublishedAt) -and [string]::IsNullOrWhiteSpace($publishedAt))
$isPublished = ($releaseExists -and -not $isDraft -and (-not [string]::IsNullOrWhiteSpace($publishedAt) -or $isMalformedPublishedAt))
$isImmutable = ($releaseExists -and $null -ne $externalRelease.PSObject.Properties['immutable'] -and [bool]$externalRelease.immutable)
$targetCommitish = if ($releaseExists -and $null -ne $externalRelease.PSObject.Properties['target_commitish']) { [string]$externalRelease.target_commitish } else { "" }

$latestTag = if ($null -ne $latestRelease -and $null -ne $latestRelease.PSObject.Properties['tag_name']) { [string]$latestRelease.tag_name } else { "" }
$isLatest = ($latestTag -eq $Tag)

$metadataVersion = if ($null -ne $channelMetadata -and $null -ne $channelMetadata.PSObject.Properties['version']) { [string]$channelMetadata.version } else { "" }
$publicMetadataVersion = if ($null -ne $rawPublicMetadata -and $null -ne $rawPublicMetadata.PSObject.Properties['version']) { [string]$rawPublicMetadata.version } else { "" }

$publicAssets = @()
if ($releaseExists -and $null -ne $externalRelease.PSObject.Properties['assets']) {
    $publicAssets = @($externalRelease.assets | ForEach-Object { [string]$_.name })
}

$expectedInstallerName = "Sky.Auto.Player_${Version}_x64-setup.exe"
$expectedSignatureName = "Sky.Auto.Player_${Version}_x64-setup.exe.sig"
$hasExactPublicAssets = ($publicAssets.Count -eq 2 -and $publicAssets.Contains($expectedInstallerName) -and $publicAssets.Contains($expectedSignatureName))

# Target source SHA resolution
$targetSourceSha = if (-not [string]::IsNullOrWhiteSpace($WorkflowSha)) {
    $WorkflowSha
} elseif ($null -ne $localState -and $null -ne $localState.PSObject.Properties['source_sha'] -and -not [string]::IsNullOrWhiteSpace([string]$localState.source_sha)) {
    [string]$localState.source_sha
} elseif ($releaseExists -and -not [string]::IsNullOrWhiteSpace($targetCommitish)) {
    $targetCommitish
} else {
    $null
}

# Query release workflow execution evidence
$workflowRun = $null
$multipleRunsAmbiguous = $false

if (-not [string]::IsNullOrWhiteSpace($RunId)) {
    $rawRun = if ($null -ne $OfflineWorkflowRun) {
        $OfflineWorkflowRun
    } else {
        if (-not $Offline) {
            Invoke-ReadOnlyGet "repos/$Repository/actions/runs/$RunId"
        } else {
            $null
        }
    }
    if ($null -ne $rawRun -and $null -ne $rawRun.PSObject.Properties['workflow_runs'] -and $rawRun.workflow_runs.Count -gt 0) {
        $rawRun = $rawRun.workflow_runs[0]
    }
    Assert-ReleaseWorkflowRunCriteria $rawRun $targetSourceSha $Repository $RunId
    $workflowRun = $rawRun
} else {
    $rawRuns = @()
    if ($null -ne $OfflineWorkflowRuns) {
        $rawRuns = @($OfflineWorkflowRuns)
    } elseif ($null -ne $OfflineWorkflowRun) {
        if ($null -ne $OfflineWorkflowRun.PSObject.Properties['workflow_runs']) {
            $rawRuns = @($OfflineWorkflowRun.workflow_runs)
        } elseif ($null -ne $OfflineWorkflowRun.PSObject.Properties['runs']) {
            $rawRuns = @($OfflineWorkflowRun.runs)
        } else {
            $rawRuns = @($OfflineWorkflowRun)
        }
    } elseif (-not $Offline -and -not [string]::IsNullOrWhiteSpace($targetSourceSha)) {
        $runsResponse = Invoke-ReadOnlyGet "repos/$Repository/actions/workflows/release-v4.yml/runs?head_sha=$targetSourceSha&per_page=10"
        if ($null -ne $runsResponse -and $null -ne $runsResponse.PSObject.Properties['workflow_runs']) {
            $rawRuns = @($runsResponse.workflow_runs)
        } elseif ($null -ne $runsResponse -and $null -ne $runsResponse.PSObject.Properties['runs']) {
            $rawRuns = @($runsResponse.runs)
        }
    }

    $validRuns = @($rawRuns | Where-Object { Test-ReleaseWorkflowRunCriteria $_ $targetSourceSha $Repository })
    if ($validRuns.Count -eq 1) {
        $workflowRun = $validRuns[0]
    } elseif ($validRuns.Count -gt 1) {
        $multipleRunsAmbiguous = $true
        $workflowRun = $null
    } else {
        $workflowRun = $null
    }
}

$workflowOutcome = "unknown"
$workflowRunId = $null
$workflowRunStatus = $null
$workflowRunConclusion = $null
$workflowRunUrl = ""

if ($null -ne $workflowRun) {
    if ($null -ne $workflowRun.PSObject.Properties['id']) {
        $workflowRunId = [string]$workflowRun.id
    }
    if ($null -ne $workflowRun.PSObject.Properties['status']) {
        $workflowRunStatus = [string]$workflowRun.status
    }
    if ($null -ne $workflowRun.PSObject.Properties['conclusion']) {
        $workflowRunConclusion = [string]$workflowRun.conclusion
    }
    if ($null -ne $workflowRun.PSObject.Properties['html_url']) {
        $workflowRunUrl = [string]$workflowRun.html_url
    }

    if ($workflowRunStatus -eq "completed") {
        $workflowOutcome = if ($workflowRunConclusion -eq "failure") {
            "failure"
        } elseif ($workflowRunConclusion -eq "success") {
            "success"
        } else {
            $workflowRunConclusion
        }
    } elseif ($workflowRunStatus -in @("in_progress", "queued", "waiting", "requested", "pending")) {
        $workflowOutcome = "in_progress"
    }
}

# 5. Diagnostic state machine: separation of monotonic lifecycle phase from diagnostic classification
$persistedPhase = if ($null -ne $localState -and $null -ne $localState.PSObject.Properties['phase'] -and -not [string]::IsNullOrWhiteSpace([string]$localState.phase)) {
    [string]$localState.phase
} else {
    $null
}

$externalPhase = $null
$classification = $null
$operatorReviewRequired = $false
$assessment = ""
$recoveryGuidance = ""
$findings = @()

if ($isPublished) {
    $metadataMatches = ($metadataVersion -eq $Version)
    $rawMetadataMatches = ($publicMetadataVersion -eq $Version)
    $latestMatches = ($Channel -ne "stable" -or $isLatest)

    if ($metadataMatches -and $rawMetadataMatches -and $latestMatches -and $hasExactPublicAssets -and $isImmutable -and -not $isMalformedPublishedAt) {
        $externalPhase = "COMPLETE"
        $classification = $null
        $operatorReviewRequired = $false
        $assessment = "Release $Tag is published, immutable, GitHub Latest policy is satisfied, public assets are canonical, and channel metadata is promoted monotonically."
        $recoveryGuidance = "Release transaction completed successfully. No recovery action needed."
    } else {
        $externalPhase = "PUBLISHED_PENDING_METADATA"

        if ($isMalformedPublishedAt) {
            $findings += "External release published_at is invalid/non-canonical: '$rawPublishedAt'"
            $operatorReviewRequired = $true
            $classification = "POST_PUBLICATION_INCIDENT"
        }

        if ($workflowOutcome -eq "failure") {
            $classification = "POST_PUBLICATION_INCIDENT"
            $operatorReviewRequired = $true
            $findings += "Release workflow run $(if ($null -ne $workflowRunId) { "($workflowRunId)" } else { '' }) completed with conclusion 'failure'"
            if (-not $metadataMatches) {
                $findings += "Channel metadata on release-metadata is version '$metadataVersion' (expected '$Version')"
            }
            if (-not $rawMetadataMatches -and -not [string]::IsNullOrWhiteSpace($publicMetadataVersion)) {
                $findings += "Unauthenticated raw metadata serves version '$publicMetadataVersion' (expected '$Version')"
            }
            if (-not $latestMatches) {
                $findings += "Stable release is not currently GitHub Latest (current Latest: '$latestTag')"
            }
            if (-not $hasExactPublicAssets) {
                $findings += "Public release assets differ from canonical pair [found: $($publicAssets -join ', ')]"
            }
            if (-not $isImmutable) {
                $findings += "GitHub Release is not marked immutable"
            }

            $assessment = "Release $Tag has been immutably published on GitHub, but post-publication workflow failed ($($findings -join '; '))."
            $recoveryGuidance = @"
POST-PUBLICATION RECOVERY CONTRACT:
1. IMMUTABILITY GUARANTEE:
   - Release $Tag and its git tag are PUBLISHED and IMMUTABLE.
   - NEVER delete, recreate, or replace tag $Tag.
   - NEVER delete, edit, or replace the published GitHub release.
   - NEVER rerun the failed workflow run (it will fail closed because published releases are immutable).
   - NEVER dispatch another release with version $Version.
2. CORRECTIVE RELEASE TARGET:
   - The default corrective path is a new SemVer release (expected next patch version).
   - Fix the underlying issue on a corrective branch and dispatch a fresh release for the new version.
3. METADATA RECOVERY:
   - Do NOT manually promote release-metadata without an explicitly reviewed and authorized post-publication recovery design compliant with the release authority contract.
"@
        } elseif ($workflowOutcome -eq "in_progress" -and -not $isMalformedPublishedAt) {
            $classification = $null
            $operatorReviewRequired = $false
            $assessment = "Release $Tag is published on GitHub, and workflow run $(if ($null -ne $workflowRunId) { "($workflowRunId)" } else { '' }) is currently in progress."
            $recoveryGuidance = "Release workflow is currently in flight. Await workflow completion. Do not interrupt, cancel, or rerun."
        } elseif ($workflowOutcome -eq "unknown" -and -not $isMalformedPublishedAt) {
            $classification = $null
            $operatorReviewRequired = $true
            if (-not $metadataMatches) {
                $findings += "Channel metadata on release-metadata is version '$metadataVersion' (expected '$Version')"
            }
            if ($multipleRunsAmbiguous) {
                $findings += "Multiple production workflow runs found for SHA '$targetSourceSha'; explicit --run-id required"
                $assessment = "Release $Tag is immutably published on GitHub, but channel metadata is not yet promoted to $Version and multiple matching workflow runs exist. Classification cannot be guessed; explicit --run-id is required."
                $recoveryGuidance = @"
OPERATOR REVIEW REQUIRED:
- Release $Tag is published and immutable.
- Channel metadata has not yet been promoted to $Version.
- Multiple workflow runs exist for source SHA '$targetSourceSha'. Provide explicit --run-id <run_id> for deterministic diagnosis.
- If the workflow run completed with failure, treat as POST_PUBLICATION_INCIDENT.
- NEVER delete or recreate tag $Tag or published release $Tag.
"@
            } else {
                $findings += "Workflow execution outcome is unknown"
                $assessment = "Release $Tag is immutably published on GitHub, but channel metadata is not yet promoted to $Version and workflow run status is unknown ($($findings -join '; '))."
                $recoveryGuidance = @"
OPERATOR REVIEW REQUIRED:
- Release $Tag is published and immutable.
- Channel metadata has not yet been promoted to $Version.
- Workflow run state is unknown. Check GitHub Actions workflow run status or supply --run-id.
- If the workflow run completed with failure, treat as POST_PUBLICATION_INCIDENT.
- NEVER delete or recreate tag $Tag or published release $Tag.
"@
            }
        } else {
            # Workflow completed with success or cancelled/timed_out, or isMalformedPublishedAt, but channel metadata is still old
            $classification = "POST_PUBLICATION_INCIDENT"
            $operatorReviewRequired = $true
            if (-not [string]::IsNullOrWhiteSpace($workflowRunConclusion)) {
                $findings += "Workflow completed with conclusion '$workflowRunConclusion' but channel metadata remains version '$metadataVersion'"
            }
            if (-not $metadataMatches) {
                $findings += "Channel metadata on release-metadata is version '$metadataVersion' (expected '$Version')"
            }
            $assessment = "Release $Tag has been immutably published on GitHub, but post-publication promotion did not complete ($($findings -join '; '))."
            $recoveryGuidance = @"
POST-PUBLICATION RECOVERY CONTRACT:
1. IMMUTABILITY GUARANTEE:
   - Release $Tag and its git tag are PUBLISHED and IMMUTABLE.
   - NEVER delete, recreate, or replace tag $Tag.
   - NEVER delete, edit, or replace the published GitHub release.
   - NEVER rerun the failed workflow run (it will fail closed because published releases are immutable).
   - NEVER dispatch another release with version $Version.
2. CORRECTIVE RELEASE TARGET:
   - The default corrective path is a new SemVer release (expected next patch version).
   - Fix the underlying issue on a corrective branch and dispatch a fresh release for the new version.
3. METADATA RECOVERY:
   - Do NOT manually promote release-metadata without an explicitly reviewed and authorized post-publication recovery design compliant with the release authority contract.
"@
        }
    }
} elseif ($releaseExists -and $isDraft) {
    $externalPhase = "QUALIFIED"
    $localAttested = ($null -ne $localState -and $null -ne $localState.PSObject.Properties['attested'] -and [bool]$localState.attested)
    $localQualified = ($null -ne $localState -and $null -ne $localState.PSObject.Properties['qualified_after_download'] -and [bool]$localState.qualified_after_download)

    if ($localAttested -and $localQualified) {
        $classification = $null
        $assessment = "Draft release exists on GitHub and candidate bytes are qualified. Ready for PublishRelease."
        $recoveryGuidance = "Proceed with immutable publication."
    } else {
        $classification = "RECOVERABLE_PRE_PUBLICATION_FAILURE"
        $assessment = "Draft release exists on GitHub but has not completed qualification/attestation, or encountered a pre-publication failure."
        $recoveryGuidance = "No immutable release has been published. The unpublished draft release may be discarded or recreated by a fresh dispatch or rerun."
    }
} elseif ($null -ne $tagRef) {
    $externalPhase = $null
    $classification = "POST_PUBLICATION_INCIDENT"
    $operatorReviewRequired = $true
    $assessment = "Git tag $Tag exists in the repository without an associated GitHub release."
    $recoveryGuidance = "Published tag exists. Requires investigation under release authority rules. Do not delete tag without authorization."
} else {
    $externalPhase = $null
    $localReady = ($null -ne $localState -and $null -ne $localState.PSObject.Properties['phase'] -and [string]$localState.phase -eq "READY")
    if ($localReady) {
        $classification = $null
        $assessment = "Candidate build is ready locally; draft release has not yet been created on GitHub."
        $recoveryGuidance = "Ready for PublishRelease."
    } else {
        $classification = "NOT_READY"
        $assessment = "Release request has not yet reached candidate build stage."
        $recoveryGuidance = "Dispatch release pipeline from canonical repository main branch."
    }
}

# 6. Output report (strictly separating monotonic phase from diagnostic classification)
$report = [ordered]@{
    tag = $Tag
    version = $Version
    channel = $Channel
    repository = $Repository
    persisted_phase = $persistedPhase
    external_phase = $externalPhase
    classification = $classification
    operator_review_required = [bool]$operatorReviewRequired
    workflow_outcome = $workflowOutcome
    workflow_run_id = $workflowRunId
    external_truth = [ordered]@{
        release_exists = $releaseExists
        status = if ($isPublished) { "PUBLISHED" } elseif ($isDraft) { "DRAFT" } else { "MISSING" }
        immutable = $isImmutable
        published_at = $publishedAt
        target_commitish = $targetCommitish
        github_latest_tag = $latestTag
        is_latest = $isLatest
        public_assets = $publicAssets
        channel_metadata_version = $metadataVersion
        public_raw_metadata_version = $publicMetadataVersion
        workflow_run_status = $workflowRunStatus
        workflow_run_conclusion = $workflowRunConclusion
        workflow_run_url = $workflowRunUrl
    }
    local_truth = if ($null -ne $localState) {
        [ordered]@{
            state_file_found = $true
            schema_version = if ($null -ne $localState.PSObject.Properties['schema_version']) { [int]$localState.schema_version } else { $null }
            phase = if ($null -ne $localState.PSObject.Properties['phase']) { [string]$localState.phase } else { $null }
            published = if ($null -ne $localState.PSObject.Properties['published']) { [bool]$localState.published } else { $null }
            immutable = if ($null -ne $localState.PSObject.Properties['immutable']) { [bool]$localState.immutable } else { $null }
            published_at = Format-CanonicalRfc3339Timestamp $localState.published_at
            promoted_at = Format-CanonicalRfc3339Timestamp $localState.promoted_at
            final_verified = if ($null -ne $localState.PSObject.Properties['final_verified']) { [bool]$localState.final_verified } else { $null }
            final_verified_at = Format-CanonicalRfc3339Timestamp $localState.final_verified_at
            reconciled_from_remote = if ($null -ne $localState.PSObject.Properties['reconciled_from_remote']) { [bool]$localState.reconciled_from_remote } else { $null }
            failure_class = if ($null -ne $localState.PSObject.Properties['failure_class']) { [string]$localState.failure_class } else { $null }
            error_message = if ($null -ne $localState.PSObject.Properties['error_message']) { [string]$localState.error_message } else { $null }
            last_reconciled_at = Format-CanonicalRfc3339Timestamp $localState.last_reconciled_at
        }
    } else {
        [ordered]@{ state_file_found = $false }
    }
    findings = $findings
    assessment = $assessment
    recovery_guidance = $recoveryGuidance
}

if ($Format -eq "Json") {
    $report | ConvertTo-Json -Depth 10
} else {
    Write-Host "================================================================="
    Write-Host " Sky Auto Player V4 - Release Diagnostic Doctor (Read-Only)"
    Write-Host "================================================================="
    Write-Host "Persisted Phase:          $(if ($null -ne $persistedPhase) { $persistedPhase } else { 'NONE' })"
    Write-Host "External Phase:           $(if ($null -ne $externalPhase) { $externalPhase } else { 'NONE' })"
    Write-Host "Classification:           $(if ($null -ne $classification) { $classification } else { 'NONE' })"
    Write-Host "Workflow Outcome:         $workflowOutcome"
    Write-Host "Operator Review Required: $operatorReviewRequired"
    Write-Host ""
    Write-Host "External Truth:"
    Write-Host "  Release Status:    $($report.external_truth.status)"
    Write-Host "  Immutable:         $($report.external_truth.immutable)"
    Write-Host "  Published At:      $(if ([string]::IsNullOrWhiteSpace($publishedAt)) { 'NONE' } else { $publishedAt })"
    Write-Host "  Target Commit:     $(if ([string]::IsNullOrWhiteSpace($targetCommitish)) { 'NONE' } else { $targetCommitish })"
    Write-Host "  GitHub Latest:     $latestTag"
    Write-Host "  Public Assets:     $(if ($publicAssets.Count -gt 0) { $publicAssets -join ', ' } else { 'NONE' })"
    Write-Host "  Channel Metadata:  $(if ([string]::IsNullOrWhiteSpace($metadataVersion)) { 'NONE' } else { $metadataVersion })"
    Write-Host "  Raw Metadata:      $(if ([string]::IsNullOrWhiteSpace($publicMetadataVersion)) { 'NONE' } else { $publicMetadataVersion })"
    Write-Host "  Workflow Run:      $(if ($null -ne $workflowRunId) { "$workflowRunId ($workflowRunStatus, conclusion=$workflowRunConclusion)" } else { 'NONE' })"
    Write-Host ""
    Write-Host "Assessment:"
    Write-Host "  $assessment"
    Write-Host ""
    Write-Host "Recovery Guidance:"
    Write-Host $recoveryGuidance
    Write-Host "================================================================="
}
