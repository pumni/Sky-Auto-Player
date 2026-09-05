# V4 Release Authority

V4 has a separate public release authority:

```text
pumni/Sky-Auto-Player-Releases
```

The source repository `pumni/Sky-Auto-Player` remains the legacy v3 release
namespace. The v4 Rust `UpdateService` has exactly two compiled metadata
endpoints:

```text
stable: https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/stable/latest.json
beta:   https://raw.githubusercontent.com/pumni/Sky-Auto-Player-Releases/main/channels/beta/latest.json
```

These URLs are selected by Rust from the persisted channel setting. They are
not supplied by React, environment variables, command-line arguments, or a
GitHub Releases fallback. The dedicated repository may remain without either
`latest.json` file until the first qualified v4 promotion; a missing endpoint
therefore fails closed.

## Canonical published asset

The Windows package uses the actual Tauri NSIS output and has one updater
asset/signature pair per release:

```text
Sky Auto Player_<version>_x64-setup.exe
Sky Auto Player_<version>_x64-setup.exe.sig
```

The immutable release URL is derived from the exact version and filename:

```text
https://github.com/pumni/Sky-Auto-Player-Releases/releases/download/v<version>/Sky%20Auto%20Player_<version>_x64-setup.exe
```

No portable ZIP, v3 `MANIFEST.json`, alias filename, source-repository
release, or signature URL is valid v4 updater metadata. The `.sig` field in
metadata contains the exact text from the published Tauri `.sig` file.

## Deterministic metadata

`cargo xtask release-authority generate` accepts only qualified-release
inputs: the canonical SemVer version, normalized release notes, a
second-precision UTC RFC3339 publication date, the exact
`windows-x86_64` Tauri platform, the immutable asset URL, and the exact
signature file. It emits the official Tauri static JSON shape:

```json
{
  "version": "4.0.0",
  "notes": "...",
  "pub_date": "2026-09-04T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of .sig>",
      "url": "<exact immutable asset URL>"
    }
  }
}
```

`cargo xtask release-authority validate --channel stable|beta` validates the
same bounded contract. Stable accepts only final SemVer releases; beta is
explicitly a prerelease channel. Stable and beta are separate destination
paths: `channels/stable/latest.json` and `channels/beta/latest.json`. The
validator rejects v3 URLs, non-HTTPS URLs, other owners/repos, query or
fragment state, SemVer build metadata, malformed dates, extra platforms, and
non-canonical artifact names.

The packaged qualification path emits a bounded
`tauri-nsis-qualified-release` evidence object containing the canonical
version/name/size pair and SHA-256 digests for both the installer and `.sig`,
  plus `unsigned-zero-budget` Authenticode-state evidence and SPDX SBOM references/digests.
  The installer is truthfully recorded as Authenticode-unsigned.
`qualified=true` is not accepted by itself: the promotion gate rejects missing
  or extra fields, `test` or other non-governed Authenticode modes, malformed evidence, digest
mismatches, and same-name assets with different bytes. Metadata promotion is a separate operation after the
immutable release has been published and the exact published assets have
passed qualification. It compares GitHub's asset digest when present, or
downloads and hashes the exact canonical asset when the API omits a digest.
The promotion action copies only the validated generated file into the
selected channel path; it never creates placeholder production metadata and
never rebuilds or replaces a published binary. Stable promotion cannot write
the beta path, and beta promotion cannot write the stable path.

The repository-owned acceptance check
`scripts/ci_v4_release_authority_acceptance.ps1` is read-only. It verifies
that the public source repository's `/releases/latest` is still a published
v3 release with its canonical v3 assets and that the dedicated authority is a
public repository. It does not create, upload, publish, edit, or delete any
release.

`scripts/promote_v4_metadata.ps1` is the separate promotion gate. It requires
the bounded evidence emitted by the packaged qualification path, runs the Rust
metadata validator, reads the dedicated repository's published release and
exact installer/`.sig` pair, and compares names, sizes, and SHA-256 digests
against the qualified bytes before atomically copying metadata into the
selected channel path in an authority checkout. Its `-SelfTest` regression
path proves arbitrary `qualified=true` evidence and same-name/different-bytes
assets are rejected. It never creates or edits a GitHub release; committing
the resulting channel file is a separate reviewed authority-repository
change.

The v4 Tauri updater public trust root is committed independently of this
release-authority metadata contract. Its operational private key remains
outside the repository. The project's production release policy does not
require Authenticode provider credentials; an optional real-signer seam is
separately governed and is not represented as current production evidence.
outside the repository. The project's production release policy is
`unsigned-zero-budget`: no Authenticode provider credentials are required; an
optional real-signer seam is separately governed and is not represented as
current production evidence. The retired v3 updater is preserved by Git
history and the `v3-maintenance` line, but is not part of the current v4
workspace or product graph.

Updater rotation is qualified with real packaged clients: the bridge trusts
`[old,new]` and verifies candidate bytes during `Update::download()`, while the
cutover client trusts `[new]` and rejects an old-root-only candidate. A full
non-PR workflow dispatch is required for provenance and SPDX attestation
evidence.
