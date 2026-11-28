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
- `xtask/src/ci_classifier.rs` & `xtask/src/ci_classifier/tests.rs`: Reduced `LANES` from 11 to 10 by removing `malware_proof_required`; removed dedicated scanner path rules; routed unrecognized `xtask/src/release_v4/*` and non-live `scripts/*` extensions (such as `.sh`/`.py`) to fail-closed `unknown` full validation so deletions/renames of retired paths preserve `release_required` without antivirus-specific routing; added real-Git deletion/rename regression tests.
- `xtask/src/checks/ci_contracts.rs`, `release_pipeline.rs`, `release_runner.rs`, `hosted_signing.rs`, `rust_toolchain.rs`, `xtask/src/release_v4/tests.rs`, `xtask/src/release_v4/selftest.rs`: Removed scanner-only assertions and the `ubuntu-24.04` scanner exception; enforced `ubuntu-latest` for all active hosted Linux jobs; strengthened `attest` step-level and `verify_attestations` body checks; added focused static and runtime negative tests for missing/inactive attestation steps, missing/wrong SBOM predicate, wrong source digest/signer workflow, premature `state.attested = true`, and omitted mandatory aggregate dependency.
- `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`, `docs/ci-validation-topology.md`, `docs/v4-release-authority.md`, `docs/v4-release-execution-topology.md`, `docs/HISTORY.md`: Removed retired scanner references, documented Decision D23 and the accepted risk that release artifacts are not antivirus-scanned in CI/CD, and archived the work order.

## 3. Before / After CI Lane, Classifier Projection, and Attestation Matrix

### CI Classifier Lanes (`xtask/src/ci_classifier.rs`)

- **Before (11 lanes)**: `rust_required`, `desktop_required`, `desktop_native_required`, `desktop_e2e_required`, `package_required`, `updater_required`, `release_required`, `signing_proof_required`, `malware_proof_required`, `supply_chain_required`, `site_required`.
- **After (10 lanes)**: `malware_proof_required` removed; all remaining 10 lanes and unknown/rename/deletion fail-closed full-validation behavior preserved.
- **Aggregate gate**: `Sky Auto Player — required CI gate` (`status` job) name preserved; `needs` reduced from 15 jobs to 14 jobs (removing `malware_proof`).

### Representative Classifier Projection Comparison (Live, Deleted, and Renamed Paths)

| Path / Scenario | Before (11 lanes) | After (10 lanes) |
| --- | --- | --- |
| `apps/desktop/src/App.tsx` | `desktop_required`, `desktop_e2e_required` | `desktop_required`, `desktop_e2e_required` |
| `apps/desktop/src-tauri/tauri.conf.json` | `desktop_required`, `desktop_native_required`, `package_required`, `updater_required`, `signing_proof_required`, `malware_proof_required` | `desktop_required`, `desktop_native_required`, `package_required`, `updater_required`, `signing_proof_required` |
| `apps/desktop/src-tauri/nsis-hooks.nsh` | `desktop_native_required`, `package_required`, `signing_proof_required`, `malware_proof_required` | `desktop_native_required`, `package_required`, `signing_proof_required` |
| `.github/workflows/release-v4.yml` | `release_required`, `signing_proof_required`, `malware_proof_required` | `release_required`, `signing_proof_required` |
| `scripts/v4_nsis_smoke_boundary.ps1` | `release_required`, `package_required`, `signing_proof_required`, `malware_proof_required` | `release_required`, `package_required`, `signing_proof_required` |
| `scripts/ci_bootstrap_clamav.sh` (deleted or `git mv` old-side) | `release_required`, `malware_proof_required` | Full validation (`LANES`, including `release_required`) via fail-closed `unknown` |
| `scripts/test_ci_bootstrap_clamav.sh` (deleted or `git mv` old-side) | `release_required`, `malware_proof_required` | Full validation (`LANES`, including `release_required`) via fail-closed `unknown` |
| `scripts/scan_v4_malware_exact.py` (deleted or `git mv` old-side) | `release_required`, `malware_proof_required` | Full validation (`LANES`, including `release_required`) via fail-closed `unknown` |
| `xtask/src/release_v4/malware.rs` (deleted or `git mv` old-side) | `rust_required`, `release_required`, `malware_proof_required` | Full validation (`LANES`, including `rust_required`, `release_required`) via fail-closed `unknown` |
| `xtask/src/checks/hosted_malware.rs` (deleted or `git mv` old-side) | Full validation (`LANES`) via `unknown` | Full validation (`LANES`) via `unknown` |

### Release OIDC Attestation Subjects (`.github/workflows/release-v4.yml`)

- **Before**: `candidate-bundle/*.exe`, `candidate-bundle/*.sig`, `SBOM.spdx.json` (on `*.exe`), and `malware-evidence.json`.
- **After**: `candidate-bundle/*.exe`, `candidate-bundle/*.sig`, and `SBOM.spdx.json` (on `*.exe`).

## 4. Test Inventory Changes

- **Workspace test selector counts (`cargo test --workspace --all-features --locked -- --list`)**:
  - **Before (`0d95d80e`)**: `1,524` test selectors total (`360` in `sky_xtask`: `358` in `src/main.rs` + `2` in `tests/credential_cli_windows.rs`).
  - **After**: `1,522` test selectors total (`359` in `sky_xtask`: `357` in `src/main.rs` + `2` in `tests/credential_cli_windows.rs`, net `-2`; all product crates unchanged).
- **Individually named removed tests**:
  - `scripts/test_ci_bootstrap_clamav.sh` (and CI step `Test fail-closed ClamAV bootstrap`)
  - `xtask::release_v4::malware::tests::exact_signed_installer_malware_evidence_is_required_and_bounded`
  - `xtask::checks::hosted_malware::tests::hosted_release_requires_archive_detection_and_fresh_definitions`
  - `xtask::checks::hosted_malware::tests::malware_proof_requires_fresh_scanner_positive_control_and_tamper_rejection`
  - `xtask::checks::ci_contracts::tests::pinned_ubuntu_2404_is_allowed_only_for_malware_scanner_jobs` (replaced by expanding `active_github_hosted_ubuntu_contract_rejects_numbered_labels` to cover `release-v4.yml` `attest`)
- **Retained mixed-purpose tests**:
  - All 34 runnable scenarios in `cargo xtask release-v4 self-test`
  - `xtask::checks::ci_contracts::tests` (`ci_control_plane_contract`, `packaged_ci_contract`, `active_github_hosted_ubuntu_contract_rejects_numbered_labels`)
  - `xtask::checks::release_runner::tests::release_contract_ci_wiring_requires_rust_mock_tests_and_independent_static_gate` (malware fixture assertions removed; remaining aggregate dependency negative assertion `missing_signing_dependency` added)
  - `xtask::checks::hosted_signing::tests::signing_proof_job_is_conditional_isolated_and_unsigned_before_test_key_signing` (fixture anchor moved from `malware_proof:` to `site:`)
  - `xtask::ci_classifier::tests` (`preserves_exact_lane_output_shape_and_reason_golden`, `preserves_current_positive_path_matrix_and_script_categories`, `matches_positive_and_union_classifier_matrix`, `unknown_or_malformed_paths_fail_closed_and_workflow_dispatch_is_full`)
- **Added regression & negative tests**:
  - `xtask::ci_classifier::tests::retired_release_and_script_paths_fail_closed_on_real_git_deletions_and_renames` (real `git rm` and `git mv` across all 5 retired release/script paths)
  - `xtask::checks::release_pipeline::tests::hosted_release_contract_rejects_missing_attestation_or_sbom` (proves missing/inactive installer, signature, or SBOM steps, missing `--source-digest` / `--signer-workflow` / SPDX predicate, or premature `state.attested = true` fail)
  - `xtask::release_v4::tests::verify_attestations_enforces_installer_signature_sbom_and_fails_closed_before_state_transition` (exercises actual production `verify_attestations` with compiled fake `gh.exe` proving installer, signature, SBOM, exact source/signer enforcement, non-`Ready` rejection, and no premature `attested = true`)

## 5. Tracked-Reference Inventory Adjudication

`git grep -inE "clamav|clamscan|freshclam|malware|antivirus|defender|eicar|verify-malware-scan|hosted-malware|malware_proof|malware-evidence" -- ":!docs/evidence/**" ":!docs/archive/**"` returns only:

| File & Line(s) | Classification | Adjudication |
| --- | --- | --- |
| `SECURITY.md:89` | Active policy | Explicit D23 disclosure that release artifacts are not antivirus-scanned by project CI/CD |
| `docs/v4-release-authority.md:147-148` | Active policy | Explicit D23 disclosure and rationale under `VerifyCandidate / VerifyAttestations` |
| `docs/v4-release-execution-topology.md:136,139,140,142` | Active policy | Explicit D23 disclosure and risk acceptance note |
| `docs/HISTORY.md:19-22` | Historical provenance | Immutable archive link to `ebbbe22ee620e00edb60a86ce7ae3cf2322c4787/docs/antivirus-retirement-work-order.md` and link to this report |
| `xtask/src/ci_classifier/tests.rs:1443-1447` | Regression fixture | Real-Git deletion/rename test paths proving fail-closed classification on retired paths |

## 6. Unchanged Invariants & Dist Build Identity

- **Product runtime & dependencies**: Zero changes to `crates/**`, `apps/**`, `Cargo.toml`, `Cargo.lock`, `apps/desktop/package.json`, `apps/desktop/bun.lock`, `rust-toolchain.toml`, `.bun-version`, `supply-chain/**`, or the 93 `ts-rs` bindings.
- **Unrelated security controls**: `cargo audit`, `cargo vet`, OSV/supply-chain checks, locked builds, Authenticode verification (`unsigned-zero-budget` policy), Ed25519 updater signing (`Key ID 19AABD2E7838818C`), OIDC provenance attestations, SPDX-2.3 SBOM generation/verification, exact-byte SHA-256 binding, NSIS install/launch/uninstall qualification, and release transaction/recovery authority remain unchanged.
- **Dist build argv & embedded identity**: `apps/desktop/package.json` `tauri:build` runs `tauri build --ci -- --locked --profile dist` (verified unbundled via `bun run tauri build --ci --no-bundle -- --locked --profile dist`). `apps/desktop/src-tauri/build.rs` embeds `SKY_NATIVE_BUILD_COMMIT` from `git rev-parse --verify HEAD` (appending `-dirty` when `git status --porcelain` is non-empty).
- **External settings**: No changes made to OS antivirus settings, branch protection, credentials, or live releases. External branch protection should continue requiring `Sky Auto Player — required CI gate` only; if any external rule references `malware_proof`, it must be cleared by the owner.

## 7. Local Validation & Clean Locked Dist Build

| Check | Command | Outcome | Notes |
| --- | --- | --- | --- |
| Release self-test | `node scripts/toolchain-pin.mjs exec -- cargo xtask release-v4 self-test` | **PASS** (exit 0) | `V4 Rust release state-machine mock self-test: PASS (34 scenarios)` |
| Clean locked dist build (prior source) | `bun install --frozen-lockfile && node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle -- --locked --profile dist` (in `apps/desktop`) | **PASS** (exit 0, 6m 40s) | `target/dist/sky_desktop_shell.exe`: `16,670,208` bytes, SHA-256 `62581F07B59BFC0963D61055FF7EFB1B7C150478D2BB53B6EE75F68C5E0F78AB` |
| Final validation | `cargo test --locked --manifest-path xtask/Cargo.toml` + `cargo xtask check all` (isolated `TS_RS_EXPORT_DIR`) | Executed on clean committed source | Exact SHA/tree, exit status, and counts reported in final `#689` handoff |

## 8. Residual Risk

Per Owner Decision D23, project CI/CD no longer performs pre-publication antivirus/malware scanning of release artifacts. Integrity, Ed25519 updater signatures, GitHub OIDC provenance attestations, SPDX SBOM attestation, and supply-chain dependency audits remain binding, but do not replace malware detection.
