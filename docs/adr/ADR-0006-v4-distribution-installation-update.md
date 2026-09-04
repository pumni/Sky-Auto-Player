# ADR-0006: V4 Clean Distribution, Installation, and Update Boundary

Status: accepted

Date: 2026-09-03

## Context

The v3 product intentionally uses an unsigned portable Windows ZIP, a repository-owned Rust updater,
PEP 440 release ordering, a signed custom `MANIFEST.json`, per-file transactional replacement, and
GitHub Releases discovery. That design is hardened for a portable application, but it also makes the
project responsible for packaging, update discovery, updater bootstrap, archive extraction,
transaction recovery, and Windows trust behavior.

The v4 desktop is a clean product boundary. It is not required to preserve the v3 portable update
protocol or support an in-place v3 -> v4 update. V4 should use the standard Tauri desktop distribution
path where it is suitable and retain project-owned code only where it represents product policy or
qualification rather than framework packaging machinery.

A clean break is also required at the release namespace. Existing v3 clients discover stable updates
from the repository's GitHub `/releases/latest` endpoint and accept a greater PEP 440 version before
checking for the v3 canonical ZIP/manifest asset contract. Publishing a normal v4 stable release into
that same discovery namespace would cause v3 clients to select v4 and then fail because the v3 assets
are intentionally absent.

## Decision

### 1. V4 is not an in-place update from v3

V4 does not participate in the v3 portable updater protocol. V3 and v4 may coexist on the same host.
V4 may later offer an explicit application-level importer for selected v3 user data, but that importer
is not an updater migration and must not require the v3 update protocol.

### 2. V3 and v4 use separate distribution/update namespaces

The v3 GitHub Releases namespace remains legacy/frozen for v3 clients. V4 binary releases and updater
metadata must be published through a separate release authority so that a v4 release cannot become the
value returned to v3 clients by their existing `/releases/latest` discovery path.

The preferred topology is:

- source repository: `pumni/Sky-Auto-Player`;
- v3 legacy releases: existing releases in the source repository;
- v4 binary release authority: `pumni/Sky-Auto-Player-Releases`, a dedicated immutable release namespace;
- v4 updater metadata: static Tauri updater metadata owned by the v4 release authority.

The authority repository exists and is reviewed by WO-04. Runtime code hard-codes only the two
allow-listed metadata paths below; it does not expose them to React or accept endpoint overrides:

```text
https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/stable/latest.json
https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/beta/latest.json
```

### 3. The canonical v4 Windows distribution is NSIS, per-user, and signed

V4 uses the Tauri bundler with the Windows NSIS target. The default supported installation scope is
current-user so normal installation and update do not require Administrator privileges and can live
under `%LOCALAPPDATA%`.

V4.0 does not ship a portable ZIP as a second canonical product. A portable build may be reconsidered
later as a secondary/manual distribution only if demonstrated user demand justifies its lifecycle and
support cost. A future portable artifact must not force the canonical installed application back onto
the v3 custom updater architecture.

MSI is not a v4.0 target. It can be added later if managed/enterprise deployment becomes a real use
case.

### 4. Tauri owns packaging and the standard updater mechanism

The v4 Tauri configuration enables bundling and updater artifacts. The canonical build is produced by
Tauri tooling rather than a repository-owned ZIP assembler.

Conceptually:

```json
{
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "createUpdaterArtifacts": true
  }
}
```

V4 uses the official Tauri updater plugin for update download, cryptographic update verification, and
installer execution. The production v4 runtime must not depend on `sky_updater`.

The React frontend must not own updater endpoints, trusted keys, arbitrary URLs, installer paths,
downgrade policy, or updater headers. A narrow Rust application service owns update policy and exposes
bounded DTO/events to the UI.

### 5. V4 uses a new permanent application identity

The permanent identifier is:

```text
io.github.pumni.skyautoplayer
```

The identifier is intentionally not versioned. Once v4 ships, changing the application identifier is a
breaking distribution decision and requires a separate ADR.

### 6. V4 version semantics are SemVer only

V4 package and updater versions use SemVer. Supported release forms include:

```text
4.0.0-alpha.1
4.0.0-beta.1
4.0.0-rc.1
4.0.0
4.0.1
4.1.0
```

Git tags may retain the conventional `v` prefix (`v4.0.0`), but canonical package/updater versions do
not. V4 runtime code does not retain the PEP 440 compatibility parser for update ordering.

### 7. Installation files and user data have separate ownership

The installer owns the complete installation root. V4 does not store mutable user-owned files beside
the executable.

Application state belongs in OS-appropriate application-data locations. Examples include settings,
logs, cache, calibration state, and local database/index state. User song files remain user-owned and
should be referenced/imported through the library contract rather than requiring an updater preserve
list inside the installation directory.

A production `.env` file beside the executable is not part of the v4 user configuration model.

### 8. V4 has two independent signing boundaries

Windows code signing and updater signing are separate requirements:

1. Project-owned Windows executables and the canonical installer are Authenticode signed so Windows
   and users receive a stable publisher identity.
2. Tauri updater artifacts use the Tauri updater signing mechanism and a new v4 update trust root.

V4 does not reuse the v3 `release-2026` custom manifest signing key or its signature envelope.

The v4 public root and reviewed overlap/cutover fixture are implemented in
`v4-tauri-packaging.md`. Before v4 GA, the release operator must keep the
matching private key in an offline encrypted vault with a separately controlled
backup. Loss requires a new root and version; compromise requires removing the
old root through the reviewed bridge/cutover sequence and publishing a new
version. No private key is recoverable from the repository, CI artifacts, or
logs.

### 9. Static updater metadata is the initial channel mechanism

V4 uses static Tauri updater metadata rather than runtime scanning/sorting of GitHub Releases. Stable
and beta, when enabled, have separate metadata authorities/endpoints. The Rust update service selects
from an allow-listed, compiled channel configuration; the frontend cannot supply an endpoint.
WO-04 validates that metadata points only to already-published, qualified assets in the dedicated
authority and promotes stable/beta files as separate post-qualification actions. Qualification
evidence binds the exact canonical installer and updater signature bytes by SHA-256; promotion
compares those digests with the published assets and fails closed on missing or mismatched evidence.
Stable metadata accepts only final SemVer releases while beta metadata is explicitly prerelease-only.
The authority may remain empty until the first qualified promotion, so a missing production metadata
file fails closed.

A dedicated dynamic update service is a non-goal for v4.0. It can be introduced later if the product
needs staged rollout, cohorts, mandatory minimum versions, or server-driven release policy.

### 10. `xtask` remains a qualification tool, not the canonical packager

The repository-local Rust `xtask` remains valuable for deterministic checks and release qualification,
but it no longer owns portable production assembly for v4.

V4 `xtask` responsibilities should include, as appropriate:

- tag/package/version consistency;
- repository and dependency-policy checks;
- packaged application smoke tests;
- installer/update artifact existence and structure checks;
- Authenticode verification;
- updater signature/metadata validation;
- fresh-install qualification;
- previous-v4 -> candidate-v4 update qualification;
- SBOM/provenance verification;
- release-state invariants.

### 11. Release publication remains exact-artifact and draft-first

V4 preserves the strongest v3 release discipline:

1. build the exact candidate once;
2. sign and verify it;
3. create release provenance and SBOM evidence;
4. attach artifacts to a draft release;
5. qualify the exact draft assets without rebuilding;
6. publish those same assets;
7. make the published release immutable;
8. promote stable/beta update metadata only after qualification/publish succeeds.

A failed qualification produces a new version/RC. Published artifacts and tags are never repaired in
place.

### 12. SBOM is a first-class release artifact

V4 generates a standard SBOM (SPDX JSON or CycloneDX JSON) and attests it alongside build provenance.
The existing cargo-vet policy evidence remains useful but does not replace an SBOM.

## Runtime update policy

The application may check for updates in the background according to user settings. It must not begin
installation while playback is active.

Before an update installation causes the Windows application to exit, the Rust application boundary
must leave the input subsystem safe: stop playback, terminate or quiesce the playback worker, release
all injected keys, persist required settings/state, and close bounded native resources. This safety
boundary must be covered by packaged tests.

Downgrades are rejected by default. A future downgrade/recovery policy requires explicit design and
qualification rather than a permissive version comparator.

## V3 retirement boundary

The following v3 concepts are legacy and are not architectural requirements for the production v4
runtime:

- `Sky-Auto-Player-Updater.exe`;
- custom signed `MANIFEST.json` / `MANIFEST.json.sig` update protocol;
- updater-owned ZIP download/extraction;
- updater preserve lists for `config.json`, `.env`, `songs/**`, and `logs/**`;
- `.sky-update-transaction` and per-file replacement/rollback journal;
- update-run/update-lock/active-update state used by the v3 updater;
- GitHub release scanning as the update oracle;
- PEP 440 update ordering.

These may remain temporarily in source while the v4 implementation is developed, but v4 release
qualification must prove that the canonical v4 product no longer depends on them before deletion.

## Non-goals

- automatic in-place v3 -> v4 update;
- preserving the v3 updater asset contract in v4 releases;
- shipping both NSIS and MSI at v4.0;
- shipping a portable build as a canonical v4.0 product;
- building a custom dynamic update backend for v4.0;
- implementing full TUF metadata/roles for v4.0;
- exposing raw Tauri updater authority to frontend code;
- re-implementing v3 per-file rollback semantics on top of the standard installer.

## Consequences

V4 gains a smaller product-owned update attack surface, standard Windows installation/uninstallation,
a publisher identity, standard Tauri updater artifacts, clearer install/data ownership, and simpler
runtime update code.

The project accepts a deliberate compatibility break with v3 and the operational requirement to keep
v3 release discovery isolated from v4. It also accepts that recovery from a bad v4 release is normally
performed by publishing a corrected greater SemVer release rather than automatically restoring an
arbitrary previous installation at per-file granularity.

## Required implementation gates before `v4.0.0-beta.1`

- dedicated v4 binary release/update namespace exists;
- permanent v4 application identifier is finalized;
- Tauri bundling is enabled with NSIS and updater artifacts;
- production Rust update service wraps the official Tauri updater;
- v4 updater key/trust root is generated and recovery/rotation is documented;
- application mutable state is outside the install root;
- production v4 no longer launches or links the custom updater;
- SemVer is canonical across Cargo/Tauri/tag/update metadata;
- fresh-install and previous-v4 update acceptance tests exist.

## Required implementation gates before `v4.0.0` GA

- Authenticode signing is enabled and verified in release CI;
- release SBOM and build provenance attestations are generated and verified;
- exact draft artifacts complete fresh-install, update, uninstall/reinstall, SmartScreen/signature,
  Defender, and packaged GUI qualification;
- stable update metadata is promoted only to the qualified immutable release;
- v3 and v4 release namespaces are demonstrably isolated;
- updater signing-key backup and rotation procedure has been exercised in a non-production fixture.

## References

- Tauri Updater: https://v2.tauri.app/plugin/updater/
- Tauri Windows Installer: https://v2.tauri.app/distribute/windows-installer/
- Tauri Windows Code Signing: https://v2.tauri.app/distribute/sign/windows/
- Tauri GitHub distribution pipeline: https://v2.tauri.app/distribute/pipelines/github/
- GitHub immutable releases: https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases
- GitHub artifact attestations: https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations
