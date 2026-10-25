# Windows Authenticode fact adapter. Rust owns mode policy and canonical evidence.
# This script supplies only platform status and the independent CMS/SPC proof.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("test", "production", "unsigned-zero-budget")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$ArtifactManifest,

    [Parameter(Mandatory = $false)]
    [string]$ExpectedThumbprint,

    [Parameter(Mandatory = $true)]
    [string]$Evidence
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "v4_authenticode_crypto.ps1")

$maximumArtifactCount = 256
$maximumManifestBytes = 1MB
$manifestFile = Get-Item -LiteralPath $ArtifactManifest -ErrorAction Stop
if ($manifestFile.PSIsContainer -or
    (($manifestFile.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) -or
    $manifestFile.Length -le 0 -or
    $manifestFile.Length -gt $maximumManifestBytes) {
    throw "Authenticode artifact manifest must be a bounded regular file"
}
$manifestBytes = [IO.File]::ReadAllBytes($manifestFile.FullName)
if ($manifestBytes.Length -gt $maximumManifestBytes) {
    throw "Authenticode artifact manifest exceeds the $maximumManifestBytes-byte limit"
}
$strictUtf8 = [Text.UTF8Encoding]::new($false, $true)
$manifestJson = $strictUtf8.GetString($manifestBytes)
$manifest = $manifestJson |
    ConvertFrom-Json -ErrorAction Stop
$manifestProperties = @($manifest.PSObject.Properties.Name)
if ($manifestProperties.Count -ne 2 -or
    -not ($manifestProperties -ccontains "schema_version") -or
    -not ($manifestProperties -ccontains "files") -or
    $manifest.schema_version -ne 1 -or
    $manifest.files -isnot [Array] -or
    $manifest.files.Count -lt 1 -or
    $manifest.files.Count -gt $maximumArtifactCount) {
    throw "Authenticode artifact manifest has an unsupported schema or file count"
}

$seen = @{}
$records = foreach ($entry in $manifest.files) {
    $entryProperties = @($entry.PSObject.Properties.Name)
    if ($entryProperties.Count -ne 3 -or
        -not ($entryProperties -ccontains "path") -or
        -not ($entryProperties -ccontains "name") -or
        -not ($entryProperties -ccontains "sha256") -or
        $entry.path -isnot [string] -or
        [string]::IsNullOrWhiteSpace($entry.path) -or
        $entry.name -isnot [string] -or
        [string]::IsNullOrWhiteSpace($entry.name) -or
        $entry.sha256 -isnot [string] -or
        $entry.sha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw "Authenticode artifact manifest contains a malformed entry"
    }

    $path = $entry.path
    $resolved = (Resolve-Path -LiteralPath $path -ErrorAction Stop).Path
    $item = Get-Item -LiteralPath $resolved -ErrorAction Stop
    if ($item.PSIsContainer -or
        (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) -or
        $item.Extension.ToLowerInvariant() -notin @(".exe", ".dll")) {
        throw "Authenticode inspection target must be a regular PE file: $path"
    }
    if ($item.Name -cne $entry.name) {
        throw "Authenticode inspection filename does not match its manifest entry"
    }
    if ($seen.ContainsKey($item.Name.ToLowerInvariant())) {
        throw "Duplicate Authenticode inspection filename: $($item.Name)"
    }
    $seen[$item.Name.ToLowerInvariant()] = $true

    $expectedHash = [string]$entry.sha256
    $initialHash = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not [string]::Equals($initialHash, $expectedHash, [StringComparison]::Ordinal)) {
        throw "Authenticode inspection file does not match its manifest SHA-256: $($item.Name)"
    }

    $signature = Get-AuthenticodeSignature -LiteralPath $item.FullName
    $signerThumbprint = $null
    $signerSubject = $null
    $integrityVerifier = $null
    $integrityStatus = $null
    $digestAlgorithm = $null
    $signedDigest = $null
    $computedDigest = $null
    if ($null -ne $signature.SignerCertificate) {
        $signerThumbprint = ([string]$signature.SignerCertificate.Thumbprint).Trim().ToUpperInvariant()
        $signerSubject = [string]$signature.SignerCertificate.Subject
        if (-not [string]::IsNullOrWhiteSpace($ExpectedThumbprint)) {
            $proof = Get-AuthenticodeIntegrityProof -Path $item.FullName -ExpectedThumbprint $ExpectedThumbprint
            $integrityVerifier = [string]$proof.IntegrityVerifier
            $integrityStatus = [string]$proof.IntegrityStatus
            $digestAlgorithm = [string]$proof.DigestAlgorithm
            $signedDigest = [string]$proof.SignedDigest
            $computedDigest = [string]$proof.ComputedDigest
        }
    }

    $finalHash = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not [string]::Equals($finalHash, $expectedHash, [StringComparison]::Ordinal)) {
        throw "Authenticode inspection file changed while it was being verified: $($item.Name)"
    }

    [ordered]@{
        name = $item.Name
        platform_status = [string]$signature.Status
        sha256 = $finalHash
        signer_thumbprint = $signerThumbprint
        signer_subject = $signerSubject
        signed_digest_algorithm = $digestAlgorithm
        signed_digest = $signedDigest
        computed_digest = $computedDigest
        integrity_verifier = $integrityVerifier
        integrity_status = $integrityStatus
    }
}

$parent = Split-Path -Parent $Evidence
if (-not [string]::IsNullOrWhiteSpace($parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
}
$payload = [ordered]@{
    schema_version = 1
    evidence_type = "authenticode-windows-inspection"
    mode = $Mode.ToLowerInvariant()
    files = @($records)
}
$payload | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Evidence -Encoding UTF8
