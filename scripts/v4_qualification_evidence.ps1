# scripts/v4_qualification_evidence.ps1
# Pure canonical V4 qualification evidence builder contract.
# Used by production release orchestration and verified by promotion contract tests.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function New-V4CanonicalQualificationEvidence {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)] [string]$Version,
        [Parameter(Mandatory = $true)] [string]$InstallerName,
        [Parameter(Mandatory = $true)] [string]$SignatureName,
        [Parameter(Mandatory = $true)] [int64]$InstallerSize,
        [Parameter(Mandatory = $true)] [int64]$SignatureSize,
        [Parameter(Mandatory = $true)] [string]$InstallerSha256,
        [Parameter(Mandatory = $true)] [string]$SignatureSha256,
        [Parameter(Mandatory = $true)] [string]$AuthenticodeEvidenceSha256,
        [Parameter(Mandatory = $true)] [string]$SbomSha256
    )

    if ($InstallerSize -le 0) { throw "InstallerSize must be positive" }
    if ($SignatureSize -le 0) { throw "SignatureSize must be positive" }
    if ($InstallerSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "InstallerSha256 must be a 64-character SHA-256 hex string" }
    if ($SignatureSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "SignatureSha256 must be a 64-character SHA-256 hex string" }
    if ($AuthenticodeEvidenceSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "AuthenticodeEvidenceSha256 must be a 64-character SHA-256 hex string" }
    if ($SbomSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "SbomSha256 must be a 64-character SHA-256 hex string" }

    return [ordered]@{
        schema_version = 1
        evidence_type = "tauri-nsis-qualified-release"
        qualified = $true
        qualification = "install-launch-uninstall"
        product_name = "Sky Auto Player"
        identifier = "io.github.pumni.skyautoplayer"
        version = $Version
        target = "nsis"
        install_mode = "currentUser"
        installer = $InstallerName
        updater_signature = $SignatureName
        installer_size = $InstallerSize
        signature_size = $SignatureSize
        installer_sha256 = $InstallerSha256.ToLowerInvariant()
        updater_signature_sha256 = $SignatureSha256.ToLowerInvariant()
        # The project production policy is intentionally Authenticode-unsigned.
        # The linked Authenticode evidence proves this exact state.
        authenticode_mode = "unsigned-zero-budget"
        authenticode_evidence = "TAURI_AUTHENTICODE_EVIDENCE.json"
        authenticode_evidence_sha256 = $AuthenticodeEvidenceSha256.ToLowerInvariant()
        sbom = "SBOM.spdx.json"
        sbom_sha256 = $SbomSha256.ToLowerInvariant()
    }
}

function New-V4CanonicalProductionEvidence {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)] [string]$SourceSha,
        [Parameter(Mandatory = $true)] [string]$Version,
        [Parameter(Mandatory = $true)] [string]$Channel,
        [Parameter(Mandatory = $true)] [string]$InstallerName,
        [Parameter(Mandatory = $true)] [string]$SignatureName,
        [Parameter(Mandatory = $true)] [int64]$InstallerSize,
        [Parameter(Mandatory = $true)] [int64]$SignatureSize,
        [Parameter(Mandatory = $true)] [string]$InstallerSha256,
        [Parameter(Mandatory = $true)] [string]$SignatureSha256,
        [Parameter(Mandatory = $true)] [string]$AuthenticodeEvidenceSha256,
        [Parameter(Mandatory = $true)] [string]$SbomSha256,
        [Parameter(Mandatory = $true)] [string]$UpdaterKeyId,
        $ObservedSignerThumbprint = $null
    )

    if ($SourceSha -notmatch '^[0-9a-fA-F]{40}$') { throw "SourceSha must be a 40-character commit SHA" }
    if ($Channel -notin @("stable", "beta")) { throw "Channel must be 'stable' or 'beta'" }
    if ($InstallerSize -le 0) { throw "InstallerSize must be positive" }
    if ($SignatureSize -le 0) { throw "SignatureSize must be positive" }
    if ($InstallerSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "InstallerSha256 must be a 64-character SHA-256 hex string" }
    if ($SignatureSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "SignatureSha256 must be a 64-character SHA-256 hex string" }
    if ($AuthenticodeEvidenceSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "AuthenticodeEvidenceSha256 must be a 64-character SHA-256 hex string" }
    if ($SbomSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "SbomSha256 must be a 64-character SHA-256 hex string" }
    if ($UpdaterKeyId -notmatch '^[0-9a-fA-F]{16}$') { throw "UpdaterKeyId must be a 16-character hex string" }

    return [ordered]@{
        schema_version = 1
        evidence_type = "v4-production-release-qualification"
        source_sha = $SourceSha.ToLowerInvariant()
        version = $Version
        channel = $Channel
        product_name = "Sky Auto Player"
        identifier = "io.github.pumni.skyautoplayer"
        target = "nsis"
        install_mode = "currentUser"
        installer = $InstallerName
        installer_size = $InstallerSize
        installer_sha256 = $InstallerSha256.ToLowerInvariant()
        updater_signature = $SignatureName
        signature_size = $SignatureSize
        updater_signature_sha256 = $SignatureSha256.ToLowerInvariant()
        authenticode_mode = "unsigned-zero-budget"
        authenticode_state = "unsigned"
        authenticode_provider = "none"
        approved_signer_thumbprint = $null
        observed_signer_thumbprint = if ($null -ne $ObservedSignerThumbprint -and [string]$ObservedSignerThumbprint -ne "") { [string]$ObservedSignerThumbprint } else { $null }
        authenticode_evidence = "TAURI_AUTHENTICODE_EVIDENCE.json"
        authenticode_evidence_sha256 = $AuthenticodeEvidenceSha256.ToLowerInvariant()
        updater_key_id = $UpdaterKeyId.ToUpperInvariant()
        updater_signature_status = "valid"
        sbom = "SBOM.spdx.json"
        sbom_sha256 = $SbomSha256.ToLowerInvariant()
        qualification_status = "PASS"
    }
}

