Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$thumbprint = ([string]$env:SKY_AUTHENTICODE_TEST_THUMBPRINT).Trim()
$subject = 'CN=Sky Auto Player CI V4 Test Code Signing'
if ([string]::IsNullOrWhiteSpace($thumbprint)) {
    Write-Host 'V4 Authenticode test certificate cleanup: no test thumbprint was published'
    exit 0
}
if ($thumbprint -notmatch '^[0-9a-fA-F]{40}$') {
    throw 'V4 Authenticode test certificate cleanup received an invalid SHA-1 thumbprint'
}

$stores = @('My', 'TrustedPublisher', 'Root')
foreach ($store in $stores) {
    $certificatePath = "Cert:\CurrentUser\${store}\$thumbprint"
    if (Test-Path -LiteralPath $certificatePath) {
        $certificate = Get-Item -LiteralPath $certificatePath -ErrorAction Stop
        if ($certificate.Subject -ne $subject) {
            throw "Refusing to remove a non-test certificate from CurrentUser/${store}"
        }
        Remove-Item -LiteralPath $certificatePath -Force -ErrorAction Stop
    }
    if (Test-Path -LiteralPath $certificatePath) {
        throw "Could not remove the ephemeral V4 Authenticode certificate from CurrentUser/${store}"
    }
    Write-Host "V4 Authenticode test certificate cleanup: CurrentUser/${store} clear"
}

Write-Host "V4 Authenticode test certificate cleanup: PASS (thumbprint=$thumbprint)"
