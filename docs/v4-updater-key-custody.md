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
Key ID: F6355260A0C663D5
Algorithm: Ed25519 (Minisign)
Public Key (Base64):
dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY2MzU1MjYwQTBDNjYzRDUKUldUVlk4YWdZRkkxOWdWRnNkRTNVY0habzA0YlQ4OFkxZk42WEM3OGVnSW5WNlc5SHlSbGF3QWEK
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
   - The private key is held exclusively on offline, air-gapped, encrypted physical media
     (e.g., hardware security module, encrypted FIPS 140-2 Level 2+ USB drive, or an
     air-gapped workstation).

2. **Access Control**:
   - Only authorized Release Operators have access to the physical media and its
     decryption passphrase.
   - Access requires multi-person verification (dual custody) for production releases.

3. **Passphrase Standards**:
   - The private key passphrase must be generated with high entropy (minimum 128 bits).
   - The passphrase must be managed via a dedicated password manager and never stored in
     plaintext scripts or shell history.

4. **In-Memory & Ephemeral Signing Rules**:
   - When signing candidate update artifacts for release qualification, the key file or
     passphrase must only be loaded into ephemeral process memory.
   - When automated workflows handle the passphrase, `::add-mask::<passphrase>` must be
     emitted before any other output to prevent log leakage.

## 3. Local Private Key Verification

Release Operators must verify that their local private key matches the canonical public root
prior to initiating release packaging. The verification tool signs an ephemeral cryptographic
nonce and verifies the resulting signature against the compiled public root, without ever
printing private key material or passphrases to stdout, stderr, or log files.

To prevent password exposure in shell history (such as `PSReadLine` history files) or process listings,
the tool avoids command-line password flags. Passwords should be entered interactively via masked
input or supplied via an in-memory environment variable:

### Running verification via xtask

```powershell
# Interactively prompts for passphrase if the key is encrypted:
cargo xtask updater-trust verify-private-key --key-file "E:\secure\v4-updater.key"

# Or with an in-memory environment variable name:
cargo xtask updater-trust verify-private-key --key-file "E:\secure\v4-updater.key" --password-env MY_KEY_PASS
```

### Running verification via script

```powershell
# Interactive prompt:
pwsh scripts/verify_v4_updater_private_key.ps1 -KeyPath "E:\secure\v4-updater.key"
```

Expected output:
```text
[PASS] Local updater private key matches canonical production v4 root (Key ID: F6355260A0C663D5)
```

If the key does not match or the password is wrong, the tool exits with code 1 and emits a
sanitized error without leaking secret bytes.

## 4. Backup Procedures

1. **Cold Encrypted Backups**:
   - Two cold backup copies of the private updater key file must exist in separate physical
     locations (e.g., Primary Safe, Offsite Safe).
   - Each backup copy is stored on dedicated encrypted offline storage.

2. **Passphrase Recovery Split**:
   - The passphrase for the encrypted backup media should be split using Shamir's Secret
     Sharing (or equivalent dual-custody envelopes) among project maintainers.

3. **Periodic Integrity Audit**:
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
     the recovery release must be published as a standalone installer (NSIS setup executable)
     signed with the approved Authenticode certificate.
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

4. **Publish Authenticode-Signed Emergency Standalone Installer**:
   - Build and sign a new release containing the new public trust root.
   - Publish the recovery release as an official standalone installer signed with the approved
     publisher Authenticode certificate. Windows Authenticode and SmartScreen provide publisher
     provenance and integrity for users performing the manual installation.
   - Direct all users to install the emergency update manually to migrate to the new trust root.

## 7. Scheduled Key Rotation

Scheduled rotation occurs every 24 months or upon operational requirement. Rotation follows
the two-phase Bridge/Cutover model:

### Phase 1: Bridge Release (`v4.N`)

1. Generate new updater key pair `new.key` / `new.key.pub`.
2. Update `native_update.rs` to carry dual trust roots: `[old_root, new_root]`.
3. Sign the `v4.N` installer with the `old.key` so that existing `v4.N-1` clients (which only trust
   `old_root`) successfully verify and download the update.
4. Once installed, `v4.N` clients trust both `old_root` and `new_root`.

### Phase 2: Cutover Release (`v4.N+1`)

1. Update `tauri.conf.json`, `native_update.rs`, and `tauri_bundle.rs` to carry only `[new_root]`.
2. Sign `v4.N+1` exclusively with `new.key`.
3. `v4.N` bridge clients verify the update against `new_root` and install successfully.
4. Old root `old.key` is permanently retired.

### Rehearsal and Evidence

Rotation procedures are validated automatically using:

```powershell
pwsh scripts/test_v4_updater_key_rotation.ps1
pwsh scripts/ci_tauri_update_e2e.ps1 -BundleDir rust/target/dist/bundle/nsis
```
