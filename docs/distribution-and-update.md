# Distribution and Update Model

This is the normative distribution and update contract for the current v4
product. The canonical Windows distribution is a Tauri NSIS installer using
`currentUser` installation semantics under `%LOCALAPPDATA%`. The official
Tauri updater, mediated by the Rust-owned `UpdateService`, owns update
discovery, signature verification, download, and installer execution.

## 1. Active V4 package contract

- **Application Identifier:** `io.github.pumni.skyautoplayer`
- **Version Source:** `desktop/src-tauri/Cargo.toml`
- **Windows Target:** NSIS only (installer named `Sky-Auto-Player-<version>-setup.exe`)
- **Installer Scope:** `currentUser` under `%LOCALAPPDATA%\io.github.pumni.skyautoplayer`
- **Updater Output:** The NSIS setup executable and its `.exe.sig` sidecar
- **Runtime Updater:** Official `tauri-plugin-updater`, behind the Rust-owned `UpdateService`
- **Frontend Boundary:** React/TypeScript surface presents update state and progress only; it cannot supply endpoints, keys, URLs, or downgrade policies.

The v4 package does not contain `Sky-Auto-Player-Updater.exe`, a portable ZIP
updater contract, or the custom `MANIFEST.json` / `MANIFEST.json.sig` protocol.
Tauri updater signatures and the retired v3 manifest signature are different
contracts; only the former is used for v4 updates.

## 2. Release metadata and channels

V4 never queries legacy v3 endpoints. The Rust `UpdateService` selects
exactly one of these fixed Tauri static metadata endpoints from the persisted
channel setting:

```text
stable: https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json
beta:   https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json
```

The metadata contains only the canonical Windows NSIS asset from an immutable
GitHub Release in `pumni/Sky-Auto-Player` and the exact contents of its `.exe.sig`
sidecar. Stable and beta metadata have separate paths and are never interchanged.
Endpoint URLs, keys, artifact paths, and downgrade policies do not cross the
Rust/React boundary. See [`v4-release-authority.md`](v4-release-authority.md)
for the generator, validator, strictly monotonic post-qualification promotion,
and final public endpoint verification.

## 3. Trust root and Authenticode governance

- **Tauri Updater Trust:** Public root embedded in `desktop/src-tauri/tauri.conf.json`.
  The private updater key is held exclusively outside the repository/workspace,
  encrypted at rest with an independent encrypted backup (see [`v4-updater-key-custody.md`](v4-updater-key-custody.md)).
- **Authenticode Policy:** Governed `unsigned-zero-budget` policy. The installer and
  application binaries are intentionally unsigned for Authenticode; no certificate secret
  or cloud signing credentials belong in PR or local CI. An optional Authenticode seam
  is specified in [`v4-authenticode-provider-seam.md`](v4-authenticode-provider-seam.md)
  for future production consideration.

Windows may display Unknown Publisher or a SmartScreen prompt while the unsigned-zero-budget
policy is active. This is an intentional publisher-identity/UX choice and does not
compromise the cryptographic signature verification performed by the Tauri updater.

## 4. Qualification and provenance verification

Qualification binds the exact NSIS installer and Tauri signature bytes by
SHA-256 and verifies:
- governed unsigned-zero-budget Authenticode status;
- SPDX SBOM generation;
- GitHub OIDC provenance/attestation;
- clean worktree guarantee;
- install, launch, and uninstall smoke verification; and
- post-download previous-v4-to-candidate-v4 fixture evidence consuming the exact
  installer and `.sig` re-downloaded from the draft release.

Release orchestration remains subject to the single-repository runbook in
[`v4-release-authority.md`](v4-release-authority.md) and the execution topology
in [`v4-release-execution-topology.md`](v4-release-execution-topology.md).

## 5. Historical v3 distribution (superseded)

The legacy v3 distribution model (portable ZIP packaging, Ed25519 `MANIFEST.json`, and standalone `Sky-Auto-Player-Updater.exe` transactional replacement) has been retired from the active product architecture and relocated to [history/v3/distribution-and-update.md](history/v3/distribution-and-update.md).
