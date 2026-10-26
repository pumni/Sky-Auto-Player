# Hosted V4 Test-Key Signing Proof and Migration Ledger

The audit and dependency table below are a historical Wave A snapshot of the release path before
the protected hosted migration. The Wave B implementation record at the end supersedes those
active-path statements without changing the original proof evidence.

## Wave A source audit snapshot

The release authority is `rust/xtask/src/release_v4.rs`. `preflight` binds the release context to
the exact canonical `main` SHA and reads channel metadata, tag, draft, and GitHub Latest state
through its transport interface before writing owned local state. `build_candidate` requires the
private-key path outside the workspace, rejects ambient Tauri signing variables, and delegates the
build/signing action through the Windows Credential Manager adapter. It accepts only the canonical
NSIS installer and `.sig` pair, freezes both into transaction state, and runs the unsigned
Authenticode, updater signature, SBOM, and package evidence gates before publication. The publish,
metadata, and cleanup operations remain under this same transaction authority and its fake
transport tests cover failures and owned-draft cleanup. Candidate construction adds the locked
four-file schema and SHA-256 inventory; the exact candidate then goes through the updater HTTP
fixture and Defender scan. The production workflow attests the exact installer, signature, and
SPDX SBOM before its read-only publication gates.

The existing CI candidate is a separate four-file contract in `rust/xtask/src/ci_candidate.rs`:
`candidate.json`, the canonical `*-setup.exe`, its exact `.sig` sidecar, and
`updater-public-key.pub`. It verifies schema, source SHA, source version, file inventory, safe
names, architecture, and recorded SHA-256 values. That CI artifact contract is not itself a
cryptographic signature check; the production release authority performs that check with the
canonical v4 root. Candidate fixtures therefore exercise contract and digest failures without
standing in for a real Tauri signature. The current PR candidate job generates an ephemeral test
key and bundles the updater signature in the same Tauri build; the hosted proof added here tests the
unsigned-build then standalone-sign route separately.

`desktop/src-tauri/tauri.conf.json` currently selects NSIS, current-user installation, updater
artifact generation during bundling, the deployed v4 public root, and the intentional unsigned
Authenticode command. `release-v4.yml` and `rehearse-v4.yml` both remain on the dedicated
self-hosted Windows runner. `rust/xtask/src/checks/release_runner.rs` statically enforces that
production runner and release workflow boundary. Wave A adds a separate static contract for the
hosted test-key workflow and does not relax that production guard.

## Proof route

`.github/workflows/hosted-v4-signing-proof.yml` runs on relevant pull requests and can be manually
dispatched from a feature branch. It uses `windows-latest`, a read-only `contents` token, pinned
checkout and Bun setup actions, and Rust 1.99.1. It does not use a protected environment, GitHub
secrets, the GitHub API, release commands, artifact uploads, or metadata writes.

The workflow builds one canonical NSIS installer with Tauri updater artifact creation disabled and
with all ambient Tauri signing variables removed. It hashes that installer, then in a later step
generates a disposable updater key under `RUNNER_TEMP` and signs the unchanged file with Tauri CLI
2.12.3 and `--app-version`. The Rust verifier checks the exact NSIS filename, installer SHA-256,
signature bytes, signed filename and version, and public key identity. The workflow first runs
`cargo xtask ci source-identity` to bind the actual clean checkout to `GITHUB_SHA`; the verifier's
`--source-sha` argument validates SHA syntax and records it but does not independently compare it to
Git HEAD. It writes bounded proof JSON to the same temporary directory. An unconditional cleanup step removes
the owned build tree and test key after success or failure.

The verifier refuses the canonical production updater public root. Rust negative tests cover a
wrong key, wrong signed version, modified installer bytes, a partial signature, and a mismatched
installer digest. Fixed synthetic Tauri signature vectors exercise that wrapper without storing
or deriving any private key in the repository.

This proof establishes that Tauri can package the canonical installer without an updater signing
credential and that its standalone signer creates a version-bound signature accepted by the Rust
Minisign verifier over the exact installer bytes. It does not use the v4 production key, validate a
production signature, or prove that the test-signed installer is accepted by the product's compiled
production trust root. The deployed v4.2.0 signature is independently checked below; validation of
new production candidates remains in the protected pre-publication gate.

## Empirical local package result

On a local Windows host, Tauri CLI 2.12.3 built the canonical `4.2.0` NSIS installer with
`createUpdaterArtifacts=false`, `--bundles nsis`, and `--profile dist`. Bun was 1.4.4 on this host;
the hosted workflow pins Bun 1.4.3. The isolated NSIS output contained exactly one file and no
updater sidecar. All three ambient `TAURI_SIGNING_PRIVATE_KEY*` variables were removed before the
build. Tauri's standalone signer then signed the same installer bytes with a freshly generated
test key, and `cargo xtask updater-trust verify-test-signature` accepted the signature and signed
filename/version.

The temporary installer was 4,627,564 bytes with SHA-256
`8d9280a0851c88621f9970139080b20095bbdf254070ce9351671fe4d528cfee`; the signature SHA-256 was
`552eba4ce72ecdf64c7dd0a64e6dac8bd60b91f19e2e40ef0dbc8e004aeadc05`. The signed trusted comment
matched `Sky Auto Player_4.2.0_x64-setup.exe` and version `4.2.0`. The ephemeral private key,
installer, signature, and evidence JSON were removed after verification. This local run used the
Wave A working tree at base HEAD `f3516430eb6d166da01c1e9754a735e875deec04`, so it proves the
packaging/signing mechanics but is not clean-commit or GitHub-hosted evidence. The hosted PR job
repeats the proof from a clean checkout and reports the commit-bound hashes in its step summary.

The existing public v4.2.0 release was also verified read-only with the canonical production public
root and the new expected-version check. The downloaded installer and `.sig` hashes matched the
GitHub release asset digests (`a6048bd8dc8691798ed2ec7078dd66abe57a5cedfd4d086eedb10cd185751105`
and `2f1603eb7448339a4a3bd43ec892298a1d82421a38d5bf72ceb0bc4984d678ce`); Rust verification
passed for version `4.2.0` and public key ID `19AABD2E7838818C`. Verification used local copies
named with the internal NSIS basename because the public release normalizes spaces to dots. No
production signing secret was read and no release asset or metadata was changed.

## Wave A decision

The installed Tauri CLI pin is 2.12.3. Its local `signer sign --app-version` command produced a
Minisign signature over a real NSIS installer whose trusted comment contains the signed installer
filename and exact app version. Wave A therefore selects unsigned hosted packaging followed by a
separate signing step as the preferred Wave B design. The Wave A workflow validates that path using
only disposable test keys. Wave B must retain one production installer build and move the production
key boundary into the protected GitHub environment; this proof does not authorize production
dispatch or alter the current v4.2.0 updater key.

## Runner-local and Credential Manager dependency ledger (Wave A baseline)

These are active production dependencies observed at the Wave A source revision. Wave A leaves them
in place so that a future hosted transaction can replace them coherently after review.

| Dependency observed at the Wave A source revision | Source and consumer at that revision | Wave B plan recorded at Wave A |
| --- | --- | --- |
| Dedicated self-hosted Windows runner labels | `.github/workflows/release-v4.yml` job `release`; `.github/workflows/rehearse-v4.yml` job `qualification`; the allowlist in `rust/xtask/src/checks/release_runner.rs`; operator requirements in `docs/v4-release-execution-topology.md` | Replace only in the protected production workflow after environment restrictions and hosted source/ref guards are implemented. Keep the test-key proof on ordinary hosted Windows. |
| Runner-local `RUNNER_TEMP` release state | Both production workflows; `rust/xtask/src/release_v4.rs` state-root validation and cleanup | Keep bounded state outside the workspace. Revalidate path ownership and cleanup on GitHub-hosted runners. |
| `V4_UPDATER_PRIVATE_KEY_PATH` and key outside workspace | Both production workflows; `scripts/verify_v4_release_runner.ps1`; `release_v4::build_candidate` in `rust/xtask/src/release_v4.rs` | Replace with a Rust-owned protected-environment secret adapter. Do not pass key contents in workflow arguments, logs, or artifacts. |
| Windows Credential Manager passphrase transport | `rust/xtask/src/credential_cli.rs`; `rust/crates/sky_windows_security/src/credential.rs` and `src/win32.rs`; `SkyAutoPlayer/V4UpdaterProduction` target | Remove from the production hosted path after a scoped adapter is proven. Inventory local maintenance users before retiring the credential CLI. |
| Operator provisioning and verification adapters | `scripts/set_v4_updater_session_credential.ps1`, `scripts/remove_v4_updater_session_credential.ps1`, `scripts/verify_v4_updater_private_key.ps1`, and `scripts/v4_updater_credential_broker.ps1`; current instructions in `docs/v4-updater-key-custody.md` | Retain any independently used local maintenance tools; update production runbooks when protected environment custody is implemented. |

At Wave A, the hosted proof workflow was not part of the then-current production runner allowlist.
Its synthetic key lifecycle was local to one hosted job and could not publish a GitHub Release or
write stable/beta metadata.

## Wave B implementation record

Wave B replaces the baseline production topology with one workflow,
`.github/workflows/release-v4.yml`, on GitHub-hosted Windows and Ubuntu jobs. It gates dispatch on
canonical `main`, a successful latest exact-head push CI run, and Rust Preflight. The separate
`rehearse-v4.yml` production rebuild is retired.

The workflow builds one unsigned canonical NSIS installer, exports a run/attempt-bound manifest,
and transfers that artifact to `sign_qualify`. Rust validates the exact file inventory, source and
workflow identity, version, size, and SHA-256 before signing. The protected key and passphrase names
are `V4_TAURI_UPDATER_PRIVATE_KEY` and `V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD`; the workflow maps
them only to `sign-candidate` in Environment `v4-production-release`. Rust verifies the key against
the unchanged public Key ID `19AABD2E7838818C`, signs the transferred bytes, confirms the installer
digest is unchanged, then exits. A following step qualifies the same bytes without signing secrets.

Qualification, OIDC attestations, exact-digest publication, scoped metadata probing/promotion, and
final read-back remain in Rust-owned release transaction phases. The public pair remains exactly
the canonical `.exe` and `.sig`; v4.2.0's stable endpoint, updater JSON/signature contract, NSIS
identity, and trust root are unchanged. The Rust credential CLI and Credential Manager helpers stay
available to independent local maintenance consumers; the production workflow no longer requires
them.

This code review and its tests do not inspect or use GitHub Environment secret values/settings and
do not dispatch the production workflow or mutate a release, tag, stable/beta metadata, or a
published artifact. A maintainer still needs to verify the live Environment's main-only deployment
restriction and human approval policy before cutover; workflow YAML cannot prove those settings.
