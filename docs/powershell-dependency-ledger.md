# PowerShell dependency ledger

This ledger records the active in-repository owners and callers for tooling migrated in Waves A-E.
The inventory was checked against workflows, Tauri configuration, Rust static contracts, tests,
dot-sourcing and command call sites, and operator documentation. The production release API has no
mocked CI caller outside the Rust fake transport. The supported production release entry point is
the protected release-v4 workflow.

Wave E uses three PowerShell states:

- MIGRATED — Rust owns the deterministic implementation and its regression fixtures cover the
  contract. A PowerShell file may remain as a thin command-line adapter.
- KEEP — the PowerShell entrypoint remains active for a documented operator, UI, physical-input,
  Windows-provider, or external-publication boundary.
- RETIRED — callers and script-only tests have moved, so the PowerShell file has been removed.

The earlier A-D rows retain their narrower `ADAPTER RETAINED` and `MANUAL KEEP` labels. Wave E
retires no physical, UI, publication, or manual evidence entrypoint.

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
| scripts/audit_dispatch_assembly.ps1, scripts/audit_prepared_normal_assembly.ps1, scripts/audit_modifier_guard_assembly.ps1 | rust/xtask/src/assembly_audit.rs | `cargo xtask audit assembly` preserves the assembly reports and pass/fail assertions; seven Rust fixtures cover report-only targets, required clean targets, prepared sender bounds, modifier order, and null-window queries | MIGRATED in Wave E; thin PowerShell adapters KEEP |
| `Write-StartupCalibrationProjection` in scripts/lib/startup_calibration_projection.ps1 | rust/xtask/src/calibration.rs, `cargo xtask calibration project-evidence` | Existing exporter self-test runs the adapter and checks LF/CRLF parity, UTF-8 without BOM, exact SHA-256 sidecars, `eol=lf`, and checkout behavior with `core.autocrlf=true` | MIGRATED in Wave E; thin PowerShell adapter KEEP |

Legacy path names remain in ci_classifier.rs regression cases where deletion of an old path must
still select release validation. These are lane-mapping data, not runtime callers.

The offline optimized assembly smoke at Wave E base `8cf0907d58f89d7dde030cf6c975dc7ff1384493`
returned the existing fail-closed outcomes on the generated `dist` artifacts: `dispatch_due_from_plan`
contains one `memcpy` despite its required-clean policy; the prepared-normal symbol was not emitted;
and the modifier audit could not find its expected admission symbol. The Rust migration keeps those
assertions and exit failures. This smoke did not run native input acceptance and is not assembly
qualification evidence.

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
| scripts/audit_dispatch_assembly.ps1, scripts/audit_prepared_normal_assembly.ps1, scripts/audit_modifier_guard_assembly.ps1 | Existing runner command lines; each forwards its current parameters to `cargo xtask audit assembly` | KEEP — thin adapters; Rust owns parsing and policy |
| scripts/lib/startup_calibration_projection.ps1 | Existing `export_startup_calibration_evidence.ps1` entrypoint; forwards serialized JSON to `cargo xtask calibration project-evidence` | KEEP — thin adapter; Rust owns canonical LF bytes and sidecar hashing |
| scripts/export_startup_calibration_evidence.ps1 | Verifies the 120-run input manifests and creates the sanitized paired-run report | KEEP — full report assembly remains PowerShell until a complete golden export fixture proves JSON projection parity |
| scripts/qualify_startup_calibration.ps1 | Runs paired native ReceiveOnly timing scenarios and collects calibration telemetry | KEEP — specialized Windows timing qualification |
| scripts/run_native_acceptance_case.ps1, scripts/native_acceptance_sink.ps1, scripts/run_rt_dispatch_evidence.ps1 | Human focus step, WinForms ReceiveOnly sink, physical acceptance scenarios, and timing/provenance orchestration | KEEP — unchanged in Wave E |
| scripts/qualify_physical_key_study.ps1, scripts/cpu_contention_load.ps1 | Physical key study and controlled host-load support | KEEP — specialized hardware evidence |
| scripts/publish_rt_dispatch_evidence.ps1 | Publishes immutable evidence to GitHub | KEEP — external publication path unchanged |
| scripts/test_startup_calibration_evidence_export.ps1 | Cross-runtime LF/hash and Git checkout self-test for the PowerShell adapter | KEEP — invokes Rust after the pinned toolchain is installed |
| scripts/capture_defender_evidence.ps1 | Separate Defender evidence workflow/tooling; removal remains blocked by issue #521 pending owner confirmation | MANUAL KEEP |

The packaged, updater_e2e, and protected release jobs retain real Windows install and update
coverage only on their guarded runner contexts. This Wave did not run a real installer or access
production credentials. Local and CI transaction tests use fake GitHub transport and synthetic
artifacts. The production release and rehearsal workflows remain on their protected dedicated
runner and existing operation deadlines.
