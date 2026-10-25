use super::workflow::{workflow_declares_only_dispatch, workflow_step_blocks};
use crate::Result;
use std::fs;
use std::path::Path;

const RELEASE_COMMANDS: &[&str] = &[
    "preflight",
    "build-candidate",
    "verify-attestations",
    "probe-metadata-token",
    "publish-release",
    "promote-metadata",
    "final-verify",
    "cleanup",
];
const REQUIRED_RELEASE_INVOCATIONS: &[(&str, &str)] = &[
    (
        "Preflight release request and repository readiness",
        "cargo xtask release-v4 preflight --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
    ),
    (
        "Build and qualify the single production candidate",
        "cargo xtask release-v4 build-candidate --state-root $env:V4_RELEASE_STATE_ROOT --key-path $env:V4_UPDATER_PRIVATE_KEY_PATH --workflow-sha $env:GITHUB_SHA",
    ),
    (
        "Verify exact-source OIDC attestations",
        "cargo xtask release-v4 verify-attestations --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
    ),
    (
        "Probe release-metadata App access before publication",
        "cargo xtask release-v4 probe-metadata-token --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
    ),
    (
        "Publish the qualified candidate immutably",
        "cargo xtask release-v4 publish-release --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
    ),
    (
        "Promote release metadata only after immutable publication",
        "cargo xtask release-v4 promote-metadata --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
    ),
    (
        "Re-fetch and verify final public release and metadata",
        "cargo xtask release-v4 final-verify --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
    ),
    (
        "Clean runner-local release state",
        "cargo xtask release-v4 cleanup --state-root $env:V4_RELEASE_STATE_ROOT",
    ),
];

fn source(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn release_command_sequence(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .filter_map(|line| line.split_once("cargo xtask release-v4 "))
        .filter_map(|(_, command)| command.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

fn require_step_command(
    workflow: &str,
    name: &str,
    command: &str,
) -> std::result::Result<(), String> {
    let steps = workflow_step_blocks(workflow);
    let block = steps
        .iter()
        .find(|(step_name, _)| step_name == name)
        .map(|(_, block)| block.as_str())
        .ok_or_else(|| format!("release workflow is missing step `{name}`"))?;
    if !block.contains(command) {
        return Err(format!(
            "release workflow step `{name}` is missing its exact Rust invocation"
        ));
    }
    Ok(())
}

fn nsis_installed_catalog_contract(harness: &str) -> std::result::Result<(), String> {
    let harness = source(harness);
    for marker in [
        "fn assert_user_songs_empty",
        "SKY_BUILTIN_CATALOG_FRESH_SELFTEST",
        "fn launch_smoke",
        "--selftest-update-active-playback",
        "assert_uninstall_registration_absent()",
        "owned_root_cleanup_requires_exact_sentinel_and_direct_runner_temp_scope",
        "incomplete_root_rollback_accepts_only_a_prefix_of_the_created_sentinel",
        "install_root_absence_polling_has_a_deadline",
        "cleanup_attempts_every_stage_and_preserves_original_failure",
    ] {
        if !harness.contains(marker) {
            return Err(format!("Rust NSIS owner is missing regression `{marker}`"));
        }
    }
    let qualification = harness
        .split_once("fn qualify_package(")
        .map(|(_, body)| body)
        .ok_or("Rust NSIS owner is missing its package qualification function")?;
    let qualification = qualification
        .split_once("\n#[cfg(windows)]\nfn verify_calibration_artifact(")
        .map(|(body, _)| body)
        .ok_or("Rust NSIS package qualification body is not bounded")?;
    let install = qualification
        .find("run_installer(\n        installer,")
        .ok_or("Rust NSIS package qualification does not install the validated candidate")?;
    let catalog = qualification
        .find("crate::builtin_catalog::run(")
        .ok_or("Rust NSIS package qualification does not verify the installed built-in catalog")?;
    let songs_guard = qualification
        .find("assert_user_songs_empty(&app_data.join(\"songs\"))")
        .ok_or("Rust NSIS fresh-install qualification does not assert empty user songs")?;
    let launch = qualification
        .find("launch_smoke(scope, &app")
        .ok_or("Rust NSIS package qualification does not launch the installed application")?;
    if install >= catalog
        || catalog >= launch
        || songs_guard >= launch
        || !qualification[catalog..launch].contains("verify-installed")
    {
        return Err(
            "installed catalog and empty user-song checks must follow install and precede app launch"
                .into(),
        );
    }
    Ok(())
}

fn updater_fixture_contract(fixture: &str) -> std::result::Result<(), String> {
    let fixture = source(fixture);
    for marker in [
        "failure_rollback",
        "n_to_n_plus_1_installer_sha256",
        "old_root_rejection",
        "assert_migration_registry_roots_absent",
        "user_song_sha256_after_uninstall",
        "loopback_client_rejects_truncated_response_bodies",
        "fixture_server_teardown_is_bounded_for_a_stalled_partial_request",
        "bridge_build_cleanup_requires_direct_runner_temp_and_exact_owner_sentinel",
        "bridge_build_entry_rollback_accepts_only_a_partial_created_sentinel",
        "git_worktree_add_and_remove_accepts_verified_ordinary_windows_paths",
    ] {
        if !fixture.contains(marker) {
            return Err(format!(
                "Rust updater fixture is missing regression `{marker}`"
            ));
        }
    }
    Ok(())
}

fn rust_release_state_machine_contract(
    workflow: &str,
    rehearsal: &str,
    rust: &str,
    main: &str,
    ci: &str,
) -> std::result::Result<(), String> {
    let workflow = source(workflow);
    let rehearsal = source(rehearsal);
    let rust = source(rust);
    let main = source(main);
    let ci = source(ci);
    if !workflow_declares_only_dispatch(&workflow) || !workflow_declares_only_dispatch(&rehearsal) {
        return Err(
            "production workflows must expose workflow_dispatch as their only trigger".into(),
        );
    }
    for marker in [
        "name: V4 Release Pipeline",
        "workflow_dispatch:",
        "runs-on: [self-hosted, windows, v4-release, single-tenant]",
        "environment: v4-production-release",
        "contents: read",
        "contents: write",
        "id-token: write",
        "attestations: write",
        "ref: ${{ github.sha }}",
        "persist-credentials: false",
        "V4_UPDATER_PRIVATE_KEY_PATH",
        "actions/attest@",
        "actions/upload-artifact@",
        "Mint release-metadata GitHub App token before publication",
        "permission-contents: write",
        "if: always()",
        "EXPECTED_REPOSITORY: pumni/Sky-Auto-Player",
        "EXPECTED_WORKFLOW: rehearse-v4.yml",
        "EXPECTED_DEFAULT_BRANCH: ${{ github.event.repository.default_branch }}",
        "test \"$GITHUB_EVENT_NAME\" = \"workflow_dispatch\"",
        "test \"$GITHUB_REPOSITORY\" = \"$EXPECTED_REPOSITORY\"",
        "test \"$GITHUB_REF\" = \"refs/heads/main\"",
        "test \"$EXPECTED_DEFAULT_BRANCH\" = \"main\"",
        "actions/workflows/$EXPECTED_WORKFLOW/runs?event=workflow_dispatch&status=completed&head_sha=$GITHUB_SHA&per_page=100",
        "V4 Production Pre-Publication Qualification",
        "head_branch == \"main\"",
        "conclusion == \"success\"",
    ] {
        if !workflow.contains(marker) {
            return Err(format!("production release workflow is missing `{marker}`"));
        }
    }
    if workflow.contains("inputs:") || workflow.contains("inputs.") {
        return Err("production release workflow must not expose semantic dispatch inputs".into());
    }
    if workflow.contains("EXPECTED_WORKFLOW: .github/workflows/") {
        return Err("exact-head qualification must pass the workflow file name".into());
    }
    let command_sequence = release_command_sequence(&workflow);
    if command_sequence
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != RELEASE_COMMANDS
    {
        return Err(format!(
            "production workflow must invoke the ordered Rust release commands {RELEASE_COMMANDS:?}, got {command_sequence:?}"
        ));
    }
    for (name, command) in REQUIRED_RELEASE_INVOCATIONS {
        require_step_command(&workflow, name, command)?;
    }
    if !workflow.contains("gh api \\")
        || !workflow.contains("actions/workflows/$EXPECTED_WORKFLOW/runs?event=workflow_dispatch")
    {
        return Err(
            "release dispatch boundary must query a completed exact-source qualification".into(),
        );
    }
    for old in [
        "scripts/v4_release_pipeline.ps1",
        "scripts/release_doctor.ps1",
        "scripts/probe_v4_metadata_app_access.ps1",
        "scripts/cleanup_v4_release_state.ps1",
        "-State Preflight",
        "-State BuildCandidate",
        "-State PublishRelease",
        "-State PromoteMetadata",
        "-State FinalVerify",
    ] {
        if workflow.contains(old) || rehearsal.contains(old) {
            return Err(format!(
                "legacy PowerShell release authority is still called: {old}"
            ));
        }
    }
    let rehearsal_sequence = release_command_sequence(&rehearsal);
    if rehearsal_sequence
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != ["preflight", "build-candidate", "cleanup"]
    {
        return Err(format!(
            "rehearsal must invoke exactly Preflight -> BuildCandidate -> cleanup, got {rehearsal_sequence:?}"
        ));
    }
    for forbidden in [
        "cargo xtask release-v4 publish-release",
        "cargo xtask release-v4 promote-metadata",
        "cargo xtask release-v4 final-verify",
        "gh release create",
        "gh release upload",
        "gh release delete",
        "actions/attest@",
        "actions/create-github-app-token@",
        "contents: write",
        "id-token: write",
        "attestations: write",
    ] {
        if rehearsal.contains(forbidden) {
            return Err(format!(
                "rehearsal workflow crosses the publication boundary: {forbidden}"
            ));
        }
    }
    for marker in [
        "trait Transport",
        "struct GhTransport",
        "fn publish_release",
        "mock_non_monotonic_candidate_never_mutates",
        "prepublication_metadata_qualification",
        "same_github_latest",
        "validate_monotonic",
        "fn delete_owned_draft",
        "mock_conflicting_draft_is_not_deleted",
        "current[\"draft\"] != true",
        "mock_upload_timeout_reconciles_exact_asset",
        "mutation_adapter_is_at_most_once",
        "MutationOutcomeUnknown",
        "fn request_with_retry_policy",
        "await_raw_metadata_convergence",
        "raw_metadata_retry_converges_after_cache_lag",
        "raw_metadata_retry_fails_with_bounded_diagnostics",
        "RuntimeIdentity::from_environment",
        "GITHUB_WORKFLOW_REF",
        "--workflow-sha is required for Preflight",
        "runtime_identity_fails_closed_without_exact_workflow_context",
        "channels/{channel}/latest.json?ref=release-metadata",
        "metadata_token_probe_rejects_wrong_repository_state",
        "fn promote_metadata",
        "fn final_verify",
        "fn doctor",
        "fn verify_attestations",
        "delete_owned_draft",
        "POST_PUBLICATION_INCIDENT",
        "release_id",
        "sha256:",
        "raw.githubusercontent.com",
        "mock_publish_succeeds",
        "mock_draft_post_timeout_reconciles_exact_marker",
        "mock_digest_mismatch_cleans_owned_draft",
        "mock_patch_timeout_reconciles_exact_release_id",
        "mock_unknown_publication_never_cleans_up",
        "canonical_upload_url",
        "RELEASE_BUILD_STEP_TIMEOUT",
        "RELEASE_DEFENDER_STEP_TIMEOUT",
        "ATTESTATION_VERIFY_TIMEOUT",
        "mock_release_doctor_detects_metadata_endpoint_divergence",
        "mock_metadata_promotion_is_idempotent_after_committed_timeout",
        "mock_publication_identity_and_asset_inventory_fail_closed",
        "metadata_token_probe_rejects_wrong_repository_state",
        "doctor_state_input_rejects_malformed_and_oversized_files",
        "malformed_github_api_json_fails_closed_with_endpoint_context",
        "retry_policy_retries_only_transient_responses",
        "env::current_exe()?",
    ] {
        if !rust.contains(marker) {
            return Err(format!(
                "Rust release authority is missing invariant `{marker}`"
            ));
        }
    }
    let publish = rust
        .split_once("fn publish_release(")
        .and_then(|(_, body)| body.split_once("\nfn prepublication_metadata_qualification("))
        .map(|(body, _)| body)
        .ok_or("Rust release authority is missing bounded publication qualification")?;
    let gate_positions = publish
        .match_indices("prepublication_metadata_qualification(")
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    let post = publish
        .find("\"POST\"")
        .ok_or("Rust release authority is missing draft creation boundary")?;
    let patch = publish
        .find("\"PATCH\"")
        .ok_or("Rust release authority is missing immutable publication boundary")?;
    if gate_positions.len() != 2
        || gate_positions[0] >= post
        || post >= gate_positions[1]
        || gate_positions[1] >= patch
    {
        return Err(
            "metadata monotonicity and Latest checks must run before draft creation and again before publication"
                .into(),
        );
    }
    let mutation_policy = rust
        .split_once("fn request_with_retry_policy")
        .and_then(|(_, body)| body.split_once("\npub(crate) fn run("))
        .map(|(body, _)| body)
        .ok_or("Rust release authority is missing mutation retry policy")?;
    let read_retry = mutation_policy
        .find("method.eq_ignore_ascii_case(\"GET\")")
        .ok_or("GitHub retry policy does not restrict retries to GET")?;
    let retry = mutation_policy
        .find("retry_github(operation, sleep)")
        .ok_or("GitHub GET retry path is missing")?;
    let mutation = mutation_policy
        .find("operation().map_err")
        .ok_or("GitHub mutation path must return an uncertain outcome without retry")?;
    if read_retry >= retry || retry >= mutation {
        return Err("GitHub mutation retries must remain disabled".into());
    }
    let delete = rust
        .split_once("fn delete_owned_draft(")
        .and_then(|(_, body)| body.split_once("\nfn verify_public_assets("))
        .map(|(body, _)| body)
        .ok_or("Rust release authority is missing owned draft deletion")?;
    let delete_get = delete
        .find("transport.request(\"GET\"")
        .ok_or("owned draft deletion must re-fetch the exact release id")?;
    let delete_mutation = delete
        .find("transport.request(\"DELETE\"")
        .ok_or("owned draft deletion mutation is missing")?;
    if delete_get >= delete_mutation {
        return Err("owned draft state must be verified immediately before deletion".into());
    }
    let identity = rust
        .split_once("fn identity(")
        .and_then(|(_, body)| body.split_once("\nfn git_text("))
        .map(|(body, _)| body)
        .ok_or("Rust release authority is missing exact runtime identity validation")?;
    if !identity
        .contains("workflow_sha\n        .ok_or(\"--workflow-sha is required for Preflight\")")
        || !identity.contains("GITHUB_WORKFLOW_REF")
        || !identity.contains("RELEASE_WORKFLOW_REF")
        || !identity.contains("REHEARSE_WORKFLOW_REF")
    {
        return Err(
            "Preflight must require the explicit workflow SHA and an exact canonical workflow ref"
                .into(),
        );
    }
    let metadata_probe = rust
        .split_once("fn probe_metadata_token(")
        .and_then(|(_, body)| body.split_once("\nfn cleanup_local_state("))
        .map(|(body, _)| body)
        .ok_or("Rust release authority is missing metadata token probe")?;
    if !metadata_probe.contains("for channel in [\"stable\", \"beta\"]")
        || !metadata_probe.contains("doctor_validate_metadata")
    {
        return Err(
            "metadata token probe must read and validate both stable and beta state".into(),
        );
    }
    if rust.contains("process::capture(\"cargo\"") || rust.contains("Command::new(\"cargo\"") {
        return Err(
            "release build adapters must invoke the current xtask binary without recursive Cargo"
                .into(),
        );
    }
    if !main.contains("\"release-v4\" => release_v4::run")
        || !main.contains("\"release-doctor\" =>")
        || !main.contains("release_v4::run(&repo::root(), &doctor_args)")
        || main.contains("scripts/release_doctor.ps1")
    {
        return Err(
            "xtask CLI must route release-v4 and release-doctor to the native Rust owner".into(),
        );
    }
    for marker in [
        "cargo xtask release-v4 self-test",
        "cargo test --locked --manifest-path rust/xtask/Cargo.toml",
    ] {
        if !ci.contains(marker) {
            return Err(format!(
                "CI release_contract must execute Rust release regressions: {marker}"
            ));
        }
    }
    let credentials = source(
        &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/credential_cli.rs"))
            .map_err(|error| error.to_string())?,
    );
    for marker in [
        "BUN_INSTALL_TIMEOUT",
        "BUILD_CANDIDATE_TIMEOUT",
        "actual_child_launcher_kills_timed_out_process_tree",
    ] {
        if !credentials.contains(marker) {
            return Err(format!(
                "Rust candidate builder is missing bounded-process contract `{marker}`"
            ));
        }
    }
    for obsolete in [
        "scripts/test_v4_release_pipeline.ps1",
        "scripts/test_v4_post_publication_transaction.ps1",
        "scripts/test_ci_v4_release_latest_guard.ps1",
        "scripts/test_ci_v4_release_latest_guard.ps1",
    ] {
        if ci.contains(obsolete) {
            return Err(format!(
                "CI still executes retired PowerShell release tests: {obsolete}"
            ));
        }
    }
    Ok(())
}

pub(super) fn v4_release_pipeline_contract(root: &Path) -> Result<()> {
    let read = |path: &str| -> Result<String> { Ok(fs::read_to_string(root.join(path))?) };
    nsis_installed_catalog_contract(&read("rust/xtask/src/nsis_harness.rs")?)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    updater_fixture_contract(&read("rust/xtask/src/updater_fixture.rs")?)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    rust_release_state_machine_contract(
        &read(".github/workflows/release-v4.yml")?,
        &read(".github/workflows/rehearse-v4.yml")?,
        &read("rust/xtask/src/release_v4.rs")?,
        &read("rust/xtask/src/main.rs")?,
        &read(".github/workflows/ci.yml")?,
    )
    .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    println!("[xtask] Rust v4 release authority contract: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> (String, String, String, String, String) {
        let release_commands = [
            (
                "Preflight release request and repository readiness",
                "cargo xtask release-v4 preflight --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
            ),
            (
                "Build and qualify the single production candidate",
                "cargo xtask release-v4 build-candidate --state-root $env:V4_RELEASE_STATE_ROOT --key-path $env:V4_UPDATER_PRIVATE_KEY_PATH --workflow-sha $env:GITHUB_SHA",
            ),
            (
                "Verify exact-source OIDC attestations",
                "cargo xtask release-v4 verify-attestations --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
            ),
            (
                "Probe release-metadata App access before publication",
                "cargo xtask release-v4 probe-metadata-token --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
            ),
            (
                "Publish the qualified candidate immutably",
                "cargo xtask release-v4 publish-release --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
            ),
            (
                "Promote release metadata only after immutable publication",
                "cargo xtask release-v4 promote-metadata --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
            ),
            (
                "Re-fetch and verify final public release and metadata",
                "cargo xtask release-v4 final-verify --state-root $env:V4_RELEASE_STATE_ROOT --workflow-sha $env:GITHUB_SHA",
            ),
            (
                "Clean runner-local release state",
                "cargo xtask release-v4 cleanup --state-root $env:V4_RELEASE_STATE_ROOT",
            ),
        ];
        let release_steps = release_commands
            .iter()
            .map(|(name, command)| format!("      - name: {name}\n        run: {command}\n"))
            .collect::<String>();
        let release = format!(
            "name: V4 Release Pipeline\non:\n  workflow_dispatch:\npermissions:\n  contents: read\nEXPECTED_REPOSITORY: pumni/Sky-Auto-Player\nEXPECTED_DEFAULT_BRANCH: ${{{{ github.event.repository.default_branch }}}}\nEXPECTED_WORKFLOW: rehearse-v4.yml\ntest \"$GITHUB_EVENT_NAME\" = \"workflow_dispatch\"\ntest \"$GITHUB_REPOSITORY\" = \"$EXPECTED_REPOSITORY\"\ntest \"$GITHUB_REF\" = \"refs/heads/main\"\ntest \"$EXPECTED_DEFAULT_BRANCH\" = \"main\"\nqualified_run=\"$(gh api \\\n  \\\"repos/$GITHUB_REPOSITORY/actions/workflows/$EXPECTED_WORKFLOW/runs?event=workflow_dispatch&status=completed&head_sha=$GITHUB_SHA&per_page=100\\\" --jq '.workflow_runs[] | select(.status == \\\"completed\\\" and .conclusion == \\\"success\\\" and .event == \\\"workflow_dispatch\\\" and .head_branch == \\\"main\\\" and .name == \\\"V4 Production Pre-Publication Qualification\\\") | .id')\"\nhead_branch == \"main\"\nconclusion == \"success\"\nruns-on: [self-hosted, windows, v4-release, single-tenant]\nenvironment: v4-production-release\ncontents: read\ncontents: write\nid-token: write\nattestations: write\nref: ${{{{ github.sha }}}}\npersist-credentials: false\nV4_UPDATER_PRIVATE_KEY_PATH\nactions/attest@\nactions/upload-artifact@\nMint release-metadata GitHub App token before publication\npermission-contents: write\nif: always()\njobs:\n  release:\n    steps:\n{}",
            release_steps
        );
        let rehearsal = "name: V4 Production Pre-Publication Qualification\non:\n  workflow_dispatch:\njobs:\n  qualification:\n    steps:\n      - name: Run Preflight\n        run: cargo xtask release-v4 preflight --state-root state --workflow-sha $env:GITHUB_SHA\n      - name: Run BuildCandidate\n        run: cargo xtask release-v4 build-candidate --state-root state --key-path key --workflow-sha $env:GITHUB_SHA\n      - name: Clean state\n        run: cargo xtask release-v4 cleanup --state-root state\n".to_owned();
        let rust = r#"trait Transport
struct GhTransport
fn publish_release(
prepublication_metadata_qualification(
transport.request(
"POST"
prepublication_metadata_qualification(
transport.request(
"PATCH"
}
fn prepublication_metadata_qualification(
validate_monotonic
same_github_latest
fn same_github_latest(
fn delete_owned_draft(
transport.request("GET"
current["id"]
current["draft"] != true
matches_transaction(&current, context)
transport.request("DELETE"
fn verify_public_assets(
fn request_with_retry_policy(
method.eq_ignore_ascii_case("GET")
retry_github(operation, sleep)
operation().map_err
pub(crate) fn run(
fn identity(
workflow_sha
        .ok_or("--workflow-sha is required for Preflight")
GITHUB_WORKFLOW_REF
RELEASE_WORKFLOW_REF
REHEARSE_WORKFLOW_REF
fn git_text(
fn probe_metadata_token(
for channel in ["stable", "beta"]
doctor_validate_metadata
channels/{channel}/latest.json?ref=release-metadata
fn cleanup_local_state(
mock_non_monotonic_candidate_never_mutates mock_conflicting_draft_is_not_deleted mock_upload_timeout_reconciles_exact_asset mutation_adapter_is_at_most_once MutationOutcomeUnknown await_raw_metadata_convergence raw_metadata_retry_converges_after_cache_lag raw_metadata_retry_fails_with_bounded_diagnostics RuntimeIdentity::from_environment GITHUB_WORKFLOW_REF runtime_identity_fails_closed_without_exact_workflow_context metadata_token_probe_rejects_wrong_repository_state fn promote_metadata fn final_verify fn doctor fn verify_attestations POST_PUBLICATION_INCIDENT release_id sha256: raw.githubusercontent.com mock_publish_succeeds mock_draft_post_timeout_reconciles_exact_marker mock_digest_mismatch_cleans_owned_draft mock_patch_timeout_reconciles_exact_release_id mock_unknown_publication_never_cleans_up canonical_upload_url RELEASE_BUILD_STEP_TIMEOUT RELEASE_DEFENDER_STEP_TIMEOUT ATTESTATION_VERIFY_TIMEOUT mock_release_doctor_detects_metadata_endpoint_divergence mock_metadata_promotion_is_idempotent_after_committed_timeout mock_publication_identity_and_asset_inventory_fail_closed doctor_state_input_rejects_malformed_and_oversized_files malformed_github_api_json_fails_closed_with_endpoint_context retry_policy_retries_only_transient_responses env::current_exe()?"#
            .to_owned();
        let main = "\"release-v4\" => release_v4::run\n\"release-doctor\" =>\nrelease_v4::run(&repo::root(), &doctor_args)".to_owned();
        let ci = "cargo xtask release-v4 self-test\ncargo test --locked --manifest-path rust/xtask/Cargo.toml".to_owned();
        (release, rehearsal, rust, main, ci)
    }

    #[test]
    fn release_state_machine_requires_ordered_rust_stages_and_security_boundaries() {
        let (release, rehearsal, rust, main, ci) = valid();
        rust_release_state_machine_contract(&release, &rehearsal, &rust, &main, &ci)
            .expect("valid Rust release authority fixture must satisfy the contract");
        let reordered = release.replace(
            "cargo xtask release-v4 publish-release --state-root",
            "cargo xtask release-v4 promote-metadata --state-root",
        );
        assert!(
            rust_release_state_machine_contract(&reordered, &rehearsal, &rust, &main, &ci).is_err()
        );
        let legacy = release.replace(
            "cargo xtask release-v4 publish-release --state-root",
            "pwsh scripts/v4_release_pipeline.ps1",
        );
        assert!(
            rust_release_state_machine_contract(&legacy, &rehearsal, &rust, &main, &ci).is_err()
        );
        let unsafe_rehearsal = format!("{rehearsal}cargo xtask release-v4 publish-release");
        assert!(
            rust_release_state_machine_contract(&release, &unsafe_rehearsal, &rust, &main, &ci)
                .is_err()
        );
        let no_negative_test = rust.replace("mock_digest_mismatch_cleans_owned_draft", "removed");
        assert!(
            rust_release_state_machine_contract(
                &release,
                &rehearsal,
                &no_negative_test,
                &main,
                &ci
            )
            .is_err()
        );
        let recursive_cargo = rust.replace("env::current_exe()?", "process::capture(\"cargo\"");
        assert!(
            rust_release_state_machine_contract(&release, &rehearsal, &recursive_cargo, &main, &ci)
                .is_err()
        );

        let no_exact_head = release.replace("head_sha=$GITHUB_SHA", "head_branch=main");
        assert!(
            rust_release_state_machine_contract(&no_exact_head, &rehearsal, &rust, &main, &ci)
                .is_err()
        );
        let missing_workflow_sha = release.replace("--workflow-sha $env:GITHUB_SHA", "");
        assert!(
            rust_release_state_machine_contract(
                &missing_workflow_sha,
                &rehearsal,
                &rust,
                &main,
                &ci
            )
            .is_err()
        );
        for (_, command) in REQUIRED_RELEASE_INVOCATIONS {
            let arguments = command.split_whitespace().collect::<Vec<_>>();
            for index in 0..arguments.len().saturating_sub(1) {
                if arguments[index].starts_with("--") {
                    let argument = format!("{} {}", arguments[index], arguments[index + 1]);
                    let missing = release.replacen(&argument, "", 1);
                    assert!(
                        rust_release_state_machine_contract(
                            &missing, &rehearsal, &rust, &main, &ci
                        )
                        .is_err(),
                        "removing `{argument}` must fail the release workflow contract"
                    );
                }
            }
        }

        let crlf_release = release.replace('\n', "\r\n");
        let crlf_rehearsal = rehearsal.replace('\n', "\r\n");
        assert!(
            rust_release_state_machine_contract(&crlf_release, &crlf_rehearsal, &rust, &main, &ci)
                .is_ok()
        );
    }

    #[test]
    fn installed_nsis_catalog_contract_keeps_safe_order_across_line_endings() {
        let harness = include_str!("../nsis_harness.rs");
        assert!(nsis_installed_catalog_contract(harness).is_ok());
        let crlf = harness.replace('\n', "\r\n");
        assert!(nsis_installed_catalog_contract(&crlf).is_ok());

        let missing_catalog = harness.replace("crate::builtin_catalog::run(", "");
        assert!(nsis_installed_catalog_contract(&missing_catalog).is_err());
        let launch_before_catalog = harness.replace(
            "crate::builtin_catalog::run(",
            "launch_smoke(scope, &app)?; crate::builtin_catalog::run(",
        );
        assert!(nsis_installed_catalog_contract(&launch_before_catalog).is_err());
    }

    #[test]
    fn updater_fixture_static_contract_keeps_required_negative_regressions() {
        let fixture = include_str!("../updater_fixture.rs");
        assert!(updater_fixture_contract(fixture).is_ok());
        let crlf = fixture.replace('\n', "\r\n");
        assert!(updater_fixture_contract(&crlf).is_ok());
        let missing_worktree_test = fixture.replace(
            "git_worktree_add_and_remove_accepts_verified_ordinary_windows_paths",
            "removed_test_name",
        );
        assert!(updater_fixture_contract(&missing_worktree_test).is_err());
    }
}
