# V4 Tauri Packaging Foundation

V4 uses the Tauri Windows bundler as its canonical package producer. The
foundation enables the NSIS package and updater artifact generation. The
Rust-owned `UpdateService` now drives the official Tauri updater through the
dedicated v4 release authority described in `v4-release-authority.md`.
Production metadata may be absent until a qualified promotion. The independent
Tauri updater trust root is committed as public material in the Tauri config
and Rust updater boundary; missing or invalid trust material fails closed.
Authenticode provider credentials remain an external Track B release input
and are never committed; Track B must also provide the approved signer
thumbprint used by production verification.

## Canonical package contract

- Application identifier: `io.github.pumni.skyautoplayer`
- Version source: `desktop/src-tauri/Cargo.toml`
- Current foundation version: `4.0.0-alpha.1`
- Windows target: NSIS only
- Installer scope: `currentUser` under `%LOCALAPPDATA%`
- Updater output: the NSIS setup executable and its `.exe.sig` sidecar
- Legacy `sky_updater`: retained and buildable until the later retirement work order
- Runtime updater: official `tauri-plugin-updater`, behind Rust `UpdateService`
- Tauri updater trust: v4-only public root; no v3 `release-2026` key reuse
- Authenticode: configured through a fail-closed `signCommand` seam
- React updater surface: bounded state, release notes, and progress only

The legacy `cargo xtask dist` portable assembler remains available for the
v3-maintenance line but fails closed when run against the canonical v4 source.
It is not a v4 packaging entry point.

The Tauri config intentionally omits its optional `version` field so Cargo
remains the single v4 version source. It contains only the v4 updater public
root; Rust owns the fixed stable/beta endpoint selection. `cargo xtask check
static` rejects version/config drift, v3 key reuse, private-key material, and
secret output. Test authority is compiled only by the explicit
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
$env:SKY_AUTHENTICODE_MODE = "test"
$testSigningEnv = Join-Path $env:TEMP "sky-v4-test-signing.env"
$env:GITHUB_ENV = $testSigningEnv
pwsh scripts/setup_v4_test_signing.ps1 -EnvFile $testSigningEnv
Get-Content $testSigningEnv | ForEach-Object {
  $name, $value = $_ -split "=", 2
  Set-Item -Path "Env:$name" -Value $value
}
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
  --summary rust/target/dist/TAURI_ARTIFACT_SUMMARY.json `
  --authenticode-evidence rust/target/dist/TAURI_AUTHENTICODE_EVIDENCE.json `
  --sbom rust/target/dist/SBOM.spdx.json
```

The build config invokes `scripts/sign_v4_authenticode.ps1`; it signs only in
test mode with the ephemeral certificate. In production mode it fails closed
until an approved Authenticode provider is configured. To create the evidence
used above, run:

```powershell
pwsh scripts/verify_v4_authenticode.ps1 `
  -Mode test `
  -Artifact (Get-ChildItem rust/target/dist/bundle/nsis/*-setup.exe).FullName `
  -Evidence rust/target/dist/TAURI_AUTHENTICODE_EVIDENCE.json
cargo xtask sbom generate --artifact-dir rust/target/dist/bundle/nsis --output rust/target/dist/SBOM.spdx.json
cargo xtask sbom verify --artifact-dir rust/target/dist/bundle/nsis --sbom rust/target/dist/SBOM.spdx.json
pwsh scripts/cleanup_v4_test_signing.ps1
```

The expected bundle directory contains exactly:

```
rust/target/dist/bundle/nsis/
  Sky Auto Player_<version>_<arch>-setup.exe
  Sky Auto Player_<version>_<arch>-setup.exe.sig
```

The Authenticode verifier requires an embedded signer certificate whose exact
thumbprint matches the test PFX identity in test mode. It proves the file
signature and digest independently of public CA trust. Windows may report the
self-signed test certificate as `NotTrusted` or `UnknownError`; only those two
statuses are accepted in test mode, and only with the explicit
`test-self-signed-untrusted-chain` evidence marker. Production verification
requires `Valid` plus the separately approved
`SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT`; it never accepts an arbitrary
trusted signer or the test exception. It records signer status and SHA-256 for
every project-owned PE in the installed tree (the generated NSIS
`uninstall.exe` is checked for presence but is not a project-owned binary);
the bundle verifier separately binds the final installer evidence to the exact
NSIS candidate. The SPDX generator
records SHA-256 for the exact two-file NSIS artifact set and binds it to the
current commit. The SBOM covers the reachable Rust production graph from
`rust/Cargo.lock` and the frontend production graph from `desktop/bun.lock`,
and binds both lockfile hashes to the same artifact set. After the
install/launch/uninstall smoke, the packaged
qualification path emits `V4_QUALIFICATION_EVIDENCE.json` with artifact,
Authenticode evidence, SBOM, and digest references. Promotion requires
production Authenticode mode and compares installer/.sig digests to the exact
published GitHub asset bytes. It rejects MSI, portable/updater ZIP, missing
signatures, empty signatures, unexpected files, and version-naming drift.

That command is the canonical package build: it uses the normal production
feature set and the generated test updater key/example endpoint only to
exercise updater artifact signing. The Authenticode certificate is an
ephemeral test fixture. The setup step creates its private key and certificate
in a password-protected PFX under `RUNNER_TEMP`; it never imports the
certificate into a Windows trust store. The CI always deletes the PFX and its
password environment variable after signing and installed-PE verification.
It does not enable the updater fixture or insecure transport. The separate
`updater_e2e` CI job builds both previous-v4 and
candidate-v4 into a `RUNNER_TEMP` `CARGO_TARGET_DIR` with
`tauri-update-fixture`; its loopback endpoint and insecure transport setting
never enter the canonical bundle or upload path.

Tauri’s updater signer reads the non-production fixture private key from
environment variables; a `.env` file is not used for this operation. Do not
commit any private updater key, certificate/PFX, updater metadata, installer,
signature, or summary. The committed updater public root is the only release
key material in the source tree.

## Updater key rotation fixture

`scripts/test_v4_updater_key_rotation.ps1` generates two disposable Tauri
signer pairs outside the repository and exercises the Rust rotation policy.
The packaged CI fixture additionally builds an actual bridge client with
`[old,new]` and a cutover client with `[new]`. Runtime fallback happens at
Tauri `Update::download()` for each trust context, so a bridge can apply a
candidate signed only by the new root; after cutover, an old-root-only
candidate is rejected. The production list is kept in
`desktop/src-tauri/src/native_update.rs`; add the next public root there for a
bridge, publish the bridge, then remove the old root after the cutover release.
The private halves are never read into repository files or logs.

## Operational key handling

The release operator stores the private updater key and any approved
Authenticode-provider credential in an offline encrypted vault, with a second
offline backup under separate access control. Neither is copied into GitHub,
build artifacts, frontend state, or logs. Loss of the updater key is fail-closed:
create a new v4 root and release version, then use the normal reviewed
qualification path. Suspected compromise requires revoking the old root through
the bridge/cutover sequence, rotating the Authenticode provider if applicable,
and publishing a new version; existing artifact bytes are never repaired in
place.

The isolated CI updater qualification runs
`scripts/ci_tauri_update_e2e.ps1`. It serves the signed candidate from a
loopback fixture, installs the previous v4 package, and verifies the official
Tauri updater reaches the candidate version after restart. It also verifies
the bridge `[old,new]` to cutover `[new]` trust transition against real
packaged clients. The same evidence records the ordered native quiesce,
key-release, state-persistence, and resource-close phases.

## Acceptance boundary

The local NSIS installer and updater signature require Windows packaging tools
and a test signing key. Fresh current-user install, launch, GUI smoke, and
uninstall remain Windows-manual acceptance evidence; the packaged update
qualification is the deterministic previous-v4 -> candidate-v4 acceptance
path. A full non-PR run must be dispatched from the branch so provenance and
SPDX attestations are generated and verified before acceptance. The production
release/channel authority is configured by WO-04. The v4 updater public trust
root and rotation policy are implemented here; an approved Authenticode
provider and release credentials remain Track B work.
