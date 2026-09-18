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
are created only by a successful qualified promotion; bootstrap must not create fabricated
`latest.json` files. If a channel file is missing or invalid, updater checks fail closed.
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

The production workflow `.github/workflows/release-v4.yml` is manual, runs only through the
protected `v4-production-release` environment on the dedicated signing runner, and uses the
canonical repository token for same-repository release operations. Its transaction is:

```text
Preflight -> BuildCandidate -> Attest -> PublishRelease
  -> Snapshot GitHub Latest -> Verify GitHub Latest policy
  -> PromoteMetadata -> FinalVerify
```

The ordering is security-critical:

1. **Preflight**: Validates the canonical repository, `main` source ref, and `release-metadata` branch
   readiness before any build or publication attempt. Enforces collision checks: if a published
   release or tag already exists, preflight fails closed. If a stale unpublished draft release exists
   with a matching transaction marker (`<!-- v4-release-tx: {...} -->`), preflight cleans it up.
   Pre-publication GitHub Latest baseline identity is captured.
2. **BuildCandidate**: Builds the exact source SHA once and completes full qualification against the
   local candidate bundle (unsigned-zero-budget Authenticode, Tauri updater signature verification,
   SPDX SBOM, exact bundle verification, previous-v4 E2E fixture, Defender exact scan, catalog
   verification, and active-playback update rejection). Freezes `candidate-manifest.json` with SHA-256
   digests and file sizes.
3. **Attest**: Records GitHub Actions OIDC attestations bound to the exact candidate binary, signature,
   and SBOM, and verifies attestation claims against the repository and signer workflow.
4. **PublishRelease**: Executes a single cohesive publication transaction boundary:
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
5. **Snapshot GitHub Latest & Verify GitHub Latest policy**: Verifies channel policy before minting the
   metadata App token; stable must be the exact new release and beta must leave the captured stable
   Latest unchanged.
6. **PromoteMetadata**: Mints scoped App token, generates, validates, and promotes only the selected
   stable/beta metadata file after the guard.
7. **FinalVerify**: Re-fetches the public release and verifies the exact unauthenticated
   `raw.githubusercontent.com` endpoint used by the client.

Published release assets and tags are never repaired in place. A failed unpublished draft is
automatically cleaned up or recreated; a published fix requires a greater SemVer release.

## Qualification and trust properties

The release gate preserves these properties while using one repository:

- build once, then qualify and publish the exact same bytes;
- mandatory Tauri updater signature verification;
- exact installer and signature SHA-256 evidence frozen in `candidate-manifest.json`;
- SPDX SBOM and GitHub OIDC provenance/attestation;
- immutable GitHub Release publication;
- strict stable/beta SemVer policy and monotonic metadata promotion;
- protected production environment and isolated signing runner;
- runner-local updater key outside the workspace, with no key-path workflow input;
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

1. **`candidate-manifest.json`**: Frozen during `BuildCandidate`, recording exact file paths, roles,
   sizes, and SHA-256 digests for all candidate assets and public release projections.
2. **Transaction marker**: When creating the draft, the pipeline embeds a hidden HTML comment marker
   into the release body:
   ```text
   <!-- v4-release-tx: {"repository":"...","run_id":"...","source_sha":"...","version":"...","tag":"..."} -->
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
If a failure occurs during `Preflight`, `BuildCandidate`, or before publication PATCH in `PublishRelease`:
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
  - **NEVER** manually promote `release-metadata` without an explicitly authorized, reviewed recovery procedure.
- **Corrective Action**:
  - The default corrective release target after engineering acceptance is a new SemVer release (e.g., `v4.1.1`).
  - Fix the underlying defect on a corrective branch, qualify locally, and dispatch a fresh release for the new SemVer.

#### Post-publication incident case study: v4.1.0

During the publication of `v4.1.0` (run `35255186714`, source SHA `5e5ab9d9a89aaced2af97a32c64fff21681c4c56`),
candidate qualification, draft asset upload, and exact-byte OIDC attestations succeeded, and the GitHub
Release was transitioned to published (`draft = false`, making it GitHub Latest). However, a state-object
property assignment error in `v4_release_pipeline.ps1` terminated the workflow immediately after publication,
before metadata promotion could execute. As a result, GitHub Latest reached `v4.1.0` while
`channels/stable/latest.json` remained at `4.0.1`.

Under issue #323, two recovery options are defined:
1. **Direct metadata promotion**: Advance `channels/stable/latest.json` to the already-qualified, immutable
   `v4.1.0` assets and signature using the verified candidate manifest and OIDC attestations without altering
   release assets or tags.
2. **Canonical corrective release**: Dispatch a fresh `release-v4.yml` release transaction for `v4.1.1` from
   `main` using the hardened pipeline, publishing `v4.1.1` and advancing stable metadata coherently.

### Diagnostic tooling

The read-only `release-doctor` queries external truth, local state, and workflow execution evidence to classify the current release:

```powershell
cargo xtask release-doctor --tag <tag> [--run-id <id>] [--workflow-sha <sha>] [--format <text|json>]
pwsh scripts/release_doctor.ps1 -Tag <tag> [-RunId <id>] [-WorkflowSha <sha>] [-Format <Text|Json>]
```
