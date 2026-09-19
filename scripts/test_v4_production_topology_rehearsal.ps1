# scripts/test_v4_production_topology_rehearsal.ps1
# Self-contained, no-argument, zero-mutation regression harness.
#
# Proves that:
#   1. The canonical verifiers (cargo xtask sbom verify, cargo xtask verify-tauri-bundle)
#      accept a clean 2-file candidate-bundle + 6-file candidate-evidence fixture.
#   2. Both verifiers fail closed when the candidate-bundle is contaminated with
#      extra JSON (reproducing the production incident that prompted this gate).
#
# No mandatory parameters. No pipeline state mutations. No GitHub API calls.
# All writes go to an isolated temp directory that is cleaned up via try/finally.
# The canonical pipeline stages (Preflight/BuildCandidate/PublishRelease/
# PromoteMetadata/FinalVerify) are not invoked.
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

function Fail([string]$Message) {
    throw "V4 production-topology rehearsal failed closed: $Message"
}

# ------------------------------------------------------------------
# Resolve package version dynamically from desktop/src-tauri/Cargo.toml
# ------------------------------------------------------------------
$cargoTomlPath = Join-Path $repoRoot "desktop/src-tauri/Cargo.toml"
$cargoToml = Get-Content -LiteralPath $cargoTomlPath -Raw
$packageSection = [regex]::Match($cargoToml, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
if (-not $packageSection.Success) {
    Fail "desktop/src-tauri/Cargo.toml is missing [package]"
}
$versionMatch = [regex]::Match($packageSection.Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
if (-not $versionMatch.Success) {
    Fail "desktop/src-tauri/Cargo.toml [package] is missing version"
}
$version = $versionMatch.Groups[1].Value
if ([string]::IsNullOrWhiteSpace($version)) {
    Fail "resolved package version is empty"
}

. (Join-Path $PSScriptRoot "v4_qualification_evidence.ps1")

$installerName      = "Sky Auto Player_${version}_x64-setup.exe"
$sigName            = "$installerName.sig"
$publicInstallerName = Get-V4SafeReleaseAssetName $installerName
$publicSigName      = Get-V4SafeReleaseAssetName $sigName

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("sky-v4-topology-rehearsal-" + [guid]::NewGuid().ToString("N"))
try {
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null

    # ------------------------------------------------------------------
    # 1. Create candidate-bundle: exactly 2 source-named files (installer + .sig)
    # ------------------------------------------------------------------
    $bundleDir = Join-Path $tmp "candidate-bundle"
    New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null

    $instPath = Join-Path $bundleDir $installerName
    $sigPath  = Join-Path $bundleDir $sigName

    # Installer: 1000 bytes of synthetic content.
    $installerBytes = [byte[]](1..100 | ForEach-Object { $_ % 256 }) * 10
    [IO.File]::WriteAllBytes($instPath, $installerBytes)
    # Signature: 64-byte mock updater signature blob.
    $sigBytes = [byte[]](0..63)
    [IO.File]::WriteAllBytes($sigPath, $sigBytes)

    # Assert exact source-named filenames in bundle — no dotted copies, no extras
    $expectedBundleNames = @($installerName, $sigName) | Sort-Object
    $actualBundleNames   = @(Get-ChildItem -LiteralPath $bundleDir -File | ForEach-Object { $_.Name } | Sort-Object)
    if (($actualBundleNames -join "`n") -ne ($expectedBundleNames -join "`n")) {
        Fail "candidate-bundle does not contain the exact source-named installer/signature pair: $($actualBundleNames -join ', ')"
    }

    # Assert no JSON evidence files in bundle
    if (@(Get-ChildItem -LiteralPath $bundleDir -Filter "*.json" -File).Count -ne 0) {
        Fail "candidate-bundle must contain zero JSON evidence files"
    }

    # Assert no forbidden dotted local binary copies
    foreach ($forbiddenPath in @(
        (Join-Path $bundleDir $publicInstallerName),
        (Join-Path $bundleDir $publicSigName),
        (Join-Path $tmp $publicInstallerName),
        (Join-Path $tmp $publicSigName)
    )) {
        if (Test-Path -LiteralPath $forbiddenPath -PathType Leaf) {
            Fail "topology rehearsal found a forbidden dotted local binary copy: $forbiddenPath"
        }
    }

    # ------------------------------------------------------------------
    # 2. Create candidate-evidence: exactly 6 named evidence files
    # ------------------------------------------------------------------
    $evidenceDir = Join-Path $tmp "candidate-evidence"
    New-Item -ItemType Directory -Path $evidenceDir -Force | Out-Null

    # --- 2a. Generate SBOM from the clean 2-file bundle (real xtask invocation)
    $sbomPath = Join-Path $evidenceDir "SBOM.spdx.json"
    & cargo xtask sbom generate --artifact-dir $bundleDir --output $sbomPath
    if ($LASTEXITCODE -ne 0) { Fail "cargo xtask sbom generate failed on 2-file candidate-bundle (exit $LASTEXITCODE)" }
    if (-not (Test-Path -LiteralPath $sbomPath -PathType Leaf)) { Fail "cargo xtask sbom generate did not create SBOM file" }

    # --- 2b. Authenticode evidence (unsigned-zero-budget policy record)
    $authPath   = Join-Path $evidenceDir "TAURI_AUTHENTICODE_EVIDENCE.json"
    $instSha256 = (Get-FileHash -LiteralPath $instPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $authPayload = [ordered]@{
        schema_version             = 1
        evidence_type              = "authenticode-verification"
        mode                       = "unsigned-zero-budget"
        expected_signer_thumbprint = $null
        verification_policy        = "unsigned-project-owned-pe-files-and-canonical-nsis"
        files                      = @(
            [ordered]@{
                name                     = $installerName
                path                     = $instPath
                status                   = "NotSigned"
                platform_status          = "NotSigned"
                verification             = "authenticode-unsigned-zero-budget"
                trust_exception          = "unsigned-zero-budget-policy"
                integrity_verifier       = "not-applicable-unsigned-zero-budget"
                integrity_status         = "NotSigned"
                signed_digest_algorithm  = $null
                signed_digest            = $null
                computed_digest          = $null
                sha256                   = $instSha256
                signer_thumbprint        = $null
                signer_subject           = $null
            }
        )
    }
    $authPayload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $authPath -Encoding utf8

    # --- 2c. Installed-Authenticode evidence (minimal stub)
    $installedAuthPath = Join-Path $evidenceDir "INSTALLED_AUTHENTICODE_EVIDENCE.json"
    [ordered]@{ schema_version = 1; evidence_type = "installed-authenticode-verification"; files = @() } |
        ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $installedAuthPath -Encoding utf8

    # --- 2d. Artifact summary (minimal stub)
    $summaryPath = Join-Path $evidenceDir "TAURI_ARTIFACT_SUMMARY.json"
    [ordered]@{ schema_version = 1; evidence_type = "tauri-artifact-summary"; artifacts = @() } |
        ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $summaryPath -Encoding utf8

    # Compute hashes needed for evidence documents
    $instSha256 = (Get-FileHash -LiteralPath $instPath  -Algorithm SHA256).Hash.ToLowerInvariant()
    $sigSha256  = (Get-FileHash -LiteralPath $sigPath   -Algorithm SHA256).Hash.ToLowerInvariant()
    $authSha256 = (Get-FileHash -LiteralPath $authPath  -Algorithm SHA256).Hash.ToLowerInvariant()
    $sbomSha256 = (Get-FileHash -LiteralPath $sbomPath  -Algorithm SHA256).Hash.ToLowerInvariant()

    # --- 2e. Qualification evidence
    $qualPath = Join-Path $evidenceDir "V4_QUALIFICATION_EVIDENCE.json"
    $qualPayload = New-V4CanonicalQualificationEvidence `
        -Version                    $version `
        -InstallerName              $installerName `
        -SignatureName              $sigName `
        -InstallerSize              ([int64](Get-Item -LiteralPath $instPath).Length) `
        -SignatureSize              ([int64](Get-Item -LiteralPath $sigPath).Length) `
        -InstallerSha256            $instSha256 `
        -SignatureSha256            $sigSha256 `
        -AuthenticodeEvidenceSha256 $authSha256 `
        -SbomSha256                 $sbomSha256
    $qualPayload | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $qualPath -Encoding utf8

    # --- 2f. Production evidence
    $prodPath = Join-Path $evidenceDir "V4_PRODUCTION_RELEASE_EVIDENCE.json"
    $prodPayload = New-V4CanonicalProductionEvidence `
        -SourceSha                  ("0" * 40) `
        -Version                    $version `
        -Channel                    "stable" `
        -InstallerName              $installerName `
        -SignatureName              $sigName `
        -InstallerSize              ([int64](Get-Item -LiteralPath $instPath).Length) `
        -SignatureSize              ([int64](Get-Item -LiteralPath $sigPath).Length) `
        -InstallerSha256            $instSha256 `
        -SignatureSha256            $sigSha256 `
        -AuthenticodeEvidenceSha256 $authSha256 `
        -SbomSha256                 $sbomSha256 `
        -UpdaterKeyId               "19AABD2E7838818C"
    $prodPayload | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $prodPath -Encoding utf8

    # Assert exact 6 evidence filenames
    $expectedEvidenceNames = @(
        "V4_QUALIFICATION_EVIDENCE.json",
        "V4_PRODUCTION_RELEASE_EVIDENCE.json",
        "TAURI_AUTHENTICODE_EVIDENCE.json",
        "INSTALLED_AUTHENTICODE_EVIDENCE.json",
        "TAURI_ARTIFACT_SUMMARY.json",
        "SBOM.spdx.json"
    ) | Sort-Object
    $actualEvidenceNames = @(Get-ChildItem -LiteralPath $evidenceDir -File | ForEach-Object { $_.Name } | Sort-Object)
    if (($actualEvidenceNames -join "`n") -ne ($expectedEvidenceNames -join "`n")) {
        Fail "candidate-evidence does not contain the exact six-file evidence set: $($actualEvidenceNames -join ', ')"
    }

    # ------------------------------------------------------------------
    # 3. Happy path: verifiers must accept the clean fixture
    # ------------------------------------------------------------------
    & cargo xtask sbom verify --artifact-dir $bundleDir --sbom $sbomPath
    if ($LASTEXITCODE -ne 0) { Fail "cargo xtask sbom verify rejected clean 2-file candidate-bundle (exit $LASTEXITCODE)" }

    & cargo xtask verify-tauri-bundle --bundle-dir $bundleDir --authenticode-evidence $authPath --sbom $sbomPath
    if ($LASTEXITCODE -ne 0) { Fail "cargo xtask verify-tauri-bundle rejected clean 2-file candidate-bundle (exit $LASTEXITCODE)" }

    # ------------------------------------------------------------------
    # 4. Fail-closed proof: inject extra JSON into candidate-bundle,
    #    both verifiers must reject (non-zero exit).
    # ------------------------------------------------------------------
    $strayJsonPath = Join-Path $bundleDir "V4_PRODUCTION_RELEASE_EVIDENCE.json"
    Set-Content -LiteralPath $strayJsonPath -Value "{}" -Encoding utf8

    $sbomMixedOutput = & cargo xtask sbom verify --artifact-dir $bundleDir --sbom $sbomPath 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { Fail "cargo xtask sbom verify accepted contaminated candidate-bundle — verifier did not fail closed" }
    if ($sbomMixedOutput -notmatch "SBOM artifact-set SHA-256 does not match|SBOM file set does not match") {
        Fail "cargo xtask sbom verify returned unexpected error on contaminated bundle: $sbomMixedOutput"
    }

    $tauriMixedOutput = & cargo xtask verify-tauri-bundle --bundle-dir $bundleDir --authenticode-evidence $authPath --sbom $sbomPath 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { Fail "cargo xtask verify-tauri-bundle accepted contaminated candidate-bundle — verifier did not fail closed" }
    if ($tauriMixedOutput -notmatch "Tauri NSIS bundle must contain only the setup executable and its \.sig") {
        Fail "cargo xtask verify-tauri-bundle returned unexpected error on contaminated bundle: $tauriMixedOutput"
    }

    Write-Host "V4 production-topology rehearsal: PASS (self-contained; zero-mutation; candidate-bundle 2-file; candidate-evidence 6-file; verifier fail-closed)"
} finally {
    if (Test-Path -LiteralPath $tmp) {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
