# V4 Authenticode Policy and Provider Seam

This document defines the Windows Authenticode boundary for the canonical v4 Tauri NSIS package.
The current security and release contracts are [`SECURITY.md`](../SECURITY.md),
[`distribution-and-update.md`](distribution-and-update.md), and
[`v4-release-authority.md`](v4-release-authority.md). The original
[`ADR-0006`](adr/ADR-0006-v4-distribution-installation-update.md) records design rationale; it is
not the sole authority for current policy.

## Current pre-provider policy

The current pre-provider policy is the temporary `unsigned-zero-budget` state.

- `scripts/sign_v4_authenticode.ps1` deliberately performs no signing in this mode.
- No production certificate, provider, or certificate thumbprint is required.
- Qualification checks every project-owned shipped PE and the final NSIS installer and accepts only
  the actual Windows `NotSigned` state with no signer certificate.
- A test/self-signed binary, a partially signed binary, or any other unexpected status fails closed.
- Qualification evidence records `authenticode_mode: "unsigned-zero-budget"` and an unsigned state;
  it must never be relabeled `production`.
- Production GA should change this release boundary to an approved real signer when a provider is
  selected and available. The intended order is build, Authenticode-sign, independently verify,
  upload the signed draft, qualify the exact downloaded signed artifact, then publish.

This is a publisher-identity and user-experience trade-off. Windows may show **Unknown Publisher**
or a SmartScreen warning. It is not a bypass of updater cryptographic trust: the official Tauri
updater still verifies the `.exe.sig` Ed25519/minisign signature against the committed v4 public
root, and exact artifact digests, SBOM, provenance, and install qualification remain mandatory.

## SignCommand boundary

The Tauri configuration retains one bounded PowerShell hook. It remains a bundler adapter because
running `cargo xtask` from Tauri's `signCommand` would recurse into Cargo during bundling:

```text
scripts/sign_v4_authenticode.ps1 <Path>
```

```json
"signCommand": "pwsh -NoProfile -ExecutionPolicy Bypass -File ../../scripts/sign_v4_authenticode.ps1 %1"
```

The hook performs the platform signing operation for Tauri. Rust validates its result after bundling;
the hook does not create canonical qualification evidence.

## Modes

| Mode | Purpose | Evidence status |
| :--- | :--- | :--- |
| `unsigned-zero-budget` | Project production packaging and promotion | Only `NotSigned` is accepted; no signer identity is allowed |
| `test` | Disposable local/CI signing and integrity tests | Test-only; never promotable production evidence |
| `production` | Optional future real-signer seam | Requires an external provider and approved thumbprint; not the project's current release policy |

The optional `production` branch remains available for a future, separately approved signer. It is
not invoked by the canonical project release path until provider onboarding is complete. In
particular, the current release orchestrator sets
`SKY_AUTHENTICODE_MODE=unsigned-zero-budget` and does not require provider inputs.

## Rust policy and Windows inspection adapter

All verification callers use `cargo xtask authenticode verify --mode <mode> --artifact <path>`.
Rust owns mode selection, approved-thumbprint validation, provider exclusivity, ephemeral PFX
confinement, platform-status policy, CMS/SPC result checks, artifact digests, and canonical evidence
generation. The command accepts `--evidence <path>` when callers need a persisted evidence file.

`scripts/verify_v4_authenticode.ps1` is a Windows fact adapter. It reports
`Get-AuthenticodeSignature` status and delegates independent signed CMS/SPC PE digest verification
to `scripts/v4_authenticode_crypto.ps1`. Rust validates those facts and creates the canonical
evidence. The independent crypto adapter remains because its PE certificate-table and
`SignedCms` proof is not replaced by a file hash.

In `unsigned-zero-budget`, Rust accepts only a Windows `NotSigned` result with no signer and emits
the explicit `unsigned-zero-budget-policy` evidence marker. Test mode requires the exact test
thumbprint and a passwordless ephemeral PFX under the temporary root; signtool receives no PFX
password on its command line.
Production mode requires a valid 40-character approved thumbprint, a provider, and exactly one
provider script or command; CI test signer identities and unexpected Windows statuses fail closed.

Repeated artifact arguments cross the Rust-to-Windows adapter boundary in a bounded UTF-8 JSON
manifest with schema_version 1 (at most 256 files and 1 MiB). Each record binds the resolved Unicode
path, exact filename, and Rust-computed SHA-256. The adapter validates that inventory, checks the
file hash before and after platform/CMS verification, and returns facts that Rust matches against
every requested filename and digest.

The bundle verifier binds the final NSIS evidence to the exact installer bytes. The installed-tree
qualification repeats the same check for every project-owned PE; the generated `uninstall.exe` is
not treated as a project-owned binary. These checks do not replace the Tauri updater signature,
SBOM, provenance, digest, or install/launch/uninstall gates.

## Optional real-signer seam

When explicitly used outside the project production policy, `SKY_AUTHENTICODE_MODE=production`
requires all of the following and fails closed otherwise:

1. `SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT`, a 40-character SHA-1 thumbprint;
2. `SKY_AUTHENTICODE_PROVIDER`; and
3. exactly one of `SKY_AUTHENTICODE_PROVIDER_SCRIPT` or `SKY_AUTHENTICODE_PROVIDER_COMMAND`.

The signer verifies the resulting certificate identity and independent Authenticode integrity. CI
self-signed credentials are rejected in this mode. No provider credentials or certificate material
belong in the repository, pull request, or ordinary CI configuration.

## Test contract

The focused Windows contract suite uses throwaway material only:

```powershell
pwsh scripts/test_v4_production_signing_contract.ps1
```

It proves that unsigned-state verification rejects signed PE files, the Rust CLI and adapter verify
two signed synthetic PE files in one request, missing and tampered artifacts fail closed, test mode
validates the independent CMS/SPC proof, and test evidence cannot satisfy production policy.
