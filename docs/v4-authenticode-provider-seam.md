# V4 Authenticode Provider Seam Specification

This document formalizes the production Windows Authenticode provider seam for Sky Auto Player v4.
It defines the provider-neutral contract between the Tauri NSIS packaging pipeline and external code
signing providers.

This specification is governed by `docs/adr/ADR-0006-v4-distribution-installation-update.md` and
`SECURITY.md`.

## 1. Provider-Neutral Architecture

Windows binaries (executables and DLLs) shipped in the canonical v4 NSIS installer and the installer
itself must be signed with an approved Authenticode code signing certificate.

Because signing providers vary across deployment environments (e.g., Azure Trusted Signing,
hardware tokens/HSMs, cloud key management services, or specialized CI runners), Sky Auto Player
isolates all signing invocation behind a single provider-neutral script seam:

```text
scripts/sign_v4_authenticode.ps1 <Path>
```

Tauri's bundle configuration binds directly to this seam in `desktop/src-tauri/tauri.conf.json`:

```json
"bundle": {
  "windows": {
    "signCommand": "pwsh -NoProfile -ExecutionPolicy Bypass -File ../../scripts/sign_v4_authenticode.ps1 %1"
  }
}
```

## 2. Modes and Security Invariants

The signer operates in two strictly separated modes:

| Mode | Purpose | Trust Model | Rejection Policy |
| :--- | :--- | :--- | :--- |
| `test` | Local developer builds and CI qualification | Ephemeral self-signed PFX generated in `RUNNER_TEMP` | Platform status `Valid` not required; cryptographic integrity enforced |
| `production` | Official release builds and release qualification | External approved code signing certificate | Fail-closed: requires approved thumbprint and provider; CI test certs rejected |

### Strict mode isolation

1. **Fail-Closed by Default**: When `SKY_AUTHENTICODE_MODE` is unset, the signer defaults to `production`.
2. **Rejection of Test Material**: In `production` mode, the presence of any test credentials
   (`SKY_AUTHENTICODE_TEST_PFX_PATH`, `SKY_AUTHENTICODE_TEST_PFX_PASSWORD`, `SKY_AUTHENTICODE_TEST_THUMBPRINT`)
   causes an immediate, non-recoverable error.
3. **No Certificate Store Tampering**: Neither mode alters Windows system certificate stores or registers
   ephemeral certificates as Trusted Publishers.

## 3. Production Seam Contract

### Required Environment Variables

When `SKY_AUTHENTICODE_MODE=production`, the release runner must supply:

1. **`SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT`** (Required):
   - A 40-character hexadecimal SHA-1 thumbprint identifying the authorized publisher certificate.
   - Example: `A1B2C3D4E5F60718293A4B5C6D7E8F9012345678`

2. **`SKY_AUTHENTICODE_PROVIDER`** (Required):
   - Identifies the provider mechanism for auditing and telemetry.
   - Examples: `azure-trusted-signing`, `signtool-hsm`, `custom-script`.

3. **Signing Invocation** (Exactly one required, mutually exclusive):
   - **`SKY_AUTHENTICODE_PROVIDER_SCRIPT`** (Preferred): Full path to a structured PowerShell script
     accepting `-Path <file>`. Structured scripts are preferred over string commands to ensure clean
     argument passing and error propagation.
   - **`SKY_AUTHENTICODE_PROVIDER_COMMAND`**: Command line template where `%1` or `$Path` is replaced
     by the target file path.
   - **Mutual Exclusivity**: If both `SKY_AUTHENTICODE_PROVIDER_SCRIPT` and `SKY_AUTHENTICODE_PROVIDER_COMMAND`
     are set (or if neither is set), signing fails closed immediately.

### Provider Execution and Post-Signing Verification

When the provider completes signing the target file, `scripts/sign_v4_authenticode.ps1` executes
an immediate post-condition verification:

1. Target file must have an embedded Authenticode signature.
2. Signer certificate thumbprint must match `SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT` byte-for-byte.
3. Signer certificate Subject must not contain `CI V4 Test Code Signing`.
4. Cryptographic integrity must pass independent verification via `scripts/v4_authenticode_crypto.ps1`:
   - Valid PKCS#7 SignedCms envelope.
   - Exact PE indirect-data hash match (`signed_digest == computed_digest`).

If any check fails, the signer immediately exits with a non-zero code, aborting packaging.

## 4. Supported Provider Integration Patterns

### Pattern A: Azure Trusted Signing (dlib / cli)

Set the following environment variables on the release runner:

```powershell
$env:SKY_AUTHENTICODE_MODE = "production"
$env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = "<approved_thumbprint>"
$env:SKY_AUTHENTICODE_PROVIDER = "azure-trusted-signing"
$env:SKY_AUTHENTICODE_PROVIDER_COMMAND = 'trusted-signing-cli sign --endpoint https://wus.codesigning.azure.net --account <account> --profile <profile> --file %1'
```

### Pattern B: Hardware Token / HSM (via SignTool)

```powershell
$env:SKY_AUTHENTICODE_MODE = "production"
$env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = "<approved_thumbprint>"
$env:SKY_AUTHENTICODE_PROVIDER = "signtool-hsm"
$env:SKY_AUTHENTICODE_PROVIDER_COMMAND = 'signtool.exe sign /sha1 <approved_thumbprint> /tr http://timestamp.digicert.com /td SHA256 /fd SHA256 %1'
```

### Pattern C: Custom Provider Script

```powershell
$env:SKY_AUTHENTICODE_MODE = "production"
$env:SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT = "<approved_thumbprint>"
$env:SKY_AUTHENTICODE_PROVIDER = "custom-script"
$env:SKY_AUTHENTICODE_PROVIDER_SCRIPT = "C:\ops\sign_artifact.ps1"
```

## 5. Contract Verification Tooling

To run automated verification proving that ephemeral CI test certificates cannot satisfy
production mode:

```powershell
pwsh scripts/test_v4_production_signing_contract.ps1
```
