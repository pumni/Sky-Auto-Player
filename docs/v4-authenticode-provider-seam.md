# V4 Authenticode Policy and Provider Seam

This document defines the Windows Authenticode boundary for the canonical v4 Tauri NSIS package.
It is governed by `docs/adr/ADR-0006-v4-distribution-installation-update.md` and `SECURITY.md`.

## Project production policy

The project's production policy is `unsigned-zero-budget`.

- `scripts/sign_v4_authenticode.ps1` deliberately performs no signing in this mode.
- No production certificate, provider, or certificate thumbprint is required.
- Qualification checks every project-owned shipped PE and the final NSIS installer and accepts only
  the actual Windows `NotSigned` state with no signer certificate.
- A test/self-signed binary, a partially signed binary, or any other unexpected status fails closed.
- Qualification evidence records `authenticode_mode: "unsigned-zero-budget"` and an unsigned state;
  it must never be relabeled `production`.

This is a publisher-identity and user-experience trade-off. Windows may show **Unknown Publisher**
or a SmartScreen warning. It is not a bypass of updater cryptographic trust: the official Tauri
updater still verifies the `.exe.sig` Ed25519/minisign signature against the committed v4 public
root, and exact artifact digests, SBOM, provenance, and install qualification remain mandatory.

## SignCommand boundary

The Tauri configuration retains one bounded hook:

```text
scripts/sign_v4_authenticode.ps1 <Path>
```

```json
"signCommand": "pwsh -NoProfile -ExecutionPolicy Bypass -File ../../scripts/sign_v4_authenticode.ps1 %1"
```

The default mode is `unsigned-zero-budget`. The hook is intentionally auditable and deterministic;
its successful no-op is followed by explicit unsigned-state verification.

## Modes

| Mode | Purpose | Evidence status |
| :--- | :--- | :--- |
| `unsigned-zero-budget` | Project production packaging and promotion | Only `NotSigned` is accepted; no signer identity is allowed |
| `test` | Disposable local/CI signing and integrity tests | Test-only; never promotable production evidence |
| `production` | Optional future real-signer seam | Requires an external provider and approved thumbprint; not the project's current release policy |

The optional `production` branch remains available for a future, separately approved signer. It is
not invoked by the canonical project release path and does not justify provider onboarding or paid
service assumptions. In particular, the current release orchestrator sets
`SKY_AUTHENTICODE_MODE=unsigned-zero-budget` and does not require provider inputs.

## Verification contract

`scripts/verify_v4_authenticode.ps1 -Mode unsigned-zero-budget` accepts only regular `.exe` and
`.dll` project artifacts whose `Get-AuthenticodeSignature` result is exactly `NotSigned` and whose
`SignerCertificate` is absent. It emits bounded evidence containing the file name and SHA-256,
with null signer fields and the explicit `unsigned-zero-budget-policy` marker.

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

It proves that the zero-budget no-op succeeds on an unsigned PE, a test-signed PE is rejected by
the zero-budget verifier, and test evidence cannot satisfy production/promotion validation.
