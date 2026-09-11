# V4 Release and Update Runbook

This is the current v4 release and update runbook. The supported architecture has one official
repository:

```text
pumni/Sky-Auto-Player
```

The repository contains the v4 source, immutable GitHub Releases, release provenance, and the
`release-metadata` deployment branch, which must be protected before production cutover.
`v4.0.1` is the first official v4 release lineage.
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
`latest.json` files. Until the first qualified promotion, a missing channel file fails closed.
Branch ruleset/protection and the approved workflow/maintainer promotion path are owner/admin
prerequisites tracked by issue #165; no official v4.0.1 dispatch is allowed before that gate passes.

Stable and beta are independent paths:

```text
release-metadata/
  channels/
    stable/latest.json
    beta/latest.json
```

Each promotion validates the complete Tauri metadata contract and requires a strictly greater
SemVer than the current file in that channel. Equal-version and downgrade writes are rejected.

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
ValidateRequest -> ValidateRepository -> BuildCandidate -> CreateDraft
  -> DownloadDraft -> QualifyDownloaded -> RecordAttestations
  -> Snapshot GitHub Latest -> PublishDraft -> Assert-ImmutableRelease
  -> Latest policy guard -> PromoteMetadata -> FinalVerify
```

The ordering is security-critical:

1. Validate the canonical repository, `main` source ref, and `release-metadata` branch readiness
   before creating a draft; immutable status is verified on the published release itself.
2. Build the exact source SHA once and create a draft in the official repository with
   `make_latest="false"`; the channel-specific Latest value is applied only at publication.
3. Re-download the draft installer and signature, then qualify those exact bytes.
4. Record SBOM and provenance/attestation evidence bound to the exact source and the two public
   assets; retain internal qualification evidence through bounded CI artifacts/attestations.
5. Snapshot the current GitHub Latest identity, then publish the already-qualified draft
   immutably with `make_latest="true"` for stable or `make_latest="false"` for beta.
6. Verify the channel-aware Latest policy before minting the metadata App token; stable must be
   the exact new release and beta must leave the captured stable Latest unchanged.
7. Generate, validate, and promote only the selected stable/beta metadata file after the guard.
8. Re-fetch the public release and verify the exact unauthenticated `raw.githubusercontent.com`
   endpoint used by the client.

Published release assets and tags are never repaired in place. A failed unpublished draft may be
recreated after correction; a published fix requires a greater SemVer release.

## Qualification and trust properties

The release gate preserves these properties while using one repository:

- build once, then qualify and publish the exact same bytes;
- mandatory Tauri updater signature verification;
- exact installer and signature SHA-256 evidence;
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

For runner isolation, evidence retention, rehearsal, and owner/admin cutover requirements, see
[`v4-release-execution-topology.md`](v4-release-execution-topology.md) and issue #165. Owner/admin
actions such as branch protection, secret removal, repository deletion, and the final v4.0.1
dispatch are not performed by this runbook.
