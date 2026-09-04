param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("test", "production")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string[]]$Artifact,

    [string]$Evidence
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "v4_authenticode_crypto.ps1")
if ($Artifact.Count -eq 0) { throw "At least one Authenticode artifact is required" }
$mode = $Mode.ToLowerInvariant()
$expectedThumbprint = if ($mode -eq "test") {
    [string]$env:SKY_AUTHENTICODE_TEST_THUMBPRINT
} else {
    [string]$env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT
}
$expectedThumbprint = $expectedThumbprint.Trim()
if ($expectedThumbprint -notmatch '^[0-9a-fA-F]{40}$') {
    $identityVariable = if ($mode -eq "test") {
        "SKY_AUTHENTICODE_TEST_THUMBPRINT"
    } else {
        "SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT"
    }
    throw "V4 Authenticode verification requires the exact approved signer thumbprint in $identityVariable"
}
$expectedThumbprint = $expectedThumbprint.ToUpperInvariant()

function Resolve-TestPfxPath {
    $pfxPath = ([string]$env:SKY_AUTHENTICODE_TEST_PFX_PATH).Trim()
    if ([string]::IsNullOrWhiteSpace($pfxPath)) {
        throw "V4 test Authenticode verification requires SKY_AUTHENTICODE_TEST_PFX_PATH"
    }
    $temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
        [IO.Path]::GetTempPath()
    } else {
        $env:RUNNER_TEMP
    }
    $temporaryRoot = [IO.Path]::GetFullPath($temporaryRoot).TrimEnd('\', '/')
    $resolvedPfxPath = [IO.Path]::GetFullPath($pfxPath)
    $rootPrefix = $temporaryRoot + [IO.Path]::DirectorySeparatorChar
    if (-not $resolvedPfxPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "V4 test Authenticode verification PFX must be under RUNNER_TEMP"
    }
    if ([IO.Path]::GetFileName($resolvedPfxPath) -notmatch '^sky-v4-test-signing-[0-9a-fA-F]{32}\.pfx$') {
        throw "V4 test Authenticode verification PFX has an unexpected filename"
    }
    if (-not (Test-Path -LiteralPath $resolvedPfxPath -PathType Leaf)) {
        throw "V4 test Authenticode verification PFX does not exist"
    }
    return $resolvedPfxPath
}

if ($mode -eq "test") {
    $pfxPath = Resolve-TestPfxPath
    $pfxPassword = [string]$env:SKY_AUTHENTICODE_TEST_PFX_PASSWORD
    if ($pfxPassword -notmatch '^[0-9a-f]{32}$') {
        throw "V4 test Authenticode verification requires the bounded ephemeral PFX password"
    }
    $pfxCertificate = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new(
        $pfxPath,
        $pfxPassword,
        [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet)
    try {
        if (-not $pfxCertificate.HasPrivateKey) {
            throw "V4 test Authenticode verification PFX does not contain a private key"
        }
        $pfxThumbprint = ([string]$pfxCertificate.Thumbprint).Trim().ToUpperInvariant()
        if ($pfxThumbprint -ne $expectedThumbprint) {
            throw "V4 test Authenticode PFX thumbprint mismatch: expected $expectedThumbprint, got $pfxThumbprint"
        }
    } finally {
        $pfxCertificate.Dispose()
    }
}

$files = foreach ($path in $Artifact) {
    $resolved = (Resolve-Path -LiteralPath $path -ErrorAction Stop).Path
    $item = Get-Item -LiteralPath $resolved -ErrorAction Stop
    if (-not $item.PSIsContainer -and $item.Extension.ToLowerInvariant() -in @(".exe", ".dll")) {
        $item
    } else {
        throw "Authenticode evidence target must be a regular PE file: $path"
    }
}

$seen = @{}
$records = foreach ($file in $files) {
    if ($seen.ContainsKey($file.Name)) { throw "Duplicate Authenticode evidence filename: $($file.Name)" }
    $seen[$file.Name] = $true
    $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
    $platformStatus = [string]$signature.Status
    if ($null -eq $signature.SignerCertificate) {
        throw "Authenticode signature has no embedded signer certificate for $($file.Name): $platformStatus"
    }
    $signerThumbprint = ([string]$signature.SignerCertificate.Thumbprint).Trim().ToUpperInvariant()
    if ($signerThumbprint -ne $expectedThumbprint) {
        throw "Authenticode signer thumbprint mismatch for $($file.Name): expected $expectedThumbprint, got $signerThumbprint"
    }
    $rejectedStatuses = @(
        "NotSigned",
        "HashMismatch",
        "Incompatible",
        "NotSupported",
        "PublisherMismatch",
        "Error"
    )
    if ($rejectedStatuses -contains $platformStatus) {
        throw "Unsupported or invalid Authenticode status for $($file.Name): $platformStatus"
    }
    $statusType = $signature.Status.GetType()
    if ($statusType.IsEnum -and [Enum]::GetNames($statusType) -notcontains $platformStatus) {
        throw "Unknown Authenticode status for $($file.Name): $platformStatus"
    }
    if ($mode -eq "production" -and $platformStatus -ne "Valid") {
        throw "Production Authenticode verification requires Windows status Valid for $($file.Name): $platformStatus"
    }
    # The platform status is recorded, but it is not the test-mode integrity
    # decision. In particular, UnknownError is never accepted as proof. The
    # independent SignedCms/SPC PE-digest proof below is the only reason a
    # non-Valid test status can continue.
    $integrity = Get-AuthenticodeIntegrityProof -Path $file.FullName -ExpectedThumbprint $expectedThumbprint
    if ($integrity.SignerThumbprint -ne $expectedThumbprint) {
        throw "Independent Authenticode signer identity mismatch for $($file.Name)"
    }
    $trustException = $null
    $verification = [string]$integrity.Verification
    if ($mode -eq "test" -and $platformStatus -ne "Valid") {
        $trustException = "test-platform-status-not-used-for-integrity"
    }
    [ordered]@{
        name = $file.Name
        status = $platformStatus
        platform_status = $platformStatus
        verification = $verification
        trust_exception = $trustException
        integrity_verifier = [string]$integrity.IntegrityVerifier
        integrity_status = [string]$integrity.IntegrityStatus
        signed_digest_algorithm = [string]$integrity.DigestAlgorithm
        signed_digest = [string]$integrity.SignedDigest
        computed_digest = [string]$integrity.ComputedDigest
        sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        signer_thumbprint = $signerThumbprint
        signer_subject = [string]$signature.SignerCertificate.Subject
    }
}

$payload = [ordered]@{
    schema_version = 1
    evidence_type = "authenticode-verification"
    mode = $mode
    expected_signer_thumbprint = $expectedThumbprint
    verification_policy = if ($mode -eq "test") {
        "exact-signer-independent-authenticode-integrity-with-platform-trust-diagnostic"
    } else {
        "windows-valid-platform-and-exact-approved-signer-independent-authenticode-integrity"
    }
    files = @($records)
}

if ([string]::IsNullOrWhiteSpace($Evidence)) {
    Write-Host "V4 Authenticode verification: PASS ($($files.Count) PE file(s), mode=$Mode)"
} else {
    $evidenceParent = Split-Path -Parent $Evidence
    if (-not [string]::IsNullOrWhiteSpace($evidenceParent)) {
        New-Item -ItemType Directory -Path $evidenceParent -Force | Out-Null
    }
    $payload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Evidence -Encoding UTF8
    Write-Host "V4 Authenticode evidence written: $Evidence"
}
