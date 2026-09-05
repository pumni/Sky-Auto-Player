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

try {
    # Ensure runner environment is clean of inherited signing keys before tests run
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PATH -ErrorAction SilentlyContinue

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

    # Canonical production bundle snapshot to guarantee contract test isolation
    $canonicalBundle = Join-Path $repoRoot "rust\target\dist\bundle\nsis"
    function Get-CanonicalBundleSnapshot {
        if (-not (Test-Path -LiteralPath $canonicalBundle -PathType Container)) {
            return @{}
        }
        $snapshot = @{}
        Get-ChildItem -LiteralPath $canonicalBundle -File | ForEach-Object {
            $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
            $snapshot[$_.Name] = @{
                Size = $_.Length
                Hash = $hash
            }
        }
        return $snapshot
    }
    $initialBundleSnapshot = Get-CanonicalBundleSnapshot

    function New-TestOutputDirectories {
        param([string]$Name)

        $root = Join-Path $fixtureRoot $Name
        $bundle = Join-Path $root "bundle"
        $evidence = Join-Path $root "evidence"

        New-Item -ItemType Directory -Path $bundle -Force | Out-Null
        New-Item -ItemType Directory -Path $evidence -Force | Out-Null

        return @{
            BundleDir = $bundle
            EvidenceDir = $evidence
        }
    }

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

        $dirs3 = New-TestOutputDirectories "test-03-dirty-worktree"
        $out3 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
            -ExpectedSourceSha $currentSha `
            -Version $currentVersion `
            -Channel "beta" `
            -UpdaterPrivateKeyPath $throwawayKeyPath `
            -AuthenticodeProvider "custom" `
            -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
            -AuthenticodeProviderScript $dummyProviderScript `
            -BundleDir $dirs3.BundleDir `
            -EvidenceDir $dirs3.EvidenceDir 2>&1 | Out-String
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
    $dirs6 = New-TestOutputDirectories "test-06-wrong-updater-key"
    $out6 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -BundleDir $dirs6.BundleDir `
        -EvidenceDir $dirs6.EvidenceDir 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted mismatched updater key" }
    if ($out6 -notmatch "Pre-packaging updater key verification failed") {
        throw "FAILED: Did not fail closed on updater key pre-flight check. Actual output:`n$out6"
    }
    Write-Host "Test 6: PASS"

    # Test 7: Secret values are never emitted by error paths
    Write-Host "Test 7: Secret values are not emitted by expected error paths..."
    $secretPassword = "SECRET_SUPER_TEST_PASS_987654321"
    $env:MY_TEST_KEY_PASSWORD = $secretPassword
    $dirs7 = New-TestOutputDirectories "test-07-secret-leak"
    $out7 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -UpdaterPasswordEnv "MY_TEST_KEY_PASSWORD" `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -BundleDir $dirs7.BundleDir `
        -EvidenceDir $dirs7.EvidenceDir 2>&1 | Out-String
    Remove-Item Env:MY_TEST_KEY_PASSWORD -ErrorAction SilentlyContinue
    if ($out7.Contains($secretPassword)) {
        throw "FAILED: Secret password was leaked to output/error stream!"
    }
    Write-Host "Test 7: PASS"

    # Test 8: Inherited signing key environment fails closed without leaking secret
    Write-Host "Test 8: Inherited signing key environment fails closed..."
    $secretRawKey = "untrusted comment: secret raw private key content`nSECRET_RAW_KEY_MATERIAL_12345"
    $env:TAURI_SIGNING_PRIVATE_KEY = $secretRawKey
    try {
        $dirs8a = New-TestOutputDirectories "test-08a-inherited-key"
        $out8a = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
            -ExpectedSourceSha $currentSha `
            -Version $currentVersion `
            -Channel "beta" `
            -UpdaterPrivateKeyPath $throwawayKeyPath `
            -AuthenticodeProvider "custom" `
            -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
            -AuthenticodeProviderScript $dummyProviderScript `
            -BundleDir $dirs8a.BundleDir `
            -EvidenceDir $dirs8a.EvidenceDir 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted inherited TAURI_SIGNING_PRIVATE_KEY" }
        if ($out8a -notmatch "Pre-existing TAURI_SIGNING_PRIVATE_KEY detected") {
            throw "FAILED: Did not reject inherited TAURI_SIGNING_PRIVATE_KEY. Actual output:`n$out8a"
        }
        if ($out8a.Contains("SECRET_RAW_KEY_MATERIAL_12345")) {
            throw "FAILED: Secret raw key material was echoed in error output!"
        }
    } finally {
        Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    }

    $env:TAURI_SIGNING_PRIVATE_KEY_PATH = "C:\fake\secret\path.key"
    try {
        $dirs8b = New-TestOutputDirectories "test-08b-inherited-key-path"
        $out8b = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
            -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
            -ExpectedSourceSha $currentSha `
            -Version $currentVersion `
            -Channel "beta" `
            -UpdaterPrivateKeyPath $throwawayKeyPath `
            -AuthenticodeProvider "custom" `
            -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
            -AuthenticodeProviderScript $dummyProviderScript `
            -BundleDir $dirs8b.BundleDir `
            -EvidenceDir $dirs8b.EvidenceDir 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator accepted inherited TAURI_SIGNING_PRIVATE_KEY_PATH" }
        if ($out8b -notmatch "Pre-existing TAURI_SIGNING_PRIVATE_KEY_PATH detected") {
            throw "FAILED: Did not reject inherited TAURI_SIGNING_PRIVATE_KEY_PATH. Actual output:`n$out8b"
        }
    } finally {
        Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PATH -ErrorAction SilentlyContinue
    }
    Write-Host "Test 8: PASS"

    # Test 9: Stale candidate artifacts and evidence are purged before packaging
    Write-Host "Test 9: Stale candidate artifacts and evidence are purged before packaging..."
    $staleBundleDir = Join-Path $fixtureRoot "stale_bundle"
    $staleEvidenceDir = Join-Path $fixtureRoot "stale_evidence"
    New-Item -ItemType Directory -Path $staleBundleDir -Force | Out-Null
    New-Item -ItemType Directory -Path $staleEvidenceDir -Force | Out-Null

    $staleInstaller = Join-Path $staleBundleDir "Sky Auto Player_${currentVersion}_x64-setup.exe"
    $staleSig = Join-Path $staleBundleDir "Sky Auto Player_${currentVersion}_x64-setup.exe.sig"
    $staleQualEvidence = Join-Path $staleEvidenceDir "V4_QUALIFICATION_EVIDENCE.json"
    $staleProdEvidence = Join-Path $staleEvidenceDir "V4_PRODUCTION_RELEASE_EVIDENCE.json"

    [IO.File]::WriteAllText($staleInstaller, "stale installer payload")
    [IO.File]::WriteAllText($staleSig, "stale signature payload")
    [IO.File]::WriteAllText($staleQualEvidence, "stale qual evidence")
    [IO.File]::WriteAllText($staleProdEvidence, "stale prod evidence")

    $out9 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -BundleDir $staleBundleDir `
        -EvidenceDir $staleEvidenceDir 2>&1 | Out-String

    if ($LASTEXITCODE -eq 0) { throw "FAILED: Orchestrator unexpectedly succeeded with wrong updater key" }

    $purgeMarker = "[Pre-Packaging Purge] Stale candidate artifacts and evidence successfully purged: PASS"
    $keyFailMarker = "Pre-packaging updater key verification failed"

    if (-not $out9.Contains($purgeMarker)) {
        throw "FAILED: Pre-packaging purge marker not found in output. Actual output:`n$out9"
    }
    if (-not $out9.Contains($keyFailMarker)) {
        throw "FAILED: Pre-packaging updater key verification failure marker not found. Actual output:`n$out9"
    }

    $purgeIndex = $out9.IndexOf($purgeMarker)
    $keyFailIndex = $out9.IndexOf($keyFailMarker)
    if ($purgeIndex -ge $keyFailIndex) {
        throw "FAILED: Stale-output purge did not execute strictly BEFORE pre-packaging updater key verification! Purge index=$purgeIndex, KeyFail index=$keyFailIndex"
    }

    # Defense-in-depth: Assert that post-run, all 4 stale files remain absent
    if (Test-Path -LiteralPath $staleInstaller) { throw "FAILED: Stale installer was not purged before packaging!" }
    if (Test-Path -LiteralPath $staleSig) { throw "FAILED: Stale signature was not purged before packaging!" }
    if (Test-Path -LiteralPath $staleQualEvidence) { throw "FAILED: Stale qualification evidence was not purged before packaging!" }
    if (Test-Path -LiteralPath $staleProdEvidence) { throw "FAILED: Stale production release evidence was not purged before packaging!" }
    Write-Host "Test 9: PASS"

    # Test 10: Updater signature verification rejects corrupted signature
    Write-Host "Test 10: Updater signature verification rejects corrupted signature..."
    $testExe = Join-Path $fixtureRoot "dummy.exe"
    $testSig = Join-Path $fixtureRoot "dummy.exe.sig"
    [IO.File]::WriteAllBytes($testExe, [Text.Encoding]::UTF8.GetBytes("MZ fake executable content"))
    [IO.File]::WriteAllText($testSig, "untrusted comment: minisign signature`ncorrupted signature data")
    $out10 = & cargo xtask updater-trust verify-signature --installer $testExe --signature $testSig 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: verify-signature succeeded with corrupt signature" }
    Write-Host "Test 10: PASS"

    # Test 11: Tampered candidate binary is detected by Authenticode verifier
    Write-Host "Test 11: Tampered candidate binary is detected..."
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
    $out11 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode test `
        -Artifact $peCopy 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Authenticode verification accepted tampered binary" }
    Write-Host "Test 11: PASS"

    # Test 12: Verification rejects test credentials in production mode
    Write-Host "Test 12: Production verification rejects CI test certificate..."
    $unmutatedPe = Join-Path $fixtureRoot "unmutated_pe.exe"
    Copy-Item -LiteralPath $sourcePe.FullName -Destination $unmutatedPe -Force
    & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "sign_v4_authenticode.ps1") `
        -Path $unmutatedPe
    if ($LASTEXITCODE -ne 0) { throw "Failed to sign unmutated PE" }

    $env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = $testThumbprint
    $out12 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "verify_v4_authenticode.ps1") `
        -Mode production `
        -Artifact $unmutatedPe 2>&1 | Out-String
    Remove-Item Env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT -ErrorAction SilentlyContinue
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Production verification accepted test certificate!" }
    if ($out12 -notmatch "rejects ephemeral CI test signer thumbprint" -and $out12 -notmatch "rejects CI test certificate") {
        throw "FAILED: Did not reject test certificate in production mode. Actual output:`n$out12"
    }
    Write-Host "Test 12: PASS"

    # Test 13: Unbound prebuilt candidate without internal fixture mode fails closed
    Write-Host "Test 13: Unbound prebuilt candidate without internal fixture mode fails closed..."
    $dirs13a = New-TestOutputDirectories "test-13a-unbound-candidate"
    $out13a = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -BundleDir $dirs13a.BundleDir `
        -EvidenceDir $dirs13a.EvidenceDir `
        -InternalFixtureCandidatePath $unmutatedPe 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted InternalFixtureCandidatePath without -InternalTestFixture" }
    if ($out13a -notmatch "InternalFixtureCandidatePath is only permitted when -InternalTestFixture is specified") {
        throw "FAILED: Did not fail closed on unbound fixture candidate path"
    }

    $dirs13b = New-TestOutputDirectories "test-13b-unbound-skip-smoke"
    $out13b = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "orchestrate_v4_production_release.ps1") `
        -ExpectedSourceSha $currentSha `
        -Version $currentVersion `
        -Channel "beta" `
        -UpdaterPrivateKeyPath $throwawayKeyPath `
        -AuthenticodeProvider "custom" `
        -ApprovedSignerThumbprint "0123456789ABCDEF0123456789ABCDEF01234567" `
        -AuthenticodeProviderScript $dummyProviderScript `
        -BundleDir $dirs13b.BundleDir `
        -EvidenceDir $dirs13b.EvidenceDir `
        -InternalSkipSmoke 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted InternalSkipSmoke without -InternalTestFixture" }
    if ($out13b -notmatch "InternalSkipSmoke is only permitted when -InternalTestFixture is specified") {
        throw "FAILED: Did not fail closed on unbound internal skip smoke"
    }
    Write-Host "Test 13: PASS"

    # Test 14: Internal test fixture with skipped smoke cannot emit promotable evidence
    Write-Host "Test 14: Skipped smoke cannot create canonical production evidence..."
    $fixtureBundleDir = Join-Path $fixtureRoot "fixture_bundle"
    $fixtureEvidenceDir = Join-Path $fixtureRoot "fixture_evidence"
    New-Item -ItemType Directory -Path $fixtureBundleDir -Force | Out-Null
    New-Item -ItemType Directory -Path $fixtureEvidenceDir -Force | Out-Null

    $out14 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
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
    if ($LASTEXITCODE -ne 0) { throw "FAILED: Internal test fixture failed to run: $out14" }

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
    $out14Reject = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $fixtureEvidencePath 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) {
        throw "FAILED: promote_v4_metadata unexpectedly accepted fixture evidence with skipped smoke!"
    }
    if ($out14Reject -notmatch "Qualification evidence type is not the canonical Tauri qualification path") {
        throw "FAILED: promote_v4_metadata did not reject fixture evidence with type mismatch"
    }
    Write-Host "Test 14: PASS"

    # Test 15: Emitted canonical qualification evidence is accepted by promote_v4_metadata
    Write-Host "Test 15: Emitted canonical evidence is accepted by the same schema validation semantics used for promotion..."
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

    $out15 = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $canonicalEvidenceFile 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "FAILED: promote_v4_metadata rejected canonical qualification evidence: $out15"
    }
    if ($out15 -notmatch "V4 qualification evidence validation: PASS") {
        throw "FAILED: promote_v4_metadata did not report PASS on valid qualification evidence"
    }
    Write-Host "Test 15: PASS"

    # Test 16: Tampered/invalid qualification evidence fields are rejected by promote_v4_metadata
    Write-Host "Test 16: Tampered qualification evidence fields are rejected by promotion validator..."
    # 16a. Test mode instead of production
    $invalidAuthMode = New-ContractEvidence
    $invalidAuthMode["authenticode_mode"] = "test"
    $invalidFile = Join-Path $canonicalEvidenceDir "invalid_auth_mode.json"
    $invalidAuthMode | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invalidFile -Encoding utf8
    $out16a = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $invalidFile 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted authenticode_mode=test" }
    if ($out16a -notmatch "Qualification evidence is not production Authenticode evidence") {
        throw "FAILED: Did not reject authenticode_mode=test"
    }

    # 16b. qualified = false
    $invalidQualified = New-ContractEvidence
    $invalidQualified["qualified"] = $false
    $invalidFile2 = Join-Path $canonicalEvidenceDir "invalid_qualified.json"
    $invalidQualified | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invalidFile2 -Encoding utf8
    $out16b = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $invalidFile2 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted qualified=false" }
    if ($out16b -notmatch "Qualification evidence is not an explicit successful result") {
        throw "FAILED: Did not reject qualified=false"
    }

    # 16c. qualification != install-launch-uninstall
    $invalidQualification = New-ContractEvidence
    $invalidQualification["qualification"] = "skipped-smoke"
    $invalidFile3 = Join-Path $canonicalEvidenceDir "invalid_qualification.json"
    $invalidQualification | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $invalidFile3 -Encoding utf8
    $out16c = & pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot "promote_v4_metadata.ps1") `
        -ValidateEvidence $invalidFile3 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { throw "FAILED: Accepted qualification=skipped-smoke" }
    if ($out16c -notmatch "Qualification evidence type is not the canonical Tauri qualification path") {
        throw "FAILED: Did not reject qualification=skipped-smoke"
    }
    # Regression assertion: Running orchestrator contract suite must not create, delete, or modify production release outputs
    $finalBundleSnapshot = Get-CanonicalBundleSnapshot
    if ($initialBundleSnapshot.Count -ne $finalBundleSnapshot.Count) {
        throw "FAILED ISOLATION CONTRACT: Canonical production bundle file count changed! Initial: $($initialBundleSnapshot.Count), Final: $($finalBundleSnapshot.Count)"
    }
    foreach ($key in $initialBundleSnapshot.Keys) {
        if (-not $finalBundleSnapshot.ContainsKey($key)) {
            throw "FAILED ISOLATION CONTRACT: File '$key' was deleted from canonical production bundle by orchestrator contract tests!"
        }
        $init = $initialBundleSnapshot[$key]
        $fin = $finalBundleSnapshot[$key]
        if ($init.Hash -ne $fin.Hash -or $init.Size -ne $fin.Size) {
            throw "FAILED ISOLATION CONTRACT: File '$key' in canonical production bundle was mutated by orchestrator contract tests! Initial Hash=$($init.Hash), Final Hash=$($fin.Hash)"
        }
    }
    foreach ($key in $finalBundleSnapshot.Keys) {
        if (-not $initialBundleSnapshot.ContainsKey($key)) {
            throw "FAILED ISOLATION CONTRACT: File '$key' was unexpectedly created in canonical production bundle by orchestrator contract tests!"
        }
    }
    Write-Host "Canonical production bundle isolation: PASS"

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