use super::workflow::{workflow_declares_only_dispatch, workflow_job_blocks, workflow_step_blocks};
use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const APPROVED_RELEASE_WORKFLOW: &str = ".github/workflows/release-v4.yml";
const APPROVED_RECOVERY_WORKFLOW: &str = ".github/workflows/v4-metadata-recovery.yml";
const PROTECTED_UPDATER_SECRETS: &[&str] = &[
    "V4_TAURI_UPDATER_PRIVATE_KEY",
    "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD",
];
const PROTECTED_METADATA_SECRET: &str = "V4_RELEASE_METADATA_APP_PRIVATE_KEY";

pub(super) fn v4_release_contract_ci_wiring(source: &str) -> std::result::Result<(), String> {
    let jobs = workflow_job_blocks(source);
    let static_job = jobs
        .iter()
        .find(|(id, _)| id == "static")
        .map(|(_, block)| block)
        .ok_or_else(|| "CI is missing the static job block".to_owned())?;
    let release = jobs
        .iter()
        .find(|(id, _)| id == "release_contract")
        .map(|(_, block)| block)
        .ok_or_else(|| "CI is missing the release_contract job block".to_owned())?;
    let status = jobs
        .iter()
        .find(|(id, _)| id == "status")
        .map(|(_, block)| block)
        .ok_or_else(|| "CI is missing the status job block".to_owned())?;
    const FULL_STATIC_CHECK: &str = "cargo xtask check static --skip-supply-chain";
    if static_job.matches(FULL_STATIC_CHECK).count() != 1 || release.contains(FULL_STATIC_CHECK) {
        return Err("the independent static check must run exactly once".to_owned());
    }
    for marker in [
        "actions/setup-go@44694675825211faa026b3c33043df3e48a5fa00",
        "go-version: 1.25.0",
        "go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12",
        "actionlint\" -color",
    ] {
        if !static_job.contains(marker) {
            return Err(format!(
                "CI static job is missing pinned workflow validation `{marker}`"
            ));
        }
    }
    if !release.lines().any(|line| line.trim() == "needs: changes")
        || release
            .lines()
            .any(|line| line.trim().starts_with("needs:") && line.contains("static"))
    {
        return Err("release_contract must remain parallel with static".to_owned());
    }
    let windows_owner_tests = workflow_step_blocks(release)
        .into_iter()
        .find(|(name, _)| name == "Run Rust NSIS and registry owner regression tests")
        .map(|(_, block)| block)
        .ok_or_else(|| {
            "release_contract is missing Rust NSIS and registry owner regression tests".to_owned()
        })?;
    for marker in [
        "cargo test --locked --manifest-path rust/xtask/Cargo.toml || {",
        "cargo test --locked --manifest-path rust/Cargo.toml -p sky_windows_security || {",
        "Rust NSIS/updater harness tests failed with exit code ${exit_code}",
        "Rust Windows registry/process ownership tests failed with exit code ${exit_code}",
    ] {
        if !windows_owner_tests.contains(marker) {
            return Err(format!(
                "Windows Rust owner test step is missing Bash failure handling `{marker}`"
            ));
        }
    }
    if windows_owner_tests.matches("exit_code=$?").count() != 2
        || windows_owner_tests.matches("exit \"$exit_code\"").count() != 2
        || windows_owner_tests.contains("$LASTEXITCODE")
        || !release.lines().any(|line| line.trim() == "shell: bash")
    {
        return Err(
            "Windows Rust owner test step must use the release job's Bash shell and status semantics"
                .to_owned(),
        );
    }
    let static_result_marker = ["STATIC_RESULT: $", "{{ needs.static.result }}"].concat();
    let release_result_marker = [
        "RELEASE_CONTRACT_RESULT: $",
        "{{ needs.release_contract.result }}",
    ]
    .concat();
    let linux_result_marker = [
        "LINUX_PROCESS_RESULT: $",
        "{{ needs.linux_process_runtime.result }}",
    ]
    .concat();
    for marker in [
        "cargo xtask release-v4 self-test",
        "cargo test --locked --manifest-path rust/xtask/Cargo.toml release_v4::tests",
        static_result_marker.as_str(),
        release_result_marker.as_str(),
        linux_result_marker.as_str(),
        r#"if [[ "$STATIC_REQUIRED" == "true" ]]; then"#,
        r#"[[ "$STATIC_RESULT" == "success" ]]"#,
        r#"if [[ "$RELEASE_REQUIRED" == "true" ]]; then"#,
        r#"[[ "$RELEASE_CONTRACT_RESULT" == "success" ]]"#,
        r#"[[ "$LINUX_PROCESS_RESULT" == "success" ]]"#,
        r#"[[ "$LINUX_PROCESS_RESULT" == "skipped" ]]"#,
    ] {
        if !source.contains(marker) {
            return Err(format!(
                "CI is missing Rust release regression or status marker: {marker}"
            ));
        }
    }
    let marker = "needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site, linux_process_runtime]";
    if !status.contains(marker) {
        return Err(format!(
            "status gate is missing independent assertion: {marker}"
        ));
    }
    if !status.lines().any(|line| line.trim() == "if: always()") {
        return Err(
            "required status gate must use always() to inspect failed or skipped dependencies"
                .into(),
        );
    }
    let release_gate = status
        .split_once(r#"if [[ "$RELEASE_REQUIRED" == "true" ]]; then"#)
        .map(|(_, body)| body)
        .ok_or("required status gate is missing release_required branching")?;
    let (required_release, skipped_release) = release_gate
        .split_once("else")
        .ok_or("required status gate is missing the release_required=false branch")?;
    let skipped_release = skipped_release
        .split_once("fi")
        .map(|(branch, _)| branch)
        .ok_or("required status gate has an unterminated release_required branch")?;
    if !required_release.contains(r#"[[ "$LINUX_PROCESS_RESULT" == "success" ]]"#)
        || !skipped_release.contains(r#"[[ "$LINUX_PROCESS_RESULT" == "skipped" ]]"#)
    {
        return Err(
            "required status gate must require Linux runtime success for release changes and skipped otherwise".into(),
        );
    }
    for forbidden in [
        "GH_TOKEN:",
        "scripts/test_v4_release_pipeline.ps1",
        "scripts/test_v4_post_publication_transaction.ps1",
        "scripts/test_ci_v4_release_latest_guard.ps1",
        "scripts/ci_v4_release_latest_guard.ps1",
        "https://api.github.com/repos/pumni/Sky-Auto-Player/releases",
    ] {
        if release.contains(forbidden) {
            return Err(format!(
                "release_contract must not call production release APIs or retired PowerShell tests: {forbidden}"
            ));
        }
    }
    Ok(())
}
pub(super) fn validate_metadata_app_token_scope(workflow: &str) -> Result<()> {
    const PROBE_MINT_STEP: &str = "Mint write-capable metadata App token before publication";
    const PROMOTE_MINT_STEP: &str = "Mint contents-scoped metadata App token";
    const PROBE_STEP: &str = "Probe release-metadata App write capability without mutation";
    const PROMOTE_STEP: &str = "Promote channel metadata after immutable release publication";
    const APP_TOKEN_OUTPUT: &str = "steps.metadata-app-token.outputs.token";
    const APP_PRIVATE_KEY: &str = "secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY";

    let steps = workflow_step_blocks(workflow);
    let probe_mint = steps
        .iter()
        .find(|(name, _)| name == PROBE_MINT_STEP)
        .map(|(_, block)| block.as_str())
        .ok_or("v4 release workflow is missing its pre-publication write-scoped App token mint")?;
    let promote_mint = steps
        .iter()
        .find(|(name, _)| name == PROMOTE_MINT_STEP)
        .map(|(_, block)| block.as_str())
        .ok_or(
            "v4 release workflow is missing the post-publication write-scoped App token mint step",
        )?;
    let promote = steps
        .iter()
        .find(|(name, _)| name == PROMOTE_STEP)
        .map(|(_, block)| block.as_str())
        .ok_or("v4 release workflow is missing the metadata promotion step")?;
    let probe = steps
        .iter()
        .find(|(name, _)| name == PROBE_STEP)
        .map(|(_, block)| block.as_str())
        .ok_or("v4 release workflow is missing the metadata App access probe")?;

    if workflow.contains("app-id:") {
        return Err("canonical release workflow must not use deprecated app-id".into());
    }
    if workflow.contains("V4_RELEASE_METADATA_APP_ID") {
        return Err(
            "canonical release workflow must not use the deprecated App ID variable".into(),
        );
    }

    let common_markers = [
        "id: metadata-app-token",
        "uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510",
        "client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}",
        "private-key: ${{ secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY }}",
        "owner: ${{ github.repository_owner }}",
        "repositories: ${{ github.event.repository.name }}",
    ];
    for (mint, permission) in [
        (probe_mint, "permission-contents: write"),
        (promote_mint, "permission-contents: write"),
    ] {
        for marker in common_markers.into_iter().chain([permission]) {
            if !mint.contains(marker) {
                return Err(format!(
                    "metadata App token mint step is missing required marker: {marker}"
                )
                .into());
            }
        }
        if mint.contains("\n        run:") || mint.contains("\n        env:") {
            return Err(format!(
                "metadata App private key must be consumed only by the token-mint action: {mint}"
            )
            .into());
        }
    }
    if workflow.matches(APP_PRIVATE_KEY).count() != 2 {
        return Err(
            "metadata App private key must occur only in the probe and promotion token-mint actions".into(),
        );
    }
    if workflow.matches(APP_TOKEN_OUTPUT).count() != 2 {
        return Err(
            "metadata App installation token must be used by exactly the probe and promotion steps"
                .into(),
        );
    }
    if !promote.contains(APP_TOKEN_OUTPUT) || !probe.contains(APP_TOKEN_OUTPUT) {
        return Err(
            "metadata App token must be consumed by the read-only capability probe and PromoteMetadata".into(),
        );
    }
    if probe.contains("GH_TOKEN: ${{ github.token }}") {
        return Err("metadata App access probe must not use the repository GITHUB_TOKEN".into());
    }
    if !probe.contains("cargo xtask release-v4 probe-metadata-token")
        || !probe.contains("V4_RELEASE_METADATA_TOKEN_REQUESTED_CONTENTS_PERMISSION: write")
        || probe.contains("probe_v4_metadata_app_access.ps1")
    {
        return Err("metadata App access probe must use the Rust read-only token probe".into());
    }
    if promote.contains("GH_TOKEN: ${{ github.token }}") {
        return Err("PromoteMetadata must not use the repository GITHUB_TOKEN".into());
    }

    for (name, block) in &steps {
        if name != PROBE_MINT_STEP && name != PROMOTE_MINT_STEP && block.contains(APP_PRIVATE_KEY) {
            return Err(
                format!("metadata App private key escaped the token-mint action: {name}").into(),
            );
        }
        if name != PROBE_STEP && name != PROMOTE_STEP && block.contains(APP_TOKEN_OUTPUT) {
            return Err(
                format!("metadata App token was used outside PromoteMetadata: {name}").into(),
            );
        }
    }

    for normal_step in [
        "Run Rust Preflight against current public release state",
        "Publish the exact qualified candidate immutably",
        "Re-fetch and verify immutable assets, stable endpoint, and metadata",
    ] {
        let block = steps
            .iter()
            .find(|(name, _)| name == normal_step)
            .map(|(_, block)| block.as_str())
            .ok_or_else(|| format!("v4 release workflow is missing release step: {normal_step}"))?;
        if !block.contains("GH_TOKEN: ${{ github.token }}") {
            return Err(format!(
                "normal release step must retain the repository GITHUB_TOKEN: {normal_step}"
            )
            .into());
        }
    }

    let probe_job = workflow_job_blocks(workflow)
        .into_iter()
        .find(|(id, _)| id == "metadata_probe")
        .map(|(_, block)| block)
        .ok_or("metadata read-only probe job is missing")?;
    let publish_job = workflow_job_blocks(workflow)
        .into_iter()
        .find(|(id, _)| id == "publish")
        .map(|(_, block)| block)
        .ok_or("immutable release publication job is missing")?;
    if !probe_job.contains("needs: [prepare, attest]")
        || !publish_job.contains("needs: [prepare, attest, metadata_probe]")
        || !probe_job
            .lines()
            .any(|line| line.trim() == "contents: read")
    {
        return Err(
            "metadata App write capability must be probed with read-only Actions permissions before publication".into(),
        );
    }

    Ok(())
}

pub(super) fn validate_release_runner_workflow(relative: &str, source: &str) -> Result<()> {
    if relative == APPROVED_RECOVERY_WORKFLOW {
        return validate_metadata_recovery_workflow(source);
    }
    if relative != APPROVED_RELEASE_WORKFLOW {
        if source.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("runs-on:") && line.contains("self-hosted")
        }) {
            return Err(format!(
                "untrusted workflow must not target a self-hosted runner: {relative}"
            )
            .into());
        }
        if PROTECTED_UPDATER_SECRETS
            .iter()
            .any(|secret| source.contains(secret))
            || source.contains(PROTECTED_METADATA_SECRET)
        {
            return Err(format!(
                "protected release credentials escaped the canonical workflow: {relative}"
            )
            .into());
        }
        return Ok(());
    }
    if !workflow_declares_only_dispatch(source) {
        return Err(format!(
            "production release workflow must whitelist workflow_dispatch as its only trigger: {relative}"
        )
        .into());
    }
    if source.contains("self-hosted")
        || source.contains("v4-release, single-tenant")
        || source.contains("V4_UPDATER_PRIVATE_KEY_PATH")
        || source.contains("rehearse-v4")
        || !source.contains("test \"$GITHUB_REF\" = \"refs/heads/main\"")
        || !source
            .contains("EXPECTED_DEFAULT_BRANCH: ${{ github.event.repository.default_branch }}")
    {
        return Err("canonical release workflow contains a retired runner path or is missing the main-only dispatch guard".into());
    }
    let jobs = workflow_job_blocks(source);
    let signer = jobs
        .iter()
        .find(|(name, _)| name == "sign_qualify")
        .map(|(_, body)| body.as_str())
        .ok_or("canonical release workflow is missing the protected signing job")?;
    validate_precompiled_signer(signer)?;
    if !signer.contains("runs-on: windows-latest")
        || !signer.contains("environment: v4-production-release")
        || !signer.contains("release-v4 sign-candidate")
        || signer.contains("contents: write")
        || signer.contains("id-token: write")
        || signer.contains("attestations: write")
    {
        return Err("production signing job must use hosted Windows, the protected environment, and read-only permissions".into());
    }
    for secret in PROTECTED_UPDATER_SECRETS {
        let reference = format!("${{{{ secrets.{secret} }}}}");
        if signer.matches(&reference).count() != 1 || source.matches(&reference).count() != 1 {
            return Err(format!(
                "protected updater secret must be mapped once and only into sign_qualify: {secret}"
            )
            .into());
        }
    }
    if !source.contains("name: V4 Release Pipeline")
        || !source.contains("contents: read")
        || source.contains("pull_request:")
        || source.contains("pull_request_target:")
    {
        return Err("canonical production workflow must remain dispatch-only with read-only default permissions".into());
    }
    Ok(())
}

fn validate_metadata_recovery_workflow(source: &str) -> Result<()> {
    const APP_PRIVATE_KEY: &str = "secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY";
    const APP_TOKEN: &str = "steps.metadata-app-token.outputs.token";
    const APP_WRITE_ENV: &str =
        "V4_METADATA_WRITE_TOKEN: ${{ steps.metadata-app-token.outputs.token }}";
    const PLAN_UPLOAD: &str =
        "actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2";
    const PLAN_DOWNLOAD: &str =
        "actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2";
    if !workflow_declares_only_dispatch(source)
        || source.contains("self-hosted")
        || source.contains("V4_TAURI_UPDATER_PRIVATE_KEY")
        || source.contains("V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD")
        || !source.contains("test \"$REF\" = \"refs/heads/main\"")
        || !source.contains("environment: v4-production-release")
        || !source.contains("--confirm-metadata-only-recovery")
        || !source.contains("--plan-sha256")
        || source.contains("gh release create")
        || source.contains("git tag")
        || source.contains("gh release upload")
    {
        return Err(
            "metadata recovery workflow must be manual, main-only, protected, and metadata-only"
                .into(),
        );
    }
    if source.matches(APP_PRIVATE_KEY).count() != 1
        || source.matches(APP_TOKEN).count() != 1
        || !source
            .contains("actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510")
        || !source.contains("permission-contents: write")
    {
        return Err(
            "metadata recovery must mint exactly one contents-scoped App token for apply".into(),
        );
    }
    let jobs = workflow_job_blocks(source);
    let observed = jobs.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>();
    if observed != ["validate", "plan", "apply"] {
        return Err(format!(
            "metadata recovery workflow has an unexpected job graph: {observed:?}"
        )
        .into());
    }
    let job = |id: &str| {
        jobs.iter()
            .find(|(job_id, _)| job_id == id)
            .map(|(_, body)| body.as_str())
            .ok_or_else(|| format!("metadata recovery workflow is missing job {id}"))
    };
    let plan = job("plan")?;
    let apply = job("apply")?;
    if !plan.contains("if: inputs.operation == 'plan'")
        || !plan.contains("actions: read")
        || !plan.contains("contents: read")
        || !plan.contains("attestations: read")
        || plan.contains(APP_PRIVATE_KEY)
        || plan.contains(APP_TOKEN)
        || !plan.contains("metadata-recovery-plan")
    {
        return Err("metadata recovery plan must remain read-only and contain no App token".into());
    }
    if !apply.contains("if: inputs.operation == 'apply' && inputs.confirm")
        || !apply.contains("environment: v4-production-release")
        || !apply.contains("group: v4-release-control-plane")
        || !apply.contains("cancel-in-progress: false")
        || !apply.contains("actions: read")
        || !apply.contains("attestations: read")
        || !apply.contains("contents: read")
        || apply.lines().any(|line| line.trim() == "contents: write")
    {
        return Err("metadata recovery apply must require explicit confirmation and protected read-only Actions permissions".into());
    }
    let steps = workflow_step_blocks(apply);
    let step_position = |name: &str| {
        steps
            .iter()
            .position(|(step_name, _)| step_name == name)
            .ok_or_else(|| format!("metadata recovery is missing step: {name}"))
    };
    let download = step_position("Download operator-reviewed plan artifact")?;
    let plan_steps = workflow_step_blocks(plan);
    let upload = plan_steps
        .iter()
        .find(|(name, _)| name == "Upload reviewed plan artifact")
        .map(|(_, body)| body.as_str())
        .ok_or("metadata recovery plan artifact upload is missing")?;
    let bind = step_position("Bind downloaded plan artifact to operator-reviewed hash")?;
    let build = steps
        .iter()
        .position(|(name, _)| name == "Build Rust metadata recovery control before App token")
        .ok_or(
            "metadata recovery apply must build its Rust executable before minting the App token",
        )?;
    let verify_control =
        step_position("Verify prebuilt recovery executable before App token mint")?;
    let mint = step_position("Mint contents-scoped metadata recovery App token")?;
    let execute = steps
        .iter()
        .find(|(name, _)| {
            name == "Revalidate and apply only stable metadata with prebuilt Rust control"
        })
        .map(|(_, body)| body.as_str())
        .ok_or("metadata recovery executable invocation is missing")?;
    if download >= bind
        || bind >= mint
        || build >= verify_control
        || verify_control >= mint
        || !upload.contains(PLAN_UPLOAD)
        || !upload.contains("name: v4-metadata-recovery-plan")
        || !steps[download].1.contains(PLAN_DOWNLOAD)
        || !steps[download]
            .1
            .contains("name: v4-metadata-recovery-plan")
        || !steps[download]
            .1
            .contains("repository: ${{ github.repository }}")
        || !steps[download]
            .1
            .contains("github-token: ${{ github.token }}")
        || !steps[download]
            .1
            .contains("run-id: ${{ inputs.plan_run_id }}")
        || !steps[bind].1.contains("sha256sum")
        || !steps[bind].1.contains("PLAN_SHA256")
        || !steps[bind]
            .1
            .contains("test \"$actual\" = \"${PLAN_SHA256,,}\"")
        || !steps[bind].1.contains(".tag")
        || !steps[bind].1.contains("jq -r .plan_run.artifact_name")
        || !steps[bind].1.contains("jq -r .plan_run.run_id")
        || !steps[bind].1.contains(".action")
        || !steps[bind].1.contains("update_stable")
        || !steps[build]
            .1
            .contains("cargo build --locked --manifest-path rust/xtask/Cargo.toml --release")
        || !steps[verify_control].1.contains("sha256sum")
        || !steps[verify_control].1.contains("source_sha=")
        || !steps[verify_control].1.contains("control_sha=")
        || !execute.contains("rust/target/release/xtask")
        || !execute.contains("sha256sum")
        || !execute.contains(
            "--plan \"$RUNNER_TEMP/reviewed-metadata-plan/v4-metadata-recovery-plan.json\"",
        )
        || !execute.contains("--plan-run-id \"$PLAN_RUN_ID\"")
        || !execute.contains("--plan-sha256 \"$PLAN_SHA256\"")
        || !execute.contains("GH_TOKEN: ${{ github.token }}")
        || !execute.contains(APP_WRITE_ENV)
        || execute.contains("GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}")
        || source.contains("GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}")
        || execute.to_ascii_lowercase().contains("cargo")
        || !execute.contains(APP_TOKEN)
        || !execute.contains("metadata-recovery-apply")
    {
        return Err("metadata recovery must verify and invoke a prebuilt executable without Cargo after App token mint".into());
    }
    for (name, body) in &steps {
        if name != "Mint contents-scoped metadata recovery App token"
            && body.contains(APP_PRIVATE_KEY)
        {
            return Err(
                format!("metadata recovery App private key escaped token mint: {name}").into(),
            );
        }
        if name != "Revalidate and apply only stable metadata with prebuilt Rust control"
            && body.contains(APP_TOKEN)
        {
            return Err(format!(
                "metadata recovery App token escaped the Rust apply process: {name}"
            )
            .into());
        }
    }
    Ok(())
}

fn validate_recovery_release_job_contract(source: &str) -> Result<()> {
    const EXPECTED: [(&str, &str, &str); 8] = [
        (
            "prepare",
            "Validate exact main source and release readiness",
            "",
        ),
        (
            "build_unsigned",
            "Build one unsigned Windows NSIS installer",
            "needs: prepare",
        ),
        (
            "sign_qualify",
            "Protected production signing and exact-byte qualification",
            "needs: [prepare, build_unsigned]",
        ),
        (
            "attest",
            "Verify qualified handoff and publish OIDC attestations",
            "needs: [prepare, sign_qualify]",
        ),
        (
            "metadata_probe",
            "Verify metadata App access before publication",
            "needs: [prepare, attest]",
        ),
        (
            "publish",
            "Verify attestations and publish immutable release assets",
            "needs: [prepare, attest, metadata_probe]",
        ),
        (
            "metadata_promote",
            "Promote only the qualified release channel metadata",
            "needs: [prepare, publish]",
        ),
        (
            "final_verify",
            "Verify public release assets and updater metadata",
            "needs: [prepare, metadata_promote]",
        ),
    ];
    let jobs = workflow_job_blocks(source);
    if jobs.len() != EXPECTED.len() {
        return Err(
            "release workflow job graph differs from the recovery eligibility contract".into(),
        );
    }
    for (id, name, needs) in EXPECTED {
        let body = jobs
            .iter()
            .find(|(job_id, _)| job_id == id)
            .map(|(_, body)| body.as_str())
            .ok_or_else(|| format!("release workflow is missing recovery-bound job {id}"))?;
        if !body
            .lines()
            .any(|line| line.trim() == format!("name: {name}"))
            || (!needs.is_empty() && !body.lines().any(|line| line.trim() == needs))
        {
            return Err(format!(
                "release workflow job {id} differs from recovery's exact name/dependency contract"
            )
            .into());
        }
    }
    Ok(())
}

fn validate_precompiled_signer(signer: &str) -> Result<()> {
    const BUILD_STEP: &str = "Build the pinned Rust release control executable before secrets";
    const VERIFY_STEP: &str = "Verify prebuilt control executable identity before secrets";
    const SIGN_STEP: &str = "Sign the independently verified exact candidate";

    let steps = workflow_step_blocks(signer);
    let block = |name: &str| {
        steps
            .iter()
            .find(|(step_name, _)| step_name == name)
            .map(|(_, body)| body.as_str())
            .ok_or_else(|| format!("protected signer is missing required step: {name}"))
    };
    let position = |name: &str| {
        steps
            .iter()
            .position(|(step_name, _)| step_name == name)
            .ok_or_else(|| format!("protected signer is missing required step order: {name}"))
    };
    let build = block(BUILD_STEP)?;
    let verify = block(VERIFY_STEP)?;
    let sign = block(SIGN_STEP)?;
    if position(BUILD_STEP)? >= position(VERIFY_STEP)?
        || position(VERIFY_STEP)? >= position(SIGN_STEP)?
        || !build.contains("cargo build --locked --manifest-path rust/xtask/Cargo.toml --release")
        || !build.contains("Get-FileHash -LiteralPath $controlPath -Algorithm SHA256")
        || !build.contains("source_sha=$sourceSha")
        || !build.contains("sha256=$controlSha")
        || build.contains(PROTECTED_UPDATER_SECRETS[0])
        || build.contains(PROTECTED_UPDATER_SECRETS[1])
        || !verify.contains("EXPECTED_RELEASE_CONTROL_SOURCE_SHA")
        || !verify.contains("EXPECTED_RELEASE_CONTROL_SHA256")
        || !verify.contains("Get-FileHash -LiteralPath $controlPath -Algorithm SHA256")
        || !verify.contains("GITHUB_SHA")
        || verify.contains(PROTECTED_UPDATER_SECRETS[0])
        || verify.contains(PROTECTED_UPDATER_SECRETS[1])
        || !sign.contains("rust/target/release/xtask.exe")
        || !sign.contains("Get-FileHash -LiteralPath $controlPath -Algorithm SHA256")
        || sign.to_ascii_lowercase().contains("cargo")
    {
        return Err("protected signing must verify a precompiled, source-bound Rust executable and invoke it without Cargo while secrets are mapped".into());
    }
    Ok(())
}

pub(super) fn release_runner_contract(root: &Path) -> Result<()> {
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    v4_release_contract_ci_wiring(&ci)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    let release = fs::read_to_string(root.join(".github/workflows/release-v4.yml"))?;
    validate_metadata_app_token_scope(&release)?;
    validate_recovery_release_job_contract(&release)?;
    let recovery = fs::read_to_string(root.join(APPROVED_RECOVERY_WORKFLOW))?;
    validate_metadata_recovery_workflow(&recovery)?;

    let workflows_root = root.join(".github/workflows");
    let mut observed = BTreeSet::new();
    for entry in WalkDir::new(&workflows_root) {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry.file_type().is_file()
            || !matches!(
                entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str()),
                Some("yml" | "yaml")
            )
        {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(entry.path())?;
        validate_release_runner_workflow(&relative, &source)?;
        if matches!(
            relative.as_str(),
            APPROVED_RELEASE_WORKFLOW | APPROVED_RECOVERY_WORKFLOW
        ) {
            observed.insert(relative);
        }
    }
    for required in [APPROVED_RELEASE_WORKFLOW, APPROVED_RECOVERY_WORKFLOW] {
        if !observed.contains(required) {
            return Err(
                format!("required v4 release authority workflow is missing: {required}").into(),
            );
        }
    }
    release_runner_documentation_contract(root)?;
    println!("[xtask] v4 hosted release trust boundary: PASS");
    Ok(())
}

fn release_runner_documentation_contract(root: &Path) -> Result<()> {
    let documentation = fs::read_to_string(root.join("docs/v4-release-execution-topology.md"))?;
    release_runner_documentation_contract_source(&documentation)
}

fn release_runner_documentation_contract_source(documentation: &str) -> Result<()> {
    let section_start = documentation
        .find("### 2.4 Hosted Release Workflow and Operator Requirements")
        .ok_or("hosted release documentation is missing section 2.4")?;
    let section = &documentation[section_start..];
    let section = section
        .find("\n### ")
        .map_or(section, |end| &section[..end]);

    let mut documented = BTreeSet::new();
    for (index, code_segment) in section.split('`').enumerate() {
        if index % 2 == 1 && code_segment.starts_with(".github/workflows/") {
            documented.insert(code_segment.to_owned());
        }
    }
    let expected = [APPROVED_RELEASE_WORKFLOW.to_owned()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    if documented != expected {
        return Err(format!(
            "documented production workflow list does not match the enforced workflow: documented={documented:?}, enforced={expected:?}"
        )
        .into());
    }
    let recovery_marker = format!("`{APPROVED_RECOVERY_WORKFLOW}`");
    if !documentation.contains("Metadata Only Recovery")
        || !documentation.contains(&recovery_marker)
    {
        return Err(
            "metadata recovery workflow must appear once in the operator documentation".into(),
        );
    }
    let workflow = APPROVED_RELEASE_WORKFLOW;
    let marker = format!("`{workflow}`");
    if section.matches(&marker).count() != 1 {
        return Err(format!(
            "documented production workflow must appear exactly once in section 2.4: {workflow}"
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_workflow() -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        fs::read_to_string(root.join(APPROVED_RELEASE_WORKFLOW))
            .expect("production workflow must exist")
    }

    fn normalize_fixture(text: &str) -> String {
        text.replace("\r\n", "\n").replace('\r', "\n")
    }

    fn replace_all_fixture(text: &str, expected: &str, replacement: &str) -> String {
        let original = normalize_fixture(text);
        assert!(
            original.contains(expected),
            "negative test mutation target must exist: {expected}"
        );
        let mutated = original.replace(expected, replacement);
        assert_ne!(
            mutated, original,
            "negative test mutation must change fixture"
        );
        mutated
    }

    fn replace_fixture_once(text: &str, expected: &str, replacement: &str) -> String {
        let original = normalize_fixture(text);
        assert!(
            original.contains(expected),
            "negative test mutation target must exist: {expected}"
        );
        let mutated = original.replacen(expected, replacement, 1);
        assert_ne!(
            mutated, original,
            "negative test mutation must change fixture"
        );
        mutated
    }

    fn mixed_line_ending_fixture(text: &str) -> String {
        let original = normalize_fixture(text);
        let mut result = String::with_capacity(original.len());
        let mut next_crlf = true;
        for line in original.split_inclusive('\n') {
            if let Some(content) = line.strip_suffix('\n') {
                result.push_str(content);
                result.push_str(if next_crlf { "\r\n" } else { "\n" });
                next_crlf = !next_crlf;
            } else {
                result.push_str(line);
            }
        }
        result
    }

    #[test]
    fn hosted_release_contract_rejects_untrusted_workflows_and_production_secrets() {
        let self_hosted =
            "on:\n  pull_request:\njobs:\n  build:\n    runs-on: [self-hosted, windows]\n";
        assert!(validate_release_runner_workflow(".github/workflows/ci.yml", self_hosted).is_err());
        let secret = "env:\n  KEY: ${{ secrets.V4_TAURI_UPDATER_PRIVATE_KEY }}\n";
        assert!(validate_release_runner_workflow(".github/workflows/ci.yml", secret).is_err());
        let app_secret = "env:\n  KEY: ${{ secrets.V4_RELEASE_METADATA_APP_PRIVATE_KEY }}\n";
        assert!(validate_release_runner_workflow(".github/workflows/ci.yml", app_secret).is_err());
    }

    #[test]
    fn hosted_release_contract_requires_dispatch_main_and_narrow_signer() {
        let workflow = current_workflow();
        validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &workflow)
            .expect("current workflow must satisfy the hosted release boundary");

        let pull_request = replace_fixture_once(
            &workflow,
            "  workflow_dispatch:\n",
            "  workflow_dispatch:\n  pull_request:\n",
        );
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &pull_request).is_err()
        );

        let unprotected =
            replace_fixture_once(&workflow, "    environment: v4-production-release\n", "");
        assert!(validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &unprotected).is_err());

        let broad_runner = replace_all_fixture(
            &workflow,
            "runs-on: windows-latest",
            "runs-on: [self-hosted, windows]",
        );
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &broad_runner).is_err()
        );

        let missing_secret = replace_all_fixture(
            &workflow,
            "          V4_TAURI_UPDATER_PRIVATE_KEY: ${{ secrets.V4_TAURI_UPDATER_PRIVATE_KEY }}\n",
            "",
        );
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &missing_secret).is_err()
        );

        let cargo_with_secrets = replace_all_fixture(
            &workflow,
            "& $controlPath release-v4 sign-candidate",
            "cargo xtask release-v4 sign-candidate",
        );
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &cargo_with_secrets)
                .is_err()
        );

        let missing_control_build = replace_all_fixture(
            &workflow,
            "- name: Build the pinned Rust release control executable before secrets",
            "- name: Omitted Rust release control build",
        );
        assert!(
            validate_release_runner_workflow(APPROVED_RELEASE_WORKFLOW, &missing_control_build)
                .is_err()
        );
    }

    #[test]
    fn metadata_recovery_contract_rejects_secret_or_cargo_after_token_mint() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join(APPROVED_RECOVERY_WORKFLOW)).unwrap();
        validate_metadata_recovery_workflow(&source).unwrap();
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "group: v4-release-control-plane",
                "group: v4-recovery-apply-only"
            ))
            .is_err()
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2",
                "actions/download-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2"
            ))
            .is_err(),
            "upload-artifact SHA must never identify the downloader"
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "run-id: ${{ inputs.plan_run_id }}",
                "run-id: ${{ github.run_id }}"
            ))
            .is_err(),
            "plan artifact download must bind the operator-selected run"
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "jq -r .plan_run.run_id",
                "jq -r .plan_run.wrong_run_id"
            ))
            .is_err(),
            "plan artifact content must bind the requested run ID"
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "      actions: read\n      contents: read\n      attestations: read\n",
                "      contents: read\n      attestations: read\n"
            ))
            .is_err(),
            "plan job must declare actions:read after narrowing job permissions"
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "GH_TOKEN: ${{ github.token }}",
                "GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}"
            ))
            .is_err(),
            "read verification must not select the metadata App token"
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "V4_METADATA_WRITE_TOKEN: ${{ steps.metadata-app-token.outputs.token }}",
                "GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}"
            ))
            .is_err(),
            "the write token must use its separate credential channel"
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "\"$control\" release-v4 metadata-recovery-apply",
                "cargo xtask release-v4 metadata-recovery-apply"
            ))
            .is_err()
        );
        assert!(validate_metadata_recovery_workflow(&replace_all_fixture(
            &source,
            "      - name: Mint contents-scoped metadata recovery App token",
            "      - name: Mint contents-scoped metadata recovery App token\n        env:\n          LEAK: ${{ secrets.V4_TAURI_UPDATER_PRIVATE_KEY }}"
        ))
        .is_err());
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "--confirm-metadata-only-recovery",
                "--publish-release"
            ))
            .is_err()
        );
        assert!(
            validate_metadata_recovery_workflow(&replace_all_fixture(
                &source,
                "test \"$actual\" = \"${PLAN_SHA256,,}\"",
                "test -n \"$actual\""
            ))
            .is_err()
        );
    }

    #[test]
    fn metadata_recovery_actions_mutation_is_detected_from_mixed_eol_checkout() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join(APPROVED_RECOVERY_WORKFLOW)).unwrap();
        let mixed = mixed_line_ending_fixture(&source);
        let missing_plan_actions = replace_fixture_once(&mixed, "      actions: read\n", "");
        assert!(validate_metadata_recovery_workflow(&missing_plan_actions).is_err());
    }

    #[test]
    fn recovery_eligibility_tracks_the_exact_release_job_graph() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join(APPROVED_RELEASE_WORKFLOW)).unwrap();
        validate_recovery_release_job_contract(&source).unwrap();
        let wrong_publish_name = replace_all_fixture(
            &source,
            "name: Verify attestations and publish immutable release assets",
            "name: Publish possibly mutable release assets",
        );
        assert!(validate_recovery_release_job_contract(&wrong_publish_name).is_err());
        let wrong_publish_boundary = replace_all_fixture(
            &source,
            "needs: [prepare, attest, metadata_probe]",
            "needs: [prepare, sign_qualify]",
        );
        assert!(validate_recovery_release_job_contract(&wrong_publish_boundary).is_err());
    }

    #[test]
    fn hosted_workflow_documentation_matches_the_single_release_path() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let documentation = fs::read_to_string(root.join("docs/v4-release-execution-topology.md"))
            .expect("release topology documentation must exist");
        release_runner_documentation_contract_source(&documentation)
            .expect("documentation must list the canonical production workflow");
        let missing = replace_all_fixture(
            &documentation,
            "`.github/workflows/release-v4.yml`",
            "`production-workflow-omitted.yml`",
        );
        assert!(release_runner_documentation_contract_source(&missing).is_err());
        let mut with_extra = normalize_fixture(&documentation);
        let section_start = with_extra
            .find("### 2.4 Hosted Release Workflow and Operator Requirements")
            .expect("section 2.4 exists");
        let insertion = section_start
            + with_extra[section_start..]
                .find('\n')
                .expect("section newline")
            + 1;
        with_extra.insert_str(
            insertion,
            "- `.github/workflows/unapproved.yml` — not approved.\n",
        );
        assert_ne!(with_extra, normalize_fixture(&documentation));
        assert!(release_runner_documentation_contract_source(&with_extra).is_err());
    }

    #[test]
    fn metadata_app_tokens_are_separate_and_only_reach_probe_or_promotion() {
        let workflow = current_workflow();
        assert!(validate_metadata_app_token_scope(&workflow).is_ok());

        let stale_pin = replace_all_fixture(
            &workflow,
            "uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510",
            "uses: actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1",
        );
        assert!(validate_metadata_app_token_scope(&stale_pin).is_err());

        let missing_client_id = replace_all_fixture(
            &workflow,
            "          client-id: ${{ vars.V4_RELEASE_METADATA_APP_CLIENT_ID }}\n",
            "",
        );
        assert!(validate_metadata_app_token_scope(&missing_client_id).is_err());

        let read_scoped_probe = replace_fixture_once(
            &workflow,
            "permission-contents: write",
            "permission-contents: read",
        );
        assert!(validate_metadata_app_token_scope(&read_scoped_probe).is_err());

        let false_write_evidence = replace_all_fixture(
            &workflow,
            "V4_RELEASE_METADATA_TOKEN_REQUESTED_CONTENTS_PERMISSION: write",
            "V4_RELEASE_METADATA_TOKEN_REQUESTED_CONTENTS_PERMISSION: read",
        );
        assert!(validate_metadata_app_token_scope(&false_write_evidence).is_err());

        let broad_token = replace_all_fixture(
            &workflow,
            "- name: Publish the exact qualified candidate immutably\n        env:\n          GH_TOKEN: ${{ github.token }}",
            "- name: Publish the exact qualified candidate immutably\n        env:\n          GH_TOKEN: ${{ steps.metadata-app-token.outputs.token }}",
        );
        assert!(validate_metadata_app_token_scope(&broad_token).is_err());

        let private_key_in_env = replace_all_fixture(
            &workflow,
            "        with:\n          client-id:",
            "        env:\n          client-id:",
        );
        assert!(validate_metadata_app_token_scope(&private_key_in_env).is_err());
    }

    #[test]
    fn release_contract_ci_wiring_requires_rust_mock_tests_and_independent_static_gate() {
        let source = r#"
jobs:
  static:
    needs: changes
    steps:
      - run: cargo xtask check static --skip-supply-chain
      - uses: actions/setup-go@44694675825211faa026b3c33043df3e48a5fa00
        with:
          go-version: 1.25.0
      - run: go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
      - run: "$GOBIN/actionlint" -color
  release_contract:
    name: V4 release contract acceptance
    needs: changes
    defaults:
      run:
        shell: bash
    steps:
      - name: Rust v4 release transaction mock tests
        run: cargo xtask release-v4 self-test
        run: cargo test --locked --manifest-path rust/xtask/Cargo.toml release_v4::tests
      - name: Run Rust NSIS and registry owner regression tests
        run: |
          cargo test --locked --manifest-path rust/xtask/Cargo.toml || {
            exit_code=$?
            echo "Rust NSIS/updater harness tests failed with exit code ${exit_code}" >&2
            exit "$exit_code"
          }
          cargo test --locked --manifest-path rust/Cargo.toml -p sky_windows_security || {
            exit_code=$?
            echo "Rust Windows registry/process ownership tests failed with exit code ${exit_code}" >&2
            exit "$exit_code"
          }
  status:
    if: always()
    needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site, linux_process_runtime]
    env:
      STATIC_RESULT: ${{ needs.static.result }}
      RELEASE_CONTRACT_RESULT: ${{ needs.release_contract.result }}
      LINUX_PROCESS_RESULT: ${{ needs.linux_process_runtime.result }}
    run: |
      if [[ "$STATIC_REQUIRED" == "true" ]]; then
        [[ "$STATIC_RESULT" == "success" ]]
      fi
      if [[ "$RELEASE_REQUIRED" == "true" ]]; then
        [[ "$RELEASE_CONTRACT_RESULT" == "success" ]]
        [[ "$LINUX_PROCESS_RESULT" == "success" ]]
      else
        [[ "$RELEASE_CONTRACT_RESULT" == "skipped" ]]
        [[ "$LINUX_PROCESS_RESULT" == "skipped" ]]
      fi
"#.to_owned();
        v4_release_contract_ci_wiring(&source)
            .expect("CI must run the pinned actionlint and Rust release contract gates");
        for marker in [
            "cargo xtask release-v4 self-test",
            "cargo test --locked --manifest-path rust/xtask/Cargo.toml release_v4::tests",
        ] {
            assert!(
                v4_release_contract_ci_wiring(&replace_all_fixture(&source, marker, "removed"))
                    .is_err()
            );
        }
        let no_static = replace_all_fixture(
            &source,
            "cargo xtask check static --skip-supply-chain",
            "cargo xtask check static",
        );
        assert!(v4_release_contract_ci_wiring(&no_static).is_err());
        let missing_linux_dependency =
            replace_all_fixture(&source, ", linux_process_runtime]", "]");
        assert!(v4_release_contract_ci_wiring(&missing_linux_dependency).is_err());
        let missing_linux_result = replace_all_fixture(
            &source,
            "      LINUX_PROCESS_RESULT: ${{ needs.linux_process_runtime.result }}\n",
            "",
        );
        assert!(v4_release_contract_ci_wiring(&missing_linux_result).is_err());
        let linux_success_not_required = replace_all_fixture(
            &source,
            "[[ \"$LINUX_PROCESS_RESULT\" == \"success\" ]]",
            "[[ \"$LINUX_PROCESS_RESULT\" == \"skipped\" ]]",
        );
        assert!(v4_release_contract_ci_wiring(&linux_success_not_required).is_err());
        let linux_skip_accepted_when_required = replace_all_fixture(
            &source,
            "[[ \"$LINUX_PROCESS_RESULT\" == \"skipped\" ]]",
            "[[ \"$LINUX_PROCESS_RESULT\" == \"success\" ]]",
        );
        assert!(v4_release_contract_ci_wiring(&linux_skip_accepted_when_required).is_err());
        let status_not_always = replace_all_fixture(&source, "if: always()", "if: true");
        assert!(v4_release_contract_ci_wiring(&status_not_always).is_err());
        let production_api = replace_all_fixture(
            &source,
            "cargo xtask release-v4 self-test",
            "gh api repos/pumni/Sky-Auto-Player/releases",
        );
        assert!(v4_release_contract_ci_wiring(&production_api).is_err());
    }
}
