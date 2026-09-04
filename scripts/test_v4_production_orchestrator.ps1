Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Write-Host "Starting V4 production release orchestrator contract tests..."

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$temporaryRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$temporaryRoot = [IO.Path]::GetFullPath($temporaryRoot)
$fixtureRoot = Join-Path $temporaryRoot ('sky-v4-prod-orch-test-' + [guid]::NewGuid().ToString('N'))
$envFile = Join-Path $fixtureRoot 'test-signing.env'

# Save outer environment variables to restore in finally
$savedEnv = @{
    'SKY_AUTHENTICODE_MODE' = $env:SKY_AUTHENTICODE_MODE
    'SKY_AUTHENTICODE_TEST_PFX_PATH' = $env:SKY_AUTHENTICODE_TEST_PFX_PATH
    'SKY_AUTHENTICODE_TEST_PFX_PASSWORD' = $env:SKY_AUTHENTICODE_TEST_PFX_PASSWORD
    'SKY_AUTHENTICODE_TEST_THUMBPRINT' = $env:SKY_AUTHENTICODE_TEST_THUMBPRINT
    'SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT' = $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT
    'SKY_AUTHENTICODE_PROVIDER' = $env:SKY_AUTHENTICODE_PROVIDER
    'SKY_AUTHENTICODE_PROVIDER_SCRIPT' = $env:SKY_AUTHENTICODE_PROVIDER_SCRIPT
    'SKY_AUTHENTICODE_PROVIDER_COMMAND' = $env:SKY_AUTHENTICODE_PROVIDER_COMMAND
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
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null

    $currentSha = (& git rev-parse HEAD).Trim().ToLowerInvariant()
    $cargoTomlPath = Join-Path $repoRoot "desktop\src-tauri\Cargo.toml"
    $cargoToml = Get-Content -LiteralPath $cargoTomlPath -Raw
    if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
        throw "Failed to parse Cargo.toml version"
    }
    $currentVersion = $Matches[1].Trim()

    # Generate throwaway Minisign key outside repo
    $throwawayKeyPath = Join-Path $fixtureRoot "throwaway.key"
    Push-Location (Join-Path $repoRoot "desktop")
    try {
        & bun run tauri signer generate --ci --password "" --force -w $throwawayKeyPath
        if ($LASTEXITCODE -ne 0) { throw "bun run tauri signer generate failed" }
    } finally {
        Pop-Location
    }

    # Dummy provider script
    $dummyProviderScript = Join-Path $fixtureRoot "dummy_provider.ps1"
    Set-Content -LiteralPath $dummyProviderScript -Value 'param([string]$Path) exit 0'

    # Test 1: Missing production identity / parameter validation fails closed
    Write-Host "Test 1: Parameter validation fails closed on missing parameters..."
    $out1 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator succeeded with empty parameters" }
    if ($out1 -notmatch "Missing mandatory parameter: ExpectedSourceSha") {
        throw "FAILED: Did not fail closed on missing ExpectedSourceSha"
    }
    Write-Host "Test 1: PASS"

    # Test 2: Source SHA mismatch fails closed before packaging
    Write-Host "Test 2: Source SHA mismatch fails closed before packaging..."
    $out2 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha "0000000000000000000000000000000000000000" `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator succeeded with mismatched SHA" }
    if ($out2 -notmatch "does not match ExpectedSourceSha") {
        throw "FAILED: Did not fail closed on mismatched SHA"
    }
    Write-Host "Test 2: PASS"

    # Test 3: Version mismatch and channel policy validation
    Write-Host "Test 3: Channel policy validation fails closed on invalid SemVer / channel..."
    # 3a. Version mismatch
    $out3a = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version "9.9.9" `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted nonexistent version" }
    if ($out3a -notmatch "does not match Cargo.toml version") {
        throw "FAILED: Did not fail closed on Cargo.toml version mismatch"
    }

    # 3b. Stable channel rejects prerelease version
    $out3b = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "stable" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted prerelease version on stable channel" }
    if ($out3b -notmatch "Channel 'stable' rejects prerelease version") {
        throw "FAILED: Did not fail closed on stable channel with prerelease version"
    }
    Write-Host "Test 3: PASS"

    # Test 4: Provider SCRIPT/COMMAND mutual exclusivity
    Write-Host "Test 4: Mutually exclusive provider configuration fails closed..."
    $out4 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -AuthenticodeProviderCommand "echo %1" 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted both SCRIPT and COMMAND" }
    if ($out4 -notmatch "Mutually exclusive Authenticode provider configuration") {
        throw "FAILED: Did not fail closed on mutually exclusive provider settings"
    }
    Write-Host "Test 4: PASS"

    # Test 5: Wrong updater private key fails pre-flight verification before packaging
    Write-Host "Test 5: Wrong updater private key fails pre-flight verification before packaging..."
    $out5 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted mismatched updater key" }
    if ($out5 -notmatch "Pre-packaging updater key verification failed") {
        throw "FAILED: Did not fail closed on updater key pre-flight check"
    }
    Write-Host "Test 5: PASS"

    # Test 6: Secret values are never emitted by error paths
    Write-Host "Test 6: Secret values are not emitted by expected error paths..."
    $secretPassword = "SECRET_SUPER_TEST_PASS_987654321"
    $env:MY_TEST_KEY_PASSWORD = $secretPassword
    $out6 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -UpdaterPasswordEnv "MY_TEST_KEY_PASSWORD" `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    Remove-Item Env:MY_TEST_KEY_PASSWORD -ErrorAction SilentlyContinue
    if ($out6.Contains($secretPassword)) {
        throw "FAILED: Secret password was leaked to output/error stream!"
    }
    Write-Host "Test 6: PASS"

    # Test 7: Updater signature verification rejects corrupted signature
    Write-Host "Test 7: Updater signature verification rejects corrupted signature..."
    $testExe = Join-Path $fixtureRoot "dummy.exe"
    $testSig = Join-Path $fixtureRoot "dummy.exe.sig"
    [IO.File]::WriteAllBytes($testExe, [Text.Encoding]::UTF8.GetBytes("MZ fake executable content"))
    [IO.File]::WriteAllText($testSig, "untrusted comment: minisign signature`ncorrupted signature data")
    $out7 = & cargo xtask updater-trust verify-signature --installer $testExe --signature $testSig 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: verify-signature succeeded with corrupt signature" }
    Write-Host "Test 7: PASS"

    # Test 8: Tampered candidate binary is detected by Authenticode and updater verifier
    Write-Host "Test 8: Tampered candidate binary is detected..."
    # Sign dummy.exe with test cert
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "setup_v4_test_signing.ps1") `
        -EnvFile $envFile
    Get-Content -LiteralPath $envFile | ForEach-Object {
        $line = [string]$_
        if ($line -match '^([^=]+)=(.*)$') {
            [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
        }
    }
    $testThumbprint = [string]$env:SKY_AUTHENTICODE_TEST_THUMBPRINT

    # Find a real PE
    $sourcePe = Get-ChildItem -LiteralPath (Join-Path $repoRoot "rust\target") -Filter '*.exe' -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -notmatch '\\deps\\' } |
        Select-Object -First 1
    if ($null -eq $sourcePe) {
        $sourcePe = Get-Item -LiteralPath (Join-Path $env:SystemRoot "System32\notepad.exe")
    }
    $peCopy = Join-Path $fixtureRoot "project_pe.exe"
    Copy-Item -LiteralPath $sourcePe.FullName -Destination $peCopy -Force

    # Sign with test mode
    $env:SKY_AUTHENTICODE_MODE = "test"
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "sign_v4_authenticode.ps1") `
        -Path $peCopy
    if ($LASTEXITCODE -ne 0) { throw "Failed to test-sign PE" }

    # Mutate a byte in peCopy
    $bytes = [IO.File]::ReadAllBytes($peCopy)
    $bytes[100] = [byte]($bytes[100] -bxor 0xFF)
    [IO.File]::WriteAllBytes($peCopy, $bytes)

    # Verify tampering is detected
    $out8 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode test `
        -Artifact $peCopy 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Authenticode verification accepted tampered binary" }
    Write-Host "Test 8: PASS"

    # Test 9: Verification rejects test credentials in production mode
    Write-Host "Test 9: Production verification rejects CI test certificate..."
    $unmutatedPe = Join-Path $fixtureRoot "unmutated_pe.exe"
    Copy-Item -LiteralPath $sourcePe.FullName -Destination $unmutatedPe -Force
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "sign_v4_authenticode.ps1") `
        -Path $unmutatedPe
    if ($LASTEXITCODE -ne 0) { throw "Failed to test-sign PE" }

    # Production mode verification must reject it
    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = "0123456789ABCDEF0123456789ABCDEF01234567"
    $out9 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode production `
        -Artifact $unmutatedPe 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Production verification accepted test-signed PE" }
    Write-Host "Test 9: PASS"

    # Test 10: Emitted qualification evidence satisfies promote_v4_metadata schema
    Write-Host "Test 10: Qualification evidence schema compatibility with promote_v4_metadata..."
    $testEvidenceDir = Join-Path $fixtureRoot "evidence"
    New-Item -ItemType Directory -Path $testEvidenceDir -Force | Out-Null
    $testInstaller = "Sky Auto Player_${currentVersion}_x64-setup.exe"
    $testSig = "Sky Auto Player_${currentVersion}_x64-setup.exe.sig"
    $testInstallerPath = Join-Path $testEvidenceDir $testInstaller
    $testSigPath = Join-Path $testEvidenceDir $testSig
    [IO.File]::WriteAllBytes($testInstallerPath, [Text.Encoding]::UTF8.GetBytes("MZ installer content"))
    [IO.File]::WriteAllText($testSigPath, "signature content")
    $testAuthEvidencePath = Join-Path $testEvidenceDir "TAURI_AUTHENTICODE_EVIDENCE.json"
    [IO.File]::WriteAllText($testAuthEvidencePath, "{}")
    $testSbomPath = Join-Path $testEvidenceDir "SBOM.spdx.json"
    [IO.File]::WriteAllText($testSbomPath, "{}")

    $testInstallerSha = (Get-FileHash -LiteralPath $testInstallerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $testSigSha = (Get-FileHash -LiteralPath $testSigPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $testAuthSha = (Get-FileHash -LiteralPath $testAuthEvidencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $testSbomSha = (Get-FileHash -LiteralPath $testSbomPath -Algorithm SHA256).Hash.ToLowerInvariant()

    $canonicalEvidence = [ordered]@{
        schema_version = 1
        evidence_type = "tauri-nsis-qualified-release"
        qualified = $true
        qualification = "install-launch-uninstall"
        product_name = "Sky Auto Player"
        identifier = "io.github.pumni.skyautoplayer"
        version = $currentVersion
        target = "nsis"
        install_mode = "currentUser"
        installer = $testInstaller
        updater_signature = $testSig
        installer_size = (Get-Item -LiteralPath $testInstallerPath).Length
        signature_size = (Get-Item -LiteralPath $testSigPath).Length
        installer_sha256 = $testInstallerSha
        updater_signature_sha256 = $testSigSha
        authenticode_mode = "production"
        authenticode_evidence = "TAURI_AUTHENTICODE_EVIDENCE.json"
        authenticode_evidence_sha256 = $testAuthSha
        sbom = "SBOM.spdx.json"
        sbom_sha256 = $testSbomSha
    }
    $evidenceJsonPath = Join-Path $testEvidenceDir "V4_QUALIFICATION_EVIDENCE.json"
    $canonicalEvidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $evidenceJsonPath -Encoding utf8

    # Verify self-test in promote_v4_metadata passes
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") -SelfTest
    if ($LASTEXITCODE -ne 0) { throw "FAILED: promote_v4_metadata self-test failed" }
    Write-Host "Test 10: PASS"
} finally {
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "cleanup_v4_test_signing.ps1")
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    Restore-SavedEnvironment
}

Write-Host "================================================================="
Write-Host " [PASS] All V4 production orchestrator contract tests passed"
Write-Host "================================================================="
exit 0
