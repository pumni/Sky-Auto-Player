# V4 Clean Distribution and Update Execution Specification

Date: 2026-09-03

Status: Phase 7 is complete on the WO-06 source-hardening branch. The current
v4 workspace and release path use Tauri NSIS and the official Tauri updater;
the retired custom updater is not a v4 dependency or release authority.

Related ADR: `docs/adr/ADR-0006-v4-distribution-installation-update.md`

## Goal

Deliver v4 as a new Windows desktop product boundary rather than an in-place evolution of the v3
portable updater. The canonical v4 application is installed per-user through an Authenticode-signed
Tauri NSIS installer and updates through the official Tauri updater. V3 remains a separate legacy
release/update namespace.

## Operating rules

- One architectural concern per PR where practical.
- No v4 production release until the release/update namespace is isolated from v3.
- Do not publish a stable v4 release into the v3 `/releases/latest` namespace.
- Do not delete v3 updater code until the replacement path is packaged, exercised, and proven not to
  depend on it.
- Do not expose updater endpoints, keys, arbitrary URLs, installer paths, or downgrade policy to React.
- Do not rebuild a release candidate after qualification begins.
- Keep `main` releasable while the v4 work is staged.
- Every removal PR must prove the removed v3 surface has no production v4 dependency.

## Target topology

```text
pumni/Sky-Auto-Player                 dedicated v4 release authority
(source + v3 legacy release history)       (binary releases + updater metadata)
            |                                      |
            | v4 source tag/commit                 | immutable v4 release
            +---------------- CI ------------------+
                                                   |
                                             stable/latest.json
                                             beta/latest.json
                                                   |
                                                   v
                                             installed v4 app
```

The release-authority name must be finalized before runtime endpoints are committed. Source and
release repositories may remain separate indefinitely; no runtime code should assume that GitHub's
source repository is also the update authority.

## Target Windows product layout

```text
Installer-owned application root
  Sky Auto Player.exe
  native_calibration.exe
  immutable application resources
  framework/runtime files

Application-owned data root
  settings
  database/index
  calibration state
  logs
  cache

User-owned music locations
  selected/imported directories and files
```

No mutable user data is required to remain beside the executable.

## Target update boundary

```text
React UI
   |
   | bounded commands/events only
   v
Rust UpdateService
   |-- configured channel policy
   |-- playback/install admission
   |-- bounded DTO mapping
   |-- pre-exit safety boundary
   v
Tauri updater plugin
   |-- check static metadata
   |-- verify updater signature
   |-- download
   |-- execute NSIS update
   v
Windows installer
```

## Phase 0 — Architecture lock

Deliverables:

- ADR-0006 reviewed and accepted.
- Dedicated v4 release authority chosen.
- Permanent v4 application identifier chosen.
- Stable/beta policy decided.
- Signing provider decision recorded: certificate/PFX, Azure Key Vault, Azure Artifact Signing, or a
  reviewed custom signing command.

Exit criteria:

- There is no unresolved decision that would change package identity, updater trust root, update
  endpoint ownership, or install/data ownership.

## Phase 1 — Tauri packaging foundation

Changes:

- Enable Tauri bundling.
- Set the v4 identifier and SemVer version source.
- Configure NSIS as the only canonical v4.0 Windows target.
- Use current-user installation semantics.
- Enable Tauri updater artifacts.
- Add the required Tauri updater/process dependencies without wiring production UI yet.
- Add a package inspection test that records the exact generated installer/updater artifacts.

Historical sequencing constraints before WO-06 (now superseded for current
v4 source):

- remove `sky_updater`;
- remove v3 release tooling;
- publish production updater metadata;
- switch normal users to v4 update checks.

Exit criteria:

- A local Windows build produces an installable NSIS package and updater artifact/signature using a
  non-production test updater key.
- Fresh install and uninstall work without Administrator privileges.

## Phase 2 — Application data boundary

Changes:

- Move mutable configuration/state out of the install root.
- Remove production `.env`-beside-executable assumptions.
- Store logs/cache/calibration/database/index under appropriate app-data roots.
- Treat music files as user-owned library sources rather than updater-managed install files.
- Add deterministic path-resolution tests.
- Define optional v3 import as an explicit, one-shot application feature if it is retained.

Exit criteria:

- A clean v4 installation can be replaced/uninstalled without losing user-owned data.
- No v4 updater/install path requires a preserved-path list.

## Phase 3 — V4 update service

Changes:

- Introduce a narrow Rust `UpdateService` around the official Tauri updater.
- Compile/allow-list channel endpoints in Rust-owned configuration.
- Model update states with bounded DTOs/events.
- Prevent install while playback is active.
- Before updater-triggered exit, stop playback, quiesce the worker, release injected input, persist
  required state, and close bounded resources.
- Add packaged tests for check/no-update/update-available and the pre-exit safety boundary.

Exit criteria:

- React contains no updater authority configuration.
- Production update install is mediated by Rust policy.
- A previous v4 fixture can update to the candidate through the standard updater path.

## Phase 4 — V4 release authority and metadata

Changes:

- Create/configure the dedicated v4 release authority.
- Define canonical artifact naming.
- Publish static Tauri updater metadata for stable and, if retained, beta.
- Ensure update metadata promotion happens only after the exact release is qualified and published.
- Validate that v3 discovery cannot observe v4 releases.

Exit criteria:

- V3 `/releases/latest` behavior is unchanged when a v4 candidate is published.
- V4 clients can discover only the intended v4 channel metadata.

## Phase 5 — Windows publisher trust and updater key management

Changes:

- Sign project-owned Windows executables and the NSIS installer with Authenticode.
- Verify signatures in CI after signing.
- Generate a new v4 Tauri updater key pair/trust root.
- Store the private updater key outside source control using the selected release-secret mechanism.
- Document backup, loss, compromise, and rotation.
- Exercise key rotation with test keys in a non-production packaged fixture.

Exit criteria:

- Unsigned canonical v4 release artifacts are rejected by release qualification.
- The updater private key can be recovered from the documented backup process.
- Rotation behavior has a repeatable acceptance test or fixture.

## Phase 6 — Release pipeline redesign

Changes:

- Make Tauri the canonical v4 package producer.
- Reframe `xtask` around verification/qualification rather than portable assembly.
- Generate SPDX JSON or CycloneDX JSON SBOM.
- Preserve cargo-vet dependency-policy evidence.
- Generate GitHub build provenance/SBOM attestations.
- Create draft-first releases and qualify exact draft assets.
- Publish only the already-qualified assets and make the release immutable.
- Promote stable/beta updater metadata only after publication succeeds.

Required qualification matrix:

- clean install;
- uninstall;
- reinstall with existing app data;
- previous-v4 -> candidate-v4 update;
- playback-active update-install rejection;
- pre-exit all-input-release safety;
- Authenticode verification;
- updater-signature verification;
- exact downloaded-installer Defender custom scan with no detection; missing
  Defender cmdlets, disabled protection, scan failure, or detection fails
  closed;
- packaged GUI smoke;
- update-channel isolation;
- immutable release/asset identity check.

## Phase 7 — Retire v3 updater ownership from v4 (WO-06)

The following v3 surfaces are removed from the current v4 dependency and
release graph:

- `sky_updater` runtime dependency;
- `Sky-Auto-Player-Updater.exe` packaging;
- custom updater launch/handoff code;
- custom release ZIP selection/download/extraction for v4;
- custom update transaction/recovery state;
- custom signed `MANIFEST.json` update protocol for v4;
- PEP 440 update ordering from the v4 update path;
- v3 preserve-list assumptions from v4 packaging.

Historical v3 tags/releases/docs remain intact. Removal must not rewrite historical evidence.

The historical implementation remains available through Git history and the
`v3-maintenance` line; it is not built, packaged, or qualified by current v4
checks.

Exit criteria:

- Search/static checks demonstrate no canonical v4 runtime or v4 release path depends on the retired
  updater.
- `cargo xtask check all` and the v4 packaged acceptance matrix pass.

## Phase 8 — V4 release progression

Recommended progression:

```text
4.0.0-alpha.1   internal package/update plumbing
4.0.0-beta.1    public clean install + update channel
4.0.0-beta.N    signing/data/update hardening
4.0.0-rc.1      exact GA-shaped release process
4.0.0           GA
```

Do not use v3 PEP 440 spellings such as `4.0.0rc1` or `.devN` in the v4 release protocol.

## Deferred work

These do not block v4.0 unless requirements change:

- a secondary portable distribution;
- MSI distribution;
- Microsoft Store packaging;
- dynamic update backend;
- staged rollout/cohorts;
- full TUF roles/threshold metadata;
- automatic v3 -> v4 updater migration.

## Rollback strategy for the implementation program

The clean break is a release-level decision, not permission to make the development branch
irreversible. Until v4 GA:

- keep v3 release code available on `main` or a stable maintenance branch as required;
- isolate v4 changes in reviewable PRs;
- do not delete legacy code in the same PR that first introduces its replacement;
- prefer feature/configuration boundaries that allow packaged A/B qualification before retirement;
- if a v4 phase fails acceptance, revert that phase rather than repairing already-published artifacts.
