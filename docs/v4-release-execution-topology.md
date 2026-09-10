# V4 Production Release Execution Topology and Candidate Lifecycle

This document specifies the release execution topology, runner trust boundary,
candidate qualification lifecycle, and provenance model for production v4 releases
of Sky Auto Player.

Governed by:
- [docs/adr/ADR-0007-single-repository-v4-release-architecture.md](adr/ADR-0007-single-repository-v4-release-architecture.md)
- [docs/adr/ADR-0006-v4-distribution-installation-update.md](adr/ADR-0006-v4-distribution-installation-update.md) (historical, superseded)
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
     (2) QUALIFY     ---> Authenticode Unsigned-State Verification + Ed25519 Minisign Verification
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
- **Fail-Closed Verification & Candidate Purge**: If any check (Authenticode state, Ed25519 Minisign,
  SBOM, bundle, smoke test) fails, the candidate binary, signature, and unpromoted evidence are
  purged from the staging directory.

---

## 2. Runner Trust Boundaries and Key Custody

### 2.1 The Custody vs. Cloud Attestation Boundary

The production v4 updater private key must remain outside the repository/workspace, encrypted at
rest, with one independent readable encrypted backup. It **must never be stored as plaintext in
GitHub Actions cloud repository secrets**. The current pre-provider `unsigned-zero-budget`
Authenticode policy is temporary; production GA should use an approved real signer when a provider
is selected and available, while PR CI remains unsigned-fixture-only.

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
|            |-- Encrypted updater-key custody outside workspace                     |
|            |-- Clean worktree pre-flight check (fails closed on dirty source)   |
|            |-- Runs `scripts/orchestrate_v4_production_release.ps1`             |
|            |     |-- Validates commit and updater key                         |
|            |     |-- Builds candidate once with canonical public root           |
|            |     |-- Authenticode-signs project PEs with approved provider at GA|
|            |     |-- Signs updater package with private key (never logged)      |
|            |     |-- Qualifies exact bytes (Authenticode state + Minisign + SBOM)|
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
|     |-- Encrypted updater key outside workspace                                |
|     |-- Runs `scripts/orchestrate_v4_production_release.ps1`                    |
|     |     |-- Builds candidate from verified clean git commit tag               |
|     |     |-- Authenticode-signs PE and signs updater payload at GA             |
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

#### Topology A (Current Pre-Provider Path / GA Target):
A dedicated, single-tenant Windows release runner registered as a GitHub Actions runner for the
repository may use the temporary `unsigned-zero-budget` Authenticode policy before GA. The GA
runner must use the approved production signer without exposing its credentials to PR CI. It still
has access to the updater key only through the
external encrypted custody boundary.
Because it executes within GitHub Actions, `actions/attest@v4` runs directly on the exact
candidate bytes produced by the orchestrator.
**Under current ADR-0006 and WO-05 acceptance contracts, Topology A is the only release
path capable of satisfying the production GitHub provenance requirement.**

#### Topology B (Documented Provenance Gap / Non-Qualifying Fallback):
An operator runs `scripts/orchestrate_v4_production_release.ps1` directly on an air-gapped
workstation. The script outputs candidate bytes, Authenticode-state evidence, updater signatures,
and local evidence. However, Authenticode and updater signatures authenticate the binary
bytes; they do **not** cryptographically prove which source commit produced those bytes.
Furthermore, `V4_PRODUCTION_RELEASE_EVIDENCE.json` is an unsigned local artifact.
**Topology B is therefore not currently accepted for production release promotion.**
Accepting Topology B in the future requires an explicit ADR amendment and a source-bound
cryptographic provenance design.

### 2.3 Self-Hosted Runner Hygiene Preconditions

Dedicated self-hosted Windows runners executing production releases must satisfy strict hygiene preconditions:
1. **Clean Workspace Checkout**: The runner workspace must be checked out cleanly to the exact target release commit SHA with no uncommitted modifications or untracked dirty files (`git status --porcelain` is empty).
2. **Unpolluted Environment Overrides**: The runner environment must not inherit ambient signing environment variables (such as pre-set `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PATH`, or `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). The orchestrator checks this pre-flight and fails closed if pre-existing keys or paths are detected.
3. **Stale Output Purge**: Target bundle and evidence directories are purged of previous candidate installers, signatures, and evidence prior to building. Stale outputs from aborted or previous runs can never be reused.
4. **Isolated Key Storage**: Private updater keys and certificate stores must reside outside the repository tree, accessible only via restricted paths or secure hardware/cloud seams.

### 2.4 GitHub Actions Runner Isolation and Operator Requirements

The labels `self-hosted, windows, v4-release, single-tenant` identify the production signing
runner boundary. The exact approved workflow/job allowlist is:

- `.github/workflows/release-v4.yml` — job `release`, the official immutable publication path.
- `.github/workflows/rehearse-v4-production-topology.yml` — job `rehearsal`, qualification-topology only.
- `.github/workflows/rehearse-v4-draft.yml` — job `draft-rehearsal`, controlled draft rehearsal only.

All three workflows are `workflow_dispatch` only, require the canonical repository's
`refs/heads/main` dispatch context, and are protected by the `v4-production-release` environment.
The draft rehearsal may create and delete only its own matching unpublished draft/tag, may create
and verify OIDC attestations for the exact downloaded candidate, and must never publish a release
or promote `release-metadata`. It uses the same exact runner labels and source-bound checkout as
the official path; it is not an official publication or metadata-promotion path.
Pull-request, fork, push, and general CI workflows must remain on ordinary hosted or non-release
runners.

The runner-local `V4_UPDATER_PRIVATE_KEY_PATH` is provisioned outside the checkout and is never a
workflow input. Release state is created under `RUNNER_TEMP`, outside `GITHUB_WORKSPACE`, and is
removed by an unconditional cleanup step after bounded JSON evidence is retained. Checkout keeps
`persist-credentials: false`; no generated token, private-key copy, or credential file may be
written into the source workspace.

The repository owner is responsible for keeping this runner dedicated to this repository's
production release environment, applying OS/toolchain security patches, and preferring an ephemeral
or regularly reimaged machine. The runner must never be registered for general public-repository CI.
If compromise is suspected, disable the runner and production environment, revoke or rotate the
updater key using [v4-updater-key-custody.md](v4-updater-key-custody.md), inspect release and
metadata history, and reimage or replace the runner before restoring release access.

### 2.5 Production Passphrase Transport: Windows Credential Manager Session Broker

Incident #140 established that parent-process environment variable inheritance does not reach Actions step processes on dedicated runner hosts (such as PUMZ). Parent-process env inheritance is explicitly **not** the production transport.

Production passphrase custody and transport is governed by the Windows Credential Manager generic session broker:

```text
[Secure Operator Prompt (Read-Host -AsSecureString)]
               |
               v
  (1) PROVISION SESSION CREDENTIAL
      - CredWriteW (CRED_TYPE_GENERIC, CRED_PERSIST_SESSION)
      - Fixed canonical target: SkyAutoPlayer/V4UpdaterProduction (public metadata)
               |
               v
  (2) ACTIONS DISPATCH (release-v4.yml)
      - Runs BuildCandidate on dedicated runner
      - No passphrase in workflow inputs, secrets, or runner .env
               |
               v
  (3) BROKER CONSUMPTION (v4_updater_credential_broker.ps1)
      - CredReadW against fixed target SkyAutoPlayer/V4UpdaterProduction
      - Fail-closed if absent or empty
      - Set TAURI_SIGNING_PRIVATE_KEY_PASSWORD at Process scope only
               |
               v
  (4) ORCHESTRATION & PRE-PACKAGING VERIFIER
      - Pre-packaging verification: cargo xtask updater-trust verify-private-key
      - Canonical Public Key ID: 19AABD2E7838818C
      - Single build once and qualify exact bytes
               |
               v
  (5) MANDATORY CLEANUP (finally block)
      - Restore / clear process-scoped TAURI_SIGNING_PRIVATE_KEY_PASSWORD
      - CredDeleteW against SkyAutoPlayer/V4UpdaterProduction
      - Clear managed references
```

**Security Invariants**:
- **No Parent-Process Inheritance**: Production does not rely on ambient shell inheritance.
- **Fixed Canonical Target**: The Credential Manager target name is `SkyAutoPlayer/V4UpdaterProduction`. This target name is public metadata committed in source; workflow dispatch never accepts arbitrary credential targets.
- **Zero Cloud Secret Exposure**: The passphrase is never stored in GitHub Secrets, runner `.env` files, command-line flags, GitHub issues/PRs/chat, or repository source files.
- **Ephemeral Session Material**: Provisioned under `CRED_PERSIST_SESSION` and deterministically deleted by the `finally` block of `BuildCandidate`.
- **Pre-Packaging Verifier**: Pre-packaging verification remains mandatory and enforces Key ID `19AABD2E7838818C`.

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
| `-AuthenticodeProvider` | String | No | Retained optional input for a future signer seam; unused by the current zero-budget policy. |
| `-ApprovedSignerThumbprint` | String | No | Retained optional future-signer input; not required or recorded for current production. |
| `-AuthenticodeProviderScript` | String | No | Retained optional future-signer input; not invoked by current production. |
| `-AuthenticodeProviderCommand` | String | No | Retained optional future-signer input; not invoked by current production. |
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
5. **Channel Policy**: Aligned with the `release-metadata` contract (`stable` requires non-prerelease
   SemVer; `beta` requires prerelease SemVer).
6. **Key Path Isolation**: `-UpdaterPrivateKeyPath` is verified to reside outside the repository tree.
7. **Key Validity Check**: Invokes `cargo xtask updater-trust verify-private-key` against the private key
   *before* building. Key ID is dynamically derived from canonical public root.
8. **Zero Secret Output**: The orchestrator never prints or leaks the updater key passphrase or key material to
   stdout, stderr, or log streams under any execution or error path.
9. **Authenticode Policy**: The current pre-provider mode is `unsigned-zero-budget`; the signer
   seam performs no signing and qualification accepts only the actual unsigned state. At GA, the
   approved provider must sign before exact-artifact qualification. Test/self-signed and unexpected
   signed states are rejected.
10. **Stale Output Purge**: Automatically purges any pre-existing installer candidate, `.sig` file,
    and qualification evidence from the resolved target bundle and evidence directories before packaging.

### 3.3 Execution Workflow

```text
Step 1: Setup & Pre-Flight Checks
  - Assert environment is unpolluted by pre-existing signing environment variables.
  - Validate parameters, commit SHA, clean worktree, version, channel rules.
  - Purge stale candidate and evidence files from resolved output directories.
  - Verify updater private key matches canonical public root via `cargo xtask updater-trust verify-private-key`.
  - Configure the temporary `SKY_AUTHENTICODE_MODE=unsigned-zero-budget` for signCommand until a
    production provider is selected; no production signing inputs belong in PR CI.

Step 2: Build & Sign Candidate (Single Build)
  - Run `bun install --frozen-lockfile` and `bun run build` in desktop/.
  - Set `TAURI_SIGNING_PRIVATE_KEY` to the validated private key file path (never raw key bytes).
  - Run `bun run tauri build --ci -- --profile dist`.
  - Tauri NSIS bundler invokes `scripts/sign_v4_authenticode.ps1` via signCommand seam.
  - Before provider onboarding, the seam deliberately performs no Authenticode signing; Tauri signs the updater artifact with
    the updater key, generating `.exe.sig`.
  - Assert working tree remains clean post-build (`git status --porcelain`); fail closed and purge
    candidate if any tracked file or manifest was mutated during packaging.

Step 3: Exact Candidate Verification
  - Check installer candidate and signature exist and are non-empty.

Step 4: Authenticode Verification
  - Run `scripts/verify_v4_authenticode.ps1 -Mode unsigned-zero-budget` for the final installer.
  - Require `NotSigned` and no signer certificate for every project-owned shipped PE; fail closed
    on test/self-signed, partially signed, or other unexpected state.

Step 5: Tauri Updater Signature Verification against Canonical Root
  - Run `cargo xtask updater-trust verify-signature --installer <EXE> --signature <SIG>`.
  - Cryptographically verifies Ed25519 signature against canonical public root.

Step 6: SPDX SBOM and Bundle Verification
  - Run `cargo xtask sbom generate` and `cargo xtask sbom verify`.
  - Run `cargo xtask verify-tauri-bundle`.

Step 7: Install Smoke Test
  - Install silently (`/S`) under `%TEMP%` test profile.
  - Verify launcher executable exists and verify every installed project PE is Authenticode-unsigned.
  - Launch application process, monitor execution, and stop.
  - Run uninstaller (`/S`) and verify clean removal.

Step 8: Evidence Emission & Self-Validation
  - Generate canonical `V4_QUALIFICATION_EVIDENCE.json` (20 fields).
  - Immediately validate emitted evidence with `scripts/promote_v4_metadata.ps1 -ValidateEvidence`.
  - Generate extended `V4_PRODUCTION_RELEASE_EVIDENCE.json`.
```

### 3.3 Dedicated v4 release state machine

The manual production entry point is `.github/workflows/release-v4.yml`. It
accepts an explicit version, channel, `v<version>` tag, source SHA, notes path,
and UTC publication timestamp. The dedicated runner supplies
`V4_UPDATER_PRIVATE_KEY_PATH` from runner-local process configuration; this is
not a workflow input. The workflow
requires the checked-out commit and workflow SHA to equal the requested source
SHA, then executes these fail-closed states:

```text
ValidateRequest -> ValidateAuthority -> BuildCandidate -> CreateDraft
  -> DownloadDraft -> QualifyDownloaded -> RecordAttestations
  -> PublishDraft -> PromoteMetadata -> FinalVerify
```

Only `BuildCandidate` calls
`scripts/orchestrate_v4_production_release.ps1`, exactly once. `BuildCandidate`
acquires the updater key passphrase through the fixed Windows Credential Manager
session broker (`scripts/v4_updater_credential_broker.ps1`, target `SkyAutoPlayer/V4UpdaterProduction`)
into process-scoped `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, passing no plaintext password
CLI arguments, and automatically deletes the session credential in its `finally` block.
The candidate manifest is the only asset upload authority. The following states download the
draft assets again and compare names, sizes, and SHA-256 digests before running
the Authenticode `unsigned-zero-budget`, Tauri updater signature, SPDX SBOM,
exact-bundle, current-user install/launch/uninstall, GUI/input-safety, and
active-playback rejection checks. The packaged official Tauri updater fixture
runs at this post-download boundary: a throwaway previous-v4 bridge consumes
the exact downloaded installer and `.sig`, while the production candidate is
never rebuilt. The packaged candidate also proves that update admission is
rejected while playback is active.

Before creating a new RC, the exact production qualification topology can be
rehearsed without creating a GitHub Release draft. The manual workflow
`.github/workflows/rehearse-v4-production-topology.yml` must be dispatched from
the ref containing the exact requested source SHA. It uses the same dedicated
runner labels and explicit updater-key path as production, builds one candidate,
then runs the script below. It has no release mutation or metadata promotion state.

After that topology gate passes, the controlled same-repository draft rehearsal
may exercise the GitHub draft boundary without publishing it. The manual workflow
`.github/workflows/rehearse-v4-draft.yml` uses the same exact source binding, protected
environment, dedicated runner, and concurrency group as the official release workflow.
It executes `ValidateRequest -> ValidateRepository -> BuildCandidate -> CreateDraft
-> DownloadDraft -> QualifyDownloaded -> RecordAttestations`, then deletes only the
matching unpublished draft/tag. Its OIDC and SPDX attestations are verified against the
exact downloaded bytes before `RecordAttestations`; `PublishDraft`, metadata promotion,
and final public-release verification are intentionally unreachable in this workflow.
The rehearsal snapshots and compares the current GitHub Latest identity plus both public
`release-metadata` channel endpoints before and after cleanup. It does not publish or promote
anything, so it remains valid both before and after the one-time `v4.0.1` Latest operation.

```powershell
pwsh scripts/test_v4_production_topology_rehearsal.ps1 -CandidateStateRoot $candidateStateRoot -StateRoot (Join-Path $env:RUNNER_TEMP "sky-v4-production-topology-rehearsal") -Version $version -Channel $channel -Tag "v$version" -SourceSha $sourceSha -WorkflowSha $sourceSha
```

The rehearsal stages the already-qualified candidate bytes under the same
`downloaded` state layout and invokes
`v4_release_pipeline.ps1 -State QualifyDownloaded` with the production
identity parameters. Its throwaway updater bridge builds into a separate
`FixtureTargetDir\dist\bundle\nsis`; it never searches for a downloaded
candidate inside that build output.

The same post-download qualification invokes a bounded Windows Defender
custom scan against the exact downloaded installer and records the artifact
name, size, SHA-256, scan result, and Defender status. This is a mandatory
production gate: missing Defender cmdlets, disabled protection, scan failure,
or a detection fails closed; no `unavailable` result is promotable.

Attestation happens on those downloaded bytes with GitHub OIDC and is verified
against the source repository, this workflow, the exact workflow SHA, and the
candidate subjects. Publication changes only the draft flag. Metadata promotion
is unreachable until publication returns a non-draft release, and final
verification re-fetches both public assets and the selected stable/beta
metadata path. An unpublished draft that fails qualification may be deleted
and recreated with the same version after the candidate is fixed. Published
assets, release tags, and metadata remain immutable; fixes then require a new
SemVer/RC.

The production preflight validates the canonical `pumni/Sky-Auto-Player` repository,
the requested `main` source, immutable-release policy, and readiness of the
`release-metadata` branch before `CreateDraft`. Bootstrap is a separately reviewed
owner/maintainer operation and never creates fabricated channel files. The production
workflow uses the same-repository `GITHUB_TOKEN` with job-scoped write permissions;
there is no cross-repository release token. Metadata promotion is a post-publication
operation and is strictly monotonic per channel. `FinalVerify` checks the public
release and the unauthenticated raw endpoint used by the runtime updater.

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
  "authenticode_mode": "unsigned-zero-budget",
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
  "authenticode_mode": "unsigned-zero-budget",
  "authenticode_state": "unsigned",
  "authenticode_provider": "none",
  "approved_signer_thumbprint": null,
  "observed_signer_thumbprint": null,
  "updater_key_id": "19AABD2E7838818C",
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
   If Tauri build fails or the Authenticode-state seam fails to qualify:
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
   If the candidate binary fails Authenticode unsigned-state verification, Ed25519 Minisign
   signature verification, SBOM validation, or smoke installation:
   - The candidate binary and signature file are purged from the bundle directory.
   - Unpromoted evidence files (`V4_QUALIFICATION_EVIDENCE.json`, `V4_PRODUCTION_RELEASE_EVIDENCE.json`)
     are deleted from the evidence directory.
   - The orchestrator throws and exits with non-zero exit code, halting promotion.
