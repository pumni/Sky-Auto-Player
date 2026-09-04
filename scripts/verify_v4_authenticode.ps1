param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("test", "production")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string[]]$Artifact,

    [string]$Evidence
)

$ErrorActionPreference = "Stop"
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
    $status = [string]$signature.Status
    if ($null -eq $signature.SignerCertificate) {
        throw "Authenticode signature has no embedded signer certificate for $($file.Name): $status"
    }
    $signerThumbprint = ([string]$signature.SignerCertificate.Thumbprint).Trim().ToUpperInvariant()
    if ($signerThumbprint -ne $expectedThumbprint) {
        throw "Authenticode signer thumbprint mismatch for $($file.Name): expected $expectedThumbprint, got $signerThumbprint"
    }
    $trustException = $null
    $verification = "signature-valid"
    if ($status -ne "Valid") {
        if ($mode -ne "test" -or $status -notin @("NotTrusted", "UnknownError")) {
            throw "Authenticode signature status is not accepted for $($file.Name): $status"
        }
        $trustException = "test-self-signed-untrusted-chain"
        $verification = "signature-valid-untrusted-chain"
    }
    [ordered]@{
        name = $file.Name
        status = $status
        verification = $verification
        trust_exception = $trustException
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
        "embedded-signature-exact-test-identity-with-narrow-untrusted-chain-allowlist"
    } else {
        "windows-valid-signature-exact-approved-production-identity"
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
