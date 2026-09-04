param(
    [Parameter(Mandatory = $true)]
    [string]$EnvFile
)

$ErrorActionPreference = "Stop"
$subject = "CN=Sky Auto Player CI V4 Test Code Signing"
$certificate = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject $subject `
    -CertStoreLocation Cert:\CurrentUser\My `
    -HashAlgorithm SHA256 `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -NotAfter (Get-Date).AddDays(2)
if ($null -eq $certificate -or [string]::IsNullOrWhiteSpace($certificate.Thumbprint)) {
    throw "Could not create the ephemeral V4 Authenticode test certificate"
}

$temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$certificatePath = Join-Path $temporaryRoot ("sky-v4-test-signing-" + [guid]::NewGuid().ToString("N") + ".cer")
try {
    Export-Certificate -Cert $certificate -FilePath $certificatePath -Type CERT | Out-Null
    Import-Certificate -FilePath $certificatePath -CertStoreLocation Cert:\CurrentUser\Root | Out-Null
} finally {
    Remove-Item -LiteralPath $certificatePath -Force -ErrorAction SilentlyContinue
}

"SKY_AUTHENTICODE_MODE=test" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
"SKY_AUTHENTICODE_TEST_THUMBPRINT=$($certificate.Thumbprint)" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
Write-Host "Ephemeral V4 Authenticode test certificate is ready"
