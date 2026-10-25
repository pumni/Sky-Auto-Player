# PowerShell dependency ledger

This ledger records the active in-repository owners and callers for tooling migrated in Waves A-D.
The inventory was checked against workflows, Tauri configuration, Rust static contracts, tests,
dot-sourcing and command call sites, and operator documentation. The production release API has no
mocked CI caller outside the Rust fake transport. The supported production release entry point is
the protected release-v4 workflow.

Disposition values:

- MIGRATED — Rust owns the policy or implementation, all documented callers use Rust, and the
  former script and script-only tests are removed.
- ADAPTER RETAINED — PowerShell remains only at a narrow Windows, credential, or provider seam.
- MANUAL KEEP — a specialized operator tool remains available by design.
- BLOCKED — ownership belongs to a later locked wave or removal needs separate evidence.

## Migrated implementations

| Former PowerShell owner | Rust owner | Caller and regression evidence | Disposition |
| --- | --- | --- | --- |
| scripts/ci_classify.ps1, scripts/test_ci_classify.ps1 | rust/xtask/src/ci_classifier.rs | changes in ci.yml; exact lane outputs, malformed Git paths, and full-validation fallback tests | MIGRATED in Wave A |
| scripts/ci_validate_candidate.ps1, scripts/test_ci_validate_candidate.ps1 | rust/xtask/src/ci_candidate.rs, ci_artifacts.rs | candidate producer, updater_e2e, and packaged jobs; typed schema, exact inventory, and tamper negatives | MIGRATED in Wave A |
| scripts/ci_validate_bridge.ps1, scripts/test_ci_validate_bridge.ps1, scripts/ci_build_updater_bridge.ps1 | rust/xtask/src/ci_bridge.rs, updater_fixture.rs | updater_bridge builds from a detached worktree; Rust creates and revalidates bridge output | MIGRATED in Waves A/C |
| scripts/build_native_calibration.ps1, scripts/verify_installed_calibration.ps1, scripts/test_installed_calibration.ps1 | rust/xtask/src/calibration.rs | Tauri prebuild, candidate packaging, bounded helper process tree, and installed-image checks | MIGRATED in Wave A |
| scripts/v4_nsis_smoke_boundary.ps1, scripts/test_v4_nsis_smoke_boundary.ps1 | rust/xtask/src/nsis_harness.rs, sky_windows_security::registry | packaged job and protected release workflow; synthetic HKCU tests preserve exact raw values and restore after failures | MIGRATED in Wave C |
| scripts/ci_tauri_update_e2e.ps1, scripts/ci_tauri_update_e2e_core.ps1, scripts/test_v4_updater_fixture_server.ps1 | rust/xtask/src/updater_fixture.rs, nsis_harness.rs, process.rs | updater_e2e and release-v4 qualification; loopback-only routes, exact bytes and digests, rollback, bounded teardown, and option negatives | MIGRATED in Wave C |
| scripts/updater_candidate_source_identity.ps1 | rust/xtask/src/ci.rs, updater_fixture.rs | source identity and detached-worktree bridge validation; canonical manifest and lock bytes remain unchanged | MIGRATED in Wave C |
| scripts/v4_release_pipeline.ps1, scripts/orchestrate_v4_production_release.ps1 | rust/xtask/src/release_v4.rs | release-v4.yml invokes the ordered Rust phases; fake transport verifies publication and compensation paths | MIGRATED in Wave D |
| scripts/release_doctor.ps1 | Rust release-doctor in rust/xtask/src/release_v4.rs | cargo xtask release-doctor; fake tests cover read-only completed state and metadata divergence | MIGRATED in Wave D |
| scripts/promote_v4_metadata.ps1, scripts/v4_qualification_evidence.ps1 | rust/xtask/src/release_v4.rs, release_metadata.rs | PromoteMetadata validates immutable release identity, monotonic SemVer, evidence schema, and scoped Contents API writes | MIGRATED in Wave D |
| scripts/v4_release_asset_upload.ps1, scripts/v4_release_draft_lookup.ps1, scripts/v4_release_latest_policy.ps1, scripts/ci_v4_release_latest_guard.ps1 | rust/xtask/src/release_v4.rs | one Transport and state machine; exact asset pair, immutable release, stable Latest, beta preservation, and bounded retries are tested | MIGRATED in Wave D |
| scripts/probe_v4_metadata_app_access.ps1, scripts/cleanup_v4_release_state.ps1 | Rust probe-metadata-token and cleanup phases | release-v4.yml uses Rust; cleanup checks RUNNER_TEMP containment and the exact ownership sentinel | MIGRATED in Wave D |
| scripts/test_v4_release_pipeline.ps1, scripts/test_v4_post_publication_transaction.ps1, scripts/test_ci_v4_release_latest_guard.ps1, scripts/test_v4_production_orchestrator.ps1 | rust/xtask/src/release_v4.rs and checks/release_pipeline.rs | CI uses the 22-scenario fake-transport self-test and Rust regression tests; production release APIs are not called by these tests | MIGRATED in Wave D |

Legacy path names remain in ci_classifier.rs regression cases where deletion of an old path must
still select release validation. These are lane-mapping data, not runtime callers.

## Retained dependencies

| PowerShell entrypoint | Active consumer or purpose | Disposition |
| --- | --- | --- |
| scripts/ci_require_windows_tools.ps1, scripts/ci_job_timing.ps1 | Windows/Linux CI toolchain and timing support | ADAPTER RETAINED |
| scripts/sign_v4_authenticode.ps1 | Tauri bundle.windows.signCommand; signing provider and Rust credential broker seam | ADAPTER RETAINED |
| scripts/verify_v4_authenticode.ps1, scripts/v4_authenticode_crypto.ps1 | Windows Authenticode and CMS facts consumed by Rust policy | ADAPTER RETAINED |
| scripts/setup_v4_test_signing.ps1, scripts/cleanup_v4_test_signing.ps1 | Disposable Windows CI test certificate lifecycle | ADAPTER RETAINED |
| scripts/v4_updater_credential_broker.ps1, scripts/set_v4_updater_session_credential.ps1, scripts/remove_v4_updater_session_credential.ps1, scripts/verify_v4_updater_private_key.ps1 | Operator interaction and compatibility entrypoints for Rust Credential Manager and updater trust owners | ADAPTER RETAINED |
| scripts/verify_v4_release_runner.ps1, scripts/scan_v4_defender_exact.ps1 | Protected runner boundary and exact-candidate Defender platform checks called during release workflow | ADAPTER RETAINED |
| scripts/test_v4_updater_private_key.ps1, scripts/test_v4_production_signing_contract.ps1, scripts/test_v4_authenticode_integrity.ps1 | Windows provider, key-output secrecy, and independent Authenticode/CMS regression coverage | ADAPTER RETAINED |
| scripts/capture_defender_evidence.ps1 | Separate Defender evidence workflow/tooling; removal remains blocked by issue #521 pending owner confirmation | MANUAL KEEP |
| scripts/run_native_acceptance_case.ps1, scripts/native_acceptance_sink.ps1, scripts/run_rt_dispatch_evidence.ps1, scripts/publish_rt_dispatch_evidence.ps1 and startup-calibration/assembly-audit scripts | Specialized physical-input, performance, and provenance tooling outside the release transaction | MANUAL KEEP / Wave E; no live input was run |

The packaged, updater_e2e, and protected release jobs retain real Windows install and update
coverage only on their guarded runner contexts. This Wave did not run a real installer or access
production credentials. Local and CI transaction tests use fake GitHub transport and synthetic
artifacts. The production release and rehearsal workflows remain on their protected dedicated
runner and existing operation deadlines.
