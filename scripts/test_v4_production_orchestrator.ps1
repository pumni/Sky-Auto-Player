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
    $initialDirty = @(& git status --porcelain | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($initialDirty.Count -gt 0) {
        throw "Cannot run orchestrator contract tests on dirty worktree. Dirty entries: $($initialDirty -join ', ')"
    }
    $cargoTomlPath = Join-Path $repoRoot "desktop\src-tauri\Cargo.toml"
    $cargoToml = Get-Content -LiteralPath $cargoTomlPath -Raw
    if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
        throw "Failed to parse Cargo.toml version"
    }
    $currentVersion = $Matches[1].Trim()

    . (Join-Path $PSScriptRoot "v4_qualification_evidence.ps1")

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

    # Test 3: Dirty git worktree fails closed before build or signing
    Write-Host "Test 3: Dirty git worktree fails closed before build/signing..."
    $trackedFilePath = Join-Path $repoRoot "README.md"
    $originalReadme = Get-Content -LiteralPath $trackedFilePath -Raw
    try {
        Add-Content -LiteralPath $trackedFilePath -Value "`n<!-- dirty worktree test marker -->"
        $statusCheck = & git status --porcelain
        if ([string]::IsNullOrWhiteSpace($statusCheck)) {
            throw "Failed to create dirty worktree fixture in README.md"
        }

        $out3 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
            -ExpectedSourceSha $currentSha `
            -Version $currentVersion `
            -Channel "beta" `
            -UpdaterPrivateKeyPath $throwawayKeyPath `
            -AuthenticodeProvider "custom" `
            -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
            -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator succeeded on dirty worktree" }
        if ($out3 -notmatch "Working tree is dirty; production release requires a clean working tree") {
            throw "FAILED: Did not fail closed with clean worktree error on dirty tree"
        }
    } finally {
        Set-Content -LiteralPath $trackedFilePath -Value $originalReadme -NoNewline
        & git checkout -- $trackedFilePath
    }
    $statusAfterClean = & git status --porcelain $trackedFilePath
    if (-not [string]::IsNullOrWhiteSpace($statusAfterClean)) {
        throw "FAILED: $trackedFilePath was not restored cleanly after Test 3"
    }
    Write-Host "Test 3: PASS"

    # Test 4: Version mismatch and channel policy validation
    Write-Host "Test 4: Channel policy validation fails closed on invalid SemVer / channel..."
    # 4a. Version mismatch
    $out4a = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version "9.9.9" `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted nonexistent version" }
    if ($out4a -notmatch "does not match Cargo.toml version") {
        throw "FAILED: Did not fail closed on Cargo.toml version mismatch"
    }

    # 4b. Stable channel rejects prerelease version
    $out4b = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "stable" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted prerelease version on stable channel" }
    if ($out4b -notmatch "Channel 'stable' rejects prerelease version") {
        throw "FAILED: Did not fail closed on stable channel with prerelease version"
    }
    Write-Host "Test 4: PASS"

    # Test 5: Provider SCRIPT/COMMAND mutual exclusivity
    Write-Host "Test 5: Mutually exclusive provider configuration fails closed..."
    $out5 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
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
    if ($out5 -notmatch "Mutually exclusive Authenticode provider configuration") {
        throw "FAILED: Did not fail closed on mutually exclusive provider settings"
    }
    Write-Host "Test 5: PASS"

    # Test 6: Wrong updater private key fails pre-flight verification before packaging
    Write-Host "Test 6: Wrong updater private key fails pre-flight verification before packaging..."
    $out6 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted mismatched updater key" }
    if ($out6 -notmatch "Pre-packaging updater key verification failed") {
        throw "FAILED: Did not fail closed on updater key pre-flight check. Actual output:`n$out6"
    }
    Write-Host "Test 6: PASS"

    # Test 7: Secret values are never emitted by error paths
    Write-Host "Test 7: Secret values are not emitted by expected error paths..."
    $secretPassword = "SECRET_SUPER_TEST_PASS_987654321"
    $env:MY_TEST_KEY_PASSWORD = $secretPassword
    $out7 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
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
    if ($out7.Contains($secretPassword)) {
        throw "FAILED: Secret password was leaked to output/error stream!"
    }
    Write-Host "Test 7: PASS"

    # Test 8: Updater signature verification rejects corrupted signature
    Write-Host "Test 8: Updater signature verification rejects corrupted signature..."
    $testExe = Join-Path $fixtureRoot "dummy.exe"
    $testSig = Join-Path $fixtureRoot "dummy.exe.sig"
    [IO.File]::WriteAllBytes($testExe, [Text.Encoding]::UTF8.GetBytes("MZ fake executable content"))
    [IO.File]::WriteAllText($testSig, "untrusted comment: minisign signature`ncorrupted signature data")
    $out8 = & cargo xtask updater-trust verify-signature --installer $testExe --signature $testSig 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: verify-signature succeeded with corrupt signature" }
    Write-Host "Test 8: PASS"

    # Test 9: Tampered candidate binary is detected by Authenticode verifier
    Write-Host "Test 9: Tampered candidate binary is detected..."
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

    # Find a real PE to sign
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
    $out9 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode test `
        -Artifact $peCopy 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Authenticode verification accepted tampered binary" }
    Write-Host "Test 9: PASS"

    # Test 10: Verification rejects test credentials in production mode
    Write-Host "Test 10: Production verification rejects CI test certificate..."
    $unmutatedPe = Join-Path $fixtureRoot "unmutated_pe.exe"
    Copy-Item -LiteralPath $sourcePe.FullName -Destination $unmutatedPe -Force
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "sign_v4_authenticode.ps1") `
        -Path $unmutatedPe
    if ($LASTEXITCODE -ne 0) { throw "Failed to sign unmutated PE" }

    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = $testThumbprint
    $out10 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode production `
        -Artifact $unmutatedPe 2>&1 | Out-String
    Remove-Item Env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT -ErrorAction SilentlyContinue
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Production verification accepted test certificate!" }
    if ($out10 -notmatch "rejects ephemeral CI test signer thumbprint" -and $out10 -notmatch "rejects CI test certificate") {
        throw "FAILED: Did not reject test certificate in production mode. Actual output:`n$out10"
    }
    Write-Host "Test 10: PASS"

    # Test 11: Unbound prebuilt candidate without internal fixture mode fails closed
    Write-Host "Test 11: Unbound prebuilt candidate without internal fixture mode fails closed..."
    $out11a = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -InternalFixtureCandidatePath $unmutatedPe 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted InternalFixtureCandidatePath without -InternalTestFixture" }
    if ($out11a -notmatch "InternalFixtureCandidatePath is only permitted when -InternalTestFixture is specified") {
        throw "FAILED: Did not fail closed on unbound fixture candidate path"
    }

    $out11b = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -InternalSkipSmoke 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted InternalSkipSmoke without -InternalTestFixture" }
    if ($out11b -notmatch "InternalSkipSmoke is only permitted when -InternalTestFixture is specified") {
        throw "FAILED: Did not fail closed on unbound internal skip smoke"
    }
    Write-Host "Test 11: PASS"

    # Test 12: Internal test fixture with skipped smoke cannot emit promotable evidence
    Write-Host "Test 12: Skipped smoke cannot create canonical production evidence..."
    $fixtureBundleDir = Join-Path $fixtureRoot "fixture_bundle"
    $fixtureEvidenceDir = Join-Path $fixtureRoot "fixture_evidence"
    New-Item -ItemType Directory -Path $fixtureBundleDir -Force | Out-Null
    New-Item -ItemType Directory -Path $fixtureEvidenceDir -Force | Out-Null

    $out12 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint $testThumbprint `
        -AuthenticodeProviderScript $dummyProviderScript `
        -BundleDir $fixtureBundleDir `
        -EvidenceDir $fixtureEvidenceDir `
        -InternalTestFixture `
        -InternalFixtureCandidatePath $unmutatedPe `
        -InternalSkipSmoke 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "FAILED: Internal test fixture failed to run: $out12" }

    # Verify that V4_QUALIFICATION_EVIDENCE.json was NOT emitted
    $prodEvidencePath = Join-Path $fixtureEvidenceDir "V4_QUALIFICATION_EVIDENCE.json"
    if (Test-Path -LiteralPath $prodEvidencePath) {
        throw "FAILED: Internal test fixture unexpectedly created canonical V4_QUALIFICATION_EVIDENCE.json!"
    }
    # Verify that V4_FIXTURE_EVIDENCE.json was emitted with non-promotable type
    $fixtureEvidencePath = Join-Path $fixtureEvidenceDir "V4_FIXTURE_EVIDENCE.json"
    if (-not (Test-Path -LiteralPath $fixtureEvidencePath)) {
        throw "FAILED: V4_FIXTURE_EVIDENCE.json was not created by fixture run"
    }
    $fixtureEvidenceObj = Get-Content -LiteralPath $fixtureEvidencePath -Raw | ConvertFrom-Json
    if ($fixtureEvidenceObj.evidence_type -ne "test-fixture-non-promotable") {
        throw "FAILED: Fixture evidence type is promotable: $($fixtureEvidenceObj.evidence_type)"
    }
    if ($fixtureEvidenceObj.authenticode_mode -ne "test") {
        throw "FAILED: Fixture evidence authenticode_mode is not test: $($fixtureEvidenceObj.authenticode_mode)"
    }
    if ($fixtureEvidenceObj.qualified -ne $false) {
        throw "FAILED: Skipped smoke fixture unexpectedly claimed qualified=true!"
    }

    # Verify that promote_v4_metadata rejects this non-promotable fixture evidence
    $out12Reject = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $fixtureEvidencePath 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) {
        throw "FAILED: promote_v4_metadata unexpectedly accepted fixture evidence with skipped smoke!"
    }
    if ($out12Reject -notmatch "Qualification evidence type is not the canonical Tauri qualification path") {
        throw "FAILED: promote_v4_metadata did not reject fixture evidence with type mismatch"
    }
    Write-Host "Test 12: PASS"

    # Test 13: Emitted canonical qualification evidence is accepted by promote_v4_metadata
    Write-Host "Test 13: Emitted canonical evidence is accepted by the same schema validation semantics used for promotion..."
    $installerName = "Sky Auto Player_${currentVersion}_x64-setup.exe"
    $signatureName = "$installerName.sig"
    $canonicalEvidenceDir = Join-Path $fixtureRoot "canonical_evidence"
    New-Item -ItemType Directory -Path $canonicalEvidenceDir -Force | Out-Null
    $canonicalEvidenceFile = Join-Path $canonicalEvidenceDir "V4_QUALIFICATION_EVIDENCE.json"

    function New-ContractEvidence {
        return New-V4CanonicalQualificationEvidence `
            -Version $currentVersion `
            -InstallerName $installerName `
            -SignatureName $signatureName `
            -InstallerSize 1234567 `
            -SignatureSize 512 `
            -InstallerSha256 ("a" * 64) `
            -SignatureSha256 ("b" * 64) `
            -AuthenticodeEvidenceSha256 ("c" * 64) `
            -SbomSha256 ("d" * 64)
    }

    $validEvidence = New-ContractEvidence
    $validEvidence | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $canonicalEvidenceFile -Encoding utf8

    $out13 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $canonicalEvidenceFile 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "FAILED: promote_v4_metadata rejected canonical qualification evidence: $out13"
    }
    if ($out13 -notmatch "V4 qualification evidence validation: PASS") {
        throw "FAILED: promote_v4_metadata did not report PASS on valid qualification evidence"
    }
    Write-Host "Test 13: PASS"

    # Test 14: Tampered/invalid qualification evidence fields are rejected by promote_v4_metadata
    Write-Host "Test 14: Tampered qualification evidence fields are rejected by promotion validator..."
    # 14a. Test mode instead of production
    $invalidAuthMode = New-ContractEvidence
    $invalidAuthMode["authenticode_mode"] = "test"
    $invalidFile = Join-Path $canonicalEvidenceDir "invalid_auth_mode.json"
    $invalidAuthMode | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invalidFile -Encoding utf8
    $out14a = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $invalidFile 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted authenticode_mode=test" }
    if ($out14a -notmatch "Qualification evidence is not production Authenticode evidence") {
        throw "FAILED: Did not reject authenticode_mode=test"
    }

    # 14b. qualified = false
    $invalidQualified = New-ContractEvidence
    $invalidQualified["qualified"] = $false
    $invalidFile2 = Join-Path $canonicalEvidenceDir "invalid_qualified.json"
    $invalidQualified | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invalidFile2 -Encoding utf8
    $out14b = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $invalidFile2 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted qualified=false" }
    if ($out14b -notmatch "Qualification evidence is not an explicit successful result") {
        throw "FAILED: Did not reject qualified=false"
    }

    # 14c. qualification != install-launch-uninstall
    $invalidQualification = New-ContractEvidence
    $invalidQualification["qualification"] = "skipped-smoke"
    $invalidFile3 = Join-Path $canonicalEvidenceDir "invalid_qualification.json"
    $invalidQualification | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invalidFile3 -Encoding utf8
    $out14c = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $invalidFile3 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted qualification=skipped-smoke" }
    if ($out14c -notmatch "Qualification evidence type is not the canonical Tauri qualification path") {
        throw "FAILED: Did not reject qualification=skipped-smoke"
    }
    Write-Host "Test 14: PASS"

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