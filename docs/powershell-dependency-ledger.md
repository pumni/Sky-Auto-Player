# PowerShell dependency ledger

This ledger records active ownership and callers for the PowerShell tooling
retired or touched by Waves A–C of the Rust-first tooling migration. It is
based on tracked source, workflow YAML, Tauri configuration, in-repository
dot-sourcing/call sites, tests, and current operator documentation at the Wave
C review boundary. No separate manual or archived-provenance caller for the
retired CI fixture scripts was documented. Out-of-repository consumers cannot
be enumerated from this checkout; the retired files were fixture entrypoints
whose documented callers were migrated below.

Disposition values:

- **MIGRATED** — the owner and all documented in-repository callers use the
  Rust implementation; the old script and its script-only tests are removed.
- **ADAPTER RETAINED** — PowerShell still owns a narrow Windows/provider/API
  boundary or transaction orchestration and calls the Rust owner where policy
  moved.
- **MANUAL KEEP** — an operator-facing or specialized Windows evidence tool
  remains intentionally available.
- **BLOCKED** — retained for a later locked wave or because deletion needs
  separate ownership evidence.

## Migrated implementations

| Former PowerShell owner | Rust owner | Caller and compatibility evidence | Disposition |
| --- | --- | --- | --- |
| `scripts/ci_classify.ps1`, `scripts/test_ci_classify.ps1` | `rust/xtask/src/ci_classifier.rs` | `changes` in `.github/workflows/ci.yml`; exact lane outputs and malformed-path fallback tests | MIGRATED in Wave A |
| `scripts/ci_validate_candidate.ps1`, `scripts/test_ci_validate_candidate.ps1` | `rust/xtask/src/ci_candidate.rs`, `ci_artifacts.rs` | candidate producer, `updater_e2e`, and `packaged` jobs; typed schema, exact inventory and tamper negatives | MIGRATED in Wave A |
| `scripts/ci_validate_bridge.ps1`, `scripts/test_ci_validate_bridge.ps1`, `scripts/ci_build_updater_bridge.ps1` | `rust/xtask/src/ci_bridge.rs`, `updater_fixture.rs` | `updater_bridge` builds from a detached worktree; bridge output is created and revalidated by Rust | MIGRATED by Waves A/C |
| `scripts/build_native_calibration.ps1`, `scripts/verify_installed_calibration.ps1`, `scripts/test_installed_calibration.ps1` | `rust/xtask/src/calibration.rs` | Tauri prebuild, candidate packaging, and Rust NSIS qualification | MIGRATED in Wave A |
| `scripts/v4_nsis_smoke_boundary.ps1`, `scripts/test_v4_nsis_smoke_boundary.ps1` | `rust/xtask/src/nsis_harness.rs`, `sky_windows_security::registry` | `packaged` job and protected release orchestrator call `cargo xtask ci nsis smoke`; synthetic HKCU tests cover absent/present keys, raw value kinds/bytes, nested keys, parent existence, and rollback | MIGRATED in Wave C |
| `scripts/ci_tauri_update_e2e.ps1`, `scripts/ci_tauri_update_e2e_core.ps1`, `scripts/test_v4_updater_fixture_server.ps1` | `rust/xtask/src/updater_fixture.rs`, `nsis_harness.rs`, `process.rs` | `updater_e2e` job and `v4_release_pipeline.ps1` call `cargo xtask ci updater-fixture run`; Rust tests cover loopback-only URLs, exact bytes/length/digest, bad manifests, truncated bodies, bounded listener/process teardown, and option rejection | MIGRATED in Wave C |
| `scripts/updater_candidate_source_identity.ps1` fixture identity/source mutation helpers | `rust/xtask/src/ci.rs`, `updater_fixture.rs` | release pipeline regression calls `cargo xtask ci source-identity`; bridge source uses a detached worktree, so canonical manifest/lock/config bytes are never modified | MIGRATED in Wave C |

Legacy path strings remain in `ci_classifier.rs` regression cases so a diff
that deletes or edits one of the migrated scripts still selects its required
CI lane. They are not runtime callers.

## Retained dependencies

| PowerShell entrypoint | Active consumer or purpose | Disposition |
| --- | --- | --- |
| `scripts/ci_require_windows_tools.ps1` | Windows updater/package jobs install or verify external toolchain executables | ADAPTER RETAINED |
| `scripts/ci_job_timing.ps1` | Windows/Linux CI job timing setup and report | ADAPTER RETAINED |
| `scripts/sign_v4_authenticode.ps1` | Tauri `bundle.windows.signCommand`; delegates credential actions to the Rust broker and invokes the platform signing provider | ADAPTER RETAINED |
| `scripts/verify_v4_authenticode.ps1`, `scripts/v4_authenticode_crypto.ps1` | Windows Authenticode/CMS platform facts consumed by the Rust evidence policy | ADAPTER RETAINED |
| `scripts/setup_v4_test_signing.ps1`, `scripts/cleanup_v4_test_signing.ps1` | Disposable Windows CI test-certificate setup and teardown | ADAPTER RETAINED |
| `scripts/v4_updater_credential_broker.ps1`, `set_v4_updater_session_credential.ps1`, `remove_v4_updater_session_credential.ps1`, `verify_v4_updater_private_key.ps1` | Operator input and compatibility entrypoints around the Rust Win32 Credential Manager owner | ADAPTER RETAINED |
| `scripts/v4_release_pipeline.ps1`, `scripts/orchestrate_v4_production_release.ps1` | Existing protected release transaction and operator orchestration; candidate/update/NSIS qualification calls the Rust owners | ADAPTER RETAINED; transaction authority is Wave D |
| `scripts/promote_v4_metadata.ps1`, `scripts/v4_release_asset_upload.ps1`, `scripts/v4_release_draft_lookup.ps1`, `scripts/v4_release_latest_policy.ps1`, `scripts/ci_v4_release_latest_guard.ps1`, `scripts/probe_v4_metadata_app_access.ps1`, `scripts/cleanup_v4_release_state.ps1`, `scripts/verify_v4_release_runner.ps1` | Active release workflow, immutable publication, metadata promotion and runner boundary | BLOCKED for Wave D; retained unchanged in this wave |
| `scripts/scan_v4_defender_exact.ps1` | Protected release pipeline scans the exact candidate installer and writes required evidence | ADAPTER RETAINED |
| `scripts/capture_defender_evidence.ps1` | Separate Defender evidence workflow/tooling; deletion is explicitly blocked by issue #521 pending owner confirmation | MANUAL KEEP |
| `scripts/run_native_acceptance_case.ps1`, `scripts/native_acceptance_sink.ps1`, `scripts/run_rt_dispatch_evidence.ps1`, `scripts/publish_rt_dispatch_evidence.ps1`, startup-calibration and assembly-audit scripts | Specialized physical-input, performance, and provenance tooling outside the installer/updater harness | MANUAL KEEP / Wave E; no live input was run |
| `scripts/release_doctor.ps1`, `scripts/v4_qualification_evidence.ps1` | Release diagnostics/evidence authority | BLOCKED for Wave D; retained |
| `scripts/test_v4_release_pipeline.ps1` | Active Windows release-contract job; transaction regressions run against mocked GitHub API behavior and statically assert Rust updater/NSIS callers | ADAPTER RETAINED; Wave D transaction test migration remains locked |

The Windows CI jobs that exercise actual NSIS install, updater rollback,
N-to-N+1 migration, and uninstall are real machine tests. They are restricted
to the guarded disposable runner contexts. Local checks use synthetic HKCU
test namespaces and fake files; this migration did not invoke an installer on
the maintainer workstation. The `updater_bridge`, `updater_e2e`, and
`packaged` CI jobs each set a 90-minute outer timeout. The protected
`release-v4.yml` and `rehearse-v4.yml` Windows jobs currently inherit GitHub
Actions' documented 360-minute job default because they do not set
`timeout-minutes`; their existing release operation deadlines remain in force.
Rust fixture process-tree teardown is bounded separately from those operation
deadlines. See [CI validation topology](ci-validation-topology.md#wave-c-execution-and-teardown-bounds)
for the exact build, child-process, pipe-drain, and loopback request bounds.
