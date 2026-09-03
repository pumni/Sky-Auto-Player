# V4 Tauri Packaging Foundation

V4 uses the Tauri Windows bundler as its canonical package producer. The
current foundation is intentionally local-only: it enables the NSIS package
and updater artifact generation without publishing an endpoint, embedding a
production updater key, or switching the runtime update path.

## Canonical package contract

- Application identifier: `io.github.pumni.skyautoplayer`
- Version source: `desktop/src-tauri/Cargo.toml`
- Current foundation version: `4.0.0-alpha.1`
- Windows target: NSIS only
- Installer scope: `currentUser` under `%LOCALAPPDATA%`
- Updater output: the NSIS setup executable and its `.exe.sig` sidecar
- Legacy `sky_updater`: retained and buildable until the later retirement work order

The Tauri config intentionally omits its optional `version` field so Cargo
remains the single v4 version source. `cargo xtask check static` rejects
version/config drift and any updater endpoint or trust key added during this
foundation phase.

## Local Windows package qualification

Run these commands from PowerShell on Windows. The signing key is a
non-production local fixture and must remain outside the repository:

```powershell
$keyPath = Join-Path $env:TEMP "sky-auto-player-v4-test.key"
Push-Location desktop
bun install --frozen-lockfile
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

Tauri’s updater signer reads the private key from environment variables; a
`.env` file is not used for this operation. Do not commit the generated key,
public key, updater metadata, installer, signature, or summary.

## Acceptance boundary

The local NSIS installer and updater signature require Windows packaging tools
and a test signing key. Fresh current-user install, launch, GUI smoke, and
uninstall remain Windows-manual acceptance evidence for this work order. The
production v4 updater service and release-authority endpoint are intentionally
deferred to later work orders.
