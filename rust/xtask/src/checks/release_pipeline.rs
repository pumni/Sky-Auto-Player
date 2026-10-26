use super::workflow::{workflow_declares_only_dispatch, workflow_job_blocks, workflow_step_blocks};
use crate::Result;
use std::fs;
use std::path::Path;

fn source(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn has_permission(job: &str, permission: &str) -> bool {
    job.lines().any(|line| line.trim() == permission)
}

fn metadata_recovery_rust_contract(recovery: &str) -> std::result::Result<(), String> {
    let recovery = source(recovery);
    for marker in [
        "struct Plan",
        "fn validate_runtime",
        "fn parse_marker",
        "fn validate_run",
        "fn build_plan(",
        "verify_updater_signature_for_version",
        "gh",
        "--source-digest",
        "--signer-workflow",
        "https://spdx.dev/Document/v2.3",
        "fn read_metadata(",
        "raw.githubusercontent.com",
        "fn apply(",
        "super::reconcile_metadata_put(",
        "metadata recovery plan is stale or differs from operator-reviewed bytes",
        "metadata branch, public endpoint, or beta pointer changed after the reviewed plan",
        "stable metadata changed at the final Contents API write boundary",
        "metadata_recovery_updates_only_stable_and_reconciles_timeout_once",
        "metadata_recovery_noop_is_idempotent_and_stale_plan_fails_before_write",
        "latest_conflict_duplicate_assets_wrong_tag_and_tampered_bytes_fail_closed",
    ] {
        if !recovery.contains(marker) {
            return Err(format!(
                "Rust metadata-only recovery is missing invariant `{marker}`"
            ));
        }
    }
    let planner = recovery
        .split_once("fn build_plan(")
        .and_then(|(_, body)| body.split_once("\nfn encode_plan("))
        .map(|(body, _)| body)
        .ok_or("Rust metadata recovery planner is not bounded")?;
    if planner.contains("transport.request(\"PUT\"")
        || planner.contains("transport.request(\"POST\"")
        || planner.contains("transport.request(\"DELETE\"")
        || planner.contains("transport.upload(")
    {
        return Err("metadata recovery plan must not contain GitHub mutations".into());
    }
    let apply = recovery
        .split_once("fn apply(")
        .and_then(|(_, body)| body.split_once("\n#[cfg(test)]\nmod tests"))
        .map(|(body, _)| body)
        .ok_or("Rust metadata recovery apply path is not bounded")?;
    if apply.matches("transport.request(\"PUT\"").count() != 1
        || apply.contains("transport.request(\"POST\"")
        || apply.contains("transport.request(\"DELETE\"")
        || apply.contains("transport.upload(")
        || !apply.contains("channels/stable/latest.json")
        || apply.contains("channels/beta/latest.json\"")
    {
        return Err("metadata recovery may issue one stable Contents PUT and must preserve every immutable or beta asset".into());
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
        "fn uninstaller_working_directory(",
        "fn finish_uninstall(",
        "wait_tree_timeout(std::time::Duration::from_secs(30))",
        "active_processes()",
        "uninstaller exit status:",
        "uninstall registration:",
        "bounded_install_root_inventory(",
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
    let uninstall = qualification
        .find("run_uninstaller(&uninstaller, &install_root, &shell_env)?")
        .ok_or("Rust NSIS qualification does not run the installed uninstaller")?;
    let finish_uninstall = qualification
        .find("finish_uninstall(&install_root, &mut uninstaller)?")
        .ok_or("Rust NSIS qualification does not await uninstall tree and root cleanup")?;
    let registry_cleanup = qualification
        .find("assert_uninstall_registration_absent()?")
        .ok_or("Rust NSIS qualification must verify uninstall registration removal")?;
    if qualification.contains("remove_dir_all(&install_root)")
        || qualification.contains("remove_dir(&install_root)")
    {
        return Err("NSIS qualification must not manually delete the install root to claim uninstall success".into());
    }
    if install >= catalog
        || catalog >= launch
        || songs_guard >= launch
        || launch >= uninstall
        || uninstall >= finish_uninstall
        || finish_uninstall >= registry_cleanup
        || !qualification[catalog..launch].contains("verify-installed")
    {
        return Err(
            "installed catalog and empty user-song checks must follow install and precede app launch"
                .into(),
        );
    }
    let working_directory = harness
        .split_once("fn uninstaller_working_directory(")
        .and_then(|(_, body)| body.split_once("\n#[cfg(windows)]\npub(crate) fn finish_uninstall("))
        .map(|(body, _)| body)
        .ok_or("NSIS uninstaller working-directory guard is not bounded")?;
    if !working_directory.contains("let cwd = install_root")
        || !working_directory.contains(".parent()")
        || !working_directory.contains("cwd.starts_with(&install_root)")
    {
        return Err(
            "NSIS uninstaller working directory must remain outside its install root".into(),
        );
    }
    let completion = harness
        .split_once("pub(crate) fn finish_uninstall(")
        .and_then(|(_, body)| {
            body.split_once("\n#[cfg(windows)]\nfn uninstall_registration_diagnostic(")
        })
        .map(|(body, _)| body)
        .ok_or("NSIS uninstall completion guard is not bounded")?;
    let tree_wait = completion
        .find("wait_tree_timeout(")
        .ok_or("NSIS uninstall must wait for the owned process tree")?;
    let root_wait = completion
        .find("wait_path_absent(")
        .ok_or("NSIS uninstall must wait for install-root removal")?;
    if tree_wait >= root_wait {
        return Err(
            "NSIS uninstall must drain its owned process tree before confirming root removal"
                .into(),
        );
    }
    if completion.contains("remove_dir") {
        return Err("NSIS qualification must not manually remove the install root to claim uninstall success".into());
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
        "run_uninstaller(&uninstaller, &install_root, &env)",
        "crate::nsis_harness::finish_uninstall",
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
    let rust = source(rust);
    let main = source(main);
    let ci = source(ci);
    if !workflow_declares_only_dispatch(&workflow) {
        return Err(
            "production release workflow must expose workflow_dispatch as its only trigger".into(),
        );
    }
    if !rehearsal.trim().is_empty() {
        return Err("the duplicate production rehearsal workflow must be retired".into());
    }
    for marker in [
        "name: V4 Release Pipeline",
        "workflow_dispatch:",
        "runs-on: ubuntu-latest",
        "runs-on: windows-latest",
        "environment: v4-production-release",
        "contents: read",
        "contents: write",
        "id-token: write",
        "attestations: read",
        "attestations: write",
        "ref: ${{ github.sha }}",
        "persist-credentials: false",
        "GITHUB_RUN_ATTEMPT",
        "workflow_dispatch",
        "EXPECTED_DEFAULT_BRANCH: ${{ github.event.repository.default_branch }}",
        "test \"$GITHUB_EVENT_NAME\" = \"workflow_dispatch\"",
        "test \"$GITHUB_REPOSITORY\" = \"$EXPECTED_REPOSITORY\"",
        "test \"$GITHUB_REF\" = \"refs/heads/main\"",
        "test \"$EXPECTED_DEFAULT_BRANCH\" = \"main\"",
        ".event == \"push\"",
        ".head_branch == \"main\"",
        ".head_sha == env.GITHUB_SHA",
        "workflows/ci.yml/runs?head_sha=$GITHUB_SHA&per_page=100",
        "max_by(.run_number)",
        "test \"$status\" = \"completed\" && test \"$conclusion\" = \"success\"",
        "v4-context-${{ github.run_id }}-${{ github.run_attempt }}",
        "v4-unsigned-${{ github.run_id }}-${{ github.run_attempt }}",
        "v4-qualified-${{ github.run_id }}-${{ github.run_attempt }}",
        "v4-published-${{ github.run_id }}-${{ github.run_attempt }}",
        "v4-promoted-${{ github.run_id }}-${{ github.run_attempt }}",
        "V4_TAURI_UPDATER_PRIVATE_KEY: ${{ secrets.V4_TAURI_UPDATER_PRIVATE_KEY }}",
        "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD: ${{ secrets.V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD }}",
        "actions/attest@",
        "actions/upload-artifact@",
        "actions/download-artifact@",
        "Mint write-capable metadata App token before publication",
        "Mint contents-scoped metadata App token",
        "Probe release-metadata App write capability without mutation",
        "V4_RELEASE_METADATA_TOKEN_REQUESTED_CONTENTS_PERMISSION: write",
        "permission-contents: write",
        "if: always()",
    ] {
        if !workflow.contains(marker) {
            return Err(format!("production release workflow is missing `{marker}`"));
        }
    }
    if workflow.contains("inputs:")
        || workflow.contains("inputs.")
        || workflow.contains("self-hosted")
        || workflow.contains("rehearse-v4")
        || workflow.contains("V4_UPDATER_PRIVATE_KEY_PATH")
        || workflow.contains("build-candidate")
    {
        return Err(
            "production workflow contains a retired dispatch, self-hosted, or combined-build path"
                .into(),
        );
    }

    let jobs = workflow_job_blocks(&workflow);
    let observed = jobs.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>();
    let expected = [
        "prepare",
        "build_unsigned",
        "sign_qualify",
        "attest",
        "metadata_probe",
        "publish",
        "metadata_promote",
        "final_verify",
    ];
    if observed != expected {
        return Err(format!(
            "production release jobs must preserve the eight bounded stages, got {observed:?}"
        ));
    }
    let job = |id: &str| {
        jobs.iter()
            .find(|(job_id, _)| job_id == id)
            .map(|(_, block)| block.as_str())
            .ok_or_else(|| format!("production release workflow is missing job {id}"))
    };
    for (id, needs) in [
        ("build_unsigned", "needs: prepare"),
        ("sign_qualify", "needs: [prepare, build_unsigned]"),
        ("attest", "needs: [prepare, sign_qualify]"),
        ("metadata_probe", "needs: [prepare, attest]"),
        ("publish", "needs: [prepare, attest, metadata_probe]"),
        ("metadata_promote", "needs: [prepare, publish]"),
        ("final_verify", "needs: [prepare, metadata_promote]"),
    ] {
        if !job(id)?.lines().any(|line| line.trim() == needs) {
            return Err(format!("release job {id} must remain behind `{needs}`"));
        }
    }
    for (id, expected_runner) in [
        ("prepare", "runs-on: ubuntu-latest"),
        ("build_unsigned", "runs-on: windows-latest"),
        ("sign_qualify", "runs-on: windows-latest"),
        ("attest", "runs-on: ubuntu-latest"),
        ("metadata_probe", "runs-on: ubuntu-latest"),
        ("publish", "runs-on: ubuntu-latest"),
        ("metadata_promote", "runs-on: ubuntu-latest"),
        ("final_verify", "runs-on: ubuntu-latest"),
    ] {
        if !job(id)?.lines().any(|line| line.trim() == expected_runner) {
            return Err(format!("release job {id} must use {expected_runner}"));
        }
        if !job(id)?.contains("ref: ${{ github.sha }}")
            || !job(id)?.contains("persist-credentials: false")
        {
            return Err(format!(
                "release job {id} must check out the exact SHA without persisted credentials"
            ));
        }
    }
    let unsigned = job("build_unsigned")?;
    if unsigned.contains("secrets.")
        || unsigned.contains("environment:")
        || has_permission(unsigned, "contents: write")
        || has_permission(unsigned, "id-token: write")
        || has_permission(unsigned, "attestations: write")
        || !unsigned.contains("release-v4 build-unsigned")
    {
        return Err("unsigned build must be hosted, credential-free, and read-only".into());
    }
    let signer = job("sign_qualify")?;
    let sign_steps = workflow_step_blocks(signer);
    let secret_step = sign_steps
        .iter()
        .find(|(name, _)| name == "Sign the independently verified exact candidate")
        .map(|(_, block)| block.as_str())
        .ok_or("protected signing job is missing its narrow Rust signing step")?;
    let control_build = sign_steps
        .iter()
        .find(|(name, _)| name == "Build the pinned Rust release control executable before secrets")
        .map(|(_, block)| block.as_str())
        .ok_or("protected signing job must build its Rust control executable before secrets")?;
    let control_verify = sign_steps
        .iter()
        .find(|(name, _)| name == "Verify prebuilt control executable identity before secrets")
        .map(|(_, block)| block.as_str())
        .ok_or("protected signing job must verify its Rust control executable before secrets")?;
    let build_index = sign_steps
        .iter()
        .position(|(name, _)| {
            name == "Build the pinned Rust release control executable before secrets"
        })
        .ok_or("protected signing job lost the Rust control build step")?;
    let verify_index = sign_steps
        .iter()
        .position(|(name, _)| name == "Verify prebuilt control executable identity before secrets")
        .ok_or("protected signing job lost the pre-secret executable verification step")?;
    let signing_index = sign_steps
        .iter()
        .position(|(name, _)| name == "Sign the independently verified exact candidate")
        .ok_or("protected signing job lost its secret-bearing signing step")?;
    if build_index >= verify_index
        || verify_index >= signing_index
        || !control_build
            .contains("cargo build --locked --manifest-path rust/xtask/Cargo.toml --release")
        || !control_build.contains("Get-FileHash -LiteralPath $controlPath -Algorithm SHA256")
        || !control_build.contains("source_sha=$sourceSha")
        || !control_build.contains("sha256=$controlSha")
        || control_verify.contains("secrets.")
        || !control_verify.contains("EXPECTED_RELEASE_CONTROL_SOURCE_SHA")
        || !control_verify.contains("EXPECTED_RELEASE_CONTROL_SHA256")
        || !control_verify.contains("GITHUB_SHA")
        || !control_verify.contains("Get-FileHash -LiteralPath $controlPath -Algorithm SHA256")
    {
        return Err("protected signing must compile and verify the exact Rust control executable before secrets are mapped".into());
    }
    if !signer.contains("environment: v4-production-release")
        || !has_permission(signer, "contents: read")
        || has_permission(signer, "contents: write")
        || has_permission(signer, "id-token: write")
        || has_permission(signer, "attestations: write")
        || secret_step
            .matches("secrets.V4_TAURI_UPDATER_PRIVATE_KEY }}")
            .count()
            != 1
        || secret_step
            .matches("secrets.V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD }}")
            .count()
            != 1
        || !secret_step.contains("rust/target/release/xtask.exe")
        || !secret_step.contains("Get-FileHash -LiteralPath $controlPath -Algorithm SHA256")
        || secret_step.to_ascii_lowercase().contains("cargo")
        || !signer.contains("release-v4 qualify-candidate")
    {
        return Err("production signing must be environment-protected, read-only, and scoped to the Rust signing step".into());
    }
    for (name, block) in &sign_steps {
        if name != "Sign the independently verified exact candidate"
            && (block.contains("secrets.V4_TAURI_UPDATER_PRIVATE_KEY")
                || block.contains("secrets.V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD"))
        {
            return Err(format!(
                "production updater secrets escaped the dedicated signing step: {name}"
            ));
        }
    }
    for (id, marker) in [
        ("attest", "id-token: write"),
        ("attest", "attestations: write"),
        ("publish", "contents: write"),
        ("publish", "attestations: read"),
    ] {
        if !job(id)?.contains(marker) {
            return Err(format!("release job {id} is missing permission {marker}"));
        }
    }
    for id in [
        "prepare",
        "build_unsigned",
        "sign_qualify",
        "attest",
        "metadata_probe",
        "publish",
        "metadata_promote",
        "final_verify",
    ] {
        let block = job(id)?;
        if block.contains("V4_RELEASE_METADATA_APP_PRIVATE_KEY")
            && id != "metadata_probe"
            && id != "metadata_promote"
        {
            return Err(format!(
                "metadata App private key escaped its protected job: {id}"
            ));
        }
        if block.contains("V4_TAURI_UPDATER_PRIVATE_KEY") && id != "sign_qualify" {
            return Err(format!(
                "production updater key escaped the protected signing job: {id}"
            ));
        }
    }
    if !job("metadata_probe")?.contains("permission-contents: write")
        || !job("metadata_probe")?
            .contains("V4_RELEASE_METADATA_TOKEN_REQUESTED_CONTENTS_PERMISSION: write")
        || has_permission(job("metadata_probe")?, "contents: write")
        || !job("metadata_promote")?.contains("permission-contents: write")
        || has_permission(job("metadata_promote")?, "contents: write")
    {
        return Err("metadata App tokens must be explicitly write-scoped for the pre-publication probe and separately minted for Rust promotion, while Actions permissions stay read-only".into());
    }
    if job("publish")?.contains("V4_RELEASE_METADATA_APP_PRIVATE_KEY")
        || job("publish")?.contains("V4_TAURI_UPDATER_PRIVATE_KEY")
        || job("metadata_promote")?.contains("V4_TAURI_UPDATER_PRIVATE_KEY")
        || workflow
            .matches("secrets.V4_TAURI_UPDATER_PRIVATE_KEY }}")
            .count()
            != 1
        || workflow
            .matches("secrets.V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD }}")
            .count()
            != 1
    {
        return Err(
            "updater production secrets must be available only to the protected signer".into(),
        );
    }
    let workflow_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    for relative in ["ci.yml", "hosted-v4-signing-proof.yml"] {
        let source =
            fs::read_to_string(workflow_root.join(relative)).map_err(|error| error.to_string())?;
        if source.contains("V4_TAURI_UPDATER_PRIVATE_KEY")
            || source.contains("V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD")
        {
            return Err(format!(
                "production updater secrets must never enter PR/general CI: {relative}"
            ));
        }
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
        if workflow.contains(old) {
            return Err(format!(
                "legacy PowerShell release authority is still called: {old}"
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
        "mock_pr_merge_sha_does_not_break_release_or_owned_cleanup",
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
        "require_runtime_context(root, &options, expected_job)?",
        "fn build_unsigned",
        "fn sign_candidate",
        "fn qualify_candidate",
        "fn verify_candidate_handoff",
        "run_attempt",
        "unsigned_handoff",
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
    let context_validation = rust
        .split_once("fn validate_context(")
        .and_then(|(_, body)| body.split_once("\nfn preflight("))
        .map(|(body, _)| body)
        .ok_or("Rust release authority is missing pure transaction-context validation")?;
    if context_validation.contains("env::var(") || context_validation.contains("GITHUB_") {
        return Err(
            "transaction-context validation must not depend on ambient GitHub Actions variables"
                .into(),
        );
    }
    let release_cli = rust
        .split_once("pub(crate) fn run(")
        .and_then(|(_, body)| body.split_once("\nfn require_runtime_context("))
        .map(|(body, _)| body)
        .ok_or("Rust release CLI is missing its production runtime guard")?;
    let runtime_guard = release_cli
        .find("require_runtime_context(root, &options, expected_job)?")
        .ok_or("release side-effect phases must require exact runtime identity")?;
    let command_dispatch = release_cli[runtime_guard..]
        .find("match command")
        .map(|offset| runtime_guard + offset)
        .ok_or("release CLI phase dispatch is missing")?;
    if runtime_guard >= command_dispatch {
        return Err("release phases must validate runtime identity before dispatch".into());
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
        || !metadata_probe.contains("contents_permission != \"write\"")
        || !metadata_probe.contains("repos/{REPOSITORY}")
        || metadata_probe.contains("transport.request(\"PUT\"")
        || metadata_probe.contains("transport.request(\"POST\"")
    {
        return Err(
            "metadata token probe must require contents:write, verify the canonical repository identity, validate both channels, and use GET only".into(),
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
        "go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12",
        "actionlint\" -color",
    ] {
        if !ci.contains(marker) {
            return Err(format!(
                "CI release_contract must execute Rust release regressions: {marker}"
            ));
        }
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let handoff = source(
        &fs::read_to_string(manifest_dir.join("src/release_handoff.rs"))
            .map_err(|error| error.to_string())?,
    );
    let signing = source(
        &fs::read_to_string(manifest_dir.join("src/release_signing.rs"))
            .map_err(|error| error.to_string())?,
    );
    for marker in [
        "struct UnsignedArtifactManifest",
        "deny_unknown_fields",
        "MAX_HANDOFF_MANIFEST_BYTES",
        "MAX_INSTALLER_BYTES",
        "validate_unsigned_handoff",
        "validate_handoff_directory",
        "artifact_name",
        "run_attempt",
        "installer_sha256",
        "producer_job",
    ] {
        if !handoff.contains(marker) {
            return Err(format!("Rust unsigned handoff is missing bound `{marker}`"));
        }
    }
    for marker in [
        "V4_TAURI_UPDATER_PRIVATE_KEY",
        "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD",
        "Zeroizing",
        "MAX_PRIVATE_KEY_BYTES",
        "MAX_PASSPHRASE_BYTES",
        "RUNNER_TEMP",
        "create_new(true)",
        "verify_local_private_key",
        "verify_updater_signature_for_version",
        "19AABD2E7838818C",
        "cleanup_run_material",
        "capture_owned_timeout_runtime_only",
    ] {
        if !signing.contains(marker) {
            return Err(format!(
                "Rust production signing adapter is missing boundary `{marker}`"
            ));
        }
    }
    let updater_trust = source(
        &fs::read_to_string(manifest_dir.join("src/updater_trust.rs"))
            .map_err(|error| error.to_string())?,
    );
    let process = source(
        &fs::read_to_string(manifest_dir.join("src/process.rs"))
            .map_err(|error| error.to_string())?,
    );
    let repo = source(
        &fs::read_to_string(manifest_dir.join("src/repo.rs")).map_err(|error| error.to_string())?,
    );
    validate_signing_child_environment(&signing, &updater_trust, &process, &repo)?;
    let recovery = source(
        &fs::read_to_string(manifest_dir.join("src/release_v4/recovery.rs"))
            .map_err(|error| error.to_string())?,
    );
    metadata_recovery_rust_contract(&recovery)?;
    let credentials = source(
        &fs::read_to_string(manifest_dir.join("src/credential_cli.rs"))
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
    if !rust.contains("createUpdaterArtifacts\":false")
        || !workflow.contains("release-v4 build-unsigned")
        || !workflow.contains("release-v4 sign-candidate")
        || !workflow.contains("release-v4 qualify-candidate")
        || workflow.matches("release-v4 build-unsigned").count() != 1
    {
        return Err("release pipeline must disable build-time updater sidecars and keep build/sign as separate Rust phases".into());
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

fn validate_signing_child_environment(
    signing: &str,
    updater_trust: &str,
    process: &str,
    repo: &str,
) -> std::result::Result<(), String> {
    let signing_phase = signing
        .split_once("pub(crate) fn sign_installer(")
        .and_then(|(_, body)| body.split_once("\nfn verify_tauri_cli("))
        .map(|(body, _)| body)
        .ok_or("protected signer is missing sign_installer")?;
    let version_probe = signing
        .split_once("fn verify_tauri_cli(")
        .and_then(|(_, body)| body.split_once("\nfn materialize_key("))
        .map(|(body, _)| body)
        .ok_or("protected signer is missing its Tauri CLI version probe")?;
    let key_probe = updater_trust
        .split_once("pub fn verify_local_private_key(")
        .and_then(|(_, body)| body.split_once("\n/// Verify a Tauri updater signature"))
        .map(|(body, _)| body)
        .ok_or("updater trust is missing its local private-key challenge")?;
    let actual_child_test = process
        .split_once("fn runtime_child_environment_is_captured_after_secret_isolation(")
        .and_then(|(_, body)| body.split_once("\n    #[test]"))
        .map(|(body, _)| body)
        .ok_or("process policy is missing its spawned-child secret isolation regression")?;
    if !version_probe.contains("capture_owned_timeout_runtime_only")
        || !signing_phase.contains("capture_owned_timeout_runtime_only")
        || !key_probe.contains("capture_owned_timeout_runtime_only")
        || signing
            .matches("capture_owned_timeout_runtime_only")
            .count()
            != 2
        || repo.matches("capture_text_runtime_only").count() < 3
        || repo.contains("capture_text(\"git\"")
        || !process.contains("RUNTIME_CHILD_ENV_ALLOWLIST")
        || !process.contains("runtime_environment_from(env::vars_os(), explicit_env)")
        || !process.contains("command.env_clear();")
        || !actual_child_test.contains("capture_owned_timeout_isolated")
        || !actual_child_test.contains("synthetic-workflow-passphrase")
        || !actual_child_test.contains("GITHUB_TOKEN")
        || !actual_child_test.contains("TAURI_SIGNING_PRIVATE_KEY_PASSWORD")
    {
        return Err("all signing-phase subprocesses must use the bounded runtime-only environment, and the spawned-child regression must verify actual secret isolation".into());
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
        "",
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

    fn current_sources() -> (String, String, String, String) {
        let root = crate::repo::root();
        (
            fs::read_to_string(root.join(".github/workflows/release-v4.yml")).expect("workflow"),
            fs::read_to_string(root.join("rust/xtask/src/release_v4.rs"))
                .expect("Rust release owner"),
            fs::read_to_string(root.join("rust/xtask/src/main.rs")).expect("xtask entrypoint"),
            fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("CI workflow"),
        )
    }

    #[test]
    fn hosted_release_contract_accepts_the_current_rust_pipeline() {
        let (workflow, rust, main, ci) = current_sources();
        rust_release_state_machine_contract(&workflow, "", &rust, &main, &ci)
            .expect("current hosted release pipeline must satisfy its contracts");
    }

    #[test]
    fn hosted_release_contract_rejects_stale_sha_and_duplicate_build_paths() {
        let (workflow, rust, main, ci) = current_sources();
        let stale_gate = workflow.replace("head_sha=$GITHUB_SHA", "head_branch=main");
        assert!(rust_release_state_machine_contract(&stale_gate, "", &rust, &main, &ci).is_err());
        assert!(
            rust_release_state_machine_contract(&workflow, "retired rehearsal", &rust, &main, &ci)
                .is_err()
        );
        let legacy = workflow.replace("release-v4 sign-candidate", "release-v4 build-candidate");
        assert!(rust_release_state_machine_contract(&legacy, "", &rust, &main, &ci).is_err());
    }

    #[test]
    fn production_signing_secrets_cannot_enter_pr_ci() {
        let root = crate::repo::root();
        let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("CI workflow");
        let proof = fs::read_to_string(root.join(".github/workflows/hosted-v4-signing-proof.yml"))
            .expect("test-key proof workflow");
        for source in [&ci, &proof] {
            assert!(!source.contains("V4_TAURI_UPDATER_PRIVATE_KEY"));
            assert!(!source.contains("V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD"));
        }
    }

    #[test]
    fn signing_subprocess_contract_rejects_any_unfiltered_child_path() {
        let root = crate::repo::root().join("rust/xtask/src");
        let signing = fs::read_to_string(root.join("release_signing.rs")).expect("signing");
        let updater_trust = fs::read_to_string(root.join("updater_trust.rs")).expect("trust");
        let process = fs::read_to_string(root.join("process.rs")).expect("process");
        let repo = fs::read_to_string(root.join("repo.rs")).expect("repo");
        validate_signing_child_environment(&signing, &updater_trust, &process, &repo)
            .expect("all signing subprocesses must be isolated");

        let unfiltered_signing = signing.replace(
            "capture_owned_timeout_runtime_only(",
            "capture_owned_timeout_filtered(",
        );
        assert!(
            validate_signing_child_environment(
                &unfiltered_signing,
                &updater_trust,
                &process,
                &repo
            )
            .is_err()
        );

        let unfiltered_key_probe = updater_trust.replace(
            "capture_owned_timeout_runtime_only(",
            "capture_owned_timeout_filtered(",
        );
        assert!(
            validate_signing_child_environment(&signing, &unfiltered_key_probe, &process, &repo)
                .is_err()
        );

        let inherited_process_environment = process.replace("command.env_clear();", "");
        assert!(
            validate_signing_child_environment(
                &signing,
                &updater_trust,
                &inherited_process_environment,
                &repo
            )
            .is_err()
        );

        let inherited_git_environment = repo.replace("capture_text_runtime_only", "capture_text");
        assert!(
            validate_signing_child_environment(
                &signing,
                &updater_trust,
                &process,
                &inherited_git_environment
            )
            .is_err()
        );
    }

    #[test]
    fn metadata_recovery_static_contract_rejects_mutation_scope_expansion() {
        let path = crate::repo::root().join("rust/xtask/src/release_v4/recovery.rs");
        let recovery = fs::read_to_string(path).expect("metadata recovery implementation");
        metadata_recovery_rust_contract(&recovery).expect("recovery scope must be bounded");
        let extra_mutation = recovery.replace(
            "transport.request(\"PUT\", &put_endpoint, Some(&payload))",
            "transport.request(\"POST\", &put_endpoint, Some(&payload))",
        );
        assert!(metadata_recovery_rust_contract(&extra_mutation).is_err());
        let planner_write = recovery.replace(
            "fn build_plan(",
            "fn build_plan(\n    // transport.request(\"PUT\") is forbidden here",
        );
        assert!(metadata_recovery_rust_contract(&planner_write).is_err());
    }
}
