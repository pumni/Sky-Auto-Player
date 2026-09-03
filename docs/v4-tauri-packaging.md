# V4 Tauri Packaging Foundation

V4 uses the Tauri Windows bundler as its canonical package producer. The
foundation enables the NSIS package and updater artifact generation. The
Rust-owned `UpdateService` now drives the official Tauri updater, but
production authority remains intentionally fail-closed until WO-04 supplies
the endpoint and trust key.

## Canonical package contract

- Application identifier: `io.github.pumni.skyautoplayer`
- Version source: `desktop/src-tauri/Cargo.toml`
- Current foundation version: `4.0.0-alpha.1`
- Windows target: NSIS only
- Installer scope: `currentUser` under `%LOCALAPPDATA%`
- Updater output: the NSIS setup executable and its `.exe.sig` sidecar
- Legacy `sky_updater`: retained and buildable until the later retirement work order
- Runtime updater: official `tauri-plugin-updater`, behind Rust `UpdateService`
- React updater surface: bounded state, release notes, and progress only

The legacy `cargo xtask dist` portable assembler remains available for the
v3-maintenance line but fails closed when run against the canonical v4 source.
It is not a v4 packaging entry point.

The Tauri config intentionally omits its optional `version` field so Cargo
remains the single v4 version source. `cargo xtask check static` rejects
version/config drift and accidental updater authority in the production Tauri
configuration. Test authority is compiled only by the explicit
`tauri-update-fixture` feature and exists for packaged qualification.

## Local Windows package qualification

Run these commands from PowerShell on Windows. The signing key is a
non-production local fixture and must remain outside the repository:

```powershell
$keyPath = Join-Path $env:TEMP "sky-auto-player-v4-test.key"
Push-Location desktop
bun install --frozen-lockfile
bun run build
bun run tauri signer generate --ci --password "" --force -w $keyPath
Pop-Location
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $keyPath -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
$testConfigPath = Join-Path $env:TEMP "sky-auto-player-v4-test-updater.json"
$testPublicKey = (Get-Content "$keyPath.pub" -Raw).Trim()
@{
  plugins = @{
    updater = @{
      pubkey = $testPublicKey
      endpoints = @("https://example.invalid/sky-auto-player/{{target}}/{{arch}}/{{current_version}}")
    }
  }
} | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 $testConfigPath

Push-Location desktop
bun run tauri build --ci --config $testConfigPath -- --profile dist
Pop-Location

cargo xtask verify-tauri-bundle `
  --bundle-dir rust/target/dist/bundle/nsis `
  --summary rust/target/dist/TAURI_ARTIFACT_SUMMARY.json
```

The expected bundle directory contains exactly:

```
rust/target/dist/bundle/nsis/
  Sky Auto Player_<version>_<arch>-setup.exe
  Sky Auto Player_<version>_<arch>-setup.exe.sig
```

The verifier records the exact installer and signature filenames, version,
identifier, target, install mode, and byte sizes in
`TAURI_ARTIFACT_SUMMARY.json`. It rejects MSI, portable/updater ZIP, missing
signatures, empty signatures, unexpected files, and version-naming drift.

That command is the canonical package build: it uses the normal production
feature set and the generated test key/example endpoint only to exercise
artifact signing. It does not enable the updater fixture or insecure
transport. The separate `updater_e2e` CI job builds both previous-v4 and
candidate-v4 into a `RUNNER_TEMP` `CARGO_TARGET_DIR` with
`tauri-update-fixture`; its loopback endpoint and insecure transport setting
never enter the canonical bundle or upload path.

Tauri’s updater signer reads the private key from environment variables; a
`.env` file is not used for this operation. Do not commit the generated key,
public key, updater metadata, installer, signature, or summary.

The isolated CI updater qualification runs
`scripts/ci_tauri_update_e2e.ps1`. It serves the signed candidate from a
loopback fixture, installs the previous v4 package, and verifies the official
Tauri updater reaches the candidate version after restart. The same evidence
records the ordered native quiesce, key-release, state-persistence, and
resource-close phases.

## Acceptance boundary

The local NSIS installer and updater signature require Windows packaging tools
and a test signing key. Fresh current-user install, launch, GUI smoke, and
uninstall remain Windows-manual acceptance evidence; the packaged update
qualification is the deterministic WO-03 acceptance path. WO-04 still owns
production release/channel authority.
