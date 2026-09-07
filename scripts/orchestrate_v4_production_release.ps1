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

    # Internal fixture seam for isolated test qualification only.
    # Structurally prohibited from emitting production / promotable evidence.
    [switch]$InternalTestFixture,
    [string]$InternalFixtureCandidatePath,
    [switch]$InternalSkipSmoke
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
# 2. Reject internal test parameters unless -InternalTestFixture is explicitly specified
if (-not $InternalTestFixture) {
    if (-not [string]::IsNullOrWhiteSpace($InternalFixtureCandidatePath)) {
        throw "Invalid parameter: InternalFixtureCandidatePath is only permitted when -InternalTestFixture is specified"
    }
    if ($InternalSkipSmoke) {
        throw "Invalid parameter: InternalSkipSmoke is only permitted when -InternalTestFixture is specified"
    }
}

# Environment preservation
$savedEnv = @{
    'SKY_AUTHENTICODE_MODE' = $env:SKY_AUTHENTICODE_MODE
    'SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT' = $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT
    'SKY_AUTHENTICODE_PROVIDER' = $env:SKY_AUTHENTICODE_PROVIDER
    'SKY_AUTHENTICODE_PROVIDER_SCRIPT' = $env:SKY_AUTHENTICODE_PROVIDER_SCRIPT
    'SKY_AUTHENTICODE_PROVIDER_COMMAND' = $env:SKY_AUTHENTICODE_PROVIDER_COMMAND
    'SKY_AUTHENTICODE_TEST_PFX_PATH' = $env:SKY_AUTHENTICODE_TEST_PFX_PATH
    'SKY_AUTHENTICODE_TEST_PFX_PASSWORD' = $env:SKY_AUTHENTICODE_TEST_PFX_PASSWORD
    'SKY_AUTHENTICODE_TEST_THUMBPRINT' = $env:SKY_AUTHENTICODE_TEST_THUMBPRINT
    'TAURI_SIGNING_PRIVATE_KEY' = $env:TAURI_SIGNING_PRIVATE_KEY
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

function Get-CanonicalUpdaterKeyId {
    $tauriConfPath = Join-Path $repoRoot "desktop\src-tauri\tauri.conf.json"
    $tauriConf = Get-Content -LiteralPath $tauriConfPath -Raw | ConvertFrom-Json
    $pubKeyB64 = [string]$tauriConf.plugins.updater.pubkey
    $decodedBytes = [System.Convert]::FromBase64String($pubKeyB64)
    $decodedText = [System.Text.Encoding]::UTF8.GetString($decodedBytes)
    if ($decodedText -match '(?m)^untrusted comment: minisign public key: ([0-9A-Fa-f]{16})') {
        return $Matches[1].ToUpperInvariant()
    }
    throw "Failed to extract Key ID from canonical public key in tauri.conf.json"
}

function Invoke-PrePackagingStaleOutputPurge {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BundleDirectory,
        [Parameter(Mandatory = $true)]
        [string]$EvidenceDirectory,
        [Parameter(Mandatory = $true)]
        [string]$InstallerName,
        [Parameter(Mandatory = $true)]
        [string]$SignatureName
    )

    $staleTargets = @(
        (Join-Path $BundleDirectory $InstallerName),
        (Join-Path $BundleDirectory $SignatureName),
        (Join-Path $EvidenceDirectory "V4_QUALIFICATION_EVIDENCE.json"),
        (Join-Path $EvidenceDirectory "V4_PRODUCTION_RELEASE_EVIDENCE.json")
    )

    foreach ($target in $staleTargets) {
        if (Test-Path -LiteralPath $target) {
            Remove-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $target) {
                throw "Hygiene failure: Failed to purge stale output file before packaging: $target"
            }
        }
    }

    Write-Host "[Pre-Packaging Purge] Stale candidate artifacts and evidence successfully purged: PASS"
}

. (Join-Path $PSScriptRoot "v4_qualification_evidence.ps1")

$expectedInstallerName = "Sky Auto Player_${Version}_x64-setup.exe"
$expectedSignatureName = "Sky Auto Player_${Version}_x64-setup.exe.sig"
$resolvedBundleDir = $null
$resolvedEvidenceDir = $null

try {
    Write-Host "================================================================="
    Write-Host " Sky Auto Player V4 - Production Release Orchestrator"
    Write-Host "================================================================="

    # 3. Validate input parameters (identities, clean worktree, references)
    # Reject inherited non-empty TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH
    # Release runner environment must not contain pre-set signing authorities
    if (-not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
        throw "Security violation: Pre-existing TAURI_SIGNING_PRIVATE_KEY detected in environment. Production release requires an unpolluted runner environment."
    }
    if (-not [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
        throw "Security violation: Pre-existing TAURI_SIGNING_PRIVATE_KEY_PATH detected in environment. Production release requires an unpolluted runner environment."
    }

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

    # Provider inputs are optional under the project policy. Preserve the
    # existing malformed-input guard for callers carrying both future-signer
    # forms, but never require either form for current production.
    $hasScript = -not [string]::IsNullOrWhiteSpace($AuthenticodeProviderScript)
    $hasCommand = -not [string]::IsNullOrWhiteSpace($AuthenticodeProviderCommand)
    if ($hasScript -and $hasCommand) {
        throw "Mutually exclusive Authenticode provider configuration: specify either AuthenticodeProviderScript or AuthenticodeProviderCommand, not both"
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
    New-Item -ItemType Directory -Path $resolvedBundleDir -Force | Out-Null

    # Workspace hygiene: purge stale candidate binaries and evidence from previous runs
    # A stale ignored candidate or evidence must never be accepted as the output of this run
    if (-not $InternalTestFixture) {
        Invoke-PrePackagingStaleOutputPurge `
            -BundleDirectory $resolvedBundleDir `
            -EvidenceDirectory $resolvedEvidenceDir `
            -InstallerName $expectedInstallerName `
            -SignatureName $expectedSignatureName
    }

    $canonicalKeyId = Get-CanonicalUpdaterKeyId

    # Fail closed on dirty worktree before any updater-key verification, provider invocation, build or signing
    if (-not $InternalTestFixture) {
        $porcelainOutput = (& git status --porcelain 2>&1)
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to check git working tree status"
        }
        $dirtyEntries = @($porcelainOutput | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        if ($dirtyEntries.Count -gt 0) {
            throw "Working tree is dirty; production release requires a clean working tree matching commit $expectedSha. Dirty entries:`n$($dirtyEntries -join "`n")"
        }
    }

    # 4. Pre-packaging Updater Key Validation (Fail closed before build)
    if (-not $InternalTestFixture) {
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
        Write-Host "  Updater private key matches canonical root $($canonicalKeyId): PASS"
    } else {
        Write-Host "[Step 1/7] Internal test fixture mode: bypassing production root key pre-check..."
    }

    # 5. Canonical Single Build under the governed unsigned-zero-budget policy
    if (-not $InternalTestFixture) {
        Write-Host "[Step 2/7] Building canonical Tauri production artifact (build-once)..."

        # Tauri NSIS invokes scripts/sign_v4_authenticode.ps1 via the configured
        # signCommand seam. The project release policy deliberately performs no
        # Authenticode signing and requires no certificate/provider/thumbprint.
        $env:SKY_AUTHENTICODE_MODE = "unsigned-zero-budget"
        Remove-Item Env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT -ErrorAction SilentlyContinue
        Remove-Item Env:SKY_AUTHENTICODE_PROVIDER -ErrorAction SilentlyContinue
        Remove-Item Env:SKY_AUTHENTICODE_PROVIDER_SCRIPT -ErrorAction SilentlyContinue
        Remove-Item Env:SKY_AUTHENTICODE_PROVIDER_COMMAND -ErrorAction SilentlyContinue
        # Tauri v2 bundler accepts a file path in TAURI_SIGNING_PRIVATE_KEY for signing.
        # Never place raw private key contents into environment variables.
        $env:TAURI_SIGNING_PRIVATE_KEY = $resolvedKeyPath
        $env:TAURI_SIGNING_PRIVATE_KEY_PATH = $resolvedKeyPath
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $passwordValue

        $cargoManifestPath = Join-Path $repoRoot "desktop\src-tauri\Cargo.toml"
        $preBuildCargoHash = (Get-FileHash -LiteralPath $cargoManifestPath -Algorithm SHA256).Hash

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

        # Re-verify clean worktree post-build to ensure build tools did not mutate tracked source/manifests
        $postBuildPorcelain = (& git status --porcelain 2>&1)
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to check git working tree status after build"
        }
        $postBuildDirtyEntries = @($postBuildPorcelain | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        if ($postBuildDirtyEntries.Count -gt 0) {
            $postBuildCargoHash = (Get-FileHash -LiteralPath $cargoManifestPath -Algorithm SHA256).Hash
            Write-Host "Pre-build Cargo.toml SHA256:  $preBuildCargoHash"
            Write-Host "Post-build Cargo.toml SHA256: $postBuildCargoHash"
            Write-Host "=== git status --porcelain ==="
            & git status --porcelain
            Write-Host "=== git diff --no-ext-diff -- desktop/src-tauri/Cargo.toml ==="
            & git diff --no-ext-diff -- $cargoManifestPath
            Write-Host "=== git diff --check -- desktop/src-tauri/Cargo.toml ==="
            & git diff --check -- $cargoManifestPath
            throw "Working tree became dirty during production build; candidate was not produced from an intact commit $expectedSha. Dirty entries:`n$($postBuildDirtyEntries -join "`n")"
        }
    } else {
        Write-Host "[Step 2/7] Internal test fixture: using provided fixture candidate bytes..."
        if (-not [string]::IsNullOrWhiteSpace($InternalFixtureCandidatePath)) {
            $fixturePe = Resolve-Path -LiteralPath $InternalFixtureCandidatePath -ErrorAction Stop
            $destPe = Join-Path $resolvedBundleDir $expectedInstallerName
            Copy-Item -LiteralPath $fixturePe.Path -Destination $destPe -Force
            # Also copy or create dummy .sig if present
            $fixtureSig = "$($fixturePe.Path).sig"
            $destSig = Join-Path $resolvedBundleDir $expectedSignatureName
            if (Test-Path -LiteralPath $fixtureSig) {
                Copy-Item -LiteralPath $fixtureSig -Destination $destSig -Force
            } else {
                Set-Content -LiteralPath $destSig -Value "untrusted comment: test fixture signature`nAAAA"
            }
        }
    }

    # 6. Exact Artifact Verification
    Write-Host "[Step 3/7] Verifying canonical NSIS artifact set in $resolvedBundleDir..."
    if (-not (Test-Path -LiteralPath $resolvedBundleDir -PathType Container)) {
        throw "Bundle directory does not exist: $resolvedBundleDir"
    }
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

    # 7. Production Authenticode-State Verification
    $authenticodeMode = if ($InternalTestFixture) { "test" } else { "unsigned-zero-budget" }
    Write-Host "[Step 4/7] Verifying Authenticode $authenticodeMode signature..."
    $authenticodeEvidencePath = Join-Path $resolvedEvidenceDir "TAURI_AUTHENTICODE_EVIDENCE.json"
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode $authenticodeMode `
        -Artifact $installerPath `
        -Evidence $authenticodeEvidencePath
    if ($LASTEXITCODE -ne 0) {
        throw "Authenticode verification failed on $installerPath (mode: $authenticodeMode)"
    }
    $authEvidence = Get-Content -LiteralPath $authenticodeEvidencePath -Raw | ConvertFrom-Json
    $observedThumbprint = $authEvidence.files[0].signer_thumbprint
    if ($authenticodeMode -eq "unsigned-zero-budget" -and $null -ne $observedThumbprint) {
        throw "unsigned-zero-budget Authenticode evidence unexpectedly contains a signer identity"
    }
    $authenticodeState = if ($authenticodeMode -eq "unsigned-zero-budget") { "unsigned" } else { "test-signed" }
    Write-Host "  Authenticode verification: PASS (State: $authenticodeState, Mode: $authenticodeMode)"

    # 8. Tauri Updater Signature Verification against Canonical Root
    if (-not $InternalTestFixture) {
        Write-Host "[Step 5/7] Cryptographically verifying updater signature against canonical public root..."
        & cargo xtask updater-trust verify-signature `
            --installer $installerPath `
            --signature $signaturePath
        if ($LASTEXITCODE -ne 0) {
            throw "Updater signature cryptographic verification failed against canonical public root"
        }
        Write-Host "  Updater signature verification: PASS"
    } else {
        Write-Host "[Step 5/7] Internal test fixture: bypassing canonical root signature verification..."
    }

    # 9. SPDX SBOM Generation and Bundle Verification
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

    # 10. Install / Smoke Test (Canonical production ALWAYS runs smoke; only internal fixture can skip)
    $smokeRanAndPassed = $false
    if (-not $InternalSkipSmoke) {
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
                -Mode $authenticodeMode `
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
            $smokeRanAndPassed = $true
            Write-Host "  Install/Launch/Uninstall smoke: PASS"
        } finally {
            if ($null -ne $appProcess -and -not $appProcess.HasExited) { Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue }
            if (Test-Path -LiteralPath $installRoot) { Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue }
        }
    } else {
        Write-Host "  Install/Launch/Uninstall smoke: SKIPPED (internal test fixture only)"
    }

    # 11. Emit Deterministic Machine-Readable Evidence
    Write-Host "[Step 7/7] Emitting qualification evidence..."
    $installerSha256 = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $signatureSha256 = (Get-FileHash -LiteralPath $signaturePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $authEvidenceSha256 = (Get-FileHash -LiteralPath $authenticodeEvidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sbomSha256 = (Get-FileHash -LiteralPath $sbomPath -Algorithm SHA256).Hash.ToLowerInvariant()

    if ($InternalTestFixture) {
        # Internal test fixture: emit non-promotable fixture evidence
        $fixtureEvidence = [ordered]@{
            schema_version = 1
            evidence_type = "test-fixture-non-promotable"
            qualified = ($smokeRanAndPassed -and -not $InternalSkipSmoke)
            qualification = if ($smokeRanAndPassed) { "install-launch-uninstall" } else { "none-skipped" }
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
            authenticode_mode = "test"
            authenticode_evidence = "TAURI_AUTHENTICODE_EVIDENCE.json"
            authenticode_evidence_sha256 = $authEvidenceSha256
            sbom = "SBOM.spdx.json"
            sbom_sha256 = $sbomSha256
        }
        $fixtureEvidencePath = Join-Path $resolvedEvidenceDir "V4_FIXTURE_EVIDENCE.json"
        $fixtureEvidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $fixtureEvidencePath -Encoding utf8
        Write-Host "Emitted non-promotable fixture evidence: $fixtureEvidencePath"
    } else {
        # Canonical 20-field V4_QUALIFICATION_EVIDENCE.json via shared builder contract
        $canonicalEvidence = New-V4CanonicalQualificationEvidence `
            -Version $Version `
            -InstallerName $expectedInstallerName `
            -SignatureName $expectedSignatureName `
            -InstallerSize (Get-Item -LiteralPath $installerPath).Length `
            -SignatureSize (Get-Item -LiteralPath $signaturePath).Length `
            -InstallerSha256 $installerSha256 `
            -SignatureSha256 $signatureSha256 `
            -AuthenticodeEvidenceSha256 $authEvidenceSha256 `
            -SbomSha256 $sbomSha256
        $canonicalEvidencePath = Join-Path $resolvedEvidenceDir "V4_QUALIFICATION_EVIDENCE.json"
        $canonicalEvidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $canonicalEvidencePath -Encoding utf8

        # Validate emitted evidence against promote_v4_metadata schema
        & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
            -ValidateEvidence $canonicalEvidencePath
        if ($LASTEXITCODE -ne 0) {
            throw "Emitted qualification evidence failed promote_v4_metadata validation"
        }

        # Comprehensive V4_PRODUCTION_RELEASE_EVIDENCE.json via shared builder contract
        $productionEvidence = New-V4CanonicalProductionEvidence `
            -SourceSha $expectedSha `
            -Version $Version `
            -Channel $Channel `
            -InstallerName $expectedInstallerName `
            -SignatureName $expectedSignatureName `
            -InstallerSize (Get-Item -LiteralPath $installerPath).Length `
            -SignatureSize (Get-Item -LiteralPath $signaturePath).Length `
            -InstallerSha256 $installerSha256 `
            -SignatureSha256 $signatureSha256 `
            -AuthenticodeEvidenceSha256 $authEvidenceSha256 `
            -SbomSha256 $sbomSha256 `
            -UpdaterKeyId $canonicalKeyId `
            -ObservedSignerThumbprint $observedThumbprint
        $productionEvidencePath = Join-Path $resolvedEvidenceDir "V4_PRODUCTION_RELEASE_EVIDENCE.json"
        $productionEvidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $productionEvidencePath -Encoding utf8

        Write-Host "================================================================="
        Write-Host " Production Qualification Result: PASS"
        Write-Host " Candidate: $expectedInstallerName ($installerSha256)"
        Write-Host " Updater Signature: $expectedSignatureName ($signatureSha256)"
        Write-Host " Evidence Path: $productionEvidencePath"
        Write-Host "================================================================="
    }
    exit 0
} catch {
    $err = $_
    Write-Host "Orchestration failure encountered: $($err.Exception.Message)"
    # Purge partial candidate artifacts and unpromoted evidence on failure
    if (-not [string]::IsNullOrWhiteSpace($resolvedBundleDir) -and (Test-Path -LiteralPath $resolvedBundleDir)) {
        Remove-Item -LiteralPath (Join-Path $resolvedBundleDir $expectedInstallerName) -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath (Join-Path $resolvedBundleDir $expectedSignatureName) -Force -ErrorAction SilentlyContinue
    }
    if (-not [string]::IsNullOrWhiteSpace($resolvedEvidenceDir) -and (Test-Path -LiteralPath $resolvedEvidenceDir)) {
        Remove-Item -LiteralPath (Join-Path $resolvedEvidenceDir "V4_QUALIFICATION_EVIDENCE.json") -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath (Join-Path $resolvedEvidenceDir "V4_PRODUCTION_RELEASE_EVIDENCE.json") -Force -ErrorAction SilentlyContinue
    }
    throw $err
} finally {
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PATH -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
    Restore-SavedEnvironment
}
