param(
    [Parameter(Mandatory = $true, ParameterSetName = "Promotion")]
    [ValidateSet("stable", "beta")]
    [string]$Channel,

    [Parameter(Mandatory = $true, ParameterSetName = "Promotion")]
    [string]$Metadata,

    [Parameter(Mandatory = $true, ParameterSetName = "Promotion")]
    [string]$QualificationEvidence,

    [Parameter(Mandatory = $true, ParameterSetName = "Promotion")]
    [string]$AuthorityCheckout,

    [Parameter(ParameterSetName = "Promotion")]
    [string]$SourceCheckout = (Split-Path -Parent $PSScriptRoot),

    [Parameter(Mandatory = $true, ParameterSetName = "SelfTest")]
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
$authorityRepository = "pumni/Sky-Auto-Player-Releases"
$platform = "windows-x86_64"
$productName = "Sky Auto Player"
$productIdentifier = "io.github.pumni.skyautoplayer"
$installerNameSuffix = "_x64-setup.exe"
$evidenceType = "tauri-nsis-qualified-release"
$evidenceSchemaVersion = 1

function Get-GitHubJson([string]$Path) {
    $payload = & gh api $Path --header "Accept: application/vnd.github+json"
    if ($LASTEXITCODE -ne 0) { throw "GitHub read failed for $Path" }
    return ($payload -join "`n") | ConvertFrom-Json
}

function Get-RequiredEvidenceString([object]$Evidence, [string]$Name, [int]$MaxLength = 512) {
    $property = $Evidence.PSObject.Properties[$Name]
    if ($null -eq $property -or $property.Value -isnot [string]) {
        throw "Qualification evidence field $Name must be a string"
    }
    $value = [string]$property.Value
    if ([string]::IsNullOrWhiteSpace($value) -or $value.Length -gt $MaxLength) {
        throw "Qualification evidence field $Name is empty or unbounded"
    }
    return $value
}

function Get-RequiredEvidenceSize([object]$Evidence, [string]$Name) {
    $property = $Evidence.PSObject.Properties[$Name]
    if ($null -eq $property -or $property.Value -is [bool] -or
        ($property.Value -isnot [int] -and $property.Value -isnot [long] -and
         $property.Value -isnot [uint32] -and $property.Value -isnot [uint64])) {
        throw "Qualification evidence field $Name must be an integer size"
    }
    try { $value = [uint64]$property.Value } catch { throw "Qualification evidence field $Name is invalid" }
    if ($value -gt 2GB) { throw "Qualification evidence field $Name exceeds the bounded artifact size" }
    return $value
}

function Get-RequiredEvidenceSha256([object]$Evidence, [string]$Name) {
    $value = Get-RequiredEvidenceString $Evidence $Name 128
    if ($value -notmatch '^[0-9a-fA-F]{64}$') {
        throw "Qualification evidence field $Name must be a 64-character SHA-256 digest"
    }
    return $value.ToLowerInvariant()
}

function Assert-QualificationEvidence(
    [object]$Evidence,
    [string]$Version,
    [string]$ExpectedInstaller,
    [string]$ExpectedSignature
) {
    if ($null -eq $Evidence -or $Evidence -is [array]) {
        throw "Qualification evidence must be one JSON object"
    }
    $required = @(
        "schema_version", "evidence_type", "qualified", "qualification",
        "product_name", "identifier", "version", "target", "install_mode",
        "installer", "updater_signature", "installer_size", "signature_size",
        "installer_sha256", "updater_signature_sha256", "authenticode_mode",
        "authenticode_evidence", "authenticode_evidence_sha256", "sbom", "sbom_sha256"
    )
    $actual = @($Evidence.PSObject.Properties.Name)
    $unknown = @($actual | Where-Object { $_ -notin $required })
    $missing = @($required | Where-Object { $_ -notin $actual })
    if ($unknown.Count -gt 0 -or $missing.Count -gt 0 -or $actual.Count -ne $required.Count) {
        throw "Qualification evidence schema is not the bounded Tauri qualification schema"
    }
    $schemaProperty = $Evidence.PSObject.Properties["schema_version"]
    if ($schemaProperty.Value -is [bool] -or
        ($schemaProperty.Value -isnot [int] -and $schemaProperty.Value -isnot [long])) {
        throw "Qualification evidence schema_version must be an integer"
    }
    if ([int]$schemaProperty.Value -ne $evidenceSchemaVersion) {
        throw "Unsupported qualification evidence schema version"
    }
    if ((Get-RequiredEvidenceString $Evidence "evidence_type") -ne $evidenceType -or
        (Get-RequiredEvidenceString $Evidence "qualification") -ne "install-launch-uninstall") {
        throw "Qualification evidence type is not the canonical Tauri qualification path"
    }
    if ((Get-RequiredEvidenceString $Evidence "authenticode_mode") -ne "production") {
        throw "Qualification evidence is not production Authenticode evidence"
    }
    $qualified = $Evidence.PSObject.Properties["qualified"]
    if ($qualified.Value -isnot [bool] -or -not $qualified.Value) {
        throw "Qualification evidence is not an explicit successful result"
    }
    foreach ($pair in @(
        @{ Name = "product_name"; Expected = $productName },
        @{ Name = "identifier"; Expected = $productIdentifier },
        @{ Name = "version"; Expected = $Version },
        @{ Name = "target"; Expected = "nsis" },
        @{ Name = "install_mode"; Expected = "currentUser" },
        @{ Name = "installer"; Expected = $ExpectedInstaller },
        @{ Name = "updater_signature"; Expected = $ExpectedSignature }
    )) {
        if ((Get-RequiredEvidenceString $Evidence $pair.Name) -ne $pair.Expected) {
            throw "Qualification evidence field $($pair.Name) does not match the canonical artifact"
        }
    }
    return [ordered]@{
        InstallerSize = Get-RequiredEvidenceSize $Evidence "installer_size"
        SignatureSize = Get-RequiredEvidenceSize $Evidence "signature_size"
        InstallerSha256 = Get-RequiredEvidenceSha256 $Evidence "installer_sha256"
        SignatureSha256 = Get-RequiredEvidenceSha256 $Evidence "updater_signature_sha256"
        AuthenticodeEvidence = Get-RequiredEvidenceString $Evidence "authenticode_evidence"
        AuthenticodeEvidenceSha256 = Get-RequiredEvidenceSha256 $Evidence "authenticode_evidence_sha256"
        Sbom = Get-RequiredEvidenceString $Evidence "sbom"
        SbomSha256 = Get-RequiredEvidenceSha256 $Evidence "sbom_sha256"
    }
}

function Get-CanonicalReleaseAssetUrl([string]$Version, [string]$Name) {
    return "https://github.com/$authorityRepository/releases/download/v$Version/$([Uri]::EscapeDataString($Name))"
}

function Get-PublishedAssetSha256([object]$Asset, [string]$ExpectedUrl, [string]$Kind) {
    $digest = [string]$Asset.digest
    if (-not [string]::IsNullOrWhiteSpace($digest)) {
        if ($digest -notmatch '^sha256:[0-9a-fA-F]{64}$') {
            throw "Published $Kind asset has a malformed SHA-256 digest"
        }
        return $digest.Substring(7).ToLowerInvariant()
    }

    $downloadUrl = [string]$Asset.browser_download_url
    if ($downloadUrl -ne $ExpectedUrl) {
        throw "Published $Kind asset has no digest and its download URL is not canonical"
    }
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-published-" + [guid]::NewGuid().ToString("N") + ".bin")
    try {
        $null = Invoke-WebRequest -UseBasicParsing -Uri $downloadUrl -OutFile $temporary -MaximumRedirection 5
        return (Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash.ToLowerInvariant()
    } catch {
        throw "Could not download/hash published $Kind asset: $($_.Exception.Message)"
    } finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

function Assert-PublishedAsset(
    [object]$Asset,
    [string]$ExpectedName,
    [string]$ExpectedUrl,
    [uint64]$ExpectedSize,
    [string]$ExpectedSha256,
    [string]$Kind
) {
    if ([string]$Asset.name -ne $ExpectedName) {
        throw "Published $Kind asset name is not canonical"
    }
    if ([string]$Asset.browser_download_url -ne $ExpectedUrl) {
        throw "Published $Kind asset URL is not canonical"
    }
    if ($Asset.size -is [bool] -or $null -eq $Asset.size) {
        throw "Published $Kind asset size is missing or malformed"
    }
    try { $actualSize = [uint64]$Asset.size } catch { throw "Published $Kind asset size is malformed" }
    if ($actualSize -ne $ExpectedSize) {
        throw "Published $Kind asset size does not match qualification evidence"
    }
    $actualSha256 = Get-PublishedAssetSha256 $Asset $ExpectedUrl $Kind
    if ($actualSha256 -ne $ExpectedSha256) {
        throw "Published $Kind asset SHA-256 mismatch: expected $ExpectedSha256, got $actualSha256"
    }
}

function Invoke-PromotionSelfTest {
    $version = "4.0.0"
    $installer = "$productName`_$version$installerNameSuffix"
    $signature = "$installer.sig"
    $url = Get-CanonicalReleaseAssetUrl $version $installer
    $evidence = [pscustomobject]@{
        schema_version = 1
        evidence_type = $evidenceType
        qualified = $true
        qualification = "install-launch-uninstall"
        product_name = $productName
        identifier = $productIdentifier
        version = $version
        target = "nsis"
        install_mode = "currentUser"
        installer = $installer
        updater_signature = $signature
        installer_size = [int64]10
        signature_size = [int64]10
        installer_sha256 = "a" * 64
        updater_signature_sha256 = "a" * 64
        authenticode_mode = "production"
        authenticode_evidence = "TAURI_AUTHENTICODE_EVIDENCE.json"
        authenticode_evidence_sha256 = "a" * 64
        sbom = "SBOM.spdx.json"
        sbom_sha256 = "a" * 64
    }
    $digests = Assert-QualificationEvidence $evidence $version $installer $signature
    $differentBytes = [pscustomobject]@{
        name = $installer
        browser_download_url = $url
        size = [int64]10
        digest = "sha256:" + ("b" * 64)
    }
    $rejected = $false
    try {
        Assert-PublishedAsset $differentBytes $installer $url $digests.InstallerSize $digests.InstallerSha256 "installer"
    } catch {
        if ($_.Exception.Message -notmatch "SHA-256 mismatch") { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw "Self-test failed: same-name/different-bytes asset was accepted" }

    $booleanOnly = [pscustomobject]@{ qualified = $true; version = $version }
    try {
        Assert-QualificationEvidence $booleanOnly $version $installer $signature
        throw "Self-test failed: arbitrary qualified=true evidence was accepted"
    } catch {
        if ($_.Exception.Message -like "Self-test failed:*") { throw }
    }
    Write-Host "V4 promotion evidence self-test: PASS (bounded schema; same-name/different-bytes rejected)"
}

if ($SelfTest) {
    Invoke-PromotionSelfTest
    exit 0
}

if (-not (Test-Path -LiteralPath $Metadata -PathType Leaf)) {
    throw "Metadata file does not exist: $Metadata"
}
if (-not (Test-Path -LiteralPath $QualificationEvidence -PathType Leaf)) {
    throw "Qualification evidence does not exist: $QualificationEvidence"
}
if (-not (Test-Path -LiteralPath $AuthorityCheckout -PathType Container)) {
    throw "Authority checkout does not exist: $AuthorityCheckout"
}

$metadataJson = Get-Content -LiteralPath $Metadata -Raw | ConvertFrom-Json
$platformJson = $metadataJson.platforms.$platform
if ($null -eq $platformJson) { throw "Metadata has no $platform entry" }
$version = [string]$metadataJson.version
$expectedInstaller = "$productName`_$version$installerNameSuffix"
$expectedSignature = "$expectedInstaller.sig"
$evidence = Get-Content -LiteralPath $QualificationEvidence -Raw | ConvertFrom-Json
$evidenceDigests = $null

$sourceRoot = (Resolve-Path -LiteralPath $SourceCheckout).Path
$metadataPath = (Resolve-Path -LiteralPath $Metadata).Path
$validation = & cargo run --manifest-path (Join-Path $sourceRoot "rust/Cargo.toml") --locked -p sky_xtask -- `
    release-authority validate --channel $Channel --metadata $metadataPath
if ($LASTEXITCODE -ne 0) { throw "v4 metadata structural validation failed" }

$evidenceDigests = Assert-QualificationEvidence $evidence $version $expectedInstaller $expectedSignature

$release = Get-GitHubJson "repos/$authorityRepository/releases/tags/v$version"
if ($release.draft -or [string]::IsNullOrWhiteSpace([string]$release.published_at)) {
    throw "Cannot promote metadata before the v4 release is published: v$version"
}
if ($Channel -eq "stable" -and $release.prerelease) {
    throw "Stable metadata cannot point at a prerelease: v$version"
}

$asset = @($release.assets | Where-Object { $_.name -eq $expectedInstaller })
$signatureAsset = @($release.assets | Where-Object { $_.name -eq $expectedSignature })
if ($asset.Count -ne 1 -or $signatureAsset.Count -ne 1) {
    throw "Published release is missing the exact Tauri installer/signature pair"
}
$expectedInstallerUrl = Get-CanonicalReleaseAssetUrl $version $expectedInstaller
$expectedSignatureUrl = Get-CanonicalReleaseAssetUrl $version $expectedSignature
if ([string]$platformJson.url -ne $expectedInstallerUrl) {
    throw "Metadata URL is not the exact published Tauri asset URL"
}
Assert-PublishedAsset $asset[0] $expectedInstaller $expectedInstallerUrl `
    $evidenceDigests.InstallerSize $evidenceDigests.InstallerSha256 "installer"
Assert-PublishedAsset $signatureAsset[0] $expectedSignature $expectedSignatureUrl `
    $evidenceDigests.SignatureSize $evidenceDigests.SignatureSha256 "signature"

$publishedSignature = & gh api $signatureAsset[0].url --header "Accept: application/octet-stream"
if ($LASTEXITCODE -ne 0) { throw "Could not read the published Tauri signature asset" }
if (($publishedSignature -join "`n").Trim() -ne ([string]$platformJson.signature).Trim()) {
    throw "Metadata signature does not match the exact published .sig contents"
}

$checkoutRoot = (Resolve-Path -LiteralPath $AuthorityCheckout).Path
$destination = Join-Path $checkoutRoot "channels\$Channel\latest.json"
$destinationParent = Split-Path -Parent $destination
New-Item -ItemType Directory -Path $destinationParent -Force | Out-Null
$temporary = "$destination.$PID.tmp"
try {
    Copy-Item -LiteralPath $Metadata -Destination $temporary -Force
    Move-Item -LiteralPath $temporary -Destination $destination -Force
} finally {
    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
}

Write-Host "Promoted validated v4 $Channel metadata for v$version to $destination"
Write-Host "The authority checkout must be reviewed and committed separately; this action never publishes or mutates a GitHub release."
