# ADR-0008: V4 GitHub Latest Policy

Status: accepted

Date: 2026-09-10

Supersedes: the temporary GitHub Latest coexistence clause in [ADR-0007](ADR-0007-single-repository-v4-release-architecture.md)

## Context

The single-repository v4 architecture initially published v4 with `make_latest="false"` so the
legacy v3.5.0 GitHub Latest release could remain discoverable during the owner/admin cutover.
That temporary coexistence policy is no longer the desired release contract for future v4
publication. GitHub Latest must now follow the v4 stable channel while beta releases remain
explicitly outside Latest.

The already published `v4.0.1` release is immutable. Its one-time promotion to GitHub Latest is
handled separately by issue #187 and must not rebuild, retag, replace assets, or rewrite release
metadata.

## Decision

The canonical same-repository release pipeline uses the GitHub Release API enum strings:

- stable releases: `make_latest="true"`;
- beta releases: `make_latest="false"`.

`draft` remains a JSON boolean. The release workflow keeps the following fail-closed order:

```text
PublishDraft
  -> Assert-ImmutableRelease
  -> channel-aware Latest policy guard
  -> mint release-metadata GitHub App token
  -> PromoteMetadata
  -> FinalVerify
```

Before publication, the workflow captures the read-only GitHub Latest identity. After publication,
the policy guard requires a stable release to be the exact published tag, source SHA, and release
identity returned by GitHub. For beta, it requires the pre-publication stable Latest identity to
remain unchanged and rejects a beta release becoming Latest. The guard uses the repository token
only for read operations; it does not mutate releases, tags, or metadata.

The CI baseline guard is transition-aware and accepts the current published stable namespace while
the one-time #187 cutover is pending. This read-only compatibility observation does not restore a
second repository, a v4 bridge, or a legacy release authority.

## Consequences

Future stable v4 releases become GitHub Latest as part of their immutable publication transaction;
beta releases never displace the stable Latest release. Metadata promotion remains after the
Latest policy guard, so a release cannot advance updater metadata until its public release
semantics have been verified.

The initial `v4.0.1` publication record remains historically accurate: it was first published with
`make_latest="false"`. Issue #187 is the only authorized one-time cutover for that immutable
release.
