[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$pipelinePath = Join-Path $PSScriptRoot "v4_release_pipeline.ps1"

function Fail([string]$Message) { throw "FAILED: $Message" }
function New-Sha([string]$Seed) { return (([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($Seed)) | ForEach-Object { $_.ToString("x2") }) -join "") }
function New-Bytes([string]$Text) { return [Text.Encoding]::UTF8.GetBytes($Text) }
function Get-BytesSha([byte[]]$Bytes) {
    $path = [IO.Path]::GetTempFileName()
    try { [IO.File]::WriteAllBytes($path, [byte[]]$Bytes); return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
    finally { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
}

function New-TransactionFixture([string]$Version, [string]$Channel, [string]$Fault) {
    $root = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-post-publication-" + [guid]::NewGuid().ToString("N"))
    $stateRoot = Join-Path $root "state"
    $inputRoot = Join-Path $root "input"
    New-Item -ItemType Directory -Path $stateRoot, $inputRoot -Force | Out-Null
    $installerSource = "Sky Auto Player_${Version}_x64-setup.exe"
    $signatureSource = "$installerSource.sig"
    $installerName = $installerSource.Replace(" ", ".")
    $signatureName = $signatureSource.Replace(" ", ".")
    $installerBytes = New-Bytes "installer-$Version-$Fault"
    $signatureBytes = New-Bytes "c2lnbmF0dXJlLWZpeHR1cmU="
    $installerPath = Join-Path $inputRoot $installerSource
    $signaturePath = Join-Path $inputRoot $signatureSource
    [IO.File]::WriteAllBytes($installerPath, $installerBytes)
    [IO.File]::WriteAllBytes($signaturePath, $signatureBytes)
    $records = @(
        [ordered]@{
            name = $installerName; release_name = $installerName; source_name = $installerSource
            role = "installer"; size = [int64]$installerBytes.Length; sha256 = (Get-BytesSha $installerBytes)
            source_path = $installerPath; state_path = "candidate-bundle/$installerSource"
        },
        [ordered]@{
            name = $signatureName; release_name = $signatureName; source_name = $signatureSource
            role = "updater-signature"; size = [int64]$signatureBytes.Length; sha256 = (Get-BytesSha $signatureBytes)
            source_path = $signaturePath; state_path = "candidate-bundle/$signatureSource"
        }
    )
    $bundle = Join-Path $stateRoot "candidate-bundle"
    New-Item -ItemType Directory -Path $bundle -Force | Out-Null
    Copy-Item $installerPath (Join-Path $bundle $installerSource)
    Copy-Item $signaturePath (Join-Path $bundle $signatureSource)
    $manifest = [ordered]@{
        schema_version = 1; source_sha = ("c" * 40); version = $Version; channel = $Channel; tag = "v$Version"
        qualification_assets = $records; public_assets = $records
    }
    $manifestPath = Join-Path $stateRoot "candidate-manifest.json"
    [IO.File]::WriteAllText($manifestPath, (($manifest | ConvertTo-Json -Depth 20) + "`n"), [Text.UTF8Encoding]::new($false))
    return [pscustomobject]@{
        Root = $root; StateRoot = $stateRoot; Version = $Version; Channel = $Channel; Tag = "v$Version"
        SourceSha = ("c" * 40); InstallerName = $installerName; SignatureName = $signatureName
        InstallerBytes = $installerBytes; SignatureBytes = $signatureBytes; ManifestPath = $manifestPath
    }
}

function New-InitialMetadata([string]$Version, [string]$Name) {
    $payload = [ordered]@{
        version = $Version; notes = "previous"; pub_date = "2026-09-01T00:00:00Z"
        platforms = [ordered]@{ "windows-x86_64" = [ordered]@{
            signature = [Convert]::ToBase64String((New-Bytes "old-signature"))
            url = "https://github.com/pumni/Sky-Auto-Player/releases/download/v$Version/$Name"
        } }
    }
    return [Text.Encoding]::UTF8.GetBytes(($payload | ConvertTo-Json -Depth 8))
}

function New-TransactionContext([object]$Fixture, [string]$Fault) {
    $oldSha = "d" * 40
    $oldTag = if ($Fixture.Channel -eq "stable") { "v4.1.1" } else { "v4.1.1" }
    $oldLatest = [pscustomobject]@{
        id = [int64]41; tag_name = $oldTag; target_commitish = $oldSha; draft = $false
        prerelease = $false; published_at = "2026-09-18T00:00:00Z"
        url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/41"
    }
    $currentBytes = if ($Fixture.Channel -eq "stable" -and $Fault -eq "NonMonotonic") {
        New-InitialMetadata "4.1.2" $Fixture.InstallerName
    } elseif ($Fixture.Channel -eq "stable") {
        New-InitialMetadata "4.0.1" "Sky.Auto.Player_4.0.1_x64-setup.exe"
    } else { $null }
    $ctx = [ordered]@{
        Fixture = $Fixture; Fault = $Fault; Release = $null; ReleaseCreated = $false
        Latest = $oldLatest; PreLatest = $oldLatest; CurrentBytes = $currentBytes; StoredBytes = $null
        PutCount = 0; RawCalls = 0; DownloadCalls = 0; ApiCalls = [Collections.Generic.List[string]]::new()
        OldSha = $oldSha; OldTag = $oldTag; NoRealTransport = $true
    }
    return $ctx
}

function New-TransactionApiHandler([System.Collections.IDictionary]$Ctx) {
    $handler = {
        param($Arguments, $AllowNotFound, $BinaryOutput, $Raw, $OutputPath)
        $command = ($Arguments -join " ")
        [void]$Ctx.ApiCalls.Add($command)
        if ($BinaryOutput) {
            $Ctx.DownloadCalls++
            $bytes = if ($command -match "/assets/101") { $Ctx.Fixture.InstallerBytes } else { $Ctx.Fixture.SignatureBytes }
            if (($Ctx.Fault -eq "InstallerBytesMismatch" -and $command -match "/assets/101") -or
                ($Ctx.Fault -eq "SignatureBytesMismatch" -and $command -match "/assets/102")) {
                $bytes = New-Bytes "tampered-download"
            }
            [IO.File]::WriteAllBytes($OutputPath, [byte[]]$bytes)
            return $null
        }
        if ($command -match "releases/latest") { return $Ctx.Latest }
        if ($command -match "releases/tags/") {
            if (-not $Ctx.ReleaseCreated -and $AllowNotFound) { return $null }
            if ($null -eq $Ctx.Release -and $AllowNotFound) { return $null }
            return $Ctx.Release
        }
        if ($command -match "POST repos/.+/releases") {
            $inputIndex = [Array]::IndexOf([string[]]$Arguments, "--input")
            $payload = Get-Content -LiteralPath $Arguments[$inputIndex + 1] -Raw | ConvertFrom-Json
            $Ctx.Release = [pscustomobject]@{
                id = [int64]42; upload_url = "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/42/assets"
                draft = $true; tag_name = [string]$payload.tag_name; target_commitish = [string]$payload.target_commitish
                body = [string]$payload.body; prerelease = [bool]$payload.prerelease; immutable = $false
                published_at = $null; assets = @(); url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/42"
            }
            $Ctx.ReleaseCreated = $true
            return $Ctx.Release
        }
        if ($command -match "PATCH repos/.+/releases/42") {
            $Ctx.Release.draft = $false
            $Ctx.Release.prerelease = ($Ctx.Fixture.Channel -eq "beta")
            $Ctx.Release.published_at = "2026-09-19T00:00:00Z"
            if ($Ctx.Fault -eq "ImmutableFalse") { $Ctx.Release.immutable = $false }
            elseif ($Ctx.Fault -eq "ImmutableMissing") { $Ctx.Release.PSObject.Properties.Remove("immutable") }
            else { $Ctx.Release.immutable = $true }
            if ($Ctx.Fault -eq "SourceMismatch") { $Ctx.Release.target_commitish = $Ctx.OldSha }
            $Ctx.Release.assets = @(
                [pscustomobject]@{
                    name = $Ctx.Fixture.InstallerName; size = [int64]$Ctx.Fixture.InstallerBytes.Length; state = "uploaded"
                    digest = "sha256:$(Get-BytesSha $Ctx.Fixture.InstallerBytes)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/101"
                    browser_download_url = if ($Ctx.Fault -eq "WrongAssetUrl") { "https://example.invalid/wrong.exe" } else { "https://github.com/pumni/Sky-Auto-Player/releases/download/$($Ctx.Fixture.Tag)/$($Ctx.Fixture.InstallerName)" }
                },
                [pscustomobject]@{
                    name = $Ctx.Fixture.SignatureName; size = [int64]$Ctx.Fixture.SignatureBytes.Length; state = "uploaded"
                    digest = "sha256:$(Get-BytesSha $Ctx.Fixture.SignatureBytes)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/102"
                    browser_download_url = "https://github.com/pumni/Sky-Auto-Player/releases/download/$($Ctx.Fixture.Tag)/$($Ctx.Fixture.SignatureName)"
                }
            )
            if ($Ctx.Fault -eq "WrongLatest") { $Ctx.Latest = $Ctx.PreLatest }
            elseif ($Ctx.Fault -eq "BetaDisplacesLatest") { $Ctx.Latest = $Ctx.Release }
            else {
                $Ctx.Latest = [pscustomobject]@{
                    id = [int64]42; tag_name = $Ctx.Fixture.Tag; target_commitish = $Ctx.Release.target_commitish
                    draft = $false; prerelease = ($Ctx.Fixture.Channel -eq "beta"); published_at = $Ctx.Release.published_at
                    url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/42"
                }
            }
            return $Ctx.Release
        }
        if ($command -match "api repos/.+/releases/42") {
            if (@($Ctx.Release.assets).Count -eq 0) {
                $Ctx.Release.assets = @(
                    [pscustomobject]@{
                        name = $Ctx.Fixture.InstallerName; size = [int64]$Ctx.Fixture.InstallerBytes.Length; state = "uploaded"
                        digest = "sha256:$(Get-BytesSha $Ctx.Fixture.InstallerBytes)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/101"
                        browser_download_url = if ($Ctx.Fault -eq "WrongAssetUrl") { "https://example.invalid/wrong.exe" } else { "https://github.com/pumni/Sky-Auto-Player/releases/download/$($Ctx.Fixture.Tag)/$($Ctx.Fixture.InstallerName)" }
                    },
                    [pscustomobject]@{
                        name = $Ctx.Fixture.SignatureName; size = [int64]$Ctx.Fixture.SignatureBytes.Length; state = "uploaded"
                        digest = "sha256:$(Get-BytesSha $Ctx.Fixture.SignatureBytes)"; url = "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/102"
                        browser_download_url = "https://github.com/pumni/Sky-Auto-Player/releases/download/$($Ctx.Fixture.Tag)/$($Ctx.Fixture.SignatureName)"
                    }
                )
            }
            return $Ctx.Release
        }
        if ($command -notmatch "--method PUT" -and $command -match "contents/channels/(stable|beta)/latest\.json") {
            if ($Matches[1] -ne $Ctx.Fixture.Channel) { return $null }
            $bytes = if ($Ctx.PutCount -gt 0) { $Ctx.StoredBytes } else { $Ctx.CurrentBytes }
            if ($null -eq $bytes) { if ($AllowNotFound) { return $null }; throw "metadata channel missing" }
            return [pscustomobject]@{ content = [Convert]::ToBase64String([byte[]]$bytes); sha = "metadata-sha" }
        }
        if ($command -match "--method PUT repos/.+/contents/channels/(stable|beta)/latest\.json") {
            $inputIndex = [Array]::IndexOf([string[]]$Arguments, "--input")
            $payload = Get-Content -LiteralPath $Arguments[$inputIndex + 1] -Raw | ConvertFrom-Json
            $bytes = [Convert]::FromBase64String([string]$payload.content)
            if ($Ctx.Fault -eq "StoredWrongBytes") { $bytes = New-Bytes "wrong-stored-metadata" }
            elseif ($Ctx.Fault -eq "TamperedMetadataSignature") {
                $doc = [Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json
                $doc.platforms.'windows-x86_64'.signature = [Convert]::ToBase64String((New-Bytes "tampered-signature"))
                $bytes = [Text.Encoding]::UTF8.GetBytes(($doc | ConvertTo-Json -Depth 8))
            }
            $Ctx.StoredBytes = [byte[]]$bytes
            $Ctx.PutCount++
            return [pscustomobject]@{ content = [pscustomobject]@{ sha = "new-sha" }; commit = [pscustomobject]@{ sha = "commit-sha" } }
        }
        if ($command -match "git/ref/heads/release-metadata") {
            return [pscustomobject]@{ ref = "refs/heads/release-metadata"; object = [pscustomobject]@{ sha = "branch-sha" } }
        }
        if ($command -match "contents/\.release-metadata/README\.md") {
            return [pscustomobject]@{ content = [Convert]::ToBase64String((New-Bytes "bootstrap")) }
        }
        if ($command -match "git/ref/tags/") { if ($AllowNotFound) { return $null }; return $null }
        if ($AllowNotFound) { return $null }
        throw "unexpected mocked GitHub API request: $command"
    }.GetNewClosure()
    return $handler
}

function New-TransactionUploadHandler([System.Collections.IDictionary]$Ctx) {
    return { param($UploadUrl, $AssetName, $FilePath) }.GetNewClosure()
}

function New-TransactionRawHandler([System.Collections.IDictionary]$Ctx) {
    return {
        param($Endpoint)
        $Ctx.RawCalls++
        $bytes = if ($Ctx.Fault -eq "RawNeverConverges" -or $Ctx.RawCalls -eq 1) {
            New-Bytes "stale-raw-metadata"
        } else { [byte[]]$Ctx.StoredBytes }
        return [pscustomobject]@{ status = 200; bytes = $bytes; sha256 = (Get-BytesSha $bytes) }
    }.GetNewClosure()
}

function Invoke-TransactionCase([string]$Name, [string]$Version, [string]$Channel, [string]$Fault, [bool]$ExpectedPass) {
    $fixture = New-TransactionFixture $Version $Channel $Fault
    $ctx = New-TransactionContext $fixture $Fault
    $failed = $false
    try {
        & {
            $script:GitHubApiHandler = New-TransactionApiHandler $ctx
            $script:AssetUploadHandler = New-TransactionUploadHandler $ctx
            $script:RawMetadataHandler = New-TransactionRawHandler $ctx
            $script:RawMetadataSleepHandler = { param($Seconds) }
            . $pipelinePath `
                -State PublishRelease -Version $fixture.Version -Channel $fixture.Channel -Tag $fixture.Tag `
                -SourceSha $fixture.SourceSha -WorkflowSha $fixture.SourceSha -StateRoot $fixture.StateRoot `
                -ReleaseNotesPath (Join-Path $repoRoot "docs/releases/v$($fixture.Version).md") -RunId "mock-run" `
                -RawMetadataRetryBudgetSeconds 1 -RawMetadataRetryIntervalSeconds 0 -NoDispatch
            Invoke-PublishRelease
            if (Test-Path -LiteralPath (Join-Path $fixture.StateRoot "release-state.json")) { Fail "release-state.json was created after PublishRelease" }
            Invoke-PromoteMetadata
            if (Test-Path -LiteralPath (Join-Path $fixture.StateRoot "release-state.json")) { Fail "release-state.json was created after PromoteMetadata" }
            Invoke-FinalVerify
            if (Test-Path -LiteralPath (Join-Path $fixture.StateRoot "release-state.json")) { Fail "release-state.json was created after FinalVerify" }
        }
    } catch {
        $failed = $true
        if ($ExpectedPass) { throw }
    } finally {
        $script:GitHubApiHandler = $null
        $script:AssetUploadHandler = $null
        $script:RawMetadataHandler = $null
        $script:RawMetadataSleepHandler = $null
        Remove-Item -LiteralPath $fixture.Root -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($ExpectedPass -and $failed) { Fail "$Name did not pass" }
    if (-not $ExpectedPass -and -not $failed) { Fail "$Name unexpectedly passed" }
    Write-Host "V4 post-publication transaction case: $Name PASS"
}

Invoke-TransactionCase "stable happy path with stale-then-converged raw endpoint" "4.1.2" "stable" "Happy" $true
foreach ($case in @(
    @("stable wrong Latest", "4.1.2", "stable", "WrongLatest"),
    @("beta publication displaces Latest", "4.0.0-rc.1", "beta", "BetaDisplacesLatest"),
    @("published source mismatch", "4.1.2", "stable", "SourceMismatch"),
    @("immutable false", "4.1.2", "stable", "ImmutableFalse"),
    @("immutable missing", "4.1.2", "stable", "ImmutableMissing"),
    @("non-monotonic metadata", "4.1.2", "stable", "NonMonotonic"),
    @("wrong stored metadata bytes", "4.1.2", "stable", "StoredWrongBytes"),
    @("tampered updater signature in metadata", "4.1.2", "stable", "TamperedMetadataSignature"),
    @("wrong asset URL", "4.1.2", "stable", "WrongAssetUrl"),
    @("raw endpoint does not converge", "4.1.2", "stable", "RawNeverConverges"),
    @("installer downloaded bytes mismatch", "4.1.2", "stable", "InstallerBytesMismatch"),
    @("signature downloaded bytes mismatch", "4.1.2", "stable", "SignatureBytesMismatch")
)) {
    Invoke-TransactionCase $case[0] $case[1] $case[2] $case[3] $false
}
Write-Host "V4 post-publication transaction behavioral test: PASS (mocked transports; zero network; zero production mutation)"
