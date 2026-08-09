# Build, Authenticode, Manifest, Packaging and Release

## Mandatory production order

```text
tests
→ build Rust wheel
→ build native updater
→ build native calibration
→ build source-matched PyInstaller bootloader
→ build frozen app
→ assemble final release tree
→ frozen smoke tests
→ Authenticode sign project-owned PE files
→ verify signatures
→ generate MANIFEST.json from signed bytes
→ verify manifest
→ create canonical ZIP
→ generate ZIP SHA256 sidecar
→ packaged self-update E2E
→ provenance attestation
→ publish ZIP + SHA + MANIFEST
```

Never generate the production manifest before signing.

## Refactor `build_app.py`

Build and manifest operations must be separable.

Recommended logical CLI:

```text
python -m build_app
python -m build_app --manifest-only --release-dir <dir>
```

Equivalent naming is acceptable if clear.

Requirements:

- normal build creates the release tree;
- manifest-only never rebuilds;
- manifest-only hashes exactly the existing staged bytes;
- release worktree must be clean;
- frozen smoke tests remain gates;
- no updater BAT/PowerShell assets are copied;
- native updater binary is copied into release root.

## Build updater

Build package `sky_updater` in release mode and copy as:

```text
Sky-Auto-Player-Updater.exe
```

Smoke:

```text
Sky-Auto-Player-Updater.exe --help
Sky-Auto-Player-Updater.exe --version
```

## Project-owned PE signing scope

Sign:

```text
Sky-Auto-Player.exe
Sky-Auto-Player-Updater.exe
native_calibration.exe
_internal/**/sky_player_rs*.pyd
```

Enumerate any additional project-owned PE explicitly.

Do not re-sign:

- Python runtime DLLs;
- Windows DLLs;
- third-party wheel DLLs;
- arbitrary dependencies.

## Signing provider

The maintainer must provision a real trusted Authenticode identity/provider.

The coding agent must **not** fabricate or self-sign a production identity.

PR/CI development builds may remain unsigned and must be clearly non-release.

Tag release must fail closed when required signing configuration is absent.

Do not silently publish unsigned production artifacts.

## Signature verification gate

Add a release verification tool/script that checks:

- every required project-owned PE is present;
- each is signed;
- signature is valid;
- chain is trusted;
- signer matches expected publisher policy.

Using PowerShell as a CI scripting shell is acceptable. `ExecutionPolicy Bypass` is not required and must not be used as a runtime/distribution mechanism.

## Manifest schema/order

Generate after signing.

Use an explicit schema version (recommended `2`).

Record:

- app ID;
- version;
- executable;
- git commit;
- dirty-worktree flag;
- native build commit;
- build time;
- interpreter metadata if still useful;
- exact file list;
- sizes;
- SHA256.

Define one explicit rule for `MANIFEST.json` self-reference. Prefer manifest as the single exact-set exception rather than trying to hash itself.

External release `MANIFEST.json` must match the embedded manifest.

## Canonical ZIP

Create exactly:

```text
Sky-Auto-Player-v<version>.zip
```

Do not create:

```text
portable.zip
legacy.zip
bridge.zip
updater.zip
```

Do not include:

- Pester/test results;
- dev caches;
- source tree;
- updater scripts;
- developer machine metadata.

## SHA sidecar

After ZIP is final:

```text
<64-hex>  Sky-Auto-Player-v<version>.zip
```

One record only.

Publish:

```text
Sky-Auto-Player-v<version>.zip.sha256
```

## GitHub provenance

Keep build provenance attestation for:

```text
ZIP
ZIP.sha256
MANIFEST.json
```

Authenticode and provenance complement each other.

## Source-built PyInstaller bootloader

Resolve exact PyInstaller version from lock/build environment.

Production process must build bootloader from matching upstream source.

Rules:

- exact source version match;
- no behavior fork;
- no anti-analysis changes;
- no UPX;
- no packer;
- log compiler/toolchain;
- record bootloader hash;
- fail production build if source bootloader build fails.

## PyInstaller collection cleanup

Keep:

```text
onedir
upx=False
```

Reduce broad `collect_all(...)` calls one at a time.

For each reduction:

1. inspect imports/data;
2. make one change;
3. frozen build;
4. all frozen selftests;
5. UI startup;
6. record file count and size delta.

Do not extend `excludes` without proving transitive non-use.

## Release workflow cleanup

Remove old updater-specific stages after source removal:

```text
Updater — UTF-8 BOM + Windows PowerShell 5.1 parse
Pester — installer tests
```

Replace with:

```text
sky_updater cargo tests
updater build
updater smoke
updater integration/dry-run tests
signing
signature verification
post-sign manifest
package verification
packaged self-update E2E
rollback E2E
```

## Suggested workflow shape

```text
quality
  ├─ ruff
  ├─ pyright
  ├─ Python tests
  ├─ security audit
  └─ free-threaded audit

rust
  ├─ fmt
  ├─ check
  ├─ clippy
  └─ workspace tests

build-native
  ├─ sky_player_rs wheel
  ├─ sky_updater.exe
  └─ native_calibration.exe

build-frozen
  ├─ source PyInstaller bootloader
  └─ Sky-Auto-Player.exe

stage
  └─ exact release tree

smoke
  ├─ app selftests
  └─ updater smoke

sign
  ├─ sign own PE
  └─ verify signer

manifest
  ├─ generate
  └─ verify exact set

package
  ├─ ZIP
  └─ SHA256

e2e-update
  ├─ upgrade
  └─ rollback

attest
  └─ GitHub provenance

publish
  └─ canonical triple
```

## Version bump

Do not bump version just because implementation starts.

Keep release version preparation in the repo's normal release process unless maintainer explicitly chooses the next version.
