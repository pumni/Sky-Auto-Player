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

Every production release follows one transaction from the exact clean commit selected by a
human-controlled `workflow_dispatch` on canonical `main`:

```text
prepare (main + exact-head CI + Rust preflight)
    -> build_unsigned (one unsigned NSIS installer + its x64 calibration resource, no updater sidecar)
    -> sign_qualify / sign-candidate (protected environment, key secrets on this step only)
    -> sign_qualify / qualify-candidate (no key secrets; qualify those same bytes)
    -> attest (Rust candidate verification + OIDC attest installer, signature, and SPDX SBOM)
    -> metadata_probe (Rust candidate revalidation + write-scoped App token, GET-only probe)
    -> publish (verify attestations; publish exact immutable .exe + .sig)
    -> metadata_promote (write-scoped GitHub App token)
    -> final_verify (public release, digests, GitHub Latest, stable/beta metadata)
```

The workflow uses GitHub-hosted `ubuntu-latest` and `windows-latest` runners. The one installer is
built in `build_unsigned`, transferred as a run-and-attempt-bound artifact, signed in a separate
protected job, and passed to qualification, attestation, and publication without rebuilding.

**Invariants**:
- The exact clean checkout, workflow SHA, canonical repository, workflow dispatch, `main` ref,
  run ID, run attempt, job identity, version, and canonical NSIS filename are checked by Rust and
  the workflow.
- The unsigned handoff manifest records the installer and the build-origin
  `native_calibration.exe`, including size, SHA-256, and x64 PE architecture. The run/attempt-bound
  artifact contains exactly `manifest.json`, the installer, and that calibration executable. The
  signer validates all three downloaded files before signing; qualification derives the installed
  calibration digest from those transferred bytes, so NSIS installation verifies the packaged
  resource against the Windows build output. Installer hashes remain fixed before and after
  signing, and GitHub's uploaded asset digest and read-back download must match the candidate
  manifest.
- The production updater public root remains Key ID `19AABD2E7838818C`. The current encrypted
  private key is required to match this compiled root before the signer creates a signature.
  Test-key artifacts cannot pass the production verifier.
- The published release contains exactly the canonical NSIS `.exe` and its `.sig`. Qualification
  evidence, SBOM, and manifests remain internal workflow artifacts.
- The deployed v4.2.0 client contract remains unchanged: identifier
  `io.github.pumni.skyautoplayer`, NSIS x64/currentUser package, stable metadata endpoint
  `https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json`,
  existing JSON/signature contract, and public key.
- Any failed, incomplete, mismatched, or ambiguous boundary stops the transaction. No second
  production build is used to qualify a candidate.

---

## 2. Hosted Runner Trust Boundaries and Key Custody

### 2.1 Protected Environment and Secret Scope

The current encrypted v4 updater private key and its passphrase are supplied only as GitHub
Environment secrets named `V4_TAURI_UPDATER_PRIVATE_KEY` and
`V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD`, in environment `v4-production-release`. They are not
repository or organization secrets and are absent from pull-request, fork, and general CI workflows.
The workflow references them only in the Rust `sign-candidate` step. Rust validates bounds and the
encrypted Minisign format, writes the encrypted key beneath a run-owned direct child of
`RUNNER_TEMP`, verifies that key against Key ID `19AABD2E7838818C`, and removes the temporary
material on success or in the unconditional cleanup step after failure. The passphrase is delivered
to the bounded Tauri signer process environment, never as a command-line argument or artifact.

The `sign-candidate` command exits before qualification starts. Its later Rust and PowerShell helper
processes are launched without the updater secret environment variables. The separate
`qualify-candidate` step has no signing secrets mapped to it. The protected signing job has only
`contents: read`; OIDC attestation and release publication permissions belong to different jobs.
Before either updater secret is mapped, the job builds the pinned release-mode `xtask.exe`, records
its SHA-256 and checked-out source SHA, and verifies both identities. The signing step checks the
executable digest again and invokes that binary directly; it does not invoke Cargo while signing
secrets are present. Rust removes inherited environment variables before starting Bun and keeps only
the Windows runtime paths and directories needed by the Tauri signer; the signer receives the
temporary key path and passphrase, while the original workflow secret variable names are removed.

The GitHub App private key for release-metadata access is consumed only as an input to the App token
mint action. Before publication, the workflow requests a short-lived token scoped to the canonical
repository with `contents: write`. The token action fails if the App installation cannot grant that
permission. Rust then confirms the canonical repository is accessible to the token and reads the
metadata branch, bootstrap file, and stable/beta channel files using GET requests only; this probe
does not write metadata. The job's `GITHUB_TOKEN` remains `contents: read`. After publication, a
separately minted write-scoped token is used only for metadata promotion. Neither token nor its
private key is placed in a candidate artifact.

The current Authenticode policy remains `unsigned-zero-budget`; no Windows certificate or paid
external signing service is introduced. Authenticode unsigned-state verification remains part of
production qualification. Updater signing remains enabled and uses the existing Ed25519/Minisign
trust root.

### 2.2 Readiness Gate and Hosted Release Topology

Only `.github/workflows/release-v4.yml` is the production release workflow. It declares
`workflow_dispatch` as its only trigger and has no release-version inputs. Its first job checks the
canonical repository, exact `refs/heads/main`, the default branch name, and the latest completed
push CI run for the exact dispatched SHA. Rust Preflight independently checks the clean checkout,
workflow identity, release notes, tag collisions, current GitHub Latest, and stable/beta metadata.
A `workflow_dispatch` run does not bypass the independent CI readiness gate.

The `build_unsigned` job has no production secrets, environment, write permission, or OIDC grant.
It disables Tauri build-time updater artifacts and produces exactly one canonical NSIS installer.
Its run/attempt-bound artifact contains the installer, the `native_calibration.exe` generated by
that Windows build, and a strict typed manifest. Rust enforces that exact three-file inventory and
checks the calibration executable's filename, size, SHA-256, and x64 PE machine both when creating
the handoff and after download. The `sign_qualify` job passes the validated incoming calibration
path to the existing candidate contract; it does not read a calibration executable generated in
its own workspace. The NSIS installed-resource check compares the installed file with that
build-origin digest.

After signing, the same job rechecks installer and signature bytes and performs unsigned
Authenticode verification, updater signature/version verification, SBOM generation and validation,
Tauri bundle checks, the updater fixture, Defender scan, and Windows NSIS install/launch/uninstall
qualification. No production key is supplied during qualification.

The `attest` job independently checks the signed candidate handoff and uses GitHub OIDC to attest the
exact `.exe`, `.sig`, and SPDX SBOM. The `publish` job verifies those attestations against the exact
source SHA and candidate bytes before Rust creates or publishes an immutable release. Rust checks
asset names, sizes, GitHub SHA-256 digests and downloaded bytes. `metadata_probe` must mint a
write-capable, repository-scoped App token and validate its read-only access before publication.
Candidate verification uses separate exact Rust dispatcher entrypoints: `verify-candidate` is
authorized only in `attest`, and `probe-verify-candidate` only in `metadata_probe`. The probe
revalidates the same candidate through the shared verifier before any metadata credentials are
used.
`metadata_promote` mints a fresh token only after publication, and `final_verify` fetches the release
and metadata again, including the stable raw endpoint.

### 2.3 Disposable Hosted Runner Hygiene

GitHub-hosted runners provide a fresh VM per job. Each job checks out the exact dispatched SHA with
`persist-credentials: false`, checks out the full Git history needed for clean source identity, and
keeps Rust transaction state under a unique `RUNNER_TEMP/sky-v4-release-<run-id>-<attempt>`
folder. Context and candidate handoffs are explicit workflow artifacts named with both run ID and
run attempt. Rust validates ownership, bounded JSON, exact files, and SHA-256 on each handoff.

The unsigned build process removes ambient Tauri signing and Authenticode provider overrides before
installing dependencies or building. It purges only the exact canonical stale NSIS output and
verifies the source SHA again after the build. The signer refuses ambient Tauri signing overrides,
checks the encrypted production key, and signs only the validated artifact. Workflow cleanup removes
owned state after success or failure; the signing adapter also removes its secret directory when it
returns or unwinds.

### 2.4 Hosted Release Workflow and Operator Requirements

The enforced production workflow list contains exactly:

- `.github/workflows/release-v4.yml` — canonical dispatch-only release transaction.

A separate manual, protected metadata-only operator workflow cannot build, sign, create or edit a
release, upload assets, or change a tag. It does not add a production installer build path.

The workflow jobs are `prepare`, `build_unsigned`, `sign_qualify`, `attest`, `metadata_probe`,
`publish`, `metadata_promote`, and `final_verify`. Only `sign_qualify` may reference the updater
signing secrets. Only `attest` receives `id-token: write` and `attestations: write`; only `publish`
receives repository `contents: write` and `attestations: read`. Both metadata App tokens request
`contents: write`: the pre-publication token is used only by the GET-only capability probe, and a
fresh post-publication token is used by Rust metadata promotion. The probe job's Actions token stays
read-only and publication depends on probe success.

Workflow YAML and Rust guards cannot establish that GitHub environment protection is configured.
Before any production cutover, a maintainer must inspect Settings > Environments >
`v4-production-release` and verify that deployment branches allow only `main`, protected secrets
are configured there under the documented names, and the required human review/approval rule is
active. Verify that no misspelled environment can be implicitly created without protection. This
repository review does not inspect or use the secret values, environment settings, release workflow,
release tags, or metadata branch.

The run remains human controlled: there is no automatic publish on pull request, push, or merge.
Pull-request, fork, and general CI jobs stay on ordinary unprivileged runners and never map either
updater signing secret into their workflow definitions.

Every Bun subprocess in the protected signing phase, including the Tauri CLI version probe, local
private-key challenge, and final installer signer, starts with an environment allowlist. Rust clears
the inherited child environment and forwards only the Windows runtime values, plus the temporary
key path and passphrase when the Tauri signer needs them. Git identity subprocesses use the same
runtime-only policy, including the Git queries reached from `sign-candidate` while loading and
validating the release context. A spawned fake-Git regression calls the release entrypoint with
synthetic secrets and verifies the actual child environment. The Bun regression also rejects
synthetic updater secrets, GitHub tokens, and unrelated secrets. The signing executable is built
and source/hash checked before updater secrets are injected; the secret-bearing step executes that
binary directly without Cargo. This narrows child exposure while the protected hosted job and its
ephemeral VM remain the trust boundary.

### Metadata Only Recovery

Use recovery only after an immutable stable release is valid and the normal transaction stopped
after publication. First dispatch `.github/workflows/v4-metadata-recovery.yml` from `main` with
`operation=plan`, the exact `v4.x.y` tag, and `confirm=false`. The read-only job validates the
transaction marker and exact original release workflow attempt, GitHub Latest, exact immutable
release ID/source/tag, the exact installer/signature inventory and API/local SHA-256 values, the
production updater signature and signed version, installer/signature build attestations, the SPDX
SBOM attestation, current stable/beta metadata, and the public stable endpoint. It emits a JSON plan
artifact and SHA-256; the artifact contains the proposed metadata bytes for operator review.
The plan run's complete job inventory is exactly `Validate operator dispatch=success`, `Verify
immutable release and publish read-only plan=success`, and `Protected metadata-only apply=skipped`;
all are completed records bound to that run ID, head SHA, and attempt. Duplicate, extra, missing,
or changed records are rejected. Every job in the original release attempt through `publish` must
have completed successfully. A failed overall
release workflow is eligible only when `metadata_promote` failed or timed out after successful
publication and `final_verify` was skipped. Failed or cancelled publication, other failed jobs,
missing jobs, and ambiguous attempts are rejected. Apply downloads the artifact from the selected
plan run and Rust verifies its artifact name, run ID, source SHA, attempt, and operator-reviewed
SHA-256 before it recomputes the plan.

To apply, dispatch the same workflow with `operation=apply`, the same tag, the plan run ID, the
reviewed plan SHA-256, and `confirm=true`. The `v4-production-release` Environment approval occurs
before the apply job runs. The job builds and hashes the Rust recovery executable before minting a
separate repository-scoped App token with `contents: write`. Rust then rebuilds the plan from current
GitHub state and requires the reviewed hash to match. It checks GitHub Latest, stable and beta state
again at the write boundary, and issues at most one Contents API `PUT` for
`channels/stable/latest.json`. It then verifies exact readback, unchanged beta state and GitHub
Latest, and raw stable endpoint convergence. Unknown outcomes are reconciled by read-only GETs and
never retried as a second mutation. No updater private key or passphrase is used.

GitHub Latest advances at immutable release publication; stable metadata is a separate later
write, so a temporary Latest-to-stable mismatch is inherent. The normal release workflow and the
recovery `apply` job share the `v4-release-control-plane` concurrency group. Recovery's human
Environment approval can delay apply and extend the mismatch window.

Both `plan` and `apply` explicitly declare `actions: read`, `attestations: read`, and `contents: read`.
The apply job's `GH_TOKEN` is the workflow token and authenticates every Actions run/job query,
release and asset read, Contents read, and `gh attestation verify` invocation. The App token is
kept separately as `V4_METADATA_WRITE_TOKEN`; Rust selects it only for the canonical stable
metadata Contents `PUT`. The post-write readback uses `GH_TOKEN`. The Rust process launches GitHub
CLI children with a cleared runtime-only environment and supplies only the token selected for that
operation, so the App write token is not inherited by other child commands.

The shared Rust subprocess adapter has production Linux and Windows implementations. Both cap each
captured stream at 64 KiB, enforce deadlines, drain pipes with bounded waits, and terminate owned
process trees when a command times out or leaves captured pipes open. On Linux, children start in
their own process group and timeout cleanup kills that group. File downloads use an exclusive new
destination and remove partial output after a failed or timed-out command; an existing file is
never overwritten. A descendant that deliberately creates a new session can escape the Linux
process group, so release commands must not daemonize.

The `Linux production subprocess runtime` CI job runs the xtask Rust tests on Ubuntu, builds the
non-test release binary, then invokes `ci linux-process-smoke`. That smoke starts the same release
binary with a temporary PATH containing fake `gh` and `curl` executables and synthetic credentials.
It exercises recovery reads, attestation verification, asset download, the single stable metadata
write token route, public metadata GET, token isolation, output bounds, timeout cleanup, and partial
file cleanup. It has no production secrets and cannot reach real GitHub or curl binaries. The
`Sky Auto Player — required CI gate` depends on this job and requires success for release changes;
for unrelated changes it requires the job to be skipped.

The recovery operation is not a second release lifecycle. It shares the Rust transport, metadata
schema, signature verifier, provenance checks, and monotonic version policy; it has no release
state file or release asset operation. A stale plan, Latest conflict, non-monotonic candidate,
unknown release state, invalid signature, missing attestation, changed asset, or endpoint mismatch
stops before a write. Beta remains unchanged and beta auto-update remains disabled. Do not roll back
a published release, tag, or channel version. Before immutable publication, use the normal owned
draft cleanup/re-dispatch path. After publication, either use this exact metadata-only recovery for
a valid immutable release or issue a new, greater stable SemVer after qualification; incident
containment outside these paths requires its own authorization.

### 2.5 Credential Manager Is Not a Production Release Requirement

The production hosted path does not use Windows Credential Manager, a runner-local key path, a
persistent self-hosted runner, or a pre-publication rehearsal workflow. The prior rehearsal workflow
was retired when the exact-head CI readiness gate and single build/sign/qualify/attest/publish
transaction were introduced.

The Rust credential CLI and Windows Credential Manager implementation remain in the repository for
independent local maintenance consumers. Their existence does not grant the production release
workflow a credential-manager path or alter the protected environment secret boundary. The local
credential tools are not removed in this wave.

### 2.6 Deployed v4.2.0 Compatibility Evidence

Read-only verification of the current public release found immutable release ID `419433937`, tag
`v4.2.0`, source SHA `9b4baddea9639e5c00212234c26a6b133657240d`, and exactly two public assets. The
installer is 4,459,233 bytes with SHA-256
`a6048bd8dc8691798ed2ec7078dd66abe57a5cedfd4d086eedb10cd185751105`; its 448-byte `.sig` has
SHA-256 `2f1603eb7448339a4a3bd43ec892298a1d82421a38d5bf72ceb0bc4984d678ce`. Both local hashes
matched the GitHub asset digests. Rust verified the downloaded installer signature for version
`4.2.0` using public Key ID `19AABD2E7838818C`. Stable metadata was read on 2026-10-26, names
version `4.2.0` and `pub_date=2026-10-21T10:39:42Z`, and its signature field exactly matches the
public `.sig` asset. Stable metadata remains at the canonical
`channels/stable/latest.json` endpoint with a valid `windows-x86_64` record and immutable release
URL; its downloaded JSON SHA-256 is
`cff4eb46c40a39ae912c53ba9f9264bbf0d1b0d008ced17025aee2868cfd6477`. The source still declares
identifier `io.github.pumni.skyautoplayer`, NSIS x64/current-user installation, the same stable
endpoint, and the existing compiled public key. A same-version `4.2.0 -> 4.2.0` metadata promotion
is rejected by the strict monotonic validator. The existing Windows updater fixture exercises
`4.1.3 -> 4.2.0`; a `4.2.0 -> newer` Windows fixture is not possible on this source version without
creating an unauthorized next-version candidate. Synthetic-key regressions separately reject
wrong-root, wrong-version, and tampered-byte cases.

---

## 3. Rust Release Authority

`rust/xtask/src/release_v4.rs` remains the only release transaction state machine. The workflow owns
runner selection, job dependencies, job-scoped GitHub permissions, artifact transfer, protected
Environment access, and OIDC actions. Rust owns release identity, state transitions, artifact
integrity, version/channel policy, GitHub API behavior, metadata generation, reconciliation, and
fail-closed cleanup.

The authoritative sequence is:

```text
Preflight -> BuildUnsigned -> SignCandidate -> QualifyCandidate
          -> VerifyCandidate(attest) -> VerifyCandidate(metadata_probe)
          -> VerifyAttestations -> PublishRelease
          -> PromoteMetadata -> FinalVerify
```

`BuildUnsigned` creates the only production installer and exports the calibration executable that
was generated during the same Windows build. Its same-run, run-attempt-bound unsigned artifact has
an exact three-file inventory: the canonical NSIS installer, typed manifest, and
`native_calibration.exe`. The typed manifest binds each file's name, size, SHA-256, source and
workflow SHAs, version, run ID, and attempt. `SignCandidate` validates all three downloaded files,
including the calibration PE's x64 identity and digest, then passes that transferred calibration
path into the existing candidate qualification contract. It does not read a generated calibration
file from its own checkout or rebuild either executable. The installed NSIS resource check compares
the installed calibration digest with those build-origin bytes. Calibration is qualification input;
the public release remains exactly the installer and detached `.sig` pair.

`SignCandidate` receives the encrypted key and
passphrase only in its own protected signing process; it verifies the key against the existing
public root and signs the manifest-bound installer. `QualifyCandidate` runs in the next workflow
step with no signing secret mapping. It uses the validated transferred calibration bytes to create
the candidate contract, then rehashes and verifies the same installer and signature before running
all production qualification checks. No release mutation occurs before
`PublishRelease`, and no metadata write occurs before immutable publication.

### 3.1 State and Identity

Preflight derives version, channel, tag, release notes path, source SHA, workflow SHA, repository,
run ID, and run attempt from GitHub Actions and the exact checkout. It requires the canonical
repository, `workflow_dispatch`, `refs/heads/main`, exact workflow/ref identity, the same clean
checkout and workflow SHA, and bounded numeric run/attempt identifiers. GitHub runtime values are
not accepted as workflow inputs.

Each job reloads typed, size-bounded context and checks it against the current clean checkout and
runtime job identity. Run-attempt-specific ownership markers and artifact names prevent a rerun from
consuming a prior attempt's files. The unsigned manifest binds the source and workflow SHAs, version,
run ID/attempt, producer job, canonical installer filename and digest, and calibration filename,
size, SHA-256, and x64 architecture. Later candidate records bind the exact signed installer,
signature, and qualification evidence. SHA syntax validation alone
is not checkout verification: Rust also compares the manifest SHA to GitHub identity and checked-out
HEAD at each trusted boundary.

The release state root is a direct child of the current runner's `RUNNER_TEMP`, outside the source
workspace. Cleanup requires the exact run/attempt marker and refuses any path or file inventory that
does not match its ownership boundary. Candidate files are never written into the Git checkout.

### 3.2 Phase Responsibilities

- **Preflight** is read only against GitHub. It verifies the exact latest successful main CI run,
  current release and tag state, stable GitHub Latest, metadata bootstrap and both channels, version
  monotonicity, and collision identity. It does not mutate a release or tag.
- **BuildUnsigned** strips ambient signing overrides, installs locked Bun dependencies, disables
  Tauri updater sidecar generation, invokes the pinned NSIS build exactly once, and exports the
  canonical installer and `native_calibration.exe` created by that build with their typed run-bound
  digest manifest.
- **SignCandidate** validates the downloaded manifest and exact file inventory, checks the installer
  hash before signing, verifies that the encrypted key matches Key ID `19AABD2E7838818C`, signs with
  the pinned Tauri CLI, verifies the detached signature and version, and confirms the installer hash
  is unchanged after signing. It does not run qualification helpers.
- **QualifyCandidate** revalidates the unsigned handoff and production signature, freezes the
  installer/signature pair, verifies the unsigned Authenticode policy, generates and verifies the
  SPDX SBOM, validates the Tauri bundle, runs the updater fixture and Defender scan, and performs
  Windows NSIS install/launch/uninstall qualification. It writes a candidate manifest whose records
  include exact sizes and SHA-256 values.
- **VerifyCandidate** runs as `verify-candidate` in `attest` and `probe-verify-candidate` in
  `metadata_probe`, each with an exact `GITHUB_JOB` authorization. Both entrypoints share the same
  verifier and qualified-artifact checks; the attestation job also verifies the production updater
  signature before attesting the exact installer, signature, and SBOM.
- **VerifyAttestations** runs before publication. It checks GitHub OIDC attestation signer workflow,
  source identity, subject digest, the installer/signature, and the SPDX SBOM against the candidate
  manifest. Failure cannot proceed to publish.
- **PublishRelease** is the only release mutation phase. It verifies candidate bytes and current
  metadata/Latest state again, reconciles only an exactly owned unpublished draft, refuses existing
  tags, and uploads exactly the manifest's `.exe` and `.sig`. Before the irreversible release PATCH,
  Rust reads back both draft assets, checks their exact sizes and SHA-256 values against the frozen
  candidate, and verifies the detached updater signature and version on those downloaded bytes. It
  then records `qualified_after_download=true` and attempts publication. That flag is pre-PATCH
  qualification evidence; publication is established only by exact-release-ID readback after the
  PATCH. A lost PATCH response is reconciled against that ID and is never retried as a second
  mutation or automatically cleaned up.
- **PromoteMetadata** requires the immutable release and revalidates the exact asset pair. Immediately
  before a no-op decision or Contents write, Rust fetches GitHub Latest again and requires the exact
  just-published stable release identity (or the unchanged preflight Latest for beta). It generates
  channel JSON, verifies schema and per-channel SemVer monotonicity, and updates only the canonical
  `release-metadata` channel file with the contents-scoped GitHub App token and existing blob-SHA
  compare-and-swap. An unknown write outcome is reconciled by GET and is never blindly repeated.
- **FinalVerify** fetches the exact release, GitHub Latest, stable/beta metadata Contents and raw
  endpoint, checks release identity and asset digests, and records final state only after convergence.

All GitHub release calls go through the existing Rust `Transport` boundary. It restricts repository
and upload/raw endpoints, bounds timeouts and diagnostics, retries transient GET requests only, and
never automatically repeats a mutation whose result is unknown. Fake-transport self-tests cover
success, idempotency, tag and draft collisions, stale CI/source identity, transient and permanent
errors, partial upload, digest and inventory mismatch, attestation failure, metadata App failure,
cleanup refusal, publication ambiguity, stable Latest divergence, and metadata convergence.
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

2. **Build/sign/qualification failure**: No GitHub release or tag is created. The Rust owner purges
   only the exact canonical stale installer, signature, and evidence outputs before starting the
   build. The workflow cleanup step removes the owned `RUNNER_TEMP` state root even after an earlier
   step fails. The next run cannot reuse a prior attempt's candidate.

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
