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
[Verified Source Commit]
           |
           v
  (1) BUILD ONCE  ---> Single NSIS Installer Candidate (.exe) + Updater Signature (.sig)
           |
           v
  (2) QUALIFY     ---> Authenticode Production Verification + Ed25519 Minisign Verification
           |           + SBOM Generation & Verification + Bundle Verification + Smoke Test
           v
  (3) ATTEST      ---> Cryptographic Qualification Evidence + GitHub SLSA Attestation (if on-runner)
           |
           v
  (4) PROMOTE     ---> Static Metadata Generation + GitHub Releases / CDN Deployment
```

**Invariants**:
- **Zero Rebuild**: Once the candidate `.exe` and `.exe.sig` are produced, they are never
  rebuilt, re-bundled, or modified.
- **Byte-Identity**: The SHA-256 hash recorded during qualification is identical to the hash
  in `V4_QUALIFICATION_EVIDENCE.json`, `latest-v4.json`, `SBOM.spdx.json`, and the GitHub
  Release download asset.
- **Fail-Closed Verification**: If any check (Authenticode, Ed25519 Minisign, SBOM, bundle,
  smoke test) fails, the entire candidate is rejected and intermediate artifacts are purged.

---

## 2. Runner Trust Boundaries and Key Custody

### 2.1 The Custody vs. Cloud Attestation Problem

Tauri updater private keys and production Authenticode certificates must be protected
with strict physical or cryptographic access controls (FIPS 140-2 Level 2+ HSM, smartcard,
or isolated key vault). They **must never be stored as plaintext secrets in GitHub Actions
cloud repository secrets**.

Conversely, GitHub Artifact Attestations (`actions/attest@v4`) and GitHub-backed SLSA
provenance rely on GitHub's OIDC minting service, which is accessible only from an active
GitHub Actions runner environment.

### 2.2 Supported Release Execution Topologies

To resolve this boundary honestly without compromising key custody or fabricating
attestations, the v4 release architecture defines two supported execution topologies:

```text
+---------------------------------------------------------------------------------+
| TOPOLOGY A: Dedicated / Self-Hosted Single-Tenant Windows Release Runner         |
| (Automated, OIDC-Attested)                                                      |
+---------------------------------------------------------------------------------+
|                                                                                 |
|   GitHub Actions Workflow (Release Dispatch)                                   |
|     |                                                                           |
|     +---> Ephemeral / Dedicated Self-Hosted Windows Runner                      |
|            |-- Isolated hardware / local key vault / cloud KMS provider         |
|            |-- Runs `scripts/orchestrate_v4_production_release.ps1`             |
|            |     |-- Validates commit, keys, providers                          |
|            |     |-- Builds candidate with canonical public root                |
|            |     |-- Signs PE via Authenticode Provider Seam                    |
|            |     |-- Signs updater package with private key (never logged)      |
|            |     \-- Qualifies exact bytes (Authenticode + Minisign + SBOM)     |
|            |-- Runs `actions/attest` with GitHub OIDC token                     |
|            |     \-- Generates authentic SLSA provenance & SBOM attestation     |
|            \-- Runs `scripts/promote_v4_metadata.ps1`                           |
|                  \-- Publishes release assets and static metadata                |
+---------------------------------------------------------------------------------+

+---------------------------------------------------------------------------------+
| TOPOLOGY B: Air-Gapped / Maintainer Workstation Operator Execution              |
| (Offline-Custody, Cryptographically Signed, Non-OIDC)                           |
+---------------------------------------------------------------------------------+
|                                                                                 |
|   Maintainer Workstation (Offline or Private Network)                           |
|     |-- Hardware Token / HSM / Offline Air-Gapped Key                           |
|     |-- Runs `scripts/orchestrate_v4_production_release.ps1`                    |
|     |     |-- Builds candidate from verified clean git commit tag               |
|     |     |-- Signs PE and updater payload                                      |
|     |     \-- Qualifies exact bytes & emits V4_QUALIFICATION_EVIDENCE.json      |
|     |-- Cryptographic evidence uploaded alongside assets to GitHub Releases     |
|     \-- NOTE: Does NOT forge GitHub Actions OIDC attestation. Provenance is     |
|         guaranteed cryptographically by maintainer Authenticode + Ed25519       |
|         signatures and published qualification hash evidence.                   |
+---------------------------------------------------------------------------------+
```

#### Topology A (Recommended for Continuous Releases):
A single-tenant Windows release runner registered as a GitHub Actions runner for the
repository. The runner has access to the cloud signing provider (e.g., Azure Trusted
Signing, DigiCert ONE) or an attached hardware security key. Because it runs within
GitHub Actions, `actions/attest` runs directly on the exact candidate bytes produced by
the orchestrator.

#### Topology B (Fallback for Air-Gapped / Physical HSM Releases):
An operator runs `scripts/orchestrate_v4_production_release.ps1` directly on a secure
workstation. The script outputs `V4_QUALIFICATION_EVIDENCE.json` and
`V4_PRODUCTION_RELEASE_EVIDENCE.json`. The maintainer publishes the release using GitHub CLI
(`gh release create`). No synthetic GitHub provenance is claimed; the cryptographic
signatures serve as the immutable trust anchor.

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
| `-AuthenticodeProvider` | String | Yes | Authenticode provider name matching provider seam (`trusted-signing`, `digicert-one`, `custom-script`, etc.). |
| `-ApprovedSignerThumbprint` | String | Yes | Expected SHA-1 thumbprint of the production Authenticode certificate. |
| `-AuthenticodeProviderScript` | String | Mutually Exclusive | Custom provider signing script path (for `custom-script` provider). |
| `-AuthenticodeProviderCommand` | String | Mutually Exclusive | Custom provider command line (for `custom-command` provider). |
| `-BundleDir` | String | No | Target bundle directory. Defaults to `rust/target/dist/bundle/nsis`. |
| `-EvidenceDir` | String | No | Target evidence directory. Defaults to `rust/target/dist`. |
| `-SkipBuild` | Switch | No | Skip compilation step if candidate binary is already built and qualified. |
| `-SkipInstallSmoke` | Switch | No | Skip executing the installer in temporary test-user scope (default: false). |

### 3.2 Security Pre-Checks (Fail-Closed)

Before initiating any build or invoking external tools, the orchestrator validates:
1. **Commit Integrity**: Current Git HEAD matches `-ExpectedSourceSha` exactly; working tree is clean.
2. **Version Alignment**: `desktop/src-tauri/Cargo.toml` package version matches `-Version`.
3. **Channel Policy**: Beta channel requires `-beta` prerelease tag; stable channel forbids prerelease tags.
4. **Key Path Isolation**: `-UpdaterPrivateKeyPath` is checked against the repository root. If the
   file is located within the repository, the script fails immediately to prevent accidental commits.
5. **Key Validity Check**: Invokes `cargo xtask updater-trust check-key` against the private key
   *before* building. If the private key does not correspond to canonical public root
   `F6355260A0C663D5`, execution halts before building or signing anything.
6. **Provider Exclusivity**: Exactly one provider switch (`-UseHardwareToken`,
   `-UseAzureTrustedSigning`, `-UseDigiCertOne`) or explicit environment configuration is permitted.
7. **Production Mode Enforcement**: Authenticode mode is locked to `production`. Test certificates
   are rejected unconditionally.

### 3.3 Execution Workflow

```text
Step 1: Setup & Pre-Flight Checks
  - Validate Git commit SHA, SemVer, channel rules.
  - Verify updater private key matches canonical public root F6355260A0C663D5.
  - Configure production Authenticode environment variables for signCommand.

Step 2: Build & Sign Candidate (Single Build)
  - Run `bun run build` in desktop/.
  - Set `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in memory.
  - Run `bun run tauri build --ci -- --profile dist`.
  - Tauri NSIS bundler invokes `scripts/sign_v4_authenticode.ps1` via signCommand seam.
  - Tauri bundler signs the installer with the private key, generating `.exe.sig`.

Step 3: Cryptographic Qualification of Candidate Bytes
  - Authenticode verification: `scripts/verify_v4_authenticode.ps1 -Mode production -ApprovedThumbprint <THUMBPRINT>`
  - Ed25519 Minisign verification: `cargo xtask updater-trust verify-signature --installer <EXE> --signature <SIG>`
  - SBOM generation & validation: `cargo xtask sbom generate` + `cargo xtask sbom verify`
  - Bundle structure validation: `cargo xtask verify-tauri-bundle`
  - Installer smoke test: execute candidate with `/S` under `%TEMP%` test profile.

Step 4: Evidence & Manifest Generation
  - Compute SHA-256 of candidate installer and signature sidecar.
  - Generate canonical `V4_QUALIFICATION_EVIDENCE.json` (strictly adhering to the 20-field schema).
  - Generate extended `V4_PRODUCTION_RELEASE_EVIDENCE.json` (recording Git SHA, build timestamp,
    signer thumbprint, Minisign signature, SBOM digest, and qualification status).
  - Clean up sensitive in-memory environment variables and temporary files.
```

---

## 4. Evidence Schemas and Promotion

### 4.1 Canonical Qualification Evidence (`V4_QUALIFICATION_EVIDENCE.json`)

To maintain full compatibility with the existing promotion engine `scripts/promote_v4_metadata.ps1`,
the orchestrator emits the 20 required qualification fields:

```json
{
  "application_id": "io.github.pumni.skyautoplayer",
  "version": "4.0.0-beta.1",
  "channel": "beta",
  "installer_name": "Sky-Auto-Player_4.0.0-beta.1_x64-setup.exe",
  "installer_sha256": "abcdef...",
  "installer_size_bytes": 12345678,
  "signature_name": "Sky-Auto-Player_4.0.0-beta.1_x64-setup.exe.sig",
  "signature_sha256": "123456...",
  "signature_size_bytes": 512,
  "bundle_dir_verified": true,
  "bundle_file_count": 2,
  "authenticode_mode": "production",
  "authenticode_verified": true,
  "authenticode_subject": "CN=Example Production Authority",
  "authenticode_thumbprint": "0123456789ABCDEF0123456789ABCDEF01234567",
  "sbom_verified": true,
  "sbom_package_count": 42,
  "smoke_install_verified": true,
  "smoke_install_exit_code": 0,
  "smoke_installed_launcher_found": true
}
```

### 4.2 Extended Release Evidence (`V4_PRODUCTION_RELEASE_EVIDENCE.json`)

Provides provenance and audit trails for release records:

```json
{
  "pipeline": "sky-auto-player-v4-production-release",
  "schema_version": "1.0.0",
  "release_type": "production",
  "git_commit_sha": "42b70d98206451495d071a935bb6a52896e0292f",
  "build_timestamp_utc": "2026-09-05T03:00:00Z",
  "version": "4.0.0-beta.1",
  "channel": "beta",
  "installer": {
    "filename": "Sky-Auto-Player_4.0.0-beta.1_x64-setup.exe",
    "sha256": "abcdef...",
    "size_bytes": 12345678
  },
  "updater_signature": {
    "filename": "Sky-Auto-Player_4.0.0-beta.1_x64-setup.exe.sig",
    "sha256": "123456...",
    "public_key_id": "F6355260A0C663D5",
    "verified_against_canonical_root": true
  },
  "authenticode": {
    "mode": "production",
    "subject": "CN=Example Production Authority",
    "thumbprint": "0123456789ABCDEF0123456789ABCDEF01234567",
    "verified": true
  },
  "sbom": {
    "filename": "SBOM.spdx.json",
    "sha256": "fedcba...",
    "verified": true
  },
  "qualification_status": "PASSED"
}
```

---

## 5. Failure Recovery and Disaster Handling

1. **Pre-Build Rejection**:
   If the Git SHA, SemVer, or updater private key fails pre-flight validation, no artifacts
   are generated. Exit code is non-zero. The working tree remains untouched.

2. **Build or Signing Failure**:
   If Tauri build fails or the Authenticode seam fails to sign:
   - In-memory keys and environment variables are wiped immediately in the `finally` block.
   - Any partial files in the staging output directory are deleted.
   - No release evidence is emitted.

3. **Qualification Failure**:
   If the candidate binary fails Authenticode thumbprint verification, Ed25519 Minisign
   signature verification, SBOM validation, or smoke installation:
   - The candidate is deemed corrupted or compromised.
   - The staging directory is marked tainted or removed.
   - The orchestrator exits with code 1, halting promotion.