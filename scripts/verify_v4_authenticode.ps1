# Windows Authenticode fact adapter. Rust owns mode policy and canonical evidence.
# This script supplies only platform status and the independent CMS/SPC proof.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("test", "production", "unsigned-zero-budget")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string[]]$Artifact,

    [Parameter(Mandatory = $false)]
    [string]$ExpectedThumbprint,

    [Parameter(Mandatory = $true)]
    [string]$Evidence
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "v4_authenticode_crypto.ps1")
if ($Artifact.Count -eq 0) { throw "At least one Authenticode artifact is required" }

$seen = @{}
$records = foreach ($path in $Artifact) {
    $resolved = (Resolve-Path -LiteralPath $path -ErrorAction Stop).Path
    $item = Get-Item -LiteralPath $resolved -ErrorAction Stop
    if ($item.PSIsContainer -or $item.Extension.ToLowerInvariant() -notin @(".exe", ".dll")) {
        throw "Authenticode inspection target must be a regular PE file: $path"
    }
    if ($seen.ContainsKey($item.Name)) {
        throw "Duplicate Authenticode inspection filename: $($item.Name)"
    }
    $seen[$item.Name] = $true

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

    [ordered]@{
        name = $item.Name
        platform_status = [string]$signature.Status
        sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
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
