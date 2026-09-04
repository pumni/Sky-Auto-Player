param(
    [Parameter(Mandatory = $true)]
    [string]$EnvFile,
    [Parameter(Mandatory = $false)]
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = "Stop"
$subject = "CN=Sky Auto Player CI V4 Test Code Signing"
$timer = [Diagnostics.Stopwatch]::StartNew()
$thumbprint = $null
$pfxPath = $null
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
    $temporaryRoot = [IO.Path]::GetFullPath($temporaryRoot)
    $workerPath = Join-Path $temporaryRoot ("sky-v4-test-signing-worker-" + [guid]::NewGuid().ToString("N") + ".ps1")
    $resultPath = Join-Path $temporaryRoot ("sky-v4-test-signing-result-" + [guid]::NewGuid().ToString("N") + ".txt")
    $errorPath = Join-Path $temporaryRoot ("sky-v4-test-signing-error-" + [guid]::NewGuid().ToString("N") + ".txt")
    $pfxPath = Join-Path $temporaryRoot ("sky-v4-test-signing-" + [guid]::NewGuid().ToString("N") + ".pfx")
    $worker = @'
param(
    [Parameter(Mandatory = $true)] [string]$OutputPath,
    [Parameter(Mandatory = $true)] [string]$PfxPath,
    [Parameter(Mandatory = $true)] [string]$CertificateSubject
)
$ErrorActionPreference = 'Stop'

$rsa = [System.Security.Cryptography.RSA]::Create(2048)
try {
    $request = [System.Security.Cryptography.X509Certificates.CertificateRequest]::new(
        $CertificateSubject,
        $rsa,
        [System.Security.Cryptography.HashAlgorithmName]::SHA256,
        [System.Security.Cryptography.RSASignaturePadding]::Pkcs1)
    $request.CertificateExtensions.Add(
        [System.Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
            [System.Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature,
            $false))
    $oids = [System.Security.Cryptography.OidCollection]::new()
    [void]$oids.Add([System.Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.3', 'Code Signing'))
    $request.CertificateExtensions.Add(
        [System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($oids, $false))
    $request.CertificateExtensions.Add(
        [System.Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $false))

    $certificate = $request.CreateSelfSigned(
        [DateTimeOffset]::Now.AddMinutes(-5),
        [DateTimeOffset]::Now.AddDays(2))
    if ($null -eq $certificate -or -not $certificate.HasPrivateKey -or
        [string]::IsNullOrWhiteSpace($certificate.Thumbprint)) {
        throw 'Could not create the ephemeral V4 Authenticode test certificate with a private key'
    }
    $password = [guid]::NewGuid().ToString('N')
    [IO.File]::WriteAllBytes(
        $PfxPath,
        $certificate.Export(
            [System.Security.Cryptography.X509Certificates.X509ContentType]::Pfx,
            $password))
    [IO.File]::WriteAllText(
        $OutputPath,
        "$($certificate.Thumbprint)`n$password",
        [Text.UTF8Encoding]::new($false))
} finally {
    if ($null -ne $rsa) {
        $rsa.Dispose()
    }
}
'@
    [IO.File]::WriteAllText($workerPath, $worker, [Text.UTF8Encoding]::new($false))
    Write-Host "Certificate worker script written"
    Write-Host "Launching certificate worker"
    $process = Start-Process -FilePath 'pwsh' -ArgumentList @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $workerPath, '-OutputPath', $resultPath, '-PfxPath', $pfxPath,
        '-CertificateSubject', ('"{0}"' -f $subject)
    ) -WindowStyle Hidden -RedirectStandardError $errorPath -PassThru
    Write-Host "Certificate worker launched (pid=$($process.Id))"
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
    }
    if (-not $process.HasExited) {
        Write-Host "Certificate worker exceeded deadline; requesting asynchronous termination (pid=$($process.Id))"
        Start-Process -FilePath 'taskkill.exe' -ArgumentList @('/PID', $process.Id, '/T', '/F') -WindowStyle Hidden | Out-Null
        Write-Host "Certificate worker termination requested"
        throw "Timed out after ${TimeoutSeconds}s creating the ephemeral V4 Authenticode test certificate"
    }
    Write-Host "Certificate worker exited (pid=$($process.Id), exit=$($process.ExitCode))"
    if ($process.ExitCode -ne 0) {
        $workerError = if (Test-Path -LiteralPath $errorPath) {
            ([IO.File]::ReadAllText($errorPath)).Trim()
        } else {
            'worker produced no diagnostics'
        }
        throw "Ephemeral V4 Authenticode certificate worker failed with exit code $($process.ExitCode): $workerError"
    }
    Write-Host "Reading certificate worker result"
    $workerResult = @(Get-Content -LiteralPath $resultPath -ErrorAction Stop |
        ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($workerResult.Count -ne 2 -or $workerResult[0] -notmatch '^[0-9a-fA-F]{40}$' -or
        $workerResult[1] -notmatch '^[0-9a-f]{32}$') {
        throw "Ephemeral V4 Authenticode certificate worker returned an invalid PFX identity"
    }
    $thumbprint = $workerResult[0].Trim().ToUpperInvariant()
    $pfxPassword = $workerResult[1].Trim()
    if (-not (Test-Path -LiteralPath $pfxPath -PathType Leaf)) {
        throw "Ephemeral V4 Authenticode certificate worker did not create its PFX output"
    }
    Write-Host "Certificate worker result read (thumbprint=$thumbprint)"
    $pfxCertificate = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new(
        $pfxPath,
        $pfxPassword,
        [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet)
    try {
        if (-not $pfxCertificate.HasPrivateKey) {
            throw 'Ephemeral V4 Authenticode PFX does not contain a private key'
        }
        $pfxThumbprint = ([string]$pfxCertificate.Thumbprint).Trim().ToUpperInvariant()
        if ($pfxThumbprint -ne $thumbprint) {
            throw "Ephemeral V4 Authenticode PFX thumbprint mismatch: expected $thumbprint, got $pfxThumbprint"
        }
    } finally {
        $pfxCertificate.Dispose()
    }

    Write-Host "Writing certificate environment"
    "SKY_AUTHENTICODE_MODE=test" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
    "SKY_AUTHENTICODE_TEST_THUMBPRINT=$thumbprint" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
    "SKY_AUTHENTICODE_TEST_PFX_PATH=$pfxPath" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
    "SKY_AUTHENTICODE_TEST_PFX_PASSWORD=$pfxPassword" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
    Write-CertificateSetupEvidence 'PASS' "thumbprint=$thumbprint"
} catch {
    Write-CertificateSetupEvidence 'FAIL' $_.Exception.Message
    if (-not [string]::IsNullOrWhiteSpace($pfxPath)) {
        Remove-Item -LiteralPath $pfxPath -Force -ErrorAction SilentlyContinue
    }
    throw
} finally {
    foreach ($temporaryPath in @($workerPath, $resultPath, $errorPath)) {
        if (-not [string]::IsNullOrWhiteSpace($temporaryPath)) {
            Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
        }
    }
}
