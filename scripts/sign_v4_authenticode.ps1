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
if ($mode -ne "test") {
    throw "V4 Authenticode signing is fail-closed: no approved production provider is configured"
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
