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
$summaryPath = $env:GITHUB_STEP_SUMMARY
$workerPath = $null
$resultPath = $null
$errorPath = $null
$importOutputPath = $null
$importErrorPath = $null

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
    $importOutputPath = Join-Path $temporaryRoot ("sky-v4-test-signing-import-" + [guid]::NewGuid().ToString("N") + ".txt")
    $importErrorPath = Join-Path $temporaryRoot ("sky-v4-test-signing-import-error-" + [guid]::NewGuid().ToString("N") + ".txt")
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
    Write-Host "Certificate worker script written"
    Write-Host "Launching certificate worker"
    $process = Start-Process -FilePath 'pwsh' -ArgumentList @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $workerPath, '-OutputPath', $resultPath, '-CertificateSubject', ('"{0}"' -f $subject)
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
    $thumbprints = @(Get-Content -LiteralPath $resultPath -ErrorAction Stop |
        ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($thumbprints.Count -ne 1 -or $thumbprints[0] -notmatch '^[0-9a-fA-F]{40}$') {
        throw "Ephemeral V4 Authenticode certificate worker returned an invalid thumbprint"
    }
    $thumbprint = $thumbprints[0].Trim()
    Write-Host "Certificate worker result read (thumbprint=$thumbprint)"
    Write-Host "Loading certificate from CurrentUser/My"
    $certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$thumbprint" -ErrorAction Stop
    Write-Host "Certificate loaded"

    $temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
        [IO.Path]::GetTempPath()
    } else {
        $env:RUNNER_TEMP
    }
    $certificatePath = Join-Path $temporaryRoot ("sky-v4-test-signing-" + [guid]::NewGuid().ToString("N") + ".cer")
    try {
        Write-Host "Exporting certificate"
        Export-Certificate -Cert $certificate -FilePath $certificatePath -Type CERT | Out-Null
        foreach ($store in @('Root', 'TrustedPublisher')) {
            Remove-Item -LiteralPath $importOutputPath, $importErrorPath -Force -ErrorAction SilentlyContinue
            Write-Host "Importing certificate into CurrentUser/${store} via certutil"
            $importProcess = Start-Process -FilePath 'certutil.exe' -ArgumentList @(
                '-f', '-user', '-addstore', $store, $certificatePath
            ) -WindowStyle Hidden -RedirectStandardOutput $importOutputPath -RedirectStandardError $importErrorPath -PassThru
            Write-Host "Certificate import worker launched (store=${store}, pid=$($importProcess.Id))"
            $importDeadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
            while (-not $importProcess.HasExited -and [DateTime]::UtcNow -lt $importDeadline) {
                Start-Sleep -Milliseconds 250
                $importProcess.Refresh()
            }
            if (-not $importProcess.HasExited) {
                Write-Host "Certificate import worker exceeded deadline; requesting asynchronous termination (store=${store}, pid=$($importProcess.Id))"
                Start-Process -FilePath 'taskkill.exe' -ArgumentList @('/PID', $importProcess.Id, '/T', '/F') -WindowStyle Hidden | Out-Null
                throw "Timed out after ${TimeoutSeconds}s importing the ephemeral V4 Authenticode test certificate into ${store}"
            }
            Write-Host "Certificate import worker exited (store=${store}, pid=$($importProcess.Id), exit=$($importProcess.ExitCode))"
            if ($importProcess.ExitCode -ne 0) {
                $importOutput = if (Test-Path -LiteralPath $importOutputPath) {
                    ([IO.File]::ReadAllText($importOutputPath)).Trim()
                } else {
                    ''
                }
                $importError = if (Test-Path -LiteralPath $importErrorPath) {
                    ([IO.File]::ReadAllText($importErrorPath)).Trim()
                } else {
                    'certutil produced no diagnostics'
                }
                throw "certutil failed to import the ephemeral V4 Authenticode test certificate into ${store}: $importOutput $importError"
            }
        }
        Write-Host "Certificate root and publisher trust imports completed"
    } finally {
        Remove-Item -LiteralPath $certificatePath -Force -ErrorAction SilentlyContinue
    }

    Write-Host "Writing certificate environment"
    "SKY_AUTHENTICODE_MODE=test" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
    "SKY_AUTHENTICODE_TEST_THUMBPRINT=$thumbprint" | Add-Content -LiteralPath $EnvFile -Encoding UTF8
    Write-CertificateSetupEvidence 'PASS' "thumbprint=$thumbprint"
} catch {
    Write-CertificateSetupEvidence 'FAIL' $_.Exception.Message
    if (-not [string]::IsNullOrWhiteSpace($thumbprint)) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$thumbprint" -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath "Cert:\CurrentUser\TrustedPublisher\$thumbprint" -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath "Cert:\CurrentUser\Root\$thumbprint" -Force -ErrorAction SilentlyContinue
    }
    throw
} finally {
    foreach ($temporaryPath in @($workerPath, $resultPath, $errorPath, $importOutputPath, $importErrorPath)) {
        if (-not [string]::IsNullOrWhiteSpace($temporaryPath)) {
            Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
        }
    }
}
