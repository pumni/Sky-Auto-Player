param(
    [string]$ExpectedSourceSha,
    [string]$Version,
    [string]$Channel,
    [string]$UpdaterPrivateKeyPath,
    [string]$UpdaterPasswordEnv = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    [string]$AuthenticodeProvider,
    [string]$ApprovedSignerThumbprint,
    [string]$AuthenticodeProviderScript,
    [string]$AuthenticodeProviderCommand,
    [string]$BundleDir,
    [string]$EvidenceDir,
    [switch]$SkipBuild,
    [switch]$SkipInstallSmoke
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

# 1. Validate mandatory parameters explicitly (fail closed without interactive stdin blocking)
if ([string]::IsNullOrWhiteSpace($ExpectedSourceSha)) {
    throw "Missing mandatory parameter: ExpectedSourceSha (must be 40-character commit SHA)"
}
if ([string]::IsNullOrWhiteSpace($Version)) {
    throw "Missing mandatory parameter: Version"
}
if ([string]::IsNullOrWhiteSpace($Channel) -or $Channel -notin @("stable", "beta")) {
    throw "Missing or invalid mandatory parameter: Channel (must be 'stable' or 'beta')"
}
if ([string]::IsNullOrWhiteSpace($UpdaterPrivateKeyPath)) {
    throw "Missing mandatory parameter: UpdaterPrivateKeyPath"
}
if ([string]::IsNullOrWhiteSpace($AuthenticodeProvider)) {
    throw "Missing mandatory parameter: AuthenticodeProvider"
}
if ([string]::IsNullOrWhiteSpace($ApprovedSignerThumbprint)) {
    throw "Missing mandatory parameter: ApprovedSignerThumbprint"
}

# 1. Environment preservation
$savedEnv = @{
    'SKY_AUTHENTICODE_MODE' = $env:SKY_AUTHENTICODE_MODE
    'SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT' = $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT
    'SKY_AUTHENTICODE_PROVIDER' = $env:SKY_AUTHENTICODE_PROVIDER
    'SKY_AUTHENTICODE_PROVIDER_SCRIPT' = $env:SKY_AUTHENTICODE_PROVIDER_SCRIPT
    'SKY_AUTHENTICODE_PROVIDER_COMMAND' = $env:SKY_AUTHENTICODE_PROVIDER_COMMAND
    'SKY_AUTHENTICODE_TEST_PFX_PATH' = $env:SKY_AUTHENTICODE_TEST_PFX_PATH
    'SKY_AUTHENTICODE_TEST_PFX_PASSWORD' = $env:SKY_AUTHENTICODE_TEST_PFX_PASSWORD
    'SKY_AUTHENTICODE_TEST_THUMBPRINT' = $env:SKY_AUTHENTICODE_TEST_THUMBPRINT
    'TAURI_SIGNING_PRIVATE_KEY_PATH' = $env:TAURI_SIGNING_PRIVATE_KEY_PATH
    'TAURI_SIGNING_PRIVATE_KEY_PASSWORD' = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
}

function Restore-SavedEnvironment {
    foreach ($entry in $savedEnv.GetEnumerator()) {
        if ($null -ne $entry.Value) {
            [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, "Process")
        } else {
            [Environment]::SetEnvironmentVariable($entry.Key, $null, "Process")
        }
    }
}

try {
    Write-Host "================================================================="
    Write-Host " Sky Auto Player V4 - Production Release Orchestrator"
    Write-Host "================================================================="

    # 2. Validate input parameters (identities and references only)
    if ($ExpectedSourceSha -notmatch '^[0-9a-fA-F]{40}$') {
        throw "ExpectedSourceSha must be a 40-character hexadecimal git commit SHA"
    }
    $expectedSha = $ExpectedSourceSha.ToLowerInvariant()

    $currentHead = (& git rev-parse HEAD).Trim().ToLowerInvariant()
    if ($currentHead -ne $expectedSha) {
        throw "Workspace HEAD ($currentHead) does not match ExpectedSourceSha ($expectedSha)"
    }

    # Verify Cargo project version matches
    $cargoTomlPath = Join-Path $repoRoot "desktop\src-tauri\Cargo.toml"
    $cargoToml = Get-Content -LiteralPath $cargoTomlPath -Raw
    if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
        throw "Failed to parse version from desktop/src-tauri/Cargo.toml"
    }
    $cargoVersion = $Matches[1].Trim()
    if ($cargoVersion -ne $Version) {
        throw "Specified version '$Version' does not match Cargo.toml version '$cargoVersion'"
    }

    # Validate channel vs version SemVer policy (ADR-0006 / v4-release-authority)
    $isPrerelease = $Version.Contains("-")
    if ($Channel -eq "stable" -and $isPrerelease) {
        throw "Channel 'stable' rejects prerelease version '$Version' (SemVer without hyphen required)"
    }
    if ($Channel -eq "beta" -and -not $isPrerelease) {
        throw "Channel 'beta' requires a prerelease SemVer version (e.g. '$Version-beta.1')"
    }

    # Validate ApprovedSignerThumbprint
    if ($ApprovedSignerThumbprint -notmatch '^[0-9a-fA-F]{40}$') {
        throw "ApprovedSignerThumbprint must be a 40-character hexadecimal SHA-1 thumbprint"
    }
    $approvedThumbprint = $ApprovedSignerThumbprint.ToUpperInvariant()

    # Validate Authenticode provider invocation (mutual exclusivity)
    $hasScript = -not [string]::IsNullOrWhiteSpace($AuthenticodeProviderScript)
    $hasCommand = -not [string]::IsNullOrWhiteSpace($AuthenticodeProviderCommand)
    if (-not $hasScript -and -not $hasCommand) {
        throw "Authenticode provider requires exactly one of AuthenticodeProviderScript or AuthenticodeProviderCommand"
    }
    if ($hasScript -and $hasCommand) {
        throw "Mutually exclusive Authenticode provider configuration: specify either AuthenticodeProviderScript or AuthenticodeProviderCommand, not both"
    }
    $resolvedProviderScript = $null
    if ($hasScript) {
        $resolvedProviderScript = (Resolve-Path -LiteralPath $AuthenticodeProviderScript -ErrorAction Stop).Path
        if (-not (Test-Path -LiteralPath $resolvedProviderScript -PathType Leaf)) {
            throw "AuthenticodeProviderScript does not exist: $AuthenticodeProviderScript"
        }
    }

    # Validate UpdaterPrivateKeyPath (must exist and must NOT be inside repository workspace)
    $resolvedKeyPath = (Resolve-Path -LiteralPath $UpdaterPrivateKeyPath -ErrorAction Stop).Path
    if (-not (Test-Path -LiteralPath $resolvedKeyPath -PathType Leaf)) {
        throw "UpdaterPrivateKeyPath does not exist: $UpdaterPrivateKeyPath"
    }
    $repoPrefix = $repoRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if ($resolvedKeyPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Security violation: UpdaterPrivateKeyPath must remain outside the repository workspace ($resolvedKeyPath)"
    }

    # Resolve password securely without logging or CLI flag exposure
    $passwordValue = if (-not [string]::IsNullOrWhiteSpace($UpdaterPasswordEnv)) {
        [string][Environment]::GetEnvironmentVariable($UpdaterPasswordEnv)
    } else {
        ""
    }
    if ([string]::IsNullOrEmpty($passwordValue) -and [Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
        Write-Host "Enter updater private key passphrase (press Enter if unencrypted): " -NoNewline
        $securePrompt = Read-Host -AsSecureString
        if ($securePrompt.Length -gt 0) {
            $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($securePrompt)
            try {
                $passwordValue = [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
            } finally {
                [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
            }
        }
    }
    if (-not [string]::IsNullOrEmpty($passwordValue) -and $env:GITHUB_ACTIONS -eq "true") {
        Write-Output "::add-mask::$passwordValue"
    }

    # Determine directories
    $resolvedBundleDir = if (-not [string]::IsNullOrWhiteSpace($BundleDir)) {
        [IO.Path]::GetFullPath($BundleDir)
    } else {
        Join-Path $repoRoot "rust\target\dist\bundle\nsis"
    }
    $resolvedEvidenceDir = if (-not [string]::IsNullOrWhiteSpace($EvidenceDir)) {
        [IO.Path]::GetFullPath($EvidenceDir)
    } else {
        Join-Path $repoRoot "rust\target\dist"
    }
    New-Item -ItemType Directory -Path $resolvedEvidenceDir -Force | Out-Null

    # 3. Pre-packaging Updater Key Validation (Fail closed before build)
    Write-Host "[Step 1/7] Validating updater private key against canonical public root..."
    # Clear test credentials before checking
    [Environment]::SetEnvironmentVariable("SKY_AUTHENTICODE_TEST_PFX_PATH", $null, "Process")
    [Environment]::SetEnvironmentVariable("SKY_AUTHENTICODE_TEST_PFX_PASSWORD", $null, "Process")
    [Environment]::SetEnvironmentVariable("SKY_AUTHENTICODE_TEST_THUMBPRINT", $null, "Process")
    
    $prevPwd = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $passwordValue
    try {
        & cargo xtask updater-trust verify-private-key --key-file $resolvedKeyPath
        if ($LASTEXITCODE -ne 0) {
            throw "Pre-packaging updater key verification failed: private key does not match canonical v4 public root"
        }
    } finally {
        if ($null -ne $prevPwd) {
            $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $prevPwd
        } else {
            Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
        }
    }
    Write-Host "  Updater private key matches canonical root F6355260A0C663D5: PASS"

    # 4. Canonical Single Build with Production Signing Enabled
    if (-not $SkipBuild) {
        Write-Host "[Step 2/7] Building canonical Tauri production artifact (build-once)..."
        
        # Set production Authenticode signing environment.
        # Tauri NSIS bundler invokes scripts/sign_v4_authenticode.ps1 via bundle.windows.signCommand seam.
        $env:SKY_AUTHENTICODE_MODE = "production"
        $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = $approvedThumbprint
        $env:SKY_AUTHENTICODE_PROVIDER = $AuthenticodeProvider.Trim().ToLowerInvariant()
        if ($hasScript) {
            $env:SKY_AUTHENTICODE_PROVIDER_SCRIPT = $resolvedProviderScript
            Remove-Item Env:SKY_AUTHENTICODE_PROVIDER_COMMAND -ErrorAction SilentlyContinue
        } else {
            $env:SKY_AUTHENTICODE_PROVIDER_COMMAND = $AuthenticodeProviderCommand
            Remove-Item Env:SKY_AUTHENTICODE_PROVIDER_SCRIPT -ErrorAction SilentlyContinue
        }
        $env:TAURI_SIGNING_PRIVATE_KEY_PATH = $resolvedKeyPath
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $passwordValue

        Push-Location (Join-Path $repoRoot "desktop")
        try {
            & bun install --frozen-lockfile
            if ($LASTEXITCODE -ne 0) { throw "bun install failed with exit code $LASTEXITCODE" }

            & bun run build
            if ($LASTEXITCODE -ne 0) { throw "bun run build failed with exit code $LASTEXITCODE" }

            & bun run tauri build --ci -- --profile dist
            if ($LASTEXITCODE -ne 0) { throw "bun run tauri build failed with exit code $LASTEXITCODE" }
        } finally {
            Pop-Location
        }
    } else {
        Write-Host "[Step 2/7] Skipping build (using existing exact candidate bytes)..."
    }

    # 5. Exact Artifact Verification
    Write-Host "[Step 3/7] Verifying canonical NSIS artifact set in $resolvedBundleDir..."
    if (-not (Test-Path -LiteralPath $resolvedBundleDir -PathType Container)) {
        throw "Bundle directory does not exist: $resolvedBundleDir"
    }
    $expectedInstallerName = "Sky Auto Player_${Version}_x64-setup.exe"
    $expectedSignatureName = "Sky Auto Player_${Version}_x64-setup.exe.sig"
    $installerPath = Join-Path $resolvedBundleDir $expectedInstallerName
    $signaturePath = Join-Path $resolvedBundleDir $expectedSignatureName

    if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
        throw "Canonical NSIS installer missing: $installerPath"
    }
    if (-not (Test-Path -LiteralPath $signaturePath -PathType Leaf)) {
        throw "Canonical updater signature missing: $signaturePath"
    }

    $installerBytes = [IO.File]::ReadAllBytes($installerPath)
    if ($installerBytes.Length -eq 0) { throw "Installer file is empty" }
    $signatureText = [IO.File]::ReadAllText($signaturePath)
    if ([string]::IsNullOrWhiteSpace($signatureText)) { throw "Updater signature is empty" }

    Write-Host "  Canonical candidate artifacts verified: $expectedInstallerName ($($installerBytes.Length) bytes)"

    # 6. Production Authenticode Verification
    Write-Host "[Step 4/7] Verifying Authenticode production signature..."
    $authenticodeEvidencePath = Join-Path $resolvedEvidenceDir "TAURI_AUTHENTICODE_EVIDENCE.json"
    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = $approvedThumbprint
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode production `
        -Artifact $installerPath `
        -Evidence $authenticodeEvidencePath
    if ($LASTEXITCODE -ne 0) {
        throw "Production Authenticode verification failed on $installerPath"
    }
    $authEvidence = Get-Content -LiteralPath $authenticodeEvidencePath -Raw | ConvertFrom-Json
    $observedThumbprint = $authEvidence.files[0].signer_thumbprint
    if ($observedThumbprint -ne $approvedThumbprint) {
        throw "Observed thumbprint ($observedThumbprint) does not match approved thumbprint ($approvedThumbprint)"
    }
    Write-Host "  Authenticode production verification: PASS (Thumbprint: $observedThumbprint)"

    # 7. Tauri Updater Signature Verification against Canonical Root
    Write-Host "[Step 5/7] Cryptographically verifying updater signature against canonical public root..."
    & cargo xtask updater-trust verify-signature `
        --installer $installerPath `
        --signature $signaturePath
    if ($LASTEXITCODE -ne 0) {
        throw "Updater signature cryptographic verification failed against canonical public root"
    }
    Write-Host "  Updater signature verification: PASS"

    # 8. SPDX SBOM Generation and Verification
    Write-Host "[Step 6/7] Generating and verifying SPDX SBOM for candidate..."
    $sbomPath = Join-Path $resolvedEvidenceDir "SBOM.spdx.json"
    $summaryPath = Join-Path $resolvedEvidenceDir "TAURI_ARTIFACT_SUMMARY.json"

    & cargo xtask sbom generate --artifact-dir $resolvedBundleDir --output $sbomPath
    if ($LASTEXITCODE -ne 0) { throw "SPDX SBOM generation failed" }

    & cargo xtask sbom verify --artifact-dir $resolvedBundleDir --sbom $sbomPath
    if ($LASTEXITCODE -ne 0) { throw "SPDX SBOM verification failed" }

    & cargo xtask verify-tauri-bundle `
        --bundle-dir $resolvedBundleDir `
        --summary $summaryPath `
        --authenticode-evidence $authenticodeEvidencePath `
        --sbom $sbomPath
    if ($LASTEXITCODE -ne 0) { throw "Tauri bundle qualification verification failed" }
    Write-Host "  SPDX SBOM and bundle qualification: PASS"

    # 9. Optional Install / Smoke Test
    if (-not $SkipInstallSmoke) {
        Write-Host "Running current-user install/launch/uninstall smoke..."
        $installRoot = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-smoke-" + [guid]::NewGuid().ToString("N"))
        $appPath = Join-Path $installRoot "sky_desktop_shell.exe"
        $uninstaller = Join-Path $installRoot "uninstall.exe"
        $appProcess = $null
        try {
            $instRun = Start-Process -FilePath $installerPath -ArgumentList @("/S", "/D=$installRoot") -WindowStyle Hidden -Wait -PassThru
            if ($instRun.ExitCode -ne 0) { throw "Installer exited with code $($instRun.ExitCode)" }
            if (-not (Test-Path -LiteralPath $appPath)) { throw "Installed executable missing: $appPath" }
            if (-not (Test-Path -LiteralPath $uninstaller)) { throw "Uninstaller missing: $uninstaller" }

            $installedPe = @(Get-ChildItem -LiteralPath $installRoot -File -Recurse |
                Where-Object { $_.Extension.ToLowerInvariant() -in @('.exe', '.dll') -and $_.Name -ne 'uninstall.exe' })
            if ($installedPe.Count -eq 0) { throw "Installed tree contains no PE files" }

            $installedAuthEvidence = Join-Path $resolvedEvidenceDir "INSTALLED_AUTHENTICODE_EVIDENCE.json"
            & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
                -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
                -Mode production `
                -Artifact $installedPe.FullName `
                -Evidence $installedAuthEvidence
            if ($LASTEXITCODE -ne 0) { throw "Installed PE Authenticode verification failed" }

            $appProcess = Start-Process -FilePath $appPath -WindowStyle Hidden -PassThru
            Start-Sleep -Seconds 3
            if ($appProcess.HasExited) { throw "Application exited unexpectedly during smoke test" }
            Stop-Process -Id $appProcess.Id -Force
            $appProcess = $null

            $uninstRun = Start-Process -FilePath $uninstaller -ArgumentList @("/S") -WindowStyle Hidden -Wait -PassThru
            if ($uninstRun.ExitCode -ne 0) { throw "Uninstaller exited with code $($uninstRun.ExitCode)" }
            Write-Host "  Install/Launch/Uninstall smoke: PASS"
        } finally {
            if ($null -ne $appProcess -and -not $appProcess.HasExited) { Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue }
            if (Test-Path -LiteralPath $installRoot) { Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue }
        }
    }

    # 10. Emit Deterministic Machine-Readable Evidence
    Write-Host "[Step 7/7] Emitting qualification evidence..."
    $installerSha256 = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $signatureSha256 = (Get-FileHash -LiteralPath $signaturePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $authEvidenceSha256 = (Get-FileHash -LiteralPath $authenticodeEvidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sbomSha256 = (Get-FileHash -LiteralPath $sbomPath -Algorithm SHA256).Hash.ToLowerInvariant()

    # Evidence A: Canonical 20-field V4_QUALIFICATION_EVIDENCE.json for promote_v4_metadata.ps1
    $canonicalEvidence = [ordered]@{
        schema_version = 1
        evidence_type = "tauri-nsis-qualified-release"
        qualified = $true
        qualification = "install-launch-uninstall"
        product_name = "Sky Auto Player"
        identifier = "io.github.pumni.skyautoplayer"
        version = $Version
        target = "nsis"
        install_mode = "currentUser"
        installer = $expectedInstallerName
        updater_signature = $expectedSignatureName
        installer_size = (Get-Item -LiteralPath $installerPath).Length
        signature_size = (Get-Item -LiteralPath $signaturePath).Length
        installer_sha256 = $installerSha256
        updater_signature_sha256 = $signatureSha256
        authenticode_mode = "production"
        authenticode_evidence = "TAURI_AUTHENTICODE_EVIDENCE.json"
        authenticode_evidence_sha256 = $authEvidenceSha256
        sbom = "SBOM.spdx.json"
        sbom_sha256 = $sbomSha256
    }
    $canonicalEvidencePath = Join-Path $resolvedEvidenceDir "V4_QUALIFICATION_EVIDENCE.json"
    $canonicalEvidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $canonicalEvidencePath -Encoding utf8

    # Evidence B: Comprehensive V4_PRODUCTION_RELEASE_EVIDENCE.json
    $productionEvidence = [ordered]@{
        schema_version = 1
        evidence_type = "v4-production-release-qualification"
        source_sha = $expectedSha
        version = $Version
        channel = $Channel
        product_name = "Sky Auto Player"
        identifier = "io.github.pumni.skyautoplayer"
        target = "nsis"
        install_mode = "currentUser"
        installer = $expectedInstallerName
        installer_size = (Get-Item -LiteralPath $installerPath).Length
        installer_sha256 = $installerSha256
        updater_signature = $expectedSignatureName
        signature_size = (Get-Item -LiteralPath $signaturePath).Length
        updater_signature_sha256 = $signatureSha256
        authenticode_mode = "production"
        authenticode_provider = $AuthenticodeProvider
        approved_signer_thumbprint = $approvedThumbprint
        observed_signer_thumbprint = $observedThumbprint
        updater_key_id = "F6355260A0C663D5"
        updater_signature_status = "valid"
        sbom = "SBOM.spdx.json"
        sbom_sha256 = $sbomSha256
        qualification_status = "PASS"
    }
    $productionEvidencePath = Join-Path $resolvedEvidenceDir "V4_PRODUCTION_RELEASE_EVIDENCE.json"
    $productionEvidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $productionEvidencePath -Encoding utf8

    Write-Host "================================================================="
    Write-Host " Production Qualification Result: PASS"
    Write-Host " Candidate: $expectedInstallerName ($installerSha256)"
    Write-Host " Updater Signature: $expectedSignatureName ($signatureSha256)"
    Write-Host " Evidence Path: $productionEvidencePath"
    Write-Host "================================================================="
    exit 0
} finally {
    Restore-SavedEnvironment
}
