param(
    [Parameter(Mandatory = $true)]
    [string]$EnvFile,
    [Parameter(Mandatory = $false)]
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 120
)

$ErrorActionPreference = "Stop"
$subject = "CN=Sky Auto Player CI V4 Test Code Signing"
$timer = [Diagnostics.Stopwatch]::StartNew()
$thumbprint = $null
$summaryPath = $env:GITHUB_STEP_SUMMARY
$workerPath = $null
$resultPath = $null
$errorPath = $null

function Write-CertificateSetupEvidence {
    param(
        [string]$Status,
        [string]$Detail
    )
    $elapsed = [math]::Round($timer.Elapsed.TotalSeconds, 3)
    Write-Host "V4 Authenticode test certificate setup: $Status (${elapsed}s) $Detail"
    if (-not [string]::IsNullOrWhiteSpace($summaryPath)) {
        "- Authenticode test certificate setup: **$Status** (${elapsed}s) — $Detail" |
            Add-Content -LiteralPath $summaryPath -Encoding UTF8
    }
}

try {
    Write-Host "Starting bounded V4 Authenticode test certificate setup (timeout=${TimeoutSeconds}s)"
    $temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
        [IO.Path]::GetTempPath()
    } else {
        $env:RUNNER_TEMP
    }
    $workerPath = Join-Path $temporaryRoot ("sky-v4-test-signing-worker-" + [guid]::NewGuid().ToString("N") + ".ps1")
    $resultPath = Join-Path $temporaryRoot ("sky-v4-test-signing-result-" + [guid]::NewGuid().ToString("N") + ".txt")
    $errorPath = Join-Path $temporaryRoot ("sky-v4-test-signing-error-" + [guid]::NewGuid().ToString("N") + ".txt")
    $worker = @'
param(
    [Parameter(Mandatory = $true)] [string]$OutputPath,
    [Parameter(Mandatory = $true)] [string]$CertificateSubject
)
$ErrorActionPreference = 'Stop'
$certificate = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject $CertificateSubject `
    -CertStoreLocation Cert:\CurrentUser\My `
    -HashAlgorithm SHA256 `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -NotAfter (Get-Date).AddDays(2)
if ($null -eq $certificate -or [string]::IsNullOrWhiteSpace($certificate.Thumbprint)) {
    throw 'Could not create the ephemeral V4 Authenticode test certificate'
}
Set-Content -LiteralPath $OutputPath -Value $certificate.Thumbprint -Encoding ASCII
'@
    [IO.File]::WriteAllText($workerPath, $worker, [Text.UTF8Encoding]::new($false))
    $process = Start-Process -FilePath 'pwsh' -ArgumentList @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $workerPath, '-OutputPath', $resultPath, '-CertificateSubject', $subject
    ) -WindowStyle Hidden -RedirectStandardError $errorPath -PassThru
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Timed out after ${TimeoutSeconds}s creating the ephemeral V4 Authenticode test certificate"
    }
    if ($process.ExitCode -ne 0) {
        $workerError = if (Test-Path -LiteralPath $errorPath) {
            ([IO.File]::ReadAllText($errorPath)).Trim()
        } else {
            'worker produced no diagnostics'
        }
        throw "Ephemeral V4 Authenticode certificate worker failed with exit code $($process.ExitCode): $workerError"
    }
    $thumbprints = @(Get-Content -LiteralPath $resultPath -ErrorAction Stop |
        ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($thumbprints.Count -ne 1 -or $thumbprints[0] -notmatch '^[0-9a-fA-F]{40}$') {
        throw "Ephemeral V4 Authenticode certificate worker returned an invalid thumbprint"
    }
    $thumbprint = $thumbprints[0].Trim()
    $certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$thumbprint" -ErrorAction Stop

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
    "SKY_AUTHENTICODE_TEST_THUMBPRINT=$thumbprint" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
    Write-CertificateSetupEvidence 'PASS' "thumbprint=$thumbprint"
} catch {
    Write-CertificateSetupEvidence 'FAIL' $_.Exception.Message
    if (-not [string]::IsNullOrWhiteSpace($thumbprint)) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$thumbprint" -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath "Cert:\CurrentUser\Root\$thumbprint" -Force -ErrorAction SilentlyContinue
    }
    throw
} finally {
    foreach ($temporaryPath in @($workerPath, $resultPath, $errorPath)) {
        if (-not [string]::IsNullOrWhiteSpace($temporaryPath)) {
            Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
        }
    }
}
