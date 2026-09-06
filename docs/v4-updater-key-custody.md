# V4 Updater Key Custody, Rotation, and Recovery Runbook

This runbook defines the operational lifecycle, physical and cryptographic custody,
backup procedures, loss and compromise response, scheduled rotation, and disaster
recovery for the production v4 Tauri updater trust root.

This document is governed by `docs/adr/ADR-0006-v4-distribution-installation-update.md`
and `SECURITY.md`.

## 1. Trust Root Architecture and Inventory

The production v4 update pipeline uses an independent Minisign/Ed25519 trust root,
completely isolated from legacy v3 release keys (`release-2026`).

The canonical production v4 updater public trust root is:

```text
Key ID: 19AABD2E7838818C
Algorithm: Ed25519 (Minisign)
Public Key (Base64):
dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5QUFCRDJFNzgzODgxOEMKUldTTWdUaDRMcjJxR2JxeE5kTUx5VlIxS1dhOHRrSTEzY2FMeE8wYldtckM2TjV2KzRwQUNaTEUK
```

### Verified repository locations

The canonical public root is committed in exactly three authoritative locations:
1. `desktop/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`)
2. `desktop/src-tauri/src/native_update.rs` (`V4_TAURI_UPDATER_PUBLIC_KEY`)
3. `rust/xtask/src/tauri_bundle.rs` (`V4_TAURI_UPDATER_PUBLIC_KEY`)

To inventory and verify that all three locations match byte-for-byte and that no
extraneous public keys or legacy keys exist:

```powershell
cargo xtask updater-trust inventory
```

## 2. Key Custody

1. **Storage Isolation**:
   - The production private updater key must NEVER be committed to Git, stored in
     cloud storage, transmitted over email/chat, or configured in GitHub Actions
     repository secrets.
   - The private key is held outside the repository and workspace, encrypted at rest. Offline
     encrypted storage is sufficient; an HSM or paid key service is not required.

2. **Access Control**:
   - Only authorized Release Operators have access to the physical media and its
     decryption passphrase.
   - Access is limited to the maintainer responsible for the release and protected by the
     encryption passphrase.

3. **Passphrase Standards**:
   - The private key passphrase must be strong and managed separately from the key file.
   - The passphrase must be managed via a dedicated password manager and never stored in
     plaintext scripts or shell history.

4. **In-Memory & Ephemeral Signing Rules**:
   - When signing candidate update artifacts for release qualification, the key file or
     passphrase must only be loaded into ephemeral process memory.
   - Release and verifier processes never print secret values to stdout, stderr, or log files.
     External automation is responsible for its own secret-management and log-masking controls;
     the child release or verifier process does not need to emit the secret or a masking command.
   - This is an output-secrecy guarantee only. PowerShell/.NET managed strings are not claimed
     to be cryptographically zeroized by this runbook.

5. **Production Release Passphrase Transport (Windows Credential Manager Broker)**:
   - Parent-process environment variable inheritance does NOT reach Actions step processes
     on dedicated runner hosts (incident #140). It is explicitly **not** the production transport.
   - For production release dispatch, the Release Operator provisions a generic session credential
     in Windows Credential Manager using `scripts/set_v4_updater_session_credential.ps1`.
   - The provisioning script uses `Read-Host -AsSecureString` and writes the credential via
     `CredWriteW` with `CRED_TYPE_GENERIC` and `CRED_PERSIST_SESSION`.
   - The credential target name is fixed public metadata: `SkyAutoPlayer/V4UpdaterProduction`.
   - The production workflow (`release-v4.yml`) `BuildCandidate` step calls the broker
     (`scripts/v4_updater_credential_broker.ps1`) using `CredReadW`, injects the passphrase
     strictly into process-scoped `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, invokes the release
     orchestrator, and automatically deletes the credential in `finally` via `CredDeleteW`.
   - The updater private key remains stored outside the repository and workspace.
   - Passphrases **never** go into GitHub Secrets, runner `.env` files, command-line arguments,
     GitHub issues/chat/PRs, or repository files.

## 3. Local Private Key Verification

Release Operators must verify that their local private key matches the canonical public root
prior to initiating release packaging. The verification tool signs an ephemeral cryptographic
nonce and verifies the resulting signature against the compiled public root, without ever
printing private key material or passphrases to stdout, stderr, or log files.

The canonical public Key ID enforced by the verifier is:
```text
Key ID: 19AABD2E7838818C
```

To prevent password exposure in shell history (such as `PSReadLine` history files) or process listings,
the verification tools avoid command-line password flags.

### Interactive Operator Verification (Recommended)

For interactive operator use, run the PowerShell verifier. It performs a non-echoing secure
interactive prompt via `Read-Host -AsSecureString` to prevent passphrase exposure in terminal
logs or history files:

```powershell
# Prompts securely for passphrase if the private key is encrypted:
pwsh scripts/verify_v4_updater_private_key.ps1 -KeyPath "E:\secure\v4-updater.key"
```

### Verification via Windows Credential Manager Broker (Pre-Release / Non-Release Proof)

To verify the private key against the provisioned session credential without interactive prompts:

```powershell
# 1. Provision session credential as operator:
pwsh scripts/set_v4_updater_session_credential.ps1

# 2. Verify private key through the credential broker:
pwsh scripts/verify_v4_updater_private_key.ps1 -KeyPath "E:\secure\v4-updater.key" -UseCredentialBroker

# 3. Clean up session credential when finished (if not consumed by BuildCandidate):
pwsh scripts/remove_v4_updater_session_credential.ps1
```

### Non-Interactive / Automation Verification via xtask

`cargo xtask updater-trust verify-private-key` is designed for automated, non-interactive environments
and does not read from stdin. Passphrases must be supplied via an in-memory environment variable:

```powershell
# Reads from custom environment variable name:
cargo xtask updater-trust verify-private-key --key-file "E:\secure\v4-updater.key" --password-env MY_KEY_PASS

# Or reads from standard TAURI_SIGNING_PRIVATE_KEY_PASSWORD:
cargo xtask updater-trust verify-private-key --key-file "E:\secure\v4-updater.key"
```

Expected output:
```text
[xtask] Local updater private key matches canonical production v4 root (Key ID: 19AABD2E7838818C)
[PASS] Local updater private key matches canonical production v4 root
```

If the key does not match or the password is wrong, the tool exits with code 1 and emits a
sanitized error without leaking secret bytes.

## 4. Backup Procedures

1. **Independent Encrypted Backup**:
   - At least one independent backup of the private updater key must be readable when needed and
     encrypted at rest. It must be stored outside the repository/workspace under separate access
     control from the working copy.
   - The maintainer should periodically perform a local read/integrity check without copying the
     key into repository files, CI, or logs.

2. **Periodic Integrity Audit**:
   - Annually, operators must perform an air-gapped read test of backup media to ensure data
     retention and media integrity, using `cargo xtask updater-trust verify-private-key`.

## 5. Key Loss Incident Response

If the operational private updater key is permanently lost (e.g., physical destruction
without accessible backups):

1. **Pre-GA Key Loss**:
   - An unrecoverable pre-GA key requires replacing the committed public root across
     `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/src/native_update.rs`, and
     `rust/xtask/src/tauri_bundle.rs`, followed by full re-qualification of the release
     trust chain before the first production v4 release.

2. **Post-GA Key Loss Assessment**:
   - Confirm key unrecoverability across all backup vaults.
   - Deployed v4 clients will continue functioning normally and will reject any forged updates
     because they verify signatures against the trusted public root. However, no new
     automatic updates can be signed with the lost key.

3. **Post-GA Replacement Root and Out-of-Band Recovery**:
   - Generate a new v4 updater key pair under clean, documented custody:
     ```powershell
     Push-Location desktop
     bun run tauri signer generate -w "E:\secure\v4-updater-replacement.key"
     Pop-Location
     ```
   - Update `tauri.conf.json`, `native_update.rs`, and `tauri_bundle.rs` with the new public root.
   - Because existing clients cannot auto-update to a package signed with an unknown root,
     the recovery release must be published as a standalone NSIS installer under the
     `unsigned-zero-budget` Authenticode policy; the new Tauri updater signature is the update
     authorization gate.
   - Publish a security notice on GitHub and project channels directing users to perform a
     manual update via the official installer.

## 6. Key Compromise Incident Response

### Threat Model and Architecture Boundary

Current runtime auto-update trust in Sky Auto Player v4 relies on Tauri updater signature
verification at download time, followed immediately by `Update::install(bytes)`.
Windows Authenticode is an OS / SmartScreen / install-time gate during manual installation;
**it is not currently a client-side authorization gate during background auto-update**.

Consequently, an attacker who possesses the private updater key could forge update signatures
that deployed client instances would accept as valid if the attacker can serve them via an
update channel. Therefore, **the primary and immediate authorization gate against updater key
compromise is the Release Authority channel freeze**.

### Incident Response Procedure

If the private updater key is suspected or confirmed to be compromised:

1. **Severity 1 Incident Declaration**:
   - Immediately invoke the security process in `SECURITY.md`.

2. **Freeze Release Authority Channels (Critical Containment)**:
   - Immediately delete or overwrite `channels/stable/latest.json` and `channels/beta/latest.json`
     in `pumni/Sky-Auto-Player-Releases` with emergency quarantine metadata (empty or revoked).
   - This immediately halts all background update polling by existing clients, preventing them
     from fetching attacker-signed payloads.

3. **Generate Clean Trust Root**:
   - On an uncompromised, air-gapped machine, generate a new key pair:
     ```powershell
     Push-Location desktop
     bun run tauri signer generate -w "E:\secure\v4-updater-emergency.key"
     Pop-Location
     ```

4. **Publish Emergency Standalone Installer**:
   - Build a new release containing the new public trust root and qualify its exact unsigned
     Authenticode state under `unsigned-zero-budget`.
   - The installer may produce an Unknown Publisher or SmartScreen warning; this does not weaken
     the Tauri updater signature check for the migrated trust root.
   - Direct all users to install the emergency update manually to migrate to the new trust root.

## 7. Scheduled Key Rotation

Scheduled rotation occurs every 24 months or upon operational requirement. Rotation follows
the two-phase Bridge/Cutover model:

### Phase 1: Bridge Release (`v4.N`)

1. Generate new updater key pair `new.key` / `new.key.pub`.
2. Update `native_update.rs` to carry dual trust roots: `[old_root, new_root]`.
3. Sign the updater artifact for the `v4.N` installer with the `old.key` so that existing `v4.N-1`
   clients (which only trust `old_root`) successfully verify and download the update. The NSIS and
   project-owned PE files remain Authenticode-unsigned under the project policy.
4. Once installed, `v4.N` clients trust both `old_root` and `new_root`.

### Phase 2: Cutover Release (`v4.N+1`)

1. Update `tauri.conf.json`, `native_update.rs`, and `tauri_bundle.rs` to carry only `[new_root]`.
2. Sign the updater artifact for `v4.N+1` exclusively with `new.key`; keep the project-owned PE
   files Authenticode-unsigned under the project policy.
3. `v4.N` bridge clients verify the update against `new_root` and install successfully.
4. Old root `old.key` is permanently retired.

### Rehearsal and Evidence

Rotation procedures are validated automatically using:

```powershell
pwsh scripts/test_v4_updater_key_rotation.ps1
pwsh scripts/ci_tauri_update_e2e.ps1 -BundleDir rust/target/dist/bundle/nsis
```
