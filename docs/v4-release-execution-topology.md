# V4 Production Release Execution Topology and Candidate Lifecycle

This document specifies the release execution topology, runner trust boundary,
candidate qualification lifecycle, and provenance model for production v4 releases
of Sky Auto Player.

Governed by:
- [docs/adr/ADR-0006-v4-distribution-installation-update.md](adr/ADR-0006-v4-distribution-installation-update.md)
- [v4-tauri-packaging.md](v4-tauri-packaging.md)
- [v4-release-authority.md](v4-release-authority.md)
- [v4-updater-key-custody.md](v4-updater-key-custody.md)
- [v4-authenticode-provider-seam.md](v4-authenticode-provider-seam.md)
- [../SECURITY.md](../SECURITY.md)

---

## 1. Core Release Principle: Build Once, Qualify Exact Bytes

Every release candidate is subject to an immutable invariant:

```text
[Verified Clean Source Commit]
              |
              v
     (1) BUILD ONCE  ---> Single NSIS Installer Candidate (.exe) + Updater Signature (.sig)
              |
              v
     (2) QUALIFY     ---> Authenticode Production Verification + Ed25519 Minisign Verification
              |           + SPDX SBOM Generation & Verification + Bundle Verification
              |           + Install/Launch/Uninstall Smoke Test
              v
     (3) ATTEST      ---> Cryptographic Qualification Evidence + GitHub OIDC SLSA Attestation (Topology A)
              |
              v
     (4) PROMOTE     ---> Static Metadata Generation + GitHub Releases / CDN Deployment
```

**Invariants**:
- **Clean Source Binding**: The Git working tree must be clean (`git status --porcelain`);
  uncommitted source modifications fail closed before any build, signing, or provider invocation.
- **Zero Rebuild**: Once candidate `.exe` and `.exe.sig` are produced, they are never
  rebuilt, re-bundled, or modified.
- **Byte-Identity**: The SHA-256 hash recorded during qualification is identical to the hash
  in `V4_QUALIFICATION_EVIDENCE.json`, `latest.json`, `SBOM.spdx.json`, and the GitHub
  Release download asset.
- **Fail-Closed Verification & Candidate Purge**: If any check (Authenticode, Ed25519 Minisign,
  SBOM, bundle, smoke test) fails, the candidate binary, signature, and unpromoted evidence are
  purged from the staging directory.

---

## 2. Runner Trust Boundaries and Key Custody

### 2.1 The Custody vs. Cloud Attestation Boundary

Tauri updater private keys and production Authenticode certificates must be protected
with strict physical or cryptographic access controls (FIPS 140-2 Level 2+ HSM, smartcard,
or isolated key vault). They **must never be stored as plaintext secrets in GitHub Actions
cloud repository secrets**.

Conversely, GitHub Artifact Attestations (`actions/attest@v4`) and GitHub-backed SLSA
provenance rely on GitHub's OIDC minting service, which is accessible only from an active
GitHub Actions runner environment.

### 2.2 Release Execution Topologies and Provenance Semantics

```text
+---------------------------------------------------------------------------------+
| TOPOLOGY A: Dedicated / Self-Hosted Single-Tenant Windows Release Runner         |
| (Accepted Production Path: Fully Qualified & OIDC-Attested)                     |
+---------------------------------------------------------------------------------+
|                                                                                 |
|   GitHub Actions Workflow (Release Dispatch)                                   |
|     |                                                                           |
|     +---> Dedicated Single-Tenant Self-Hosted Windows Runner                    |
|            |-- Isolated HSM / local key vault / KMS boundary (outside workspace)|
|            |-- Clean worktree pre-flight check (fails closed on dirty source)   |
|            |-- Runs `scripts/orchestrate_v4_production_release.ps1`             |
|            |     |-- Validates commit, keys, providers                          |
|            |     |-- Builds candidate once with canonical public root           |
|            |     |-- Signs PE via Authenticode Provider Seam                    |
|            |     |-- Signs updater package with private key (never logged)      |
|            |     |-- Qualifies exact bytes (Authenticode + Minisign + SBOM)     |
|            |     \-- Executes mandatory install/launch/uninstall smoke test     |
|            |-- Runs `actions/attest` with GitHub OIDC token                     |
|            |     \-- Generates authentic SLSA provenance & SBOM attestation     |
|            \-- Runs `scripts/promote_v4_metadata.ps1`                           |
|                  \-- Publishes release assets and static metadata                |
+---------------------------------------------------------------------------------+

+---------------------------------------------------------------------------------+
| TOPOLOGY B: Air-Gapped / Maintainer Workstation Operator Execution              |
| (Non-Qualifying Fallback: Provenance Gap for WO-05)                             |
+---------------------------------------------------------------------------------+
|                                                                                 |
|   Maintainer Workstation (Offline or Private Network)                           |
|     |-- Hardware Token / HSM / Offline Air-Gapped Key                           |
|     |-- Runs `scripts/orchestrate_v4_production_release.ps1`                    |
|     |     |-- Builds candidate from verified clean git commit tag               |
|     |     |-- Signs PE and updater payload                                      |
|     |     |-- Qualifies exact bytes & emits local evidence                      |
|     |     \-- Executes install/launch/uninstall smoke test                      |
|     |-- Cryptographic signatures authenticate bytes, BUT do NOT prove which    |
|     |   source commit produced them; V4_PRODUCTION_RELEASE_EVIDENCE.json is not |
|     |   attested by an external OIDC authority.                                 |
|     \-- STATUS: NOT currently sufficient for WO-05 production acceptance.       |
|         Accepting Topology B requires an explicit ADR amendment and a           |
|         source-bound provenance design (e.g. in-toto / cosign key binding).     |
+---------------------------------------------------------------------------------+
```

#### Topology A (Accepted Production Path):
A dedicated, single-tenant Windows release runner registered as a GitHub Actions runner for the
repository. The runner has access to the cloud signing provider (e.g., Azure Trusted
Signing, DigiCert ONE) or attached hardware security key via external custody boundary.
Because it executes within GitHub Actions, `actions/attest@v4` runs directly on the exact
candidate bytes produced by the orchestrator.
**Under current ADR-0006 and WO-05 acceptance contracts, Topology A is the only release
path capable of satisfying the production GitHub provenance requirement.**

#### Topology B (Documented Provenance Gap / Non-Qualifying Fallback):
An operator runs `scripts/orchestrate_v4_production_release.ps1` directly on an air-gapped
workstation. The script outputs candidate bytes, Authenticode signatures, updater signatures,
and local evidence. However, Authenticode and updater signatures authenticate the binary
bytes; they do **not** cryptographically prove which source commit produced those bytes.
Furthermore, `V4_PRODUCTION_RELEASE_EVIDENCE.json` is an unsigned local artifact.
**Topology B is therefore not currently accepted for production release promotion.**
Accepting Topology B in the future requires an explicit ADR amendment and a source-bound
cryptographic provenance design.

### 2.3 Self-Hosted Runner Hygiene Preconditions

Dedicated self-hosted Windows runners executing production releases must satisfy strict hygiene preconditions:
1. **Clean Workspace Checkout**: The runner workspace must be checked out cleanly to the exact target release commit SHA with no uncommitted modifications or untracked dirty files (`git status --porcelain` is empty).
2. **Unpolluted Environment Overrides**: The runner environment must not inherit ambient signing environment variables (such as pre-set `TAURI_SIGNING_PRIVATE_KEY` or `TAURI_SIGNING_PRIVATE_KEY_PATH`). The orchestrator checks this pre-flight and fails closed if pre-existing keys or paths are detected.
3. **Stale Output Purge**: Target bundle and evidence directories are purged of previous candidate installers, signatures, and evidence prior to building. Stale outputs from aborted or previous runs can never be reused.
4. **Isolated Key Storage**: Private updater keys and certificate stores must reside outside the repository tree, accessible only via restricted paths or secure hardware/cloud seams.

---

## 3. Orchestration Specification

The canonical release orchestrator is implemented in
`scripts/orchestrate_v4_production_release.ps1`.

### 3.1 Parameter Contract

| Parameter | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `-ExpectedSourceSha` | String | Yes | Exact 40-character Git commit SHA to build. Rejects dirty working tree or SHA mismatch. |
| `-Version` | String | Yes | Exact SemVer (e.g. `4.0.0-beta.1`) matching `Cargo.toml`. |
| `-Channel` | String | Yes | Release channel: `beta` or `stable`. Must conform to channel versioning rules. |
| `-UpdaterPrivateKeyPath` | String | Yes | Path to private Minisign key. **Must be outside repository tree**. |
| `-UpdaterPasswordEnv` | String | No | Name of environment variable holding key passphrase (default: `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). |
| `-AuthenticodeProvider` | String | Yes | Authenticode provider name matching provider seam (`trusted-signing`, `digicert-one`, `custom`, etc.). |
| `-ApprovedSignerThumbprint` | String | Yes | Expected SHA-1 thumbprint of the production Authenticode certificate. |
| `-AuthenticodeProviderScript` | String | Mutually Exclusive | Custom provider signing script path (for custom provider). |
| `-AuthenticodeProviderCommand` | String | Mutually Exclusive | Custom provider command line (for command-based provider). |
| `-BundleDir` | String | No | Target bundle directory. Defaults to `rust/target/dist/bundle/nsis`. |
| `-EvidenceDir` | String | No | Target evidence directory. Defaults to `rust/target/dist`. |

*Note: Canonical production releases always build from source and always execute the install/launch/uninstall smoke test. No prebuilt or smoke-skipping switches exist in the production transaction.*

### 3.2 Security Pre-Checks and Runner Hygiene (Fail-Closed)

Before initiating any build or invoking external tools, the orchestrator validates:
1. **Unpolluted Environment**: Fails closed immediately if `TAURI_SIGNING_PRIVATE_KEY` or
   `TAURI_SIGNING_PRIVATE_KEY_PATH` are already present in the runner environment. Signing credentials
   must never be ambiently inherited from previous runner jobs.
2. **Commit Integrity**: Current Git HEAD matches `-ExpectedSourceSha` exactly.
3. **Clean Worktree (Pre-Flight & Post-Build)**: Git working tree must be completely clean
   (`git status --porcelain`). Any uncommitted changes fail closed before any updater-key check,
   provider invocation, build, or signing. Furthermore, a second clean-worktree check runs
   immediately after the production build to ensure build tools did not mutate tracked source or
   manifest files.
4. **Version Alignment**: `desktop/src-tauri/Cargo.toml` package version matches `-Version`.
5. **Channel Policy**: Aligned with WO-04 release authority (`stable` requires non-prerelease SemVer;
   `beta` requires prerelease SemVer).
6. **Key Path Isolation**: `-UpdaterPrivateKeyPath` is verified to reside outside the repository tree.
7. **Key Validity Check**: Invokes `cargo xtask updater-trust verify-private-key` against the private key
   *before* building. Key ID is dynamically derived from canonical public root.
8. **Zero Secret Output**: The orchestrator never prints or leaks the updater key passphrase or key material to
   stdout, stderr, or log streams under any execution or error path.
9. **Provider Exclusivity**: Exactly one of `-AuthenticodeProviderScript` or `-AuthenticodeProviderCommand`.
10. **Production Mode Enforcement**: Authenticode mode is locked to `production`. Test certificates
    are rejected unconditionally.
11. **Stale Output Purge**: Automatically purges any pre-existing installer candidate, `.sig` file,
    and qualification evidence from the resolved target bundle and evidence directories before packaging.

### 3.3 Execution Workflow

```text
Step 1: Setup & Pre-Flight Checks
  - Assert environment is unpolluted by pre-existing signing environment variables.
  - Validate parameters, commit SHA, clean worktree, version, channel rules.
  - Purge stale candidate and evidence files from resolved output directories.
  - Verify updater private key matches canonical public root via `cargo xtask updater-trust verify-private-key`.
  - Configure production Authenticode environment variables for signCommand.

Step 2: Build & Sign Candidate (Single Build)
  - Run `bun install --frozen-lockfile` and `bun run build` in desktop/.
  - Set `TAURI_SIGNING_PRIVATE_KEY` to the validated private key file path (never raw key bytes).
  - Run `bun run tauri build --ci -- --profile dist`.
  - Tauri NSIS bundler invokes `scripts/sign_v4_authenticode.ps1` via signCommand seam.
  - Tauri bundler signs the installer with the private key, generating `.exe.sig`.
  - Assert working tree remains clean post-build (`git status --porcelain`); fail closed and purge
    candidate if any tracked file or manifest was mutated during packaging.

Step 3: Exact Candidate Verification
  - Check installer candidate and signature exist and are non-empty.

Step 4: Authenticode Verification
  - Run `scripts/verify_v4_authenticode.ps1 -Mode production`.
  - Fail closed if thumbprint does not match `-ApprovedSignerThumbprint` or if CI test cert is detected.

Step 5: Tauri Updater Signature Verification against Canonical Root
  - Run `cargo xtask updater-trust verify-signature --installer <EXE> --signature <SIG>`.
  - Cryptographically verifies Ed25519 signature against canonical public root.

Step 6: SPDX SBOM and Bundle Verification
  - Run `cargo xtask sbom generate` and `cargo xtask sbom verify`.
  - Run `cargo xtask verify-tauri-bundle`.

Step 7: Install Smoke Test
  - Install silently (`/S`) under `%TEMP%` test profile.
  - Verify launcher executable exists and verify installed PE Authenticode signature.
  - Launch application process, monitor execution, and stop.
  - Run uninstaller (`/S`) and verify clean removal.

Step 8: Evidence Emission & Self-Validation
  - Generate canonical `V4_QUALIFICATION_EVIDENCE.json` (20 fields).
  - Immediately validate emitted evidence with `scripts/promote_v4_metadata.ps1 -ValidateEvidence`.
  - Generate extended `V4_PRODUCTION_RELEASE_EVIDENCE.json`.
```

---

## 4. Evidence Schemas and Promotion

### 4.1 Canonical Qualification Evidence (`V4_QUALIFICATION_EVIDENCE.json`)

To maintain full compatibility with `scripts/promote_v4_metadata.ps1`, the orchestrator
constructs qualification evidence via the shared builder contract `scripts/v4_qualification_evidence.ps1`
(`New-V4CanonicalQualificationEvidence`), emitting the exact 20 required qualification fields:

```json
{
  "schema_version": 1,
  "evidence_type": "tauri-nsis-qualified-release",
  "qualified": true,
  "qualification": "install-launch-uninstall",
  "product_name": "Sky Auto Player",
  "identifier": "io.github.pumni.skyautoplayer",
  "version": "4.0.0-beta.1",
  "target": "nsis",
  "install_mode": "currentUser",
  "installer": "Sky Auto Player_4.0.0-beta.1_x64-setup.exe",
  "updater_signature": "Sky Auto Player_4.0.0-beta.1_x64-setup.exe.sig",
  "installer_size": 12345678,
  "signature_size": 512,
  "installer_sha256": "abcdef...",
  "updater_signature_sha256": "123456...",
  "authenticode_mode": "production",
  "authenticode_evidence": "TAURI_AUTHENTICODE_EVIDENCE.json",
  "authenticode_evidence_sha256": "789abc...",
  "sbom": "SBOM.spdx.json",
  "sbom_sha256": "fedcba..."
}
```

### 4.2 Extended Release Evidence (`V4_PRODUCTION_RELEASE_EVIDENCE.json`)

Provides provenance and audit trails for release records:

```json
{
  "schema_version": 1,
  "evidence_type": "v4-production-release-qualification",
  "source_sha": "42b70d98206451495d071a935bb6a52896e0292f",
  "version": "4.0.0-beta.1",
  "channel": "beta",
  "product_name": "Sky Auto Player",
  "identifier": "io.github.pumni.skyautoplayer",
  "target": "nsis",
  "install_mode": "currentUser",
  "installer": "Sky Auto Player_4.0.0-beta.1_x64-setup.exe",
  "installer_size": 12345678,
  "installer_sha256": "abcdef...",
  "updater_signature": "Sky Auto Player_4.0.0-beta.1_x64-setup.exe.sig",
  "signature_size": 512,
  "updater_signature_sha256": "123456...",
  "authenticode_mode": "production",
  "authenticode_provider": "trusted-signing",
  "approved_signer_thumbprint": "0123456789ABCDEF0123456789ABCDEF01234567",
  "observed_signer_thumbprint": "0123456789ABCDEF0123456789ABCDEF01234567",
  "updater_key_id": "F6355260A0C663D5",
  "updater_signature_status": "valid",
  "sbom": "SBOM.spdx.json",
  "sbom_sha256": "fedcba...",
  "qualification_status": "PASS"
}
```

---

## 5. Failure Recovery, Stale Candidate Purge, and Environment Cleanup

1. **Pre-Build Rejection**:
   If the Git SHA, clean worktree check, SemVer, or updater private key fails pre-flight
   validation, or if inherited signing variables (`TAURI_SIGNING_PRIVATE_KEY*`) are detected,
   no artifacts are generated. Exit code is non-zero. The working tree remains untouched.

2. **Stale Output Pre-Purge**:
   Before packaging begins, existing candidate installers, signature files, and qualification
   evidence in the resolved output directories are deterministically deleted. This guarantees
   that a failed packaging run never leaves stale artifacts that could be erroneously reused.

3. **Build or Signing Failure**:
   If Tauri build fails or the Authenticode seam fails to sign:
   - Any temporary signing environment overrides (`TAURI_SIGNING_PRIVATE_KEY*`, `SKY_AUTHENTICODE_*`)
     are removed and prior saved environment variables are restored in the `finally` block.
   - Canonical candidate installer (`.exe`), updater signature (`.exe.sig`), and unpromoted
     production qualification evidence files (`V4_QUALIFICATION_EVIDENCE.json`,
     `V4_PRODUCTION_RELEASE_EVIDENCE.json`) are purged from the staging bundle and evidence directories.
   - No release evidence is emitted.
   - Note: The orchestrator guarantees process environment variable cleanup and ensures secrets are
     never printed to stdout, stderr, or logs. It does not claim runtime memory zeroization of managed
     .NET strings, which is not guaranteed by the PowerShell/.NET CLR runtime. Key material remains
     in isolated files outside the repository.

4. **Qualification Failure & Candidate Purge**:
   If the candidate binary fails Authenticode thumbprint verification, Ed25519 Minisign
   signature verification, SBOM validation, or smoke installation:
   - The candidate binary and signature file are purged from the bundle directory.
   - Unpromoted evidence files (`V4_QUALIFICATION_EVIDENCE.json`, `V4_PRODUCTION_RELEASE_EVIDENCE.json`)
     are deleted from the evidence directory.
   - The orchestrator throws and exits with non-zero exit code, halting promotion.