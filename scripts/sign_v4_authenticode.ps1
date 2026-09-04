param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Path
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "Authenticode signing target does not exist: $Path"
}
Write-Host "V4 Authenticode signer invoked: target=$Path"

$mode = if ([string]::IsNullOrWhiteSpace($env:SKY_AUTHENTICODE_MODE)) {
    "production"
} else {
    $env:SKY_AUTHENTICODE_MODE.Trim().ToLowerInvariant()
}

if ($mode -eq "production") {
    # Guard: Fail closed if ephemeral CI test credentials are provided in production mode
    if (-not [string]::IsNullOrWhiteSpace($env:SKY_AUTHENTICODE_TEST_PFX_PATH) -or
        -not [string]::IsNullOrWhiteSpace($env:SKY_AUTHENTICODE_TEST_PFX_PASSWORD) -or
        -not [string]::IsNullOrWhiteSpace($env:SKY_AUTHENTICODE_TEST_THUMBPRINT)) {
        throw "V4 production Authenticode signing is fail-closed: ephemeral CI test credentials cannot satisfy production mode"
    }

    $approvedThumbprint = [string]$env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT
    if ($approvedThumbprint -notmatch '^[0-9a-fA-F]{40}$') {
        throw "V4 production Authenticode signing requires the approved 40-character SHA-1 certificate thumbprint in SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT"
    }
    $approvedThumbprint = $approvedThumbprint.Trim().ToUpperInvariant()

    $provider = [string]$env:SKY_AUTHENTICODE_PROVIDER
    if ([string]::IsNullOrWhiteSpace($provider)) {
        throw "V4 Authenticode signing is fail-closed: no approved production provider is configured (set SKY_AUTHENTICODE_PROVIDER and SKY_AUTHENTICODE_PROVIDER_COMMAND or SKY_AUTHENTICODE_PROVIDER_SCRIPT)"
    }
    $provider = $provider.Trim().ToLowerInvariant()

    $providerCommand = [string]$env:SKY_AUTHENTICODE_PROVIDER_COMMAND
    $providerScript = [string]$env:SKY_AUTHENTICODE_PROVIDER_SCRIPT
    $hasCommand = -not [string]::IsNullOrWhiteSpace($providerCommand)
    $hasScript = -not [string]::IsNullOrWhiteSpace($providerScript)
    if (-not $hasCommand -and -not $hasScript) {
        throw "V4 Authenticode signing is fail-closed: provider '$provider' requires exactly one of SKY_AUTHENTICODE_PROVIDER_SCRIPT or SKY_AUTHENTICODE_PROVIDER_COMMAND"
    }
    if ($hasCommand -and $hasScript) {
        throw "V4 Authenticode signing is fail-closed: mutually exclusive configuration (both SKY_AUTHENTICODE_PROVIDER_SCRIPT and SKY_AUTHENTICODE_PROVIDER_COMMAND are set; specify exactly one)"
    }

    Write-Host "V4 Authenticode production signer invoked: provider=$provider, approved_thumbprint=$approvedThumbprint, target=$Path"

    if (-not [string]::IsNullOrWhiteSpace($providerScript)) {
        $resolvedScript = (Resolve-Path -LiteralPath $providerScript -ErrorAction Stop).Path
        if (-not (Test-Path -LiteralPath $resolvedScript -PathType Leaf)) {
            throw "Production Authenticode provider script does not exist: $providerScript"
        }
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $resolvedScript -Path $Path
        if ($LASTEXITCODE -ne 0) {
            throw "Production Authenticode provider script failed with exit code $LASTEXITCODE"
        }
    } else {
        $expandedCommand = $providerCommand.Replace('%1', "`"$Path`"").Replace('$Path', "`"$Path`"")
        & pwsh -NoProfile -NonInteractive -Command $expandedCommand
        if ($LASTEXITCODE -ne 0) {
            throw "Production Authenticode provider command failed with exit code $LASTEXITCODE"
        }
    }

    # Post-signing seam contract verification:
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($null -eq $signature.SignerCertificate) {
        throw "Production Authenticode provider seam failed: target $Path has no embedded signer certificate after signing"
    }
    $actualThumbprint = ([string]$signature.SignerCertificate.Thumbprint).Trim().ToUpperInvariant()
    if ($actualThumbprint -ne $approvedThumbprint) {
        throw "Production Authenticode signer thumbprint mismatch: expected $approvedThumbprint, got $actualThumbprint"
    }
    if ($signature.SignerCertificate.Subject -match 'CI V4 Test Code Signing') {
        throw "Production Authenticode signing rejected: signer certificate is a CI test certificate"
    }
    if ([string]$signature.Status -eq "NotSigned") {
        throw "Production Authenticode signing rejected: target remains unsigned"
    }
    . (Join-Path $PSScriptRoot "v4_authenticode_crypto.ps1")
    $integrity = Get-AuthenticodeIntegrityProof -Path $Path -ExpectedThumbprint $approvedThumbprint
    if ($integrity.IntegrityStatus -ne "Valid" -or $integrity.Verification -ne "signature-valid-independent-cryptographic-integrity") {
        throw "Production Authenticode provider seam failed: cryptographic integrity verification failed on signed target"
    }
    Write-Host "V4 Authenticode production signing PASS: verified signed target $Path against approved thumbprint $approvedThumbprint"
    exit 0
}

if ($mode -ne "test") {
    throw "V4 Authenticode signing is fail-closed: unrecognized mode '$mode' (must be 'test' or 'production')"
}

$thumbprint = [string]$env:SKY_AUTHENTICODE_TEST_THUMBPRINT
if ($thumbprint -notmatch '^[0-9a-fA-F]{40}$') {
    throw "V4 test Authenticode signing requires a bounded SHA-1 certificate thumbprint"
}
$thumbprint = $thumbprint.Trim().ToUpperInvariant()
$pfxPath = ([string]$env:SKY_AUTHENTICODE_TEST_PFX_PATH).Trim()
$pfxPassword = [string]$env:SKY_AUTHENTICODE_TEST_PFX_PASSWORD
if ([string]::IsNullOrWhiteSpace($pfxPath) -or [string]::IsNullOrWhiteSpace($pfxPassword)) {
    throw "V4 test Authenticode signing requires the ephemeral PFX path and password"
}
if (-not (Test-Path -LiteralPath $pfxPath -PathType Leaf)) {
    throw "V4 test Authenticode signing PFX does not exist: $pfxPath"
}
$pfxCertificate = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new(
    $pfxPath,
    $pfxPassword,
    [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet)
try {
    if (-not $pfxCertificate.HasPrivateKey) {
        throw "V4 test Authenticode signing PFX does not contain a private key"
    }
    $pfxThumbprint = ([string]$pfxCertificate.Thumbprint).Trim().ToUpperInvariant()
    if ($pfxThumbprint -ne $thumbprint) {
        throw "V4 test Authenticode signing PFX thumbprint mismatch: expected $thumbprint, got $pfxThumbprint"
    }
} finally {
    $pfxCertificate.Dispose()
}

function Find-SignTool {
    if (-not [string]::IsNullOrWhiteSpace($env:TAURI_WINDOWS_SIGNTOOL_PATH)) {
        $candidate = (Resolve-Path -LiteralPath $env:TAURI_WINDOWS_SIGNTOOL_PATH -ErrorAction Stop).Path
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) { return $command.Source }
    $kits = Get-ItemProperty -Path "HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots" -ErrorAction SilentlyContinue
    if ($null -ne $kits -and -not [string]::IsNullOrWhiteSpace($kits.KitsRoot10)) {
        $matches = @(Get-ChildItem -LiteralPath (Join-Path $kits.KitsRoot10 "bin") -Filter signtool.exe -File -Recurse -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
            Sort-Object FullName -Descending)
        if ($matches.Count -gt 0) { return $matches[0].FullName }
    }
    throw "signtool.exe is not available on the Windows signing runner"
}

$signTool = Find-SignTool
Write-Host "V4 Authenticode signer using signtool: $signTool"
Write-Host "V4 Authenticode signer certificate thumbprint: $thumbprint"
& $signTool sign /fd SHA256 /f $pfxPath /p $pfxPassword $Path
if ($LASTEXITCODE -ne 0) {
    throw "signtool.exe failed to sign the Authenticode target"
}
