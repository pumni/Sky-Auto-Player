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
  in `V4_QUALIFICATION_EVIDENCE.json`, `latest.json`, `SBOM.spdx.json`, and the exact GitHub
  Release installer/signature downloads. The public release contains only that canonical pair;
  the evidence and SBOM remain internal CI state.
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

The protected GitHub Actions workflow is the only supported production release path. It runs on the
dedicated single tenant Windows runner, checks out the exact dispatch SHA, builds one candidate,
qualifies those exact bytes, obtains OIDC attestations, publishes the immutable release, promotes
channel metadata, and performs final public verification.

The production path is workflow_dispatch from canonical main, followed by the protected runner,
Preflight, BuildCandidate, exact-source OIDC attestation verification, PublishRelease,
PromoteMetadata, FinalVerify, and runner-local cleanup.

A maintainer workstation may run Rust validation and fake-transport tests. It is not a production
release runner and has no supported local publication entry point. Authenticode and updater
signatures establish artifact integrity; production provenance comes from the protected workflow
and its exact source identity and OIDC attestations.
### 2.3 Self-Hosted Runner Hygiene Preconditions

Dedicated self-hosted Windows runners executing production releases must satisfy strict hygiene preconditions:
1. **Clean Workspace Checkout**: The runner workspace must be checked out cleanly to the exact target release commit SHA with no uncommitted modifications or untracked dirty files (`git status --porcelain` is empty).
2. **Unpolluted Environment Overrides**: The runner environment must not inherit ambient signing environment variables (such as pre-set `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PATH`, or `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Rust Preflight rejects untrusted dispatch identity, and BuildCandidate removes inherited signing overrides before it starts the Tauri build.
3. **Stale Output Purge**: Target bundle and evidence directories are purged of previous candidate installers, signatures, and evidence prior to building. Stale outputs from aborted or previous runs can never be reused.
4. **Isolated Key Storage**: Private updater keys and certificate stores must reside outside the repository tree, accessible only via restricted paths or secure hardware/cloud seams.

### 2.4 GitHub Actions Runner Isolation and Operator Requirements

The labels `self-hosted, windows, v4-release, single-tenant` identify the production signing
runner boundary. The exact approved workflow/job allowlist is:

- `.github/workflows/release-v4.yml` — job `release`, the official immutable publication path.
- `.github/workflows/rehearse-v4.yml` — job `qualification`, production pre-publication qualification only.

Both workflows are `workflow_dispatch` only, require the canonical repository's
`refs/heads/main` dispatch context, and are protected by the `v4-production-release` environment.
The qualification workflow runs the same production `Preflight` and `BuildCandidate` states on the
dedicated runner and stops before any GitHub Release, tag, metadata, or attestation mutation. It
uses the same exact runner labels and source-bound checkout as the official path; it is the
pre-publication production qualification path, not a second release state machine.
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

### 2.5 Production Passphrase Transport: Rust Windows Credential Manager Owner

Incident #140 established that parent-process environment variable inheritance does not reach Actions step processes on dedicated runner hosts (such as PUMZ). Parent-process env inheritance is explicitly **not** the production transport.

Production passphrase custody and transport is governed by the Rust Windows Credential Manager
owner in `sky_windows_security` and its `cargo xtask credential` CLI:

```text
[Secure Operator Prompt (Read-Host -AsSecureString)]
               |
               v
  (1) PROVISION SESSION CREDENTIAL
      - SecureString is sent only over child stdin to Rust
      - CredWriteW (CRED_TYPE_GENERIC, CRED_PERSIST_SESSION)
      - Fixed canonical target: SkyAutoPlayer/V4UpdaterProduction (public metadata)
               |
               v
  (2) ACTIONS DISPATCH (release-v4.yml)
      - Runs BuildCandidate on dedicated runner
      - No passphrase in workflow inputs, secrets, or runner .env
               |
               v
  (3) RUST ONE-SHOT ACTION (cargo xtask credential run-action)
      - CredReadW against the fixed production target
      - Fail-closed if absent or empty
      - Passes TAURI_SIGNING_PRIVATE_KEY_PASSWORD only to the allowlisted child action
      - Captures and redacts action output
               |
               v
  (4) ORCHESTRATION & PRE-PACKAGING VERIFIER
      - Pre-packaging verification: cargo xtask updater-trust verify-private-key
      - Canonical Public Key ID: 19AABD2E7838818C
      - Single build once and qualify exact bytes
               |
               v
  (5) MANDATORY CLEANUP (Rust action boundary)
      - Caller process environment is never modified
      - CredDeleteW runs after success, action failure, or read failure
      - Action and cleanup failures are both reported without the secret
```

**Security Invariants**:
- **No Parent-Process Inheritance**: Production does not rely on ambient shell inheritance.
- **Fixed Canonical Target**: The Credential Manager target name is `SkyAutoPlayer/V4UpdaterProduction`. This target name is public metadata committed in source; workflow dispatch never accepts arbitrary credential targets.
- **Zero Cloud Secret Exposure**: The passphrase is never stored in GitHub Secrets, runner `.env` files, command-line flags, GitHub issues/PRs/chat, or repository source files.
- **Ephemeral Session Material**: Provisioned under `CRED_PERSIST_SESSION` and deterministically deleted by the Rust one-shot action boundary.
- **Pre-Packaging Verifier**: Pre-packaging verification remains mandatory and enforces Key ID `19AABD2E7838818C`.

---

## 3. Rust Release Authority

The file rust/xtask/src/release_v4.rs is the single transaction authority. The production caller is
.github/workflows/release-v4.yml; its release steps invoke Rust phase commands with one runner-local
state root. The workflow supplies the protected runner, exact checkout, GitHub token scopes, OIDC
attestation actions, and Windows provider adapters. It does not implement release policy or make
release state transitions itself.

The authoritative sequence is:

Preflight -> BuildCandidate -> PublishRelease -> PromoteMetadata -> FinalVerify

The separate exact-source attestation check must succeed after BuildCandidate and before
PublishRelease. The rehearsal workflow runs only Preflight and BuildCandidate; it does not publish
releases, create tags, promote metadata, or mint the metadata App token.

### 3.1 State and identity

Preflight derives version, channel, tag, release notes path, source SHA, workflow SHA, and run ID
from the exact GitHub Actions context and repository contents. It requires the canonical repository,
workflow_dispatch, refs/heads/main, matching checkout and workflow SHAs, and a numeric Actions
run ID. It validates the release notes and reads release, tag, Latest, and metadata state before
writing a bounded release-context.json.

All phases reload and validate that context. Strict JSON structs reject unknown fields. The state
root must be a dedicated direct child of RUNNER_TEMP, outside the workspace. Cleanup requires the
ownership marker and refuses paths outside that boundary. No release identity or private key path is
accepted as a workflow input.

### 3.2 Phase responsibilities

- Preflight is read only against GitHub. It checks immutable release policy, stable Latest
  monotonicity, beta Latest preservation, metadata branch/bootstrap readiness, and collision
  identity. It does not delete a stale draft.
- BuildCandidate runs the Rust-owned build action once from a clean exact SHA. It validates the
  updater private key with the Rust trust owner, exposes the passphrase only to the allowlisted
  Tauri child through the Credential Manager action, and never puts it in arguments or the caller
  environment. Rust purges only the exact stale candidate outputs inside the canonical build
  directories, verifies the installer/signature pair, runs Authenticode, updater signature, SBOM,
  bundle, updater fixture, Defender, and NSIS qualification checks, then writes and validates the
  canonical qualification and production evidence. The frozen candidate manifest binds every
  artifact and evidence file to its exact bytes. Rust qualification adapters launch the current
  xtask executable directly with process-tree deadlines; they do not invoke Cargo recursively while
  the release command is running.
- PublishRelease is the only phase that creates or publishes the GitHub release. It reconciles
  only a draft with a matching transaction marker, never modifies a tag, uploads exactly the
  manifest's installer and signature, checks server size and SHA-256 values, downloads both assets
  for exact-byte comparison, and publishes only after every pre-publication check passes. After a
  publication attempt it reconciles by the exact release ID. Unknown publication status is an
  incident and never triggers cleanup.
- PromoteMetadata requires an immutable published release and revalidates its exact asset pair.
  It generates metadata in Rust, validates its schema and per-channel SemVer monotonicity, then
  updates only the canonical channel file with the contents-scoped GitHub App token.
- FinalVerify re-fetches the exact release, validates tag, target SHA, channel, immutable status,
  exact asset names, sizes and digests, then compares the validated metadata branch bytes with the
  unauthenticated raw endpoint used by the runtime updater. It records final state only after all
  checks pass.

All release API operations go through the Transport boundary. The production GhTransport restricts
endpoints to the canonical GitHub repository and upload/raw hosts, applies process deadlines with
process-tree teardown, bounds captured diagnostics, and retries only HTTP 429 and selected 5xx
responses a fixed number of times. The fake transport covers success, idempotency, conflicting
drafts, transient/permanent errors, partial upload, digest and asset mismatch, publication
ambiguity, cleanup refusal, metadata divergence, and workflow identity failures. The CI self-test
uses only this fake transport and does not call the production release API.

### 3.3 Read-only release doctor

cargo xtask release-doctor [--tag <tag>] [--version <semver>] [--channel <stable|beta>]
[--run-id <id>] [--workflow-sha <sha>] [--state-root <dir>] [--format <text|json>] is a
read-only Rust diagnostic. It validates local state with a 16 MiB input bound, queries the release,
tag, Latest, metadata Contents and raw endpoints, and can bind workflow evidence to one run ID or
exact SHA. It reports COMPLETE, POST_PUBLICATION_INCIDENT, REMOTE_STATE_UNKNOWN,
RECOVERABLE_PRE_PUBLICATION_FAILURE, or NOT_READY; it never mutates a release, tag, metadata
branch, or local state file. Malformed local or remote identity and divergent metadata bytes fail
closed.
## 4. Evidence Schemas and Promotion

### 4.1 Canonical Qualification Evidence (`V4_QUALIFICATION_EVIDENCE.json`)

Rust writes and validates the canonical qualification evidence schema, emitting the exact required fields:

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

## 5. Failure Recovery and Cleanup

1. **Preflight failure**: Invalid dispatch identity, source SHA, SemVer/channel, release notes,
   current Latest state, or metadata readiness stops before candidate build and publication.

2. **BuildCandidate failure**: No GitHub release or tag is created. The Rust owner purges only the
   exact canonical stale installer, signature, and evidence outputs before starting the build.
   The workflow cleanup step removes the owned RUNNER_TEMP state root even after an earlier step
   fails. The next build cannot reuse a prior candidate.

3. **Pre-publication failure**: PublishRelease may remove only an unpublished draft whose exact
   transaction marker matches the owned transaction. It never deletes a tag. Cleanup errors are
   reported together with the original failure. If remote state is ambiguous, or publication may
   have committed, automatic cleanup is refused and the transaction is reported as an incident.

4. **Post-publication failure**: Published releases, tags, and their assets are immutable. A
   promotion or final verification failure remains a post-publication incident; operators must not
   delete, edit, replace, or rerun that release. Any correction requires a separately reviewed
   release with a greater SemVer version.

5. **Metadata failure**: Metadata promotion runs only after immutable publication, uses the
   contents-scoped App token, and enforces per-channel monotonicity. If promotion does not complete,
   metadata remains pending and the read-only release doctor reports the state for reconciliation.
