use super::release_runner::validate_metadata_app_token_scope;
use super::workflow::{
    validate_v4_release_state_invocations, workflow_declares_only_dispatch,
    workflow_state_invocations,
};
use crate::Result;
use std::fs;
use std::path::Path;

fn v4_release_pipeline_contract_source(
    workflow: &str,
    pipeline: &str,
    regression: &str,
) -> Result<()> {
    let workflow = workflow.replace("\r\n", "\n");
    for marker in [
        "name: V4 Release Pipeline",
        "workflow_dispatch:",
        "runs-on: [self-hosted, windows, v4-release, single-tenant]",
        "contents: read",
        "id-token: write",
        "attestations: write",
        "contents: write",
        "actions: read",
        "GH_TOKEN: ${{ github.token }}",
        "EXPECTED_WORKFLOW: rehearse-v4.yml",
        "ref: ${{ github.sha }}",
        "release-dispatch-boundary",
        "Require exact-head production qualification",
        "actions/workflows/$EXPECTED_WORKFLOW/runs?event=workflow_dispatch&status=completed&head_sha=$GITHUB_SHA",
        "V4 Production Pre-Publication Qualification",
        "github.event.repository.default_branch",
        "refs/heads/main",
        "environment: v4-production-release",
        "Verify isolated production runner boundary",
        "verify_v4_release_runner.ps1",
        "cleanup_v4_release_state.ps1",
        "V4_UPDATER_PRIVATE_KEY_PATH",
        "-UpdaterPrivateKeyPath $env:V4_UPDATER_PRIVATE_KEY_PATH",
        "persist-credentials: false",
        "actions/attest@",
        "actions/upload-artifact@",
        "--source-digest $env:GITHUB_SHA",
        "Build and qualify the single production candidate",
        "concurrency:",
        "group: v4-release-control-plane",
        "Mint release-metadata GitHub App token before publication",
        "Probe release-metadata App access before publication",
        "probe_v4_metadata_app_access.ps1",
        "Publish the qualified candidate immutably",
        "PromoteMetadata",
        "FinalVerify",
    ] {
        if !workflow.contains(marker) {
            return Err(
                format!("v4 release workflow is missing its required marker: {marker}").into(),
            );
        }
    }
    if workflow.contains("inputs:") || workflow.contains("inputs.") {
        return Err(
            "production release workflow must not expose semantic workflow_dispatch inputs".into(),
        );
    }
    if workflow.contains("EXPECTED_WORKFLOW: .github/workflows/") {
        return Err(
            "exact-head qualification must pass the workflow file name, not its path".into(),
        );
    }
    let workflow_states = [
        "-State Preflight",
        "-State BuildCandidate",
        "-State PublishRelease",
        "-State PromoteMetadata",
        "-State FinalVerify",
    ];
    let mut previous = 0;
    for marker in workflow_states {
        let position = workflow
            .find(marker)
            .ok_or_else(|| format!("v4 release workflow is missing state marker: {marker}"))?;
        if position < previous {
            return Err("v4 release workflow states are not ordered fail-closed".into());
        }
        previous = position;
    }
    validate_v4_release_state_invocations(&workflow, pipeline)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    println!("[xtask] v4 release workflow state CLI invocation contract: PASS");
    for forbidden in [
        "cargo xtask dist",
        "softprops/action-gh-release",
        "secrets.TAURI_SIGNING_PRIVATE_KEY",
        "secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
        "secrets.UPDATER_PRIVATE_KEY",
        "secrets.UPDATER_PASSWORD",
        "secrets.V4_UPDATER_PASSWORD",
        "updater_password_env",
        "credential_target",
        "updater_private_key_path:",
        "inputs.updater_private_key_path",
        "Mask updater key path",
        "::add-mask::",
        "Sky-Auto-Player-Updater.exe",
        "MANIFEST.json.sig",
        "ci_tauri_update_e2e.ps1",
    ] {
        if workflow.contains(forbidden) {
            return Err(
                format!("v4 release workflow contains forbidden marker: {forbidden}").into(),
            );
        }
    }
    if workflow.matches("GH_TOKEN: ${{ github.token }}").count() < 1 {
        return Err(
            "repository GITHUB_TOKEN must be present on every GitHub-mutating/read step".into(),
        );
    }
    validate_metadata_app_token_scope(&workflow)?;

    let publish_step = workflow
        .find("- name: Publish the qualified candidate immutably")
        .ok_or("v4 release workflow is missing the publication step")?;
    let metadata_token_step = workflow
        .find("- name: Mint release-metadata GitHub App token before publication")
        .ok_or("v4 release workflow is missing the metadata App token step")?;
    let probe_step = workflow
        .find("- name: Probe release-metadata App access before publication")
        .ok_or("v4 release workflow is missing the metadata App access probe")?;
    if metadata_token_step >= probe_step || probe_step >= publish_step {
        return Err(
            "metadata App mint and read-only probe must precede immutable publication".into(),
        );
    }
    let probe_end = workflow[probe_step..]
        .find("\n      - name:")
        .map_or(workflow.len(), |relative| probe_step + relative);
    let probe_block = &workflow[probe_step..probe_end];
    if !probe_block.contains("GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}")
        || probe_block.contains("GH_TOKEN: ${{ github.token }}")
        || !probe_block.contains("probe_v4_metadata_app_access.ps1")
    {
        return Err(
            "metadata App access probe must be read-only and use only the short-lived App token"
                .into(),
        );
    }
    for forbidden in [
        "Snapshot GitHub Latest before publication",
        "Verify GitHub Latest channel policy before metadata promotion",
        "-Mode Capture",
        "-Mode Verify",
        "release-state.json",
    ] {
        if workflow.contains(forbidden) {
            return Err(format!(
                "production workflow retains retired Latest/state handoff: {forbidden}"
            )
            .into());
        }
    }

    if pipeline
        .matches("cargo xtask credential run-action")
        .count()
        != 1
        || pipeline.contains("orchestrate_v4_production_release.ps1")
    {
        return Err(
            "production build must have exactly one Rust-owned credential action call site".into(),
        );
    }
    for marker in [
        "Preflight",
        "BuildCandidate",
        "PublishRelease",
        "PromoteMetadata",
        "FinalVerify",
        "unsigned-zero-budget",
        "canonical repository main is not initialized",
        "refs/heads/main",
        "release-metadata branch is not initialized",
        "Assert-MetadataBranchReadiness",
        "metadataBootstrapContract",
        "release-metadata readiness",
        "repository already contains published release/tag",
        "V4 unpublished draft cleanup",
        "published tags are immutable",
        "git/refs/tags/$Tag",
        "GitHub's successful DELETE endpoints return an empty body",
        "Get-FileHash",
        "verify-signature",
        "verify-tauri-bundle",
        "v4_release_latest_policy.ps1",
        "Get-RemoteLatestRelease",
        "Assert-V4GitHubLatestPolicy",
        "Invoke-PrePublicationMetadataQualification",
        "Assert-RawMetadataRetryConfiguration",
        "promote_v4_metadata.ps1",
        "release-metadata",
        "published_at",
        "draft = $true",
        "draft = $false",
        "Get-V4ReleaseMakeLatestValue",
        "Get-V4ReleaseDraftMakeLatestValue",
        "make_latest = Get-V4ReleaseDraftMakeLatestValue",
        "draft false; stable publish true; beta publish false",
        "target_commitish = $SourceSha.ToLowerInvariant()",
        "branch = \"release-metadata\"",
        "validate-monotonic",
        "Write-RepositoryContentFile",
        "New-ExpectedV4Metadata",
        "Assert-ExactFileBytes",
        "Assert-ExactPublishedPublicAssetRecords",
        "Assert-RawMetadataConverges",
        "RawMetadataRetryBudgetSeconds",
        "raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json",
        "raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json",
        "AllowAutoRedirect",
        "Headers.Authorization",
        "ReadAsByteArrayAsync",
        "GITHUB_REPOSITORY",
        "Invoke-GitHubApi",
        "v4_release_asset_upload.ps1",
        "upload_url",
        "Assert-ImmutableRelease",
        "Assert-ImmutableRelease $published",
        "repository release is not marked immutable",
        "\"xtask\", \"ci\", \"updater-fixture\", \"run\"",
        "\"xtask\", \"ci\", \"candidate\", \"create\"",
        "--build-bridge",
        "--bridge-version",
        "export-public-key",
        "Start-MpScan",
        "scan_performed",
        "scan_v4_defender_exact.ps1",
        "fixture-http-evidence.json",
        "cargo xtask credential run-action",
        "v4_release_draft_lookup.ps1",
        "Select-V4ReleaseByTag",
        "--paginate",
        "--slurp",
        "releases?per_page=100",
        "existing draft source does not match the requested source",
        "draft release could not be removed by release id",
    ] {
        if !pipeline.contains(marker) {
            return Err(
                format!("v4 release coordinator is missing its required marker: {marker}").into(),
            );
        }
    }
    let preflight_position = pipeline
        .find("function Invoke-Preflight")
        .ok_or("v4 release coordinator is missing Preflight")?;
    let build_position = pipeline
        .find("function Invoke-BuildCandidate")
        .ok_or("v4 release coordinator is missing BuildCandidate")?;
    let preflight_block = &pipeline[preflight_position..build_position];
    if preflight_block.contains("--method")
        || preflight_block.contains("DELETE")
        || preflight_block.contains("Invoke-DraftSelfCleanup")
    {
        return Err("Preflight must be externally read-only and must not delete drafts".into());
    }
    let publish_position = pipeline
        .find("function Invoke-PublishRelease")
        .ok_or("v4 release coordinator is missing PublishRelease")?;
    if pipeline[publish_position..]
        .find("Remove-V4StaleMatchingDraft")
        .is_none()
    {
        return Err("PublishRelease must own stale matching-draft cleanup".into());
    }
    if pipeline.contains("repos/$repository/immutable-releases") {
        return Err(
            "repository policy validation must not call the administration-only immutable-releases endpoint"
                .into(),
        );
    }
    if pipeline.contains("repo\", \"clone") || pipeline.contains("release-metadata checkout") {
        return Err("PromoteMetadata must not clone or checkout release-metadata".into());
    }
    if pipeline.contains("release-state.json") {
        return Err("production release pipeline must not depend on release-state.json".into());
    }
    for marker in [
        "PublishRelease",
        "Preflight",
        "Format-V4TransactionMarker",
        "Get-V4TransactionMarker",
        "Test-V4TransactionMarkerMatch",
        "Invoke-DraftSelfCleanup",
        "candidate-manifest.json",
        "release-context.json",
        "Write-V4ReleaseContext",
        "Import-V4ReleaseContext",
        "Get-SourceReleaseIdentity",
        "Remove-V4StaleMatchingDraft",
        "stale_matching_draft",
        "V4 unpublished draft cleanup",
    ] {
        if !pipeline.contains(marker) {
            return Err(
                format!("v4 release coordinator is missing its required marker: {marker}").into(),
            );
        }
    }
    let publish_release_position = pipeline
        .find("function Invoke-PublishRelease")
        .ok_or("v4 release coordinator is missing the publication transaction boundary")?;
    let promote_position = pipeline
        .find("function Invoke-PromoteMetadata")
        .ok_or("v4 release coordinator is missing the metadata promotion state")?;
    if publish_release_position >= promote_position {
        return Err("PublishRelease must precede PromoteMetadata".into());
    }
    let publish_block = &pipeline[publish_release_position..promote_position];
    let draft_latest_pos = publish_block
        .find("make_latest = Get-V4ReleaseDraftMakeLatestValue")
        .ok_or("PublishRelease must create draft with make_latest=false helper")?;
    let publish_latest_pos = publish_block
        .find("make_latest = (Get-V4ReleaseMakeLatestValue $Channel)")
        .or_else(|| publish_block.find("make_latest = Get-V4ReleaseMakeLatestValue $Channel"))
        .ok_or("PublishRelease must publish with channel-aware make_latest helper")?;
    let immutable_pos = publish_block
        .find("Assert-ImmutableRelease $published")
        .ok_or("PublishRelease must assert immutable release before metadata promotion")?;
    if draft_latest_pos >= publish_latest_pos || publish_latest_pos >= immutable_pos {
        return Err(
            "PublishRelease draft creation, publication patch, and immutable check must be strictly ordered"
                .into(),
        );
    }
    if pipeline[publish_release_position..].contains("orchestrate_v4_production_release.ps1") {
        return Err("v4 release coordinator may not rebuild after publication start".into());
    }
    if !regression.contains("MockReleaseApi")
        || !regression.contains("candidate rebuilt")
        || !regression.contains("promotion before immutable publication")
        || !regression.contains("BuildCount -ne 1")
        || !regression.contains("UploadedThroughReleaseUrl")
        || !regression.contains("ExactDownloadedBytes")
        || !regression.contains("immutable")
    {
        return Err(
            "v4 release coordinator regression test is missing build-once/publication guards"
                .into(),
        );
    }
    let xtask_source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let nsis_harness = fs::read_to_string(xtask_source.join("nsis_harness.rs"))?;
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
        if !nsis_harness.contains(marker) {
            return Err(format!(
                "Rust NSIS owner regression is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let updater_fixture = fs::read_to_string(xtask_source.join("updater_fixture.rs"))?;
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
    ] {
        if !updater_fixture.contains(marker) {
            return Err(format!(
                "Rust updater fixture regression is missing its required marker: {marker}"
            )
            .into());
        }
    }
    if !regression.contains("Test-DraftLookupFallback") {
        return Err(
            "v4 release coordinator regression test is missing the hidden-draft collection fallback"
                .into(),
        );
    }
    nsis_installed_catalog_contract(include_str!("../nsis_harness.rs"))?;
    Ok(())
}

fn v4_production_qualification_contract_source(workflow: &str, pipeline: &str) -> Result<()> {
    let workflow = workflow.replace("\r\n", "\n");
    for marker in [
        "name: V4 Production Pre-Publication Qualification",
        "workflow_dispatch:",
        "qualification-dispatch-boundary",
        "github.event.repository.default_branch",
        "refs/heads/main",
        "runs-on: [self-hosted, windows, v4-release, single-tenant]",
        "environment: v4-production-release",
        "ref: ${{ github.sha }}",
        "persist-credentials: false",
        "V4_UPDATER_PRIVATE_KEY_PATH",
        "verify_v4_release_runner.ps1",
        "-State Preflight",
        "-State BuildCandidate",
        "release-context.json",
        "cleanup_v4_release_state.ps1",
        "if: always()",
    ] {
        if !workflow.contains(marker) {
            return Err(format!(
                "production qualification workflow is missing its required marker: {marker}"
            )
            .into());
        }
    }
    if !workflow_declares_only_dispatch(&workflow) {
        return Err(
            "production qualification must whitelist workflow_dispatch as its only trigger".into(),
        );
    }
    let states = workflow_state_invocations(&workflow);
    if states != ["Preflight", "BuildCandidate"] {
        return Err(format!(
            "production qualification must invoke exactly Preflight -> BuildCandidate, got {states:?}"
        )
        .into());
    }
    validate_v4_release_state_invocations(&workflow, pipeline)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    for forbidden in [
        "-State PublishRelease",
        "-State PromoteMetadata",
        "-State FinalVerify",
        "actions/attest@",
        "create-github-app-token",
        "metadata-app-token",
        "contents: write",
        "gh release",
        "softprops/action-gh-release",
        "updater_private_key_path:",
        "inputs.updater_private_key_path",
    ] {
        if workflow.contains(forbidden) {
            return Err(format!(
                "production qualification contains a forbidden publication or legacy marker: {forbidden}"
            )
            .into());
        }
    }
    Ok(())
}

fn nsis_installed_catalog_contract(harness: &str) -> Result<()> {
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

pub(super) fn v4_release_pipeline_contract(root: &Path) -> Result<()> {
    let workflow_path = root.join(".github/workflows/release-v4.yml");
    let draft_workflow_path = root.join(".github/workflows/rehearse-v4.yml");
    let pipeline_path = root.join("scripts/v4_release_pipeline.ps1");
    let regression_path = root.join("scripts/test_v4_release_pipeline.ps1");
    let pipeline = fs::read_to_string(&pipeline_path)?;
    let regression = fs::read_to_string(&regression_path)?;
    let orchestrator =
        fs::read_to_string(root.join("scripts/orchestrate_v4_production_release.ps1"))?;
    let draft_workflow = fs::read_to_string(&draft_workflow_path)?;
    for script_name in [
        "scripts/v4_nsis_smoke_boundary.ps1",
        "scripts/test_v4_nsis_smoke_boundary.ps1",
        "scripts/ci_tauri_update_e2e.ps1",
        "scripts/ci_tauri_update_e2e_core.ps1",
        "scripts/test_v4_updater_fixture_server.ps1",
        "scripts/ci_build_updater_bridge.ps1",
        "scripts/updater_candidate_source_identity.ps1",
    ] {
        if root.join(script_name).exists() {
            return Err(format!(
                "Wave C Rust-owned fixture migration left its retired PowerShell entrypoint present: {script_name}"
            )
            .into());
        }
    }
    for marker in [
        "cargo xtask ci candidate create",
        "cargo xtask ci nsis smoke",
        "--allow-protected-release-runner",
        "--registry-mode",
        "fresh-install",
    ] {
        if !orchestrator.contains(marker) {
            return Err(format!(
                "protected release NSIS caller is missing its Rust owner marker: {marker}"
            )
            .into());
        }
    }
    v4_release_pipeline_contract_source(
        &fs::read_to_string(&workflow_path)?,
        &pipeline,
        &regression,
    )
    .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
        format!("v4 release pipeline contract: {error}").into()
    })?;
    v4_production_qualification_contract_source(&draft_workflow, &pipeline).map_err(
        |error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("production qualification contract: {error}").into()
        },
    )?;
    for script_name in [
        "scripts/v4_updater_credential_broker.ps1",
        "scripts/set_v4_updater_session_credential.ps1",
        "scripts/remove_v4_updater_session_credential.ps1",
        "scripts/test_v4_updater_credential_broker.ps1",
    ] {
        if !root.join(script_name).exists() {
            return Err(
                format!("v4 release pipeline is missing required helper: {script_name}").into(),
            );
        }
    }
    let broker = fs::read_to_string(root.join("scripts/v4_updater_credential_broker.ps1"))?;
    for forbidden in [
        "Add-Type",
        "PSTypeName",
        "DllImport",
        "CREDENTIAL_WRITE",
        "CredReadW",
    ] {
        if broker.contains(forbidden) {
            return Err(format!(
                "PowerShell credential adapter still owns native operation {forbidden}"
            )
            .into());
        }
    }
    let credential_owner =
        fs::read_to_string(root.join("rust/crates/sky_windows_security/src/credential.rs"))?;
    let credential_native =
        fs::read_to_string(root.join("rust/crates/sky_windows_security/src/win32.rs"))?;
    for marker in [
        "CANONICAL_UPDATER_CREDENTIAL_TARGET",
        "test_target",
        "MAX_CREDENTIAL_BLOB",
        "write_session",
        "write_machine_persistent_for_test",
        "run_one_shot",
        "reject_ambient_password",
        "CredentialBoundaryError",
    ] {
        if !credential_owner.contains(marker) {
            return Err(format!(
                "Rust credential owner is missing its required contract: {marker}"
            )
            .into());
        }
    }
    for marker in [
        "CredReadW",
        "CredWriteW",
        "CredDeleteW",
        "CRED_TYPE_GENERIC",
        "CRED_PERSIST_SESSION",
        "credential is not session-persistent",
        "credential_write_machine_for_test",
        "CredFree",
    ] {
        if !credential_native.contains(marker) {
            return Err(format!(
                "Rust Windows credential adapter is missing its required operation: {marker}"
            )
            .into());
        }
    }
    let provision = fs::read_to_string(root.join("scripts/set_v4_updater_session_credential.ps1"))?;
    for marker in [
        "Read-Host",
        "-AsSecureString",
        "RedirectStandardInput",
        "StandardInput.Write($passwordValue)",
        "ZeroFreeBSTR",
    ] {
        if !provision.contains(marker) {
            return Err(format!(
                "credential provisioning adapter is missing its hidden-input boundary: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "[string]$Password",
        "-Password ",
        "Write-Output $passwordValue",
    ] {
        if provision.contains(forbidden) {
            return Err(format!(
                "credential provisioning adapter exposes plaintext through {forbidden}"
            )
            .into());
        }
    }
    let credential_cli = fs::read_to_string(root.join("rust/xtask/src/credential_cli.rs"))?;
    if credential_cli
        .matches("scripts/orchestrate_v4_production_release.ps1")
        .count()
        != 1
    {
        return Err(
            "Rust credential action must allowlist exactly one production orchestrator".into(),
        );
    }
    for marker in [
        "run_one_shot",
        "reject_ambient_password",
        "redact_secret_bytes",
        "execute_one_shot_action",
        "actual_child_launcher_redacts_environment_secret_and_deletes_after_success_or_failure",
        "actual_child_launcher_kills_early_parent_descendant_holding_output_pipes",
        "action_environment_rejects_ambient_password_and_private_key_material",
        "credential action exited with code",
    ] {
        if !credential_cli.contains(marker) {
            return Err(
                format!("Rust credential CLI is missing its lifecycle contract: {marker}").into(),
            );
        }
    }
    let credential_cli_windows_test =
        fs::read_to_string(root.join("rust/xtask/tests/credential_cli_windows.rs"))?;
    for marker in [
        "actual_cli_action_rejects_ambient_password_and_deletes_after_child_failure",
        "BENIGN_RUST_ACTION_SECRET_",
        "write_session",
        "has_nonempty",
    ] {
        if !credential_cli_windows_test.contains(marker) {
            return Err(format!(
                "Windows credential CLI regression is missing its isolated action marker: {marker}"
            )
            .into());
        }
    }
    for marker in [
        "cargo xtask credential run-action",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    ] {
        if !pipeline.contains(marker) {
            return Err(format!(
                "v4 release coordinator is missing Rust credential ownership: {marker}"
            )
            .into());
        }
    }
    let key_verifier = fs::read_to_string(root.join("scripts/verify_v4_updater_private_key.ps1"))?;
    if !key_verifier.contains("cargo xtask credential run-action")
        || key_verifier.contains("Get-V4UpdaterProductionCredential")
    {
        return Err("broker-backed key verification must delegate to Rust without exposing the credential to PowerShell".into());
    }
    let upload_helper_path = root.join("scripts/v4_release_asset_upload.ps1");
    let upload_helper = fs::read_to_string(&upload_helper_path)?;
    for (name, source) in [("production release pipeline", pipeline.as_str())] {
        for forbidden in [
            "gh @Arguments --output",
            "gh.exe @Arguments --output",
            "--output $OutputPath",
            "\"$uploadUrl?name=",
        ] {
            if source.contains(forbidden) {
                return Err(format!(
                    "{name} must not use gh api --output for binary asset downloads"
                )
                .into());
            }
        }
        for marker in [
            "Invoke-GhBinaryOutput",
            "Invoke-V4ReleaseAssetUpload",
            "PSVersionTable.PSVersion",
            "7.4.0",
            "RedirectStandardOutput",
            "RedirectStandardError",
            "StandardOutput.BaseStream",
            "ReadToEndAsync",
            "ArgumentList",
        ] {
            if !source.contains(marker) {
                return Err(
                    format!("{name} binary download helper is missing marker: {marker}").into(),
                );
            }
        }
    }
    for marker in [
        "System.Net.Http.HttpClient",
        "System.Net.Http.StreamContent",
        "System.IO.FileStream",
        "Headers.Authorization",
        "UserAgent",
        "application/vnd.github+json",
        "X-GitHub-Api-Version",
        "2026-03-10",
        "uploads.github.com",
        "ContentLength",
        "fileLength",
        "StatusCode",
        "System.Net.HttpStatusCode",
        "Created",
        "SendAsync",
        "ReadAsStringAsync",
        "application/octet-stream",
        "$client.Timeout = [TimeSpan]::FromMinutes(10)",
    ] {
        if !upload_helper.contains(marker) {
            return Err(
                format!("raw release asset upload helper is missing marker: {marker}").into(),
            );
        }
    }
    if upload_helper.contains("InfiniteTimeSpan") {
        return Err("raw release asset upload helper must use a finite timeout".into());
    }
    if upload_helper.contains("gh ") || upload_helper.contains("ArgumentList") {
        return Err("raw release asset upload helper must not invoke GitHub CLI".into());
    }
    if upload_helper.contains("$UploadUrl?name=") {
        return Err(
            "raw release asset upload helper uses ambiguous PowerShell URL interpolation".into(),
        );
    }
    for marker in [
        "UploadUrl.Contains(\"?\")",
        "[string]::Concat($UploadUrl, \"?name=\"",
        "escapedAssetName",
    ] {
        if !upload_helper.contains(marker) {
            return Err(format!(
                "raw release asset upload URL construction guard is missing: {marker}"
            )
            .into());
        }
    }
    println!("[xtask] v4 release pipeline state-machine contract: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::release_runner::{
        RELEASE_RUNNER_LABEL_MARKER, validate_release_runner_workflow,
    };
    use super::*;

    #[test]
    fn production_qualification_contract_rejects_publication_and_weak_runner_regressions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let workflow = fs::read_to_string(root.join(".github/workflows/rehearse-v4.yml"))
            .expect("production qualification workflow fixture must exist");
        let pipeline = fs::read_to_string(root.join("scripts/v4_release_pipeline.ps1"))
            .expect("release pipeline fixture must exist");

        v4_production_qualification_contract_source(&workflow, &pipeline)
            .expect("production qualification contract should pass its repository fixture");

        let publication_regression = workflow.replace(
            "-State BuildCandidate",
            "-State BuildCandidate\n            -State PublishRelease",
        );
        assert!(
            v4_production_qualification_contract_source(&publication_regression, &pipeline)
                .is_err(),
            "production qualification must reject a publication state"
        );

        let weak_runner = workflow.replace(
            RELEASE_RUNNER_LABEL_MARKER,
            "runs-on: [self-hosted, windows, v4-release]",
        );
        let error =
            validate_release_runner_workflow(".github/workflows/rehearse-v4.yml", &weak_runner)
                .expect_err("production qualification must retain the exact signing runner labels");
        assert!(
            error
                .to_string()
                .contains("exact dedicated release runner labels")
        );
    }

    #[test]
    fn v4_release_pipeline_contract_requires_single_path_order() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let workflow = fs::read_to_string(root.join(".github/workflows/release-v4.yml"))
            .expect("production release workflow fixture must exist");
        let pipeline = fs::read_to_string(root.join("scripts/v4_release_pipeline.ps1"))
            .expect("release pipeline fixture must exist");
        let regression = fs::read_to_string(root.join("scripts/test_v4_release_pipeline.ps1"))
            .expect("release pipeline regression fixture must exist");

        v4_release_pipeline_contract_source(&workflow, &pipeline, &regression)
            .expect("release pipeline contract should pass its repository fixture");

        let reordered = workflow.replacen("-State Preflight `", "-State BuildCandidate `", 1);
        assert!(v4_release_pipeline_contract_source(&reordered, &pipeline, &regression).is_err());

        let duplicated_build = pipeline.replace(
            "function Invoke-PublishRelease",
            "orchestrate_v4_production_release.ps1\nfunction Invoke-PublishRelease",
        );
        assert!(
            v4_release_pipeline_contract_source(&workflow, &duplicated_build, &regression).is_err()
        );

        let nsis_harness = include_str!("../nsis_harness.rs");
        nsis_installed_catalog_contract(nsis_harness)
            .expect("Rust NSIS qualification must retain built-in catalog verification");
        let missing_builtin_qualification =
            nsis_harness.replace("crate::builtin_catalog::run(", "");
        assert!(
            nsis_installed_catalog_contract(&missing_builtin_qualification).is_err(),
            "production qualification must retain installed built-in catalog verification"
        );

        let missing_empty_directory_regression =
            nsis_harness.replace("assert_user_songs_empty(&app_data.join(\"songs\"))?;", "");
        assert!(
            nsis_installed_catalog_contract(&missing_empty_directory_regression).is_err(),
            "production qualification must retain the executable StrictMode empty-directory regression"
        );
    }
}
