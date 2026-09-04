param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("stable", "beta")]
    [string]$Channel,

    [Parameter(Mandatory = $true)]
    [string]$Metadata,

    [Parameter(Mandatory = $true)]
    [string]$QualificationEvidence,

    [Parameter(Mandatory = $true)]
    [string]$AuthorityCheckout,

    [string]$SourceCheckout = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = "Stop"
$authorityRepository = "pumni/Sky-Auto-Player-Releases"
$platform = "windows-x86_64"
$productName = "Sky Auto Player"
$installerNameSuffix = "_x64-setup.exe"

function Get-GitHubJson([string]$Path) {
    $payload = & gh api $Path --header "Accept: application/vnd.github+json"
    if ($LASTEXITCODE -ne 0) { throw "GitHub read failed for $Path" }
    return ($payload -join "`n") | ConvertFrom-Json
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
if (-not [bool]$evidence.qualified -or [string]$evidence.version -ne $version) {
    throw "Qualification evidence is not an explicit qualified result for v$version"
}
if ([string]$evidence.installer -ne $expectedInstaller -or
    [string]$evidence.updater_signature -ne $expectedSignature) {
    throw "Qualification evidence does not identify the canonical Tauri artifact pair"
}

$sourceRoot = (Resolve-Path -LiteralPath $SourceCheckout).Path
$metadataPath = (Resolve-Path -LiteralPath $Metadata).Path
$validation = & cargo run --manifest-path (Join-Path $sourceRoot "rust/Cargo.toml") --locked -p sky_xtask -- `
    release-authority validate --channel $Channel --metadata $metadataPath
if ($LASTEXITCODE -ne 0) { throw "v4 metadata structural validation failed" }

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
if ([string]$asset[0].browser_download_url -ne [string]$platformJson.url) {
    throw "Metadata URL is not the exact published Tauri asset URL"
}

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
