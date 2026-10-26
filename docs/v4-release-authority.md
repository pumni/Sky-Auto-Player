# V4 Release and Update Runbook

This is the current v4 release and update runbook. The supported architecture has one official
repository:

```text
pumni/Sky-Auto-Player
```

The repository contains the v4 source, immutable GitHub Releases, release provenance, and the
protected `release-metadata` deployment branch. `v4.0.1` is the first official v4 release lineage.
The earlier v4.0.0 builds were rehearsal/pre-release infrastructure and are not a supported
compatibility endpoint or migration target. There is no bridge, fallback, compatibility shim, or
permanent second-repository release topology.

## Public update channels

The Rust `UpdateService` selects exactly one of these compiled, unauthenticated raw endpoints from
the persisted channel setting:

```text
stable: https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json
beta:   https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json
```

The frontend cannot provide an endpoint, key, URL, or downgrade policy. The `release-metadata`
branch is mutable deployment state separate from immutable GitHub Release assets. Its channel files
are created or advanced by a successful qualified promotion, or by the separately protected
metadata-only recovery procedure for an already immutable, fully verified stable release; bootstrap
must not create fabricated `latest.json` files. If a channel file is missing or invalid, updater
checks fail closed.
Branch ruleset/protection and the approved workflow/maintainer promotion path ensure only qualified
releases update these endpoints.

Stable and beta are independent paths:

```text
release-metadata/
  channels/
    stable/latest.json
    beta/latest.json
```

Each promotion validates the complete Tauri metadata contract and requires a strictly greater
SemVer than the current file in that channel. Equal-version and downgrade writes are rejected.

### Beta channel operational status

The beta update channel is currently unpopulated (`channels/beta/latest.json` does not exist on
`release-metadata`). Because no v4 beta release has been qualified and published, the beta channel
is not operational. Normal users and runtime environments must not be exposed to a dead/404 channel;
the desktop update channel selector marks Beta as unavailable until a qualified prerelease is
promoted through the canonical release pipeline.

## Canonical release asset

The supported Windows package is the Tauri NSIS installer and its updater signature:

```text
Sky Auto Player_<version>_x64-setup.exe
Sky Auto Player_<version>_x64-setup.exe.sig
```

The existing release-asset normalization maps spaces to dots for the GitHub Release names:

```text
Sky.Auto.Player_<version>_x64-setup.exe
Sky.Auto.Player_<version>_x64-setup.exe.sig
```

The immutable asset URL is derived from the version and filename in the official repository:

```text
https://github.com/pumni/Sky-Auto-Player/releases/download/v<version>/Sky.Auto.Player_<version>_x64-setup.exe
```

Metadata contains the exact `.sig` text and the canonical `windows-x86_64` platform only. Portable
ZIPs, custom v3 manifests, aliases, non-HTTPS URLs, other repositories, query/fragment state,
malformed dates, extra platforms, and non-canonical artifact names are invalid v4 metadata.

GitHub Release is the end-user distribution surface and must contain exactly the two canonical
public assets: the normalized installer filename and its `.sig`. Qualification JSON, Authenticode
evidence, artifact summaries, and `SBOM.spdx.json` are internal CI evidence. They remain bound by
the candidate manifest and bounded GitHub Actions artifacts/attestations; they are never uploaded
as additional GitHub Release downloads.

## Deterministic metadata commands

The repository-owned generator and validator are exposed through the `release-metadata` command:

```text
cargo xtask release-metadata generate --channel <stable|beta> --version <semver> \
  --notes-file <path> --pub-date <rfc3339> --platform windows-x86_64 \
  --asset-url <url> --signature-file <path> --output <path>

cargo xtask release-metadata validate --channel <stable|beta> --metadata <path>

cargo xtask release-metadata validate-monotonic --channel <stable|beta> \
  --current <path> --candidate <path>
```

Stable accepts final SemVer releases; beta accepts prereleases. The validator also checks release
identity, exact platform shape, signature contents, canonical URLs, and channel policy.

## Release transaction

The production workflow `.github/workflows/release-v4.yml` is `workflow_dispatch` only and requires
canonical `main`, the latest exact-head successful CI run, and Rust Preflight. It uses GitHub-hosted
Windows and Ubuntu runners. The protected `v4-production-release` environment is referenced by the
signing and metadata jobs; updater signing secrets are mapped only into the Rust signing step. Its
transaction is:

```text
Preflight -> BuildUnsigned -> SignCandidate -> QualifyCandidate -> VerifyCandidate
          -> VerifyAttestations -> PublishRelease -> PromoteMetadata -> FinalVerify
```

The ordering is security-critical:

1. **Preflight**: Derives the release identity once from the checked-out source into the bounded
   `release-context.json` state artifact, then validates the canonical repository, `main` source ref,
   and `release-metadata` branch readiness. It is externally read-only: collision state and any
   matching stale draft are observed and classified, but not mutated.
2. **BuildUnsigned**: Builds one unsigned canonical NSIS installer with Tauri updater sidecar
   creation disabled. A typed run/attempt-bound manifest records its exact size and SHA-256. This job
   has no production secrets or write permissions.
3. **SignCandidate**: Independently verifies the downloaded artifact name, inventory, source and
   workflow SHAs, run ID/attempt, canonical filename, size, and digest. The Rust adapter verifies the
   encrypted updater key against Key ID `19AABD2E7838818C`, signs the same installer bytes, and
   verifies the unchanged installer hash and version-bound signature. The key and passphrase are
   mapped only to this signing step.
4. **QualifyCandidate**: Runs in the next step without signing secrets. It revalidates the handoff
   and signed installer, then completes unsigned Authenticode verification, Tauri updater signature
   verification, SPDX SBOM, exact bundle verification, updater fixture, Defender exact scan, and
   Windows NSIS install/launch/uninstall qualification. It freezes `candidate-manifest.json` with
   SHA-256 digests and file sizes.
5. **VerifyCandidate / VerifyAttestations**: Independently revalidates the candidate in the OIDC
   attestation job, attests the exact installer, signature, and SPDX SBOM, then verifies those
   attestations against the exact source, workflow, and candidate digests before publication.
6. **PublishRelease**: Executes a single cohesive publication transaction boundary:
   - Reconciles and deletes only a matching unpublished stale draft immediately before creating the
     new draft; mismatched or published releases fail closed.
   - Creates a draft in the official repository (`name = $Tag`, `draft = true`, `make_latest = "false"`)
     embedding a hidden machine-readable transaction marker (`<!-- v4-release-tx: {...} -->`).
   - If draft POST times out, reconciles via tag lookup and transaction marker match.
   - Uploads the canonical public installer and updater signature.
   - Queries GitHub release assets API to server-verify exact byte sizes and SHA-256 digests matching
     `candidate-manifest.json`.
   - Commits irreversible publication PATCH (`draft = false`, channel-aware `make_latest`).
   - Reconciles publication status strictly via exact `release_id` GET.
   - Fail-closed draft self-cleanup: on any failure prior to the irreversible publication PATCH,
     the transaction immediately deletes the remote draft release.
7. **PromoteMetadata**: Mints a contents-scoped App token, generates, validates, and promotes only the selected
   stable/beta metadata file after the guard.
8. **FinalVerify**: Re-fetches the public release and verifies the exact unauthenticated
   `raw.githubusercontent.com` endpoint used by the client.

Published release assets, tags, and release records are never repaired in place. A failed
unpublished draft is automatically cleaned up or recreated. A valid immutable stable release may
advance only `channels/stable/latest.json` through the explicit metadata-only recovery procedure
below. A defective published artifact still requires a greater SemVer release.

## Qualification and trust properties

The release gate preserves these properties while using one repository:

- build once, then qualify and publish the exact same bytes;
- mandatory Tauri updater signature verification;
- exact installer and signature SHA-256 evidence frozen in `candidate-manifest.json`;
- SPDX SBOM and GitHub OIDC provenance/attestation;
- immutable GitHub Release publication;
- strict stable/beta SemVer policy and monotonic metadata promotion;
- protected production environment and GitHub-hosted signing job;
- encrypted updater key and passphrase are available only as Environment secrets to the signer;
- no Windows Credential Manager or permanent self-hosted runner requirement in production;
- least-privilege permissions and `persist-credentials: false` checkout;
- fail-closed validation when metadata, release assets, signatures, or public endpoints differ.

`FinalVerify` fails closed unless the published release asset set is exactly the canonical installer
and updater signature pair. Missing, extra, aliased, or digest/size-mismatched assets are invalid.
Evidence files and SBOMs are not part of that public release set.

The current Authenticode policy is `unsigned-zero-budget`: updater cryptographic trust remains
mandatory even while Windows publisher identity is intentionally unsigned. Any future production
signer requires separate qualification and does not change the repository or metadata topology.

For runner isolation, evidence retention, and production execution topology, see
[`v4-release-execution-topology.md`](v4-release-execution-topology.md). Owner/admin actions such as
branch protection and secret custody are separate governance operations outside individual workflow
dispatches.

## Authoritative state model and recovery lifecycle

The v4 release architecture treats external GitHub release and `release-metadata` deployment branch
state as authoritative, eliminating complex mutable intermediate phase files:

### Transaction marker and candidate manifest

1. **`candidate-manifest.json`**: Frozen during `QualifyCandidate`, recording exact file paths, roles,
   sizes, and SHA-256 digests for all candidate assets and public release projections.
2. **Transaction marker**: When creating the draft, the pipeline embeds a hidden HTML comment marker
   into the release body:
   ```text
   <!-- v4-release-tx: {"repository":"...","run_id":"...","run_attempt":2,"source_sha":"...","version":"...","tag":"..."} -->
   ```
   This uniquely binds the remote draft to the specific workflow run, repository, source SHA, and tag,
   allowing deterministic reconciliation upon network timeout and safe cleanup of stale matching drafts.

### Diagnostic classifications

The diagnostic release doctor (`release-doctor`) evaluates external truth and GitHub Actions workflow
run evidence to report the observed lifecycle phases and diagnostic classification:

- **`persisted_phase`**: Monotonic phase recorded in local release state if present (`READY | QUALIFIED | PUBLISHED_PENDING_METADATA | COMPLETE | null`).
- **`external_phase`**: Monotonic phase confirmed by external truth on GitHub (`READY | QUALIFIED | PUBLISHED_PENDING_METADATA | COMPLETE | null`).
- **`classification`**: Failure or incident diagnostic classification:
  1. **`NOT_READY`**: Request or workspace prerequisites not yet satisfied (e.g., dirty working tree, uncommitted changes, missing release notes, non-canonical branch or tag).
  2. **`RECOVERABLE_PRE_PUBLICATION_FAILURE`**: Failure occurred before irreversible publication. External truth contains no published release. Any unpublished draft tag or draft release remains mutable and may be safely deleted and recreated.
  3. **`POST_PUBLICATION_INCIDENT`**: Irreversible publication occurred on GitHub, but post-publication workflow execution failed or invariants remain unsatisfied (e.g. channel metadata was not promoted).
  4. **`REMOTE_STATE_UNKNOWN`**: Publication was attempted but external state could not be verified (e.g. network partition or GitHub API unavailability). Fails closed without assuming success or failure.
  5. **`null`**: No failure or incident observed (e.g. release completed successfully or workflow is currently running in progress).

### Publication reconciliation and diagnostic invariants

1. **Exact `release_id` reconciliation only**: Following an irreversible GitHub publication PATCH attempt, the pipeline queries external truth strictly via `GET repos/$repository/releases/$($state.release_id)`. Tag equality is a verification invariant, not a transaction identifier; the pipeline never falls back to adopting a release via tag lookup. If the exact `release_id` cannot be retrieved, the pipeline classifies `REMOTE_STATE_UNKNOWN` and fails closed.
2. **Strict workflow run repository identity**: Diagnostic workflow run resolution (`release-doctor`) enforces exact repository matching (`repository.full_name == pumni/Sky-Auto-Player`). Workflow runs with missing, empty, or mismatched repository identities are refused and never implicitly trusted.
3. **Fail-closed canonical UTC RFC3339 timestamps**: Timestamps across machine-readable JSON outputs are strictly canonical UTC formatted with trailing `Z` under invariant culture. Any invalid or unparseable timestamp returns `null` and triggers operator review rather than falling back to unvalidated raw strings.

### Recovery lifecycle and governance rules

#### Fresh dispatch
Dispatched manually from `refs/heads/main` via `workflow_dispatch`. Progresses through the canonical pipeline. A fresh run refuses adoption of any existing published release.

#### Pre-publication failure (`RECOVERABLE_PRE_PUBLICATION_FAILURE`)
If a failure occurs during `Preflight`, `BuildUnsigned`, `SignCandidate`, `QualifyCandidate`, or before publication PATCH in `PublishRelease`:
- No external release is published.
- Any draft release on GitHub is automatically deleted by draft self-cleanup, or by `Preflight` on the next run.
- Recovery: Safe to re-dispatch.

#### Post-publication incident (`POST_PUBLICATION_INCIDENT`)
Once the GitHub Release PATCH is committed, the publication is irreversible:
- The release is published (`draft = false`), has an immutable `published_at` timestamp, and is marked immutable.
- The git tag points permanently to the exact source commit SHA.
- **Strict Governance Rules**:
  - **NEVER** rerun the failed workflow run (it will fail closed because published releases cannot be recreated).
  - **NEVER** delete, recreate, or replace the published git tag.
  - **NEVER** delete, edit, or replace the published GitHub release.
  - **NEVER** dispatch another release with the same version number.
  - **NEVER** manually edit `release-metadata` or invoke the normal promotion command for incident
    recovery. The only exception is the explicit metadata-only workflow documented below, with an
    operator-reviewed plan and protected Environment approval.
- **Corrective Action**:
  - The default corrective release target after engineering acceptance is a new SemVer release (`v4.1.2` for the prior v4.1.1 incident; `v4.1.3` for the current v4.1.2 incident).
  - Fix the underlying defect on a corrective branch, qualify locally, and dispatch a fresh release for the new SemVer.

#### Post-publication incident case study: v4.1.0

During the publication of `v4.1.0` (run `35255186714`, source SHA `5e5ab9d9a89aaced2af97a32c64fff21681c4c56`),
candidate qualification, draft asset upload, and exact-byte OIDC attestations succeeded, and the GitHub
Release was transitioned to published (`draft = false`, making it GitHub Latest). However, a state-object
a release workflow error terminated the then-current PowerShell transaction immediately after publication,
before metadata promotion could execute. As a result, GitHub Latest reached `v4.1.0` while
`channels/stable/latest.json` remained at `4.0.1`.

Under coordinator decision for program #322 / issue #323:
- **No manual metadata promotion**: Out-of-band manual mutation to `release-metadata` for `v4.1.0` is prohibited.
- **No immediate corrective release**: A corrective release (e.g. `v4.1.1`) is not dispatched at this time solely to reconcile metadata while v3 retirement and update UX standardization remain in flight.
- **Immutable release preserved**: `v4.1.0` and its assets remain an immutable published incident record.
- **Reconciliation deferred to final program release**: Stable metadata remains at `4.0.1` until the final stable release of program #322. That final release will execute through the canonical `release-v4.yml` pipeline, simultaneously becoming GitHub Latest and advancing stable metadata in a single transaction.

### Metadata-only recovery for a valid immutable stable release

`.github/workflows/v4-metadata-recovery.yml` supports one narrowly scoped exception when normal
publication completed but stable metadata promotion did not. The operator first dispatches a
read-only plan from `main`, reviews its artifact and plan SHA-256, then dispatches apply with that
plan run ID/hash and explicit confirmation. The protected `v4-production-release` Environment
approval is required before the write-capable App token is minted.

The `plan` and `apply` jobs each declare `actions: read`, `attestations: read`, and `contents: read`.
Apply maps the workflow token to `GH_TOKEN` for Actions run/job inspection, release and asset
reads, Contents reads, and `gh attestation verify`. The App token remains a separate
`V4_METADATA_WRITE_TOKEN` with only repository `contents: write`; Rust uses it solely for the
single canonical `PUT` to `channels/stable/latest.json`. All GitHub API reads, including
write-result readback, use the workflow token; the public stable raw endpoint is checked without
credentials. Runtime-only child environments keep the App token out of unrelated GitHub CLI and
helper processes.

The shared process adapter is implemented for both Windows and Linux production builds. Captured
stdout and stderr are each limited to 64 KiB, subprocesses have explicit deadlines, and timeout or
stranded-pipe cleanup terminates the owned process tree (a Unix process group on Linux). Asset
downloads create a new destination exclusively and remove partial bytes after command failure or
timeout. The Linux CI lane runs the Rust tests, builds the release-mode xtask, and exercises its
recovery transport against temporary fake `gh` and `curl` executables with disposable tokens. This
checks the actual Linux production subprocess path without production credentials or network access.

Rust revalidates the exact release ID, tag, transaction source SHA and run attempt, Latest identity,
exact public asset inventory and digests, production signature and version-bound trusted comment,
installer/signature provenance, SPDX SBOM provenance, stable/beta metadata, and public stable
endpoint. Every prepublication job through the exact immutable `publish` job must have completed
successfully. The original workflow may conclude `failure` only when `metadata_promote` failed or
timed out and `final_verify` was skipped; cancelled runs, failed publication, other failed jobs,
missing jobs, and ambiguous attempts fail closed. Apply binds the downloaded plan to the selected
run ID, artifact name, source SHA, attempt, and operator-reviewed SHA-256, then recomputes the plan
and checks state again at the write boundary. It can perform at most one Contents API write for `channels/stable/latest.json`;
beta metadata, releases, assets and tags are never changed. Readback, GitHub Latest, beta
immutability and public endpoint convergence must all pass. Unknown write outcomes are reconciled
with GET only and are never retried as a second mutation.

This recovery is for advancing the pointer to an already valid immutable release. It does not
support version rollback, replacing a defective installer, repairing an asset or retagging. If a
release is not fully valid, stop and use the approved incident containment procedure or prepare a
new greater SemVer release after qualification. Beta remains unavailable until separately
qualified and promoted through the normal beta release path.

Operational sequence:

1. Dispatch `.github/workflows/v4-metadata-recovery.yml` from `main` with `operation=plan`, the exact
   stable tag, and `confirm=false`.
2. Review the plan artifact and recorded `plan_sha256`. Stop if any release, Latest, digest,
   signature, provenance, channel, or endpoint fact is unexpected.
3. Dispatch `operation=apply` with the same tag, the plan run ID and SHA-256, and `confirm=true`.
   Complete the protected Environment approval.
4. Verify the successful run summary and final stable endpoint. If the run reports an unknown
   outcome, perform read-only inspection and do not repeat the write until the exact state is known.

### Diagnostic tooling

The read-only `release-doctor` queries external truth, local state, and workflow execution evidence to classify the current release:

```powershell
cargo xtask release-doctor --tag <tag> [--run-id <id>] [--workflow-sha <sha>] [--format <text|json>]
```

## Release and update domain authority ownership map

To maintain single authoritative ownership and prevent duplicated domain rules across languages,
the v4 architecture defines exact ownership boundaries:

```text
GitHub Actions = protected runner lifecycle, permissions, and attestation
cargo xtask    = deterministic release/update domain validation
PowerShell     = Windows adapters and bounded glue
Tauri Rust     = runtime updater authority
React          = presentation
```

### Responsibility breakdown

| Layer | Component / Tooling | Canonical Ownership | Non-Goals / Excluded Roles |
|---|---|---|---|
| **GitHub Actions** | `.github/workflows/release-v4.yml`, `ci.yml` | Hosted job lifecycle, exact source checkout, scoped permissions, protected environment references, artifact handoff, and OIDC attestations. | Does not define SemVer, artifact identity, release policy, metadata schemas, or transaction state transitions. |
| **cargo xtask (Rust)** | `rust/xtask/src/release_v4.rs`, `release_handoff.rs`, `release_signing.rs`, and related release modules | Single transaction authority for Preflight, BuildUnsigned, SignCandidate, QualifyCandidate, VerifyCandidate, VerifyAttestations, PublishRelease, PromoteMetadata, FinalVerify, and read-only release-doctor. Owns deterministic release policy, evidence, artifact schemas, exact-byte checks, API reconciliation, and fake-transport regressions. | Uses constrained platform adapters; does not mint GitHub workflow tokens or select the release runner. |
| **PowerShell** | `scripts/verify_v4_authenticode.ps1`, `scripts/v4_authenticode_crypto.ps1`, `scripts/scan_v4_defender_exact.ps1`, local Credential Manager helpers | Narrow Windows and provider adapters for Authenticode facts, Defender scanning, and independent local maintenance. | Does not own the production release transaction, GitHub API policy, release-doctor, state transitions, evidence schemas, or SemVer. |
| **Tauri Rust** | `desktop/src-tauri/` (`UpdateService`, `tauri-plugin-updater`) | Runtime updater state machine; minisign signature verification against compiled-in public keys; update discovery via persisted channel endpoints; safe application quiescence, note release, and restart. | Does not manage distribution metadata creation or remote releases. |
| **React / TypeScript** | `desktop/src/` (`UpdateModal`, `UpdateBanner`, `SettingsView`) | Presentation of update status, download progress, release notes, and channel selector options. | Strictly unprivileged; cannot supply custom endpoints, override public keys, bypass signatures, or initiate downgrade installations. |

### Concrete domain authority enforcement

1. **Version and Channel Authority**:
   - `cargo xtask version check [--version <semver>] [--channel <stable|beta>] [--tag <tag>] [--no-repo-match]` is the single canonical source of truth for:
     - Canonical SemVer parsing without build metadata
     - Channel prerelease classification (`stable` rejects prerelease versions; `beta` requires prerelease versions)
     - Release tag exactness (`v<version>`)
     - Cargo project version matching (`desktop/src-tauri/Cargo.toml`)
   - cargo xtask release-v4 uses the canonical cargo xtask version owner; PowerShell does not implement release policy.

2. **Isolated NSIS and Updater Qualification**:
   - `cargo xtask ci nsis smoke` and `cargo xtask ci updater-fixture run` own real package execution, restricted to a disposable hosted Windows runner or exact protected release workflow context.
   - The Rust registry owner snapshots only allowlisted product/uninstall HKCU roots, preserving absent/present state, recursive keys, parent presence, raw value kinds and bytes. It restores and verifies the snapshot even when setup or qualification fails.
   - The smoke scope routes application data and install output under an owned `RUNNER_TEMP` child, tracks child process trees, snapshots product shortcuts, attempts every cleanup stage, aggregates original and cleanup failures, and refuses recursive deletion without its exact ownership sentinel.
   - The updater fixture reuses the validated candidate and bridge bytes, serves only explicit loopback routes, verifies HTTP body length and SHA-256, checks rollback and N-to-N+1 state, and preserves a synthetic user-song sentinel through update and uninstall.
   - Local development must use synthetic isolated registry namespaces, fake files, and process fixtures; do not run NSIS on a maintainer workstation.

The script retirement and retained caller evidence is recorded in
[`powershell-dependency-ledger.md`](powershell-dependency-ledger.md).
