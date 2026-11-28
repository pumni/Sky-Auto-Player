# Antivirus Retirement Acceptance Report (Decision D23)

## 1. Decision, Scope, and Provenance

- **Decision**: Owner Decision **D23** (Option A: complete removal of ClamAV and all project-operated artifact antivirus scanning from CI/CD, without replacement).
- **Integration branch**: `refactor/source-architecture` (draft PR `#690`, coordination issue `#689`).
- **Accepted architecture source base**: [`0d95d80e24da0d900848b3f8897c5b364f1dd670`](https://github.com/pumni/Sky-Auto-Player/blob/0d95d80e24da0d900848b3f8897c5b364f1dd670/docs/evidence/source-architecture/WP6-final-acceptance.md).
- **Authorized starting HEAD**: [`ebbbe22ee620e00edb60a86ce7ae3cf2322c4787`](https://github.com/pumni/Sky-Auto-Player/blob/ebbbe22ee620e00edb60a86ce7ae3cf2322c4787/docs/antivirus-retirement-work-order.md) (archived temporary work order).
- **Supersession and historical integrity**: This report supersedes only the antivirus-specific requirements of the archived Source Architecture SPEC, implementation plan, and [WP6 final acceptance](WP6-final-acceptance.md). Historical CI failures on prior policy (including ClamAV freshness failures in runs `46127181869` and `46134444972`) remain historical `FAIL` records under the retired policy and are not retroactively waived or labeled `PASS`.
- **Revised Gate G09**: Release authority, updater Ed25519 signatures, exact-byte OIDC provenance attestation, and SPDX SBOM attestation remain intact and fail-closed, with **no antivirus proof required**. All other SPEC gates remain unchanged.

## 2. Complete Removal Ledger

### Deleted scanner scripts, Rust modules, and temporary work order

| Removed Path | Category | Purpose / Disposition |
| --- | --- | --- |
| `scripts/ci_bootstrap_clamav.sh` | Scanner script | Hosted ClamAV + NSIS apt/freshclam bootstrap removed |
| `scripts/test_ci_bootstrap_clamav.sh` | Scanner test | Fail-closed ClamAV bootstrap test harness removed |
| `scripts/scan_v4_malware_exact.py` | Scanner script | Hosted ClamAV exact-byte & EICAR scanner adapter removed |
| `scripts/scan_v4_defender_exact.ps1` | Scanner script | Retired standalone Defender exact-byte scanner removed |
| `scripts/capture_defender_evidence.ps1` | Scanner script | Retired standalone Defender evidence capture helper removed |
| `xtask/src/release_v4/malware.rs` | Rust release module | `verify-malware-scan`, `malware-evidence.json` schema/verifier, and scanner-only unit test removed |
| `xtask/src/checks/hosted_malware.rs` | Rust static check | `hosted-malware` check and `hosted_release_scanner_contract` + scanner-only unit tests removed |
| `docs/antivirus-retirement-work-order.md` | Temporary work order | Archived at immutable commit `ebbbe22ee620e00edb60a86ce7ae3cf2322c4787` in [HISTORY.md](../../HISTORY.md) and removed from active tree |

### Modified control-plane, release, test, and documentation files

- `.github/workflows/ci.yml`: Removed `malware_proof_required` classifier output projection, `Test fail-closed ClamAV bootstrap` step in `static`, the `malware_proof` job (including EICAR/NSIS fixture), and `malware_proof` from `status` needs/env/checks.
- `.github/workflows/release-v4.yml`: Restored `attest` job runner from `ubuntu-24.04` to `ubuntu-latest`; removed ClamAV bootstrap, `scan_v4_malware_exact.py`, `cargo xtask release-v4 verify-malware-scan`, `malware-evidence.json` attestation, `v4-malware-scan-*` artifact upload/download, and `malware-evidence.json` from `v4-published-*` handoff.
- `xtask/src/release_v4.rs`: Removed `mod malware`, `verify-malware-scan` CLI command/usage/job-authorization, and `malware::verify_scan` call in `verify_attestations`; retained installer, updater signature, and SPDX SBOM OIDC verification and `attested = true` state transition.
- `xtask/src/checks.rs` & `xtask/src/audits/architecture.rs`: Removed `hosted-malware` static leaf check registration, pointer, invocation, and module threshold entry.
- `xtask/src/ci_classifier.rs` & `xtask/src/ci_classifier/tests.rs`: Reduced `LANES` from 11 to 10 by removing `malware_proof_required`, removed dedicated scanner path rules and `malware_proof_required` projections, and updated classifier tests.
- `xtask/src/checks/ci_contracts.rs`, `release_pipeline.rs`, `release_runner.rs`, `hosted_signing.rs`, `rust_toolchain.rs`: Removed scanner-only assertions and the `ubuntu-24.04` scanner exception; enforced `ubuntu-latest` for all active hosted Linux jobs; added focused negative tests for missing installer/signature/SBOM attestation subjects, missing `verify_attestations` owner, and omitted mandatory aggregate dependency.
- `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`, `docs/ci-validation-topology.md`, `docs/v4-release-authority.md`, `docs/v4-release-execution-topology.md`, `docs/HISTORY.md`: Removed retired scanner references, documented Decision D23 and the accepted risk that release artifacts are not antivirus-scanned in CI/CD, and archived the work order.

## 3. Before / After CI Lane & Attestation Matrix

### CI Classifier Lanes (`xtask/src/ci_classifier.rs`)

- **Before (11 lanes)**: `rust_required`, `desktop_required`, `desktop_native_required`, `desktop_e2e_required`, `package_required`, `updater_required`, `release_required`, `signing_proof_required`, `malware_proof_required`, `supply_chain_required`, `site_required`.
- **After (10 lanes)**: `malware_proof_required` removed; all remaining 10 lanes and unknown/rename/deletion fail-closed full-validation behavior preserved.
- **Aggregate gate**: `Sky Auto Player — required CI gate` (`status` job) name preserved; `needs` reduced from 15 jobs to 14 jobs (removing `malware_proof`).

### Release OIDC Attestation Subjects (`.github/workflows/release-v4.yml`)

- **Before**: `candidate-bundle/*.exe`, `candidate-bundle/*.sig`, `SBOM.spdx.json` (on `*.exe`), and `malware-evidence.json`.
- **After**: `candidate-bundle/*.exe`, `candidate-bundle/*.sig`, and `SBOM.spdx.json` (on `*.exe`).

## 4. Test Inventory Changes

- **Removed scanner-only tests**:
  - `scripts/test_ci_bootstrap_clamav.sh` (and CI step `Test fail-closed ClamAV bootstrap`)
  - `xtask::release_v4::malware::tests::exact_signed_installer_malware_evidence_is_required_and_bounded`
  - `xtask::checks::hosted_malware::tests::*` (3 unit tests covering `hosted_malware_proof_contract` and `hosted_release_scanner_contract`)
  - `xtask::checks::ci_contracts::tests::pinned_ubuntu_2404_is_allowed_only_for_malware_scanner_jobs` (replaced by expanding `active_github_hosted_ubuntu_contract_rejects_numbered_labels` to cover `release-v4.yml` `attest`)
  - `xtask::checks::release_runner::tests` malware-specific fixture assertions
- **Retained tests**:
  - All 34 runnable scenarios in `cargo xtask release-v4 self-test` (unchanged)
  - All remaining `xtask` unit, architecture, security, docs-integrity, classifier, signing, SBOM, and release contract tests
- **Added negative tests**:
  - `xtask::checks::release_pipeline::tests::hosted_release_contract_rejects_missing_attestation_or_sbom` (proves omitting `candidate-bundle/*.exe`, `candidate-bundle/*.sig`, `candidate-evidence/SBOM.spdx.json`, or `fn verify_attestations` fails)
  - `xtask::checks::release_runner::tests` (`missing_signing_dependency` in `release_contract_ci_wiring_requires_rust_mock_tests_and_independent_static_gate`, proving omitting a mandatory remaining aggregate dependency fails)

## 5. Unchanged Invariants

- **Product runtime & dependencies**: Zero changes to `crates/**`, `apps/**`, `Cargo.toml`, `Cargo.lock`, `apps/desktop/package.json`, `apps/desktop/bun.lock`, `rust-toolchain.toml`, `.bun-version`, `supply-chain/**`, or the 93 `ts-rs` bindings.
- **Unrelated security controls**: `cargo audit`, `cargo vet`, OSV/supply-chain checks, locked builds, Authenticode verification (`unsigned-zero-budget` policy), Ed25519 updater signing (`Key ID 19AABD2E7838818C`), OIDC provenance attestations, SPDX-2.3 SBOM generation/verification, exact-byte SHA-256 binding, NSIS install/launch/uninstall qualification, and release transaction/recovery authority remain unchanged.
- **External settings**: No changes made to OS antivirus settings, branch protection, credentials, or live releases. Note for leader/owner review: external branch protection should continue requiring `Sky Auto Player — required CI gate` only; if any external rule references `malware_proof`, it must be cleared by the owner.

## 6. Local Validation & Clean Locked Dist Build

| Check | Command | Outcome | Notes |
| --- | --- | --- | --- |
| Release self-test | `node scripts/toolchain-pin.mjs exec -- cargo xtask release-v4 self-test` | **PASS** (exit 0) | `V4 Rust release state-machine mock self-test: PASS (34 scenarios)` |
| Clean locked dist build | `bun install --frozen-lockfile && node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle -- --locked --profile dist` (in `apps/desktop`) | **PASS** (exit 0, 6m 40s) | `target/dist/sky_desktop_shell.exe`: `16,670,208` bytes, SHA-256 `62581F07B59BFC0963D61055FF7EFB1B7C150478D2BB53B6EE75F68C5E0F78AB` |
| xtask unit & contract suite | `node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path xtask/Cargo.toml` | **PASS** (exit 0, 10.04s) | `351 passed; 0 failed; 4 ignored` (`sky_xtask` unit/contract suite) + `2 passed; 0 failed` |
| Canonical `check all` | `node scripts/toolchain-pin.mjs exec -- cargo xtask check all` (isolated `TS_RS_EXPORT_DIR`) | **PASS** (exit 0) | Full `static` (including `cargo vet`, `docs-integrity`, `ci-control-plane`, `release-pipeline`, `release-runner`, `hosted-signing`), `rust` (`fmt`, `clippy --profile dist`, workspace tests), and `desktop` (`bun run check`, `bun run test:e2e`, `tauri` clippy/check/tests) |

## 7. Residual Risk

Per Owner Decision D23, project CI/CD no longer performs pre-publication antivirus/malware scanning of release artifacts. Integrity, Ed25519 updater signatures, GitHub OIDC provenance attestations, SPDX SBOM attestation, and supply-chain dependency audits remain binding, but do not replace malware detection.
