# ADR-0007: Single-Repository V4 Release and Update Architecture

Status: accepted

Date: 2026-09-09

Supersedes: [ADR-0006](ADR-0006-v4-distribution-installation-update.md) for the v4 release and
update topology.

## Context

ADR-0006 separated v4 releases from the legacy v3 GitHub Latest namespace by introducing a second
release repository. That design was useful for pre-release qualification and rehearsal, but it made
the supported architecture depend on a second repository, a cross-repository credential, a second
publication authority, and a compatibility boundary that would have to be maintained indefinitely.

The implementation work completed for the first official v4 release now provides the required
isolation without a second repository. The source repository can publish v4 releases with
channel-specific GitHub Latest behavior, and the v4 runtime can consume a separate protected
metadata branch in that same repository. The temporary v3 Latest coexistence policy is retired by
[ADR-0008](ADR-0008-v4-github-latest-policy.md); the one-time promotion of the already immutable
`v4.0.1` release remains a separate owner/admin gate in issue #187.

## Decision

`pumni/Sky-Auto-Player` is the only active repository for v4 source, GitHub Releases, updater
metadata, provenance, and release documentation.

`pumni/Sky-Auto-Player-Releases` was pre-release/rehearsal infrastructure only. It is not part of
the supported production architecture and must not be used as a v4 compatibility endpoint.

`v4.0.1` is the first official v4 release lineage. There is no supported bridge from v4.0.0
rehearsal builds, no compatibility shim, no dual endpoint, and no permanent second-repository
topology.

V4 update deployment state is held on the protected `release-metadata` branch in the official
repository:

```text
https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json
https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json
```

The branch contains mutable stable/beta channel pointers; GitHub Release assets and tags remain
immutable. Channel files are created only by a qualified post-publication promotion, and promotion
is strictly monotonic by SemVer. Bootstrap does not fabricate `latest.json` files.

## Release transaction

The same-repository production pipeline is draft-first and builds the exact source SHA once:

```text
ValidateRequest -> ValidateRepository -> BuildCandidate -> CreateDraft
  -> DownloadDraft -> QualifyDownloaded -> RecordAttestations
  -> PublishDraft -> Assert-ImmutableRelease -> Latest policy guard
  -> PromoteMetadata -> FinalVerify
```

`ValidateRepository` checks the canonical repository, `main`, and metadata-branch readiness before
`CreateDraft`. Stable publication uses the canonical repository `GITHUB_TOKEN` with
`make_latest="true"`; beta publication uses `make_latest="false"`. The read-only Latest policy
guard runs after publication and immutable verification, and before minting the metadata App
token. Metadata promotion occurs only after that guard. `FinalVerify` checks the public release
and the exact unauthenticated raw metadata endpoint used by the client.

## Security properties retained

Simplifying repository topology does not weaken the release boundary. The architecture retains:

- exact-byte qualification after draft asset re-download;
- mandatory Tauri updater signature verification;
- SHA-256 evidence for the installer and signature sidecar;
- SPDX SBOM and source-bound provenance/attestations;
- immutable published releases and tags;
- protected `v4-production-release` environment and isolated signing runner;
- least-privilege GitHub Actions permissions with credentials not persisted in the workspace;
- fail-closed canonical repository, channel, platform, URL, signature, and SemVer validation.

The updater private key remains outside the repository and runner workspace. The updater runtime
does not accept endpoint overrides or a release-repository fallback. Authenticode policy remains
the separately documented `unsigned-zero-budget` policy; it is independent of updater signature
authorization.

## Consequences

The supported release path has one repository identity, one release API authority, and one clearly
bounded metadata branch. Operators no longer need a dedicated release-authority token or a runtime
compatibility path. The former repository can be removed as an owner/admin cutover action after the
rehearsal and final release gate in issue #165.

The temporary v3 Latest coexistence namespace is historical context only. ADR-0008 defines the
current channel-aware Latest policy, while issue #187 separately gates the one-time promotion of
the immutable `v4.0.1` release without rebuilding or changing its tag, assets, or metadata.
